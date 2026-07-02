use crate::error::AppError;
use std::fs::File;
use std::io::{Seek, Write};
use std::path::PathBuf;
use walkdir::WalkDir;
use zip::write::SimpleFileOptions;

pub fn get_extension_for_compression_type() -> String {
    // compress_files always writes ZIP data (there is no 7z writer yet), so
    // the extension must say zip regardless of the configured preference —
    // labelling ZIP bytes as .7z breaks tools that dispatch on the extension.
    "zip".to_string()
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

/// Compress `paths` into a ZIP written to `writer`. The progress callback
/// receives each file's size in KiB and returns whether to CONTINUE —
/// returning false aborts the archive (used for user cancellation).
pub fn compress_files<T, F>(
    paths: &[PathBuf],
    writer: T,
    mut progress_callback: Option<F>,
) -> Result<(), AppError>
where
    T: Write + Seek,
    F: FnMut(u32) -> bool,
{
    let mut zip = zip::ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    for path in paths {
        if path.is_dir() {
            // Handle directories
            for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                let name = entry_path.strip_prefix(path).unwrap();
                let path_as_string = name.to_str().map(str::to_owned).ok_or_else(|| {
                    AppError::FileProcessingError(format!(
                        "Invalid UTF-8 sequence in path: {:?}",
                        entry_path
                    ))
                })?;

                if entry_path.is_file() {
                    // Add file to ZIP, streaming so multi-GB files don't
                    // occupy RAM in full
                    zip.start_file(path_as_string, options).map_err(|e| {
                        AppError::FileProcessingError(format!("Error starting file: {}", e))
                    })?;
                    let mut f = File::open(entry_path)?;
                    std::io::copy(&mut f, &mut zip)?;

                    if let Some(ref mut callback) = progress_callback {
                        let size_kb = (entry
                            .metadata()
                            .map_err(|e| {
                                AppError::FileProcessingError(format!(
                                    "Failed to read metadata from entry: {}",
                                    e
                                ))
                            })?
                            .len()
                            / 1024) as u32;
                        if !callback(size_kb) {
                            return Err(AppError::UserCancelled(
                                "Compression cancelled by user".to_string(),
                            ));
                        }
                    }
                } else if !name.as_os_str().is_empty() {
                    zip.add_directory(path_as_string, options).map_err(|e| {
                        AppError::FileProcessingError(format!("Error adding directory: {}", e))
                    })?;
                }
            }
        } else if path.is_file() {
            // Handle individual files
            let file_name = path
                .file_name()
                .ok_or_else(|| {
                    AppError::FileProcessingError(format!("Invalid file name for path: {:?}", path))
                })?
                .to_string_lossy()
                .into_owned();

            zip.start_file(file_name, options).map_err(|e| {
                AppError::FileProcessingError(format!("Error starting file: {}", e))
            })?;
            let mut f = File::open(path)?;
            std::io::copy(&mut f, &mut zip)?;

            if let Some(ref mut callback) = progress_callback {
                if !callback((path.metadata()?.len() / 1024) as u32) {
                    return Err(AppError::UserCancelled(
                        "Compression cancelled by user".to_string(),
                    ));
                }
            }
        }
    }

    zip.finish()
        .map_err(|e| AppError::FileProcessingError(format!("Error finishing zip: {}", e)))?;
    Ok(())
}
