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
        let (name, description, uuid, source, preview, inferred) = match &info.metadata {
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
                    None,
                )
            }
            None => {
                let inferred = infer_model_identity(root, dir_path);
                (
                    inferred.name.clone(),
                    None,
                    None,
                    "heuristic",
                    info.first_image.clone(),
                    Some(inferred),
                )
            }
        };

        // metadata models group under their own name (usually 1:1);
        // heuristic models under the inferred base
        let group_name = inferred
            .as_ref()
            .map(|i| i.group_name.clone())
            .unwrap_or_else(|| name.clone());

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
            pose: inferred.as_ref().and_then(|i| i.pose.clone()),
            scale: None,
            support_status: inferred.as_ref().and_then(|i| i.support_status.clone()),
            release_date: inferred.as_ref().and_then(|i| i.release_date.clone()),
            group_name: Some(group_name),
        });
    }

    Ok(ScanOutcome {
        files,
        models,
        metadata_tags,
    })
}

// ---- stacked-folder identity inference (heuristic models only) ----

struct InferredModel {
    name: String,
    /// The base name without the pose suffix — variants of one model share
    /// it, which is what groups "galeb duhr A/B/C" onto one catalog card.
    group_name: String,
    pose: Option<String>,
    support_status: Option<String>,
    release_date: Option<String>,
}

/// Read identity out of "stacked" library structures like
/// `<release-05-2026>/<model>/<unsupported>/<A>`. The dirs that directly
/// hold the files are often packaging variants, and naming models after
/// them yields a catalog full of "A"s and "supported"s. Climb toward the
/// root until a segment says something, and keep what the packaging
/// segments told us as metadata instead of throwing it away.
fn infer_model_identity(root: &Path, dir_path: &str) -> InferredModel {
    let mut pose: Option<String> = None;
    let mut support_status: Option<String> = None;
    let mut release_date: Option<String> = None;
    let mut base_name: Option<String> = None;

    let mut current = Some(Path::new(dir_path));
    while let Some(dir) = current {
        if dir == root {
            break;
        }
        let Some(segment) = dir.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            break;
        };

        if release_date.is_none() {
            release_date = date_from_segment(&segment);
        }
        if let Some(status) = support_from_segment(&segment) {
            support_status.get_or_insert_with(|| status.to_string());
        } else if base_name.is_none() {
            // Pose markers only count BELOW the name: once a real name is
            // found, a short ancestor dir is a collection, not a variant
            if let Some(p) = pose_from_segment(&segment) {
                pose.get_or_insert(p);
            } else if !is_generic_segment(&segment) {
                base_name = Some(prettify_segment(&segment));
            }
        }
        current = dir.parent();
    }

    // nothing but packaging all the way up: keep the old leaf-dir name
    let group_name = base_name.clone().unwrap_or_else(|| {
        prettify_segment(
            &Path::new(dir_path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| dir_path.to_string()),
        )
    });
    let name = match &pose {
        Some(p) if base_name.is_some() => format!("{} {}", group_name, p),
        _ => group_name.clone(),
    };

    InferredModel {
        name,
        group_name,
        pose,
        support_status,
        release_date,
    }
}

/// "galeb_duhr" reads like a filename; "galeb duhr" reads like a name.
/// Underscores are transfer-armor, not identity — swap them for spaces and
/// collapse the leftovers. Hyphens stay: they're often part of the name.
fn prettify_segment(segment: &str) -> String {
    segment
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn support_from_segment(segment: &str) -> Option<&'static str> {
    // presupported means supports are present — same answer as supported
    match segment
        .trim()
        .to_lowercase()
        .replace(['-', '_'], "")
        .as_str()
    {
        "supported" | "presupported" => Some("supported"),
        "unsupported" => Some("unsupported"),
        _ => None,
    }
}

/// "A", "B2", "01", "pose 3" — variant markers, not names.
fn pose_from_segment(segment: &str) -> Option<String> {
    let lower = segment.trim().to_lowercase();
    let candidate = lower
        .strip_prefix("pose")
        .map(|rest| rest.trim_start_matches([' ', '-', '_']))
        .unwrap_or(&lower);
    (!candidate.is_empty()
        && candidate.len() <= 2
        && candidate.chars().all(|c| c.is_ascii_alphanumeric()))
    .then(|| candidate.to_uppercase())
}

/// Container words that describe packaging, not the model.
fn is_generic_segment(segment: &str) -> bool {
    let normalized = segment.trim().to_lowercase();
    matches!(
        normalized.as_str(),
        "stl" | "stls" | "obj" | "files" | "parts" | "lys" | "chitubox"
    ) || support_from_segment(&normalized).is_some()
        || pose_from_segment(&normalized).is_some()
}

/// A MM-YYYY or YYYY-MM digit pair anywhere in the segment, e.g.
/// "dungeon_classics-05-2026" -> "2026-05".
fn date_from_segment(segment: &str) -> Option<String> {
    let tokens: Vec<&str> = segment
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .collect();
    for pair in tokens.windows(2).rev() {
        let (month_token, year_token) = match (pair[0].len(), pair[1].len()) {
            (2, 4) => (pair[0], pair[1]),
            (4, 2) => (pair[1], pair[0]),
            _ => continue,
        };
        if let (Ok(month), Ok(year)) = (month_token.parse::<u32>(), year_token.parse::<u32>()) {
            if (1..=12).contains(&month) && (1900..=2200).contains(&year) {
                return Some(format!("{:04}-{:02}", year, month));
            }
        }
    }
    None
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

    #[test]
    fn infers_identity_from_stacked_folders() {
        let root = Path::new("/lib");

        // the minihoard shape that motivated this:
        // release-with-date / model / support / pose
        let inferred = infer_model_identity(
            root,
            "/lib/dungeon_classics-05-2026/galeb_duhr/unsupported/A",
        );
        assert_eq!(inferred.name, "galeb duhr A", "underscores prettified");
        assert_eq!(inferred.group_name, "galeb duhr");
        assert_eq!(inferred.pose.as_deref(), Some("A"));
        assert_eq!(inferred.support_status.as_deref(), Some("unsupported"));
        assert_eq!(inferred.release_date.as_deref(), Some("2026-05"));

        // presupported counts as supported; "pose 2" is a variant marker
        let inferred = infer_model_identity(root, "/lib/rats/pre-supported/pose 2");
        assert_eq!(inferred.name, "rats 2");
        assert_eq!(inferred.group_name, "rats", "poses share the group");
        assert_eq!(inferred.pose.as_deref(), Some("2"));
        assert_eq!(inferred.support_status.as_deref(), Some("supported"));

        // a plain named dir stays exactly what it was
        let inferred = infer_model_identity(root, "/lib/loose");
        assert_eq!(inferred.name, "loose");
        assert!(inferred.pose.is_none());
        assert!(inferred.support_status.is_none());

        // packaging all the way to the root: fall back to the leaf name
        let inferred = infer_model_identity(root, "/lib/supported");
        assert_eq!(inferred.name, "supported");

        // a short ancestor above the name is a collection, not a pose
        let inferred = infer_model_identity(root, "/lib/xx/minotaur/stls");
        assert_eq!(inferred.name, "minotaur");
        assert!(inferred.pose.is_none());
    }

    #[test]
    fn date_from_segment_handles_both_orders_and_junk() {
        assert_eq!(
            date_from_segment("dungeon_classics-05-2026").as_deref(),
            Some("2026-05")
        );
        assert_eq!(
            date_from_segment("2025-11 heroes").as_deref(),
            Some("2025-11")
        );
        assert_eq!(date_from_segment("warhammer 40k"), None);
        assert_eq!(date_from_segment("release-13-2026"), None, "month 13");
        assert_eq!(date_from_segment("plain name"), None);
    }
}
