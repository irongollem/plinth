use crate::error::AppError;
use crate::models::{Release, StlModel};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

use super::{FileRow, ModelRow, IMAGE_EXTENSIONS, MODEL_EXTENSIONS};

pub struct ScanOutcome {
    pub files: Vec<FileRow>,
    pub models: Vec<ModelRow>,
    pub metadata_tags: Vec<(String, String)>,
}

/// Per-directory accumulator built during the walk.
#[derive(Default)]
struct DirInfo {
    model_files: u32,
    model_bytes: i64,
    first_image: Option<String>,
    metadata: Option<StlModel>,
}

/// Info from a release.json, applied to model dirs beneath it.
struct ReleaseInfo {
    name: String,
    designer: Option<String>,
}

/// Walk `root`, indexing every model file and assembling one model entry
/// per directory that directly contains model files. `on_progress` is
/// invoked periodically with (files_seen, current_dir).
pub fn scan(
    root: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u32, &str),
) -> Result<ScanOutcome, AppError> {
    if !root.is_dir() {
        return Err(AppError::NotFoundError(format!(
            "Catalog root '{}' is not a directory",
            root.display()
        )));
    }

    let mut files: Vec<FileRow> = Vec::new();
    // BTreeMap: deterministic order, and ancestor lookups for releases
    let mut dirs: BTreeMap<String, DirInfo> = BTreeMap::new();
    let mut releases: BTreeMap<String, ReleaseInfo> = BTreeMap::new();
    let mut seen: u32 = 0;

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !is_hidden(e.path()))
        .filter_map(|e| e.ok())
    {
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::UserCancelled("Scan cancelled".to_string()));
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(dir_path) = path.parent().map(|p| p.to_string_lossy().into_owned()) else {
            continue;
        };
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if MODEL_EXTENSIONS.contains(&extension.as_str()) {
            let metadata = entry.metadata().map_err(|e| {
                AppError::IoError(format!("Failed to stat {}: {}", path.display(), e))
            })?;
            let size_bytes = metadata.len() as i64;
            let modified_at = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            files.push(FileRow {
                path: path.to_string_lossy().into_owned(),
                dir_path: dir_path.clone(),
                file_name,
                extension,
                size_bytes,
                modified_at,
            });

            let info = dirs.entry(dir_path.clone()).or_default();
            info.model_files += 1;
            info.model_bytes += size_bytes;

            seen += 1;
            if seen.is_multiple_of(250) {
                on_progress(seen, &dir_path);
            }
        } else if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            let info = dirs.entry(dir_path).or_default();
            if info.first_image.is_none() {
                info.first_image = Some(path.to_string_lossy().into_owned());
            }
        } else if file_name == "model.json" {
            if let Ok(parsed) = read_json::<StlModel>(path) {
                dirs.entry(dir_path).or_default().metadata = Some(parsed);
            }
        } else if file_name == "release.json" {
            if let Ok(parsed) = read_json::<Release>(path) {
                releases.insert(
                    dir_path,
                    ReleaseInfo {
                        name: parsed.name,
                        designer: Some(parsed.designer).filter(|d| !d.is_empty()),
                    },
                );
            }
        }
    }
    on_progress(seen, "");

    // Assemble one model per directory that directly holds model files
    let mut models = Vec::new();
    let mut metadata_tags = Vec::new();
    for (dir_path, info) in &dirs {
        if info.model_files == 0 {
            continue;
        }
        let release = nearest_release(&releases, dir_path);
        let (name, description, uuid, source, preview) = match &info.metadata {
            Some(meta) => {
                // model.json image paths are relative to the model dir
                let preview = meta
                    .images
                    .first()
                    .map(|rel| Path::new(dir_path).join(rel))
                    .filter(|p| p.is_file())
                    .map(|p| p.to_string_lossy().into_owned())
                    .or_else(|| info.first_image.clone());
                for tag in &meta.tags {
                    metadata_tags.push((dir_path.clone(), tag.clone()));
                }
                (
                    meta.name.clone(),
                    meta.description.clone(),
                    meta.id.map(|id| id.to_string()),
                    "metadata",
                    preview,
                )
            }
            None => {
                let name = Path::new(dir_path)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir_path.clone());
                (name, None, None, "heuristic", info.first_image.clone())
            }
        };

        models.push(ModelRow {
            dir_path: dir_path.clone(),
            name,
            description,
            designer: release.as_ref().and_then(|r| r.designer.clone()),
            release_name: release.map(|r| r.name.clone()),
            preview_path: preview,
            source: source.to_string(),
            uuid,
            file_count: info.model_files,
            total_size_bytes: info.model_bytes,
        });
    }

    Ok(ScanOutcome {
        files,
        models,
        metadata_tags,
    })
}

fn is_hidden(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, AppError> {
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&text)?)
}

/// The release.json in the closest ancestor directory, if any.
fn nearest_release<'a>(
    releases: &'a BTreeMap<String, ReleaseInfo>,
    dir_path: &str,
) -> Option<&'a ReleaseInfo> {
    let mut current = Some(Path::new(dir_path));
    while let Some(dir) = current {
        if let Some(info) = releases.get(&dir.to_string_lossy().into_owned()) {
            return Some(info);
        }
        current = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scans_metadata_and_heuristic_models() {
        let root = std::env::temp_dir().join(format!("stlpack_scan_test_{}", std::process::id()));
        fs::create_dir_all(root.join("release/newt")).unwrap();
        fs::create_dir_all(root.join("loose")).unwrap();

        fs::write(root.join("release/newt/newt.stl"), b"solid fake").unwrap();
        fs::write(root.join("release/newt/newt-main.png"), b"png").unwrap();
        fs::write(
            root.join("release/newt/model.json"),
            r#"{"id":null,"name":"Giant Newt","description":"big","tags":["amphibian"],"images":["newt-main.png"],"model_files":["newt.stl"],"group":null}"#,
        )
        .unwrap();
        fs::write(
            root.join("release/release.json"),
            r#"{"name":"Critterfolk","designer":"DTL","description":"","date":"06/2026","version":"1","model_references":[],"groups":[],"release_dir":"","images":[],"other_files":[]}"#,
        )
        .unwrap();
        fs::write(root.join("loose/dragon.stl"), b"solid fake dragon").unwrap();
        fs::write(root.join("loose/notes.txt"), b"ignore me").unwrap();

        let cancel = AtomicBool::new(false);
        let outcome = scan(&root, &cancel, |_, _| {}).unwrap();

        assert_eq!(outcome.files.len(), 2, "only model files are indexed");

        let newt = outcome
            .models
            .iter()
            .find(|m| m.name == "Giant Newt")
            .expect("metadata model");
        assert_eq!(newt.source, "metadata");
        assert_eq!(newt.release_name.as_deref(), Some("Critterfolk"));
        assert_eq!(newt.designer.as_deref(), Some("DTL"));
        assert!(newt
            .preview_path
            .as_deref()
            .unwrap()
            .ends_with("newt-main.png"));

        let dragon = outcome
            .models
            .iter()
            .find(|m| m.name == "loose")
            .expect("heuristic model named after its dir");
        assert_eq!(dragon.source, "heuristic");

        assert_eq!(
            outcome.metadata_tags,
            vec![(
                root.join("release/newt").to_string_lossy().into_owned(),
                "amphibian".to_string()
            )]
        );

        fs::remove_dir_all(&root).ok();
    }
}
