use crate::error::AppError;
use crate::models::events::CompressionProgessEvent;
use crate::models::models::CompressionType;
use crate::settings::SETTINGS_CACHE;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::Path;
use tauri::AppHandle;
use tauri_specta::Event;
use walkdir::{DirEntry, WalkDir};
use zip;
use zip::write::SimpleFileOptions;

pub fn get_compression_type() -> Result<CompressionType, AppError> {
    let compression_type = {
        SETTINGS_CACHE
            .lock()
            .map_err(|e| AppError::ConfigError(format!("{}", e)))?
            .compression_type
            .clone()
    };
    match compression_type {
        Some(comp) => Ok(comp),
        None => Err(AppError::ConfigError(
            "Compression type not set".to_string(),
        )),
    }
}

pub fn get_extension_for_compression_type() -> String {
    let compression_type_result = get_compression_type();

    match compression_type_result {
        Ok(compression_type) => match compression_type {
            CompressionType::SevenZip => "7z",
            CompressionType::Zip => "zip",
        }
        .to_string(),
        Err(_) => "zip".to_string(), // Default to zip if settings access fails
    }
}

pub fn compress_dir_with_progress(
    source_dir: &Path,
    target_path: &Path,
    app_handle: &AppHandle,
) -> Result<(), AppError> {
    let mut total_size: u32 = 0;
    let mut total_files: u32 = 0;

    for entry in WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let file_size_bytes = entry
                .metadata()
                .map_err(|e| AppError::IoError(format!("File Metadata Error: {}", e)))?
                .len();

            let file_size_mb = ((file_size_bytes / (1024 * 1024)) as u32).max(1);

            total_size += file_size_mb;
            total_files += 1;
        }
    }

    let file = fs::File::create(target_path)?;
    let method = zip::CompressionMethod::Deflated;

    let mut processed_size = 0;
    let mut processed_files = 0;
    let progress_cb = |file_size_mb: u32| {
        processed_size += file_size_mb;
        processed_files += 1;
        let percent_size = (processed_size * 100) / total_size;
        let percent_files = (processed_files * 100) / total_files;

        let progress = CompressionProgessEvent {
            processed_files,
            total_files,
            processed_size,
            total_size,
            percent_size,
            percent_files,
        };

        progress.emit(app_handle).ok(); // TODO: check if ok is enough here
    };

    zip_dir(
        &mut WalkDir::new(source_dir).into_iter().filter_map(|e| e.ok()),
        source_dir,
        file,
        method,
        progress_cb,
    )?;

    Ok(())
}

fn zip_dir<T, F>(
    it: &mut dyn Iterator<Item = DirEntry>,
    prefix: &Path,
    writer: T,
    method: zip::CompressionMethod,
    mut progress_callback: F,
) -> Result<(), AppError>
where
    T: Write + Seek,
    F: FnMut(u32),
{
    let mut zip = zip::ZipWriter::new(writer);
    let options = SimpleFileOptions::default()
        .compression_method(method)
        .unix_permissions(0o755);

    let prefix = Path::new(prefix);
    let mut buffer = Vec::new();

    for entry in it {
        let path = entry.path();
        let name = path.strip_prefix(prefix).unwrap();
        let path_as_string = name.to_str().map(str::to_owned).ok_or_else(|| {
            AppError::FileProcessingError(format!("Invalid UTF-8 sequence in path: {:?}", path))
        })?;
        // Write file or directory explicitly
        // Some unzip tools unzip files with directory paths correctly, some do not!
        if path.is_file() {
            println!("adding file {path:?} as {name:?} ...");
            zip.start_file(path_as_string, options).map_err(|e| {
                AppError::FileProcessingError(format!("Error starting file: {}", e))
            })?;
            let mut f = File::open(path)?;

            let file_size_bytes = entry
                .metadata()
                .map_err(|e| {
                    AppError::FileProcessingError(format!("Error getting metadata: {}", e))
                })?
                .len();
            let file_size_mb = ((file_size_bytes / (1024 * 1024)) as u32).max(1);

            f.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
            buffer.clear();
            progress_callback(file_size_mb);
        } else if !name.as_os_str().is_empty() {
            println!("adding dir {path_as_string:?} as {name:?} ...");
            zip.add_directory(path_as_string, options).map_err(|e| {
                AppError::FileProcessingError(format!("Error adding directory: {}", e))
            })?;
        }
    }
    zip.finish()
        .map_err(|e| AppError::FileProcessingError(format!("Error finishing zip: {}", e)))?;
    Ok(())
}
