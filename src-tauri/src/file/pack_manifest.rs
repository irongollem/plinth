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
                    Some(ManifestFile {
                        name,
                        checksum: entry.checksum.clone(),
                        size_bytes: entry.size_bytes,
                        pose: None,
                        support_status: None,
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
                designer: Some(release.designer.clone()),
                sculptor: None,
                pose: None,
                scale: None,
                support_status: None,
                release_date: Some(date.clone()),
                release_name: Some(release.name.clone()),
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
}
