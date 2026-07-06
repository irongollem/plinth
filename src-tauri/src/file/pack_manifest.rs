//! Builds the `.3pk` manifest at pack time — after the component archives
//! exist (their checksums are of the final bytes) and before `release.3pk`
//! is zipped (the manifest travels inside it). The metadata comes from the
//! release dir the builder staged: `release.json` for release-level info and
//! each model's `model.json` sidecar. Fields the sidecar doesn't carry yet
//! (pose/scale/support enrichment is a tracked todo) are emitted as null —
//! additive to fill in later, not a format change.

use crate::error::AppError;
use crate::file::compressors::ArchiveFileEntry;
use crate::manifest::{Component, Manifest, ManifestFile, ManifestModel, ManifestRelease};
use crate::models::{Release, StlModel};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

/// "MM/YYYY" (the builder's input form) → canonical "YYYY-MM"; anything
/// already canonical (or unrecognized) passes through untouched.
fn canonical_date(date: &str) -> String {
    let parts: Vec<&str> = date.split('/').collect();
    if let [month, year] = parts.as_slice() {
        if let (Ok(m), Ok(y)) = (month.trim().parse::<u8>(), year.trim().parse::<u16>()) {
            if (1..=12).contains(&m) {
                return format!("{}-{:02}", y, m);
            }
        }
    }
    date.to_string()
}

/// Zip entry names always use '/'; on Windows the staged relative paths
/// carry '\'.
fn archive_name(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// One packed component: its name, the finished archive on disk, and the
/// per-file accounting compress_files returned for it.
pub struct PackedComponent {
    pub name: String,
    pub archive_path: std::path::PathBuf,
    pub entries: Vec<ArchiveFileEntry>,
}

pub fn build_manifest(
    release_dir: &Path,
    components: &[PackedComponent],
    app_version: &str,
) -> Result<Manifest, AppError> {
    let release: Release = serde_json::from_str(
        &std::fs::read_to_string(release_dir.join("release.json"))
            .map_err(|e| AppError::NotFoundError(format!("release.json missing: {}", e)))?,
    )
    .map_err(|e| AppError::InvalidInput(format!("Invalid release.json: {}", e)))?;
    let date = canonical_date(&release.date);

    let mut manifest_components = Vec::with_capacity(components.len());
    for packed in components {
        let component_dir = release_dir.join(&packed.name);
        let by_name: HashMap<&str, &ArchiveFileEntry> = packed
            .entries
            .iter()
            .map(|e| (e.name.as_str(), e))
            .collect();

        // Every model.json under the component dir: the dir itself for an
        // ungrouped model, one level down per model for a group.
        let mut models = Vec::new();
        for entry in WalkDir::new(&component_dir)
            .max_depth(2)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() == "model.json")
        {
            let Ok(model) = serde_json::from_str::<StlModel>(
                &std::fs::read_to_string(entry.path()).unwrap_or_default(),
            ) else {
                continue; // unreadable sidecar: pack the files, skip the metadata
            };
            let model_dir = entry.path().parent().unwrap_or(&component_dir);
            let to_archive_name = |rel: &str| -> Option<String> {
                model_dir
                    .join(rel)
                    .strip_prefix(&component_dir)
                    .ok()
                    .map(archive_name)
            };
            let files = model
                .model_files
                .iter()
                .filter_map(|rel| {
                    let name = to_archive_name(rel)?;
                    let entry = by_name.get(name.as_str())?;
                    // file_poses are keyed by basename (how the scanner
                    // matches them back on import)
                    let basename = name.rsplit('/').next().unwrap_or(&name);
                    let file_pose = model.file_poses.iter().find(|fp| fp.name == basename);
                    Some(ManifestFile {
                        name,
                        checksum: entry.checksum.clone(),
                        size_bytes: entry.size_bytes,
                        pose: file_pose.and_then(|fp| fp.pose.clone()),
                        support_status: file_pose.and_then(|fp| fp.support_status.clone()),
                    })
                })
                .collect();
            models.push(ManifestModel {
                id: model.id.map(|id| id.to_string()),
                name: model.name.clone(),
                custom_name: None,
                description: model.description.clone(),
                group: model.group.clone(),
                tags: model.tags.clone(),
                designer: model
                    .designer
                    .clone()
                    .or_else(|| Some(release.designer.clone())),
                sculptor: model.sculptor.clone(),
                variant: model.variant.clone(),
                pose: model.pose.clone(),
                scale: model.scale.clone(),
                support_status: model.support_status.clone(),
                release_date: model.release_date.clone().or_else(|| Some(date.clone())),
                release_name: model
                    .release_name
                    .clone()
                    .or_else(|| Some(release.name.clone())),
                base_round_mm: model.base_round_mm,
                base_square_mm: model.base_square_mm,
                preview: model.images.first().and_then(|rel| to_archive_name(rel)),
                files,
            });
        }

        let checksum = crate::manifest::hash_file(&packed.archive_path)?;
        let size_bytes = packed.archive_path.metadata()?.len();
        manifest_components.push(Component {
            name: packed.name.clone(),
            archive: packed
                .archive_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| packed.name.clone()),
            checksum,
            size_bytes,
            dedup: packed.entries.iter().any(|e| !e.stored),
            models,
        });
    }

    Ok(Manifest::new(
        ManifestRelease {
            name: release.name,
            designer: release.designer,
            date,
            version: release.version,
            description: release.description,
            tags: Vec::new(),
            images: release.images,
        },
        manifest_components,
        app_version,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_builder_dates_and_passes_canonical_through() {
        assert_eq!(canonical_date("5/2026"), "2026-05");
        assert_eq!(canonical_date("12/2026"), "2026-12");
        assert_eq!(canonical_date("2026-05"), "2026-05");
        assert_eq!(canonical_date("nonsense"), "nonsense");
    }

    #[test]
    fn manifest_carries_the_staged_curation_end_to_end() {
        let dir = std::env::temp_dir().join(format!("stlpack_manifest_e2e_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let component = dir.join("galeb duhr");
        std::fs::create_dir_all(&component).unwrap();
        std::fs::write(component.join("body.stl"), b"solid body").unwrap();
        std::fs::write(component.join("arm.stl"), b"solid arms").unwrap();
        std::fs::write(
            dir.join("release.json"),
            r#"{"name":"Dungeon Classics","designer":"DTL","description":"d","date":"5/2026",
                "version":"1.0.0","model_references":[],"groups":[],"release_dir":"x",
                "images":["cover.png"],"other_files":[]}"#,
        )
        .unwrap();
        // The enriched sidecar add_model now writes: curation + file_poses
        std::fs::write(
            component.join("model.json"),
            r#"{"id":null,"name":"galeb duhr","description":null,"tags":["earth"],
                "images":[],"model_files":["body.stl","arm.stl"],"group":"galeb duhr",
                "variant":"hammer","pose":"A","scale":"32mm","support_status":"unsupported",
                "sculptor":"A. Artist",
                "file_poses":[{"name":"arm.stl","pose":"A","support_status":"unsupported"}]}"#,
        )
        .unwrap();

        let archive_path = dir.join("galeb duhr.zip");
        let archive = std::fs::File::create(&archive_path).unwrap();
        let entries = crate::file::compressors::compress_files(
            &[component],
            archive,
            None::<fn(u32) -> bool>,
        )
        .unwrap();

        let manifest = build_manifest(
            &dir,
            &[PackedComponent {
                name: "galeb duhr".into(),
                archive_path,
                entries,
            }],
            "0.1.0",
        )
        .unwrap();

        assert_eq!(manifest.release.date, "2026-05");
        let model = &manifest.components[0].models[0];
        assert_eq!(model.variant.as_deref(), Some("hammer"));
        assert_eq!(model.pose.as_deref(), Some("A"));
        assert_eq!(model.scale.as_deref(), Some("32mm"));
        assert_eq!(model.support_status.as_deref(), Some("unsupported"));
        assert_eq!(model.sculptor.as_deref(), Some("A. Artist"));
        assert_eq!(model.designer.as_deref(), Some("DTL"), "release fallback");
        assert_eq!(model.release_date.as_deref(), Some("2026-05"));
        let arm = model.files.iter().find(|f| f.name == "arm.stl").unwrap();
        assert_eq!(arm.pose.as_deref(), Some("A"), "file_poses reach the wire");
        let body = model.files.iter().find(|f| f.name == "body.stl").unwrap();
        assert!(body.pose.is_none());
        assert!(model.files.iter().all(|f| f.checksum.starts_with("blake3:")));

        std::fs::remove_dir_all(&dir).ok();
    }
}
