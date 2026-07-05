use crate::error::AppError;
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

pub fn get_extension_for_compression_type() -> String {
    // compress_files always writes ZIP data (there is no 7z writer yet), so
    // the extension must say zip regardless of the configured preference —
    // labelling ZIP bytes as .7z breaks tools that dispatch on the extension.
    "zip".to_string()
}

/// One file the archive accounts for. `stored` is false when the bytes were
/// elided as a duplicate of an earlier entry with the same checksum — the
/// manifest still lists the name, and extraction rematerializes it.
#[derive(Debug, Clone)]
pub struct ArchiveFileEntry {
    /// Path inside the archive (relative to the archive root).
    pub name: String,
    /// `blake3:<hex>` of the file's content.
    pub checksum: String,
    pub size_bytes: u64,
    pub stored: bool,
}

pub fn determine_dir_size_kb(paths: &[PathBuf]) -> Result<(u32, u32), AppError> {
    let mut total_size_kb = 0;
    let mut total_files = 0;

    for path in paths {
        if path.is_dir() {
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                if entry.file_type().is_file() {
                    total_size_kb += (entry
                        .metadata()
                        .map_err(|e| AppError::IoError(format!("Failed to read metadata: {}", e)))?
                        .len()
                        / 1024) as u32;
                    total_files += 1;
                }
            }
        } else if path.is_file() {
            total_size_kb += (path.metadata()?.len() / 1024) as u32;
            total_files += 1;
        }
    }

    Ok((total_size_kb, total_files))
}

/// (source file on disk, its name inside the archive)
type ArchivePlan = Vec<(PathBuf, String)>;

/// Flatten dirs/files into archive entries up front, so duplicate detection
/// can see every file's size before any bytes are written.
fn plan_entries(paths: &[PathBuf]) -> Result<(ArchivePlan, Vec<String>), AppError> {
    let mut files: ArchivePlan = Vec::new();
    let mut dirs: Vec<String> = Vec::new();
    for path in paths {
        if path.is_dir() {
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                let name = entry_path.strip_prefix(path).unwrap();
                let name = name.to_str().map(str::to_owned).ok_or_else(|| {
                    AppError::FileProcessingError(format!(
                        "Invalid UTF-8 sequence in path: {:?}",
                        entry_path
                    ))
                })?;
                if entry_path.is_file() {
                    files.push((entry_path.to_path_buf(), name));
                } else if !name.is_empty() {
                    dirs.push(name);
                }
            }
        } else if path.is_file() {
            let name = path
                .file_name()
                .ok_or_else(|| {
                    AppError::FileProcessingError(format!("Invalid file name for path: {:?}", path))
                })?
                .to_string_lossy()
                .into_owned();
            files.push((path.clone(), name));
        }
    }
    Ok((files, dirs))
}

/// Stream a file into the zip while hashing it — one read for both jobs.
fn write_hashing<T: Write + Seek>(
    zip: &mut zip::ZipWriter<T>,
    path: &Path,
) -> Result<(String, u64), AppError> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut written: u64 = 0;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        zip.write_all(&buffer[..read])?;
        written += read as u64;
    }
    Ok((format!("blake3:{}", hasher.finalize().to_hex()), written))
}

/// Compress `paths` into a ZIP written to `writer`, storing each unique file
/// content ONCE: a later file whose bytes match an earlier entry is elided
/// from the archive but still returned (stored=false), for the manifest to
/// list against the same checksum. Extraction rematerializes elided names
/// (see manifest::extract_component_archive). Only files sharing a size can
/// be duplicates, so unique-size files are hashed in the same pass that
/// writes them; same-size candidates are hashed first (one extra read) to
/// decide elision before any bytes land.
///
/// The progress callback receives each file's size in KiB and returns
/// whether to CONTINUE — returning false aborts the archive (user cancel).
pub fn compress_files<T, F>(
    paths: &[PathBuf],
    writer: T,
    mut progress_callback: Option<F>,
) -> Result<Vec<ArchiveFileEntry>, AppError>
where
    T: Write + Seek,
    F: FnMut(u32) -> bool,
{
    let (files, dirs) = plan_entries(paths)?;

    // Sizes that appear more than once — only these need a pre-hash
    let mut size_counts: HashMap<u64, u32> = HashMap::new();
    let mut sizes: Vec<u64> = Vec::with_capacity(files.len());
    for (path, _) in &files {
        let size = path.metadata()?.len();
        *size_counts.entry(size).or_default() += 1;
        sizes.push(size);
    }
    let dup_sizes: HashSet<u64> = size_counts
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(size, _)| size)
        .collect();

    let mut zip = zip::ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    for dir in dirs {
        zip.add_directory(dir, options)
            .map_err(|e| AppError::FileProcessingError(format!("Error adding directory: {}", e)))?;
    }

    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut entries: Vec<ArchiveFileEntry> = Vec::with_capacity(files.len());
    for ((path, name), size) in files.into_iter().zip(sizes) {
        let entry = if dup_sizes.contains(&size) {
            let checksum = crate::manifest::hash_file(&path)?;
            if seen.contains_key(&checksum) {
                // Bytes already in the archive under another name: list it,
                // skip storing it
                ArchiveFileEntry {
                    name,
                    checksum,
                    size_bytes: size,
                    stored: false,
                }
            } else {
                seen.insert(checksum.clone(), ());
                zip.start_file(&name, options).map_err(|e| {
                    AppError::FileProcessingError(format!("Error starting file: {}", e))
                })?;
                let mut f = File::open(&path)?;
                std::io::copy(&mut f, &mut zip)?;
                ArchiveFileEntry {
                    name,
                    checksum,
                    size_bytes: size,
                    stored: true,
                }
            }
        } else {
            // Unique size can't be a duplicate: hash while writing
            zip.start_file(&name, options)
                .map_err(|e| AppError::FileProcessingError(format!("Error starting file: {}", e)))?;
            let (checksum, written) = write_hashing(&mut zip, &path)?;
            seen.insert(checksum.clone(), ());
            ArchiveFileEntry {
                name,
                checksum,
                size_bytes: written,
                stored: true,
            }
        };

        if let Some(ref mut callback) = progress_callback {
            if !callback((size / 1024) as u32) {
                return Err(AppError::UserCancelled(
                    "Compression cancelled by user".to_string(),
                ));
            }
        }
        entries.push(entry);
    }

    zip.finish()
        .map_err(|e| AppError::FileProcessingError(format!("Error finishing zip: {}", e)))?;
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Cursor;

    #[test]
    fn dedups_identical_blobs_and_lists_every_name() {
        let dir = std::env::temp_dir().join(format!("stlpack_zip_test_{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("model/variant_b")).unwrap();
        fs::write(dir.join("model/base.stl"), b"identical-base-bytes").unwrap();
        fs::write(dir.join("model/variant_b/base.stl"), b"identical-base-bytes").unwrap();
        // same size, different bytes: must NOT be elided
        fs::write(dir.join("model/other.stl"), b"differing-base-bytes").unwrap();
        fs::write(dir.join("model/unique.stl"), b"tiny").unwrap();

        let mut buffer = Cursor::new(Vec::new());
        let entries = compress_files(
            &[dir.join("model")],
            &mut buffer,
            None::<fn(u32) -> bool>,
        )
        .unwrap();

        assert_eq!(entries.len(), 4, "every name is listed");
        let elided: Vec<_> = entries.iter().filter(|e| !e.stored).collect();
        assert_eq!(elided.len(), 1, "one of the two identical files is elided");
        let stored_twin = entries
            .iter()
            .find(|e| e.stored && e.checksum == elided[0].checksum)
            .expect("the elided entry's bytes exist under another name");
        assert_ne!(stored_twin.name, elided[0].name);

        // the archive itself holds only the 3 stored files
        let mut archive = zip::ZipArchive::new(Cursor::new(buffer.into_inner())).unwrap();
        let names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .filter(|n| n.ends_with(".stl"))
            .collect();
        assert_eq!(names.len(), 3);
        assert!(!names.contains(&elided[0].name));

        fs::remove_dir_all(&dir).ok();
    }
}
