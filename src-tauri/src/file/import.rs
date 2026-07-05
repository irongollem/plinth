//! Import a packed release: read `manifest.json` out of `release.3pk`,
//! verify each sibling component archive against its BLAKE3 checksum, and
//! extract everything into a library folder — rematerializing names a
//! deduplicated archive elided (hardlink where the volume supports it).
//! The extracted tree matches what the builder staged, so a normal catalog
//! scan restores the packed curation via the model.json sidecars.

use crate::error::AppError;
use crate::file::utils::clean_name;
use crate::manifest::{self, Manifest};
use serde::{Deserialize, Serialize};
use specta::Type;
use std::io::Read;
use std::path::Path;

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct ImportOutcome {
    pub release_name: String,
    pub designer: String,
    /// The directory the release landed in.
    pub dest_dir: String,
    pub components: u32,
    pub files: u32,
    /// Per-component problems (checksum mismatch, missing archive); the
    /// rest of the release still imports.
    pub errors: Vec<String>,
}

/// Read `manifest.json` from inside a `release.3pk`.
pub fn read_manifest(package_path: &Path) -> Result<Manifest, AppError> {
    let file = std::fs::File::open(package_path)
        .map_err(|e| AppError::IoError(format!("Cannot open {}: {}", package_path.display(), e)))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::InvalidInput(format!("Not a readable package: {}", e)))?;
    let mut entry = archive.by_name("manifest.json").map_err(|_| {
        AppError::InvalidInput(
            "No manifest.json inside — this package predates the 3pk manifest".into(),
        )
    })?;
    let mut text = String::new();
    entry
        .read_to_string(&mut text)
        .map_err(|e| AppError::IoError(format!("Failed to read manifest: {}", e)))?;
    let manifest = Manifest::from_json(&text)?;
    if !manifest.is_readable() {
        return Err(AppError::InvalidInput(format!(
            "This package uses 3pk format v{} — this app reads v{}",
            manifest.version,
            manifest::VERSION
        )));
    }
    Ok(manifest)
}

pub fn import_release(package_path: &Path, library_dir: &Path) -> Result<ImportOutcome, AppError> {
    let manifest = read_manifest(package_path)?;
    let package_dir = package_path
        .parent()
        .ok_or_else(|| AppError::InvalidInput("Package has no parent directory".into()))?;

    // Same naming scheme create_release uses, so imported and self-made
    // releases sit uniformly in the library. Manifest date is YYYY-MM;
    // the on-disk convention is MM-YYYY.
    let date = manifest
        .release
        .date
        .split_once('-')
        .map(|(y, m)| format!("{}-{}", m, y))
        .unwrap_or_else(|| manifest.release.date.clone());
    let dir_name = format!(
        "{}-{}-{}",
        clean_name(&manifest.release.designer),
        date,
        clean_name(&manifest.release.name)
    );
    let dest = library_dir.join(dir_name);
    if dest.exists() {
        return Err(AppError::InvalidInput(format!(
            "'{}' already exists — remove it first to re-import",
            dest.display()
        )));
    }
    std::fs::create_dir_all(&dest)
        .map_err(|e| AppError::IoError(format!("Failed to create release dir: {}", e)))?;

    // Release-level payload (images, release.json, the manifest itself)
    let package_file = std::fs::File::open(package_path)
        .map_err(|e| AppError::IoError(format!("Cannot open {}: {}", package_path.display(), e)))?;
    zip::ZipArchive::new(package_file)
        .and_then(|mut a| a.extract(&dest))
        .map_err(|e| AppError::IoError(format!("Failed to extract release files: {}", e)))?;

    let mut components = 0u32;
    let mut files = 0u32;
    let mut errors: Vec<String> = Vec::new();
    for component in &manifest.components {
        let archive_path = package_dir.join(&component.archive);
        if !archive_path.is_file() {
            errors.push(format!("{}: archive '{}' is missing", component.name, component.archive));
            continue;
        }
        // The checksum is the integrity promise of the format — a truncated
        // download or bit-rot surfaces here, not as broken STLs later
        let actual = manifest::hash_file(&archive_path)?;
        if actual != component.checksum {
            errors.push(format!(
                "{}: checksum mismatch — the archive is corrupted or was modified",
                component.name
            ));
            continue;
        }
        let manifest_files: Vec<_> = component
            .models
            .iter()
            .flat_map(|m| m.files.iter().cloned())
            .collect();
        let component_dest = dest.join(&component.name);
        manifest::extract_component_archive(&archive_path, &component_dest, &manifest_files)?;
        components += 1;
        files += manifest_files.len() as u32;
    }

    Ok(ImportOutcome {
        release_name: manifest.release.name.clone(),
        designer: manifest.release.designer.clone(),
        dest_dir: dest.to_string_lossy().into_owned(),
        components,
        files,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file::compressors::compress_files;
    use crate::file::pack_manifest::{build_manifest, PackedComponent};

    /// Pack a release with the real writer, then import it elsewhere and
    /// check the tree + curation sidecar arrived intact — the full loop.
    #[test]
    fn packed_release_imports_verified_and_complete() {
        let dir = std::env::temp_dir().join(format!("stlpack_import_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let staged = dir.join("staged");
        let component = staged.join("knight");
        std::fs::create_dir_all(component.join("variant_b")).unwrap();
        std::fs::write(component.join("base.stl"), b"shared-base-bytes").unwrap();
        std::fs::write(component.join("variant_b/base.stl"), b"shared-base-bytes").unwrap();
        std::fs::write(component.join("body.stl"), b"knight-body-bytes").unwrap();
        std::fs::write(
            component.join("model.json"),
            r#"{"id":null,"name":"knight","description":null,"tags":[],"images":[],
                "model_files":["base.stl","variant_b/base.stl","body.stl"],"group":null,
                "pose":"A","support_status":"unsupported"}"#,
        )
        .unwrap();
        std::fs::write(
            staged.join("release.json"),
            r#"{"name":"Knights","designer":"DTL","description":"","date":"5/2026",
                "version":"1","model_references":[],"groups":[],"release_dir":"x",
                "images":[],"other_files":[]}"#,
        )
        .unwrap();

        // Pack: component archive (with dedup) + manifest + release.3pk
        let out = dir.join("packed");
        std::fs::create_dir_all(&out).unwrap();
        let archive_path = out.join("knight.zip");
        let entries = compress_files(
            &[component.clone()],
            std::fs::File::create(&archive_path).unwrap(),
            None::<fn(u32) -> bool>,
        )
        .unwrap();
        let manifest = build_manifest(
            &staged,
            &[PackedComponent {
                name: "knight".into(),
                archive_path,
                entries,
            }],
            "0.1.0",
        )
        .unwrap();
        std::fs::write(staged.join("manifest.json"), manifest.to_json().unwrap()).unwrap();
        compress_files(
            &[staged.join("manifest.json"), staged.join("release.json")],
            std::fs::File::create(out.join("release.3pk")).unwrap(),
            None::<fn(u32) -> bool>,
        )
        .unwrap();

        // Import into a fresh library
        let library = dir.join("library");
        std::fs::create_dir_all(&library).unwrap();
        let outcome = import_release(&out.join("release.3pk"), &library).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert_eq!(outcome.components, 1);

        let release_dir = Path::new(&outcome.dest_dir);
        assert!(release_dir.ends_with("dtl-05-2026-knights"));
        // Every manifest name exists — including the dedup-elided twin
        for name in [
            "knight/base.stl",
            "knight/variant_b/base.stl",
            "knight/body.stl",
            "knight/model.json",
            "release.json",
            "manifest.json",
        ] {
            assert!(release_dir.join(name).is_file(), "{} imported", name);
        }
        assert_eq!(
            std::fs::read(release_dir.join("knight/base.stl")).unwrap(),
            b"shared-base-bytes"
        );

        // A corrupted component archive is refused by checksum, not imported
        std::fs::write(out.join("knight.zip"), b"tampered").unwrap();
        std::fs::remove_dir_all(release_dir).ok();
        let outcome = import_release(&out.join("release.3pk"), &library).unwrap();
        assert_eq!(outcome.components, 0);
        assert!(outcome.errors[0].contains("checksum mismatch"));

        std::fs::remove_dir_all(&dir).ok();
    }
}
