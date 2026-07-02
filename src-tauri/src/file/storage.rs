use crate::error::AppError;
use crate::settings::SETTINGS_CACHE;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub fn get_scratch_path(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    let scratch_dir = {
        SETTINGS_CACHE
            .lock()
            .map_err(|e| AppError::ConfigError(format!("{}", e)))?
            .scratch_dir
            .clone()
    };

    if let Some(dir) = scratch_dir {
        Ok(PathBuf::from(dir))
    } else {
        // The scratchdir is temporary in nature so the default is the app_data dir
        Ok(app_handle.path().app_data_dir()?)
    }
}

pub fn create_dir_on_scratch(
    app_handle: &AppHandle,
    dir_name: String,
) -> Result<PathBuf, AppError> {
    let scratch_root = get_scratch_path(app_handle)?;
    let temp_dir = scratch_root.join(dir_name);

    fs::create_dir_all(&temp_dir)?;

    Ok(temp_dir)
}

pub fn get_target_path(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    let target_dir = {
        SETTINGS_CACHE
            .lock()
            .map_err(|e| AppError::ConfigError(format!("{}", e)))?
            .target_dir
            .clone()
    };

    if let Some(dir) = target_dir {
        Ok(PathBuf::from(dir))
    } else {
        Ok(app_handle
            .path()
            .document_dir()?
            .join("STL-Pack")
            .join("exports"))
    }
}

pub fn rename_image(model_name: &str, original_path: &Path, index: usize) -> String {
    let extension = original_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("");

    if index == 0 {
        format!("{}-main.{}", model_name, extension)
    } else {
        format!("{}-detail_{}.{}", model_name, index, extension)
    }
}

pub fn get_destination_folder(model_folder: &Path, file_path: &Path) -> PathBuf {
    let extension = file_path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "chitubox" => {
            let subfolder = model_folder.join("chitubox");
            fs::create_dir_all(&subfolder).unwrap_or_default();
            subfolder
        }
        "lys" => {
            let subfolder = model_folder.join("lychee");
            fs::create_dir_all(&subfolder).unwrap_or_default();
            subfolder
        }
        "stl" => {
            let filename = file_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("")
                .to_lowercase();

            // Split on every non-alphanumeric character so spaced or
            // parenthesised names ("knight (supported).stl", "dragon sup.stl")
            // tokenize correctly too
            let is_presupported = filename.split(|c: char| !c.is_alphanumeric()).any(|part| {
                part == "sup" || part == "supported" || part == "presupported" || part == "ps"
            });

            if is_presupported {
                let subfolder = model_folder.join("supported");
                fs::create_dir_all(&subfolder).unwrap_or_default();
                subfolder
            } else {
                model_folder.to_path_buf()
            }
        }
        _ => model_folder.to_path_buf(),
    }
}

pub fn copy_images(
    image_paths: &[String],
    model_folder: &Path,
    clean_model_name: &str,
) -> Result<Vec<String>, AppError> {
    let mut copied_images = Vec::new();

    for (i, path) in image_paths.iter().enumerate() {
        let source_path = Path::new(path);
        let new_name = rename_image(clean_model_name, source_path, i);
        let destination_path = model_folder.join(&new_name);

        fs::copy(source_path, &destination_path)
            .map_err(|e| AppError::IoError(format!("failed to copy image; {}", e)))?;
        copied_images.push(destination_path.to_string_lossy().into_owned());
    }

    Ok(copied_images)
}

pub fn copy_files(file_paths: &[String], model_folder: &Path) -> Result<Vec<String>, AppError> {
    let mut copied_files = Vec::new();

    for path in file_paths {
        let source_path = Path::new(path);
        let file_name = source_path
            .file_name()
            .ok_or_else(|| AppError::IoError(format!("Invalid file path: {}", path)))?;

        let destination_folder = get_destination_folder(model_folder, source_path);
        let destination_path = destination_folder.join(file_name);

        // Same basename from two different source folders would silently
        // overwrite here — fail loudly instead of losing a part
        if destination_path.exists() {
            return Err(AppError::InvalidInput(format!(
                "Duplicate file name '{}': a file with this name was already added to this model. Rename one of the files and try again.",
                file_name.to_string_lossy()
            )));
        }

        fs::copy(source_path, &destination_path)
            .map_err(|e| AppError::IoError(format!("failed to copy file; {}", e)))?;
        copied_files.push(destination_path.to_string_lossy().into_owned());
    }

    Ok(copied_files)
}

pub fn convert_to_relative_path(absolute_path: &str, base_dir: &Path) -> Result<String, AppError> {
    // strip_prefix is component-aware: unlike string starts_with it can't
    // mistake a sibling like /x/ScratchOld for being inside /x/Scratch
    Path::new(absolute_path)
        .strip_prefix(base_dir)
        .map(|rel| rel.to_string_lossy().into_owned())
        .map_err(|_| {
            AppError::InvalidInput(format!(
                "Path '{}' is not within base directory '{}'",
                absolute_path,
                base_dir.display()
            ))
        })
}

pub fn convert_to_relative_paths(
    paths: &[String],
    base_dir: &Path,
) -> Result<Vec<String>, AppError> {
    paths
        .iter()
        .map(|path| {
            convert_to_relative_path(path, base_dir).map_err(|e| {
                AppError::IoError(format!("Failed to convert path to relative: {}", e))
            })
        })
        .collect()
}

/// (group/model directories, files destined for release.3pk, files destined for release.zip)
pub type CompressionFileSets = (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>);

pub fn collect_files_for_compression(
    release_dir_path: &PathBuf,
) -> Result<CompressionFileSets, AppError> {
    let mut group_and_model_dirs = Vec::new();
    let mut files_for_3pk = Vec::new();
    let mut files_for_zip = Vec::new();

    for entry in fs::read_dir(release_dir_path)
        .map_err(|e| AppError::IoError(format!("Failed to read release directory: {}", e)))?
    {
        let entry = entry.map_err(|e| AppError::IoError(format!("Failed to read entry: {}", e)))?;
        let path = entry.path();

        if path.is_dir() {
            group_and_model_dirs.push(path.clone());
        } else if path.is_file() {
            let file_name = path
                .file_name()
                .ok_or_else(|| AppError::ConfigError("Invalid file name".to_string()))?
                .to_string_lossy()
                .into_owned();

            if file_name.ends_with(".json") || file_name.ends_with(".png") {
                files_for_3pk.push(path.clone());
            }

            files_for_zip.push(path.clone());
        }
    }

    Ok((group_and_model_dirs, files_for_3pk, files_for_zip))
}
