use super::storage;
use super::writer;
use crate::error::AppError;
use crate::file::compression_jobs;
use crate::file::utils::clean_name;
use crate::models::events::CancelledStatus;
use crate::models::events::CompletedStatus;
use crate::models::events::CompressionStatus;
use crate::models::events::FailedStatus;
use crate::models::{Release, StlModel};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use uuid::Uuid;

static ACTIVE_COMPRESSIONS: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[tauri::command]
#[specta::specta]
pub async fn add_model(
    model: StlModel,
    release_dir: String,
    file_paths: Vec<String>,
    image_paths: Vec<String>,
) -> Result<(StlModel, String), AppError> {
    let release_path = PathBuf::from(release_dir);

    let clean_model_name = clean_name(&model.name);
    let model_folder = match model.group {
        Some(ref group_name) => {
            let clean_group_name = clean_name(group_name);
            let group_dir = release_path.join(&clean_group_name);
            group_dir.join(&clean_model_name)
        }
        None => release_path.join(&clean_model_name),
    };

    fs::create_dir_all(&model_folder)
        .map_err(|e| AppError::IoError(format!("failed to create model folder; {}", e)))?;

    let copied_images = storage::copy_images(&image_paths, &model_folder, &clean_model_name)?;
    let copied_files = storage::copy_files(&file_paths, &model_folder)?;

    let relative_image_paths = storage::convert_to_relative_paths(&copied_images, &model_folder)?;
    let relative_file_paths = storage::convert_to_relative_paths(&copied_files, &model_folder)?;

    let model_id = model.id.unwrap_or(Uuid::new_v4());
    let model_with_relative_paths = StlModel {
        id: Some(model_id),
        name: clean_model_name,
        description: model.description,
        tags: model.tags,
        group: model.group.clone(),
        images: relative_image_paths,
        model_files: relative_file_paths,
    };

    let model_json_path = model_folder.join("model.json");
    let model_json = serde_json::to_string_pretty(&model_with_relative_paths)?;
    writer::write_json(model_json, model_json_path.clone())?;
    writer::add_model_to_release_json(release_path, &model_with_relative_paths)?;

    Ok((
        model_with_relative_paths,
        model_json_path.to_string_lossy().into_owned(),
    ))
}

#[tauri::command]
#[specta::specta]
pub async fn create_release(
    app_handle: AppHandle,
    release: Release,
    image_paths: Vec<String>,
    other_file_paths: Vec<String>,
) -> Result<String, AppError> {
    let release_name = clean_name(&release.name);
    let designer_name = clean_name(&release.designer);

    // The directory name doubles as the catalog's on-disk key; refuse a
    // malformed date instead of silently stamping a wrong one
    let release_date = {
        let invalid = || {
            AppError::InvalidInput(format!(
                "Invalid release date '{}': expected MM/YYYY",
                release.date
            ))
        };
        let date_parts = release.date.split('/').collect::<Vec<_>>();
        match date_parts.as_slice() {
            [month, year] => {
                let month: u8 = month.trim().parse().map_err(|_| invalid())?;
                let year: u16 = year.trim().parse().map_err(|_| invalid())?;
                if !(1..=12).contains(&month) {
                    return Err(invalid());
                }
                format!("{:02}-{}", month, year)
            }
            _ => return Err(invalid()),
        }
    };

    let release_dir_name = format!("{}-{}-{}", designer_name, release_date, release_name);
    let release_path = storage::create_dir_on_scratch(&app_handle, release_dir_name.clone())?;

    let copied_images = storage::copy_images(&image_paths, &release_path, &release_name)?;
    let copied_files = storage::copy_files(&other_file_paths, &release_path)?;

    let relative_image_paths = storage::convert_to_relative_paths(&copied_images, &release_path)?;
    let relative_file_paths = storage::convert_to_relative_paths(&copied_files, &release_path)?;

    // The backend owns the directory name — persisting a frontend-computed
    // copy invites the two to drift apart
    let release_with_paths = Release {
        images: relative_image_paths,
        other_files: relative_file_paths,
        release_dir: release_dir_name,
        ..release
    };

    let release_json = serde_json::to_string_pretty(&release_with_paths)?;

    fs::write(release_path.join("release.json"), release_json)?;

    if let Some(window) = app_handle.get_webview_window("main") {
        window.set_title(&format!(
            "STL-Pack - Creating release: {}",
            release_with_paths.name
        ))?;
    }

    Ok(release_path.to_string_lossy().into_owned())
}

#[tauri::command]
#[specta::specta]
pub async fn finalize_release(
    app_handle: AppHandle,
    release_dir: String,
) -> Result<String, AppError> {
    let release_dir_path = PathBuf::from(&release_dir);

    if !release_dir_path.exists() {
        return Err(AppError::NotFoundError(format!(
            "Release directory '{}' not found",
            release_dir_path.display()
        )));
    }

    // Generate a unique ID for this compression job
    let job_id = Uuid::new_v4().to_string();

    // Create cancellation token
    let cancel_token = Arc::new(AtomicBool::new(false));

    // Register the token in our global state
    {
        let mut compressions = ACTIVE_COMPRESSIONS.lock().map_err(|e| {
            AppError::ConfigError(format!("Failed to access compressions registry: {}", e))
        })?;
        compressions.insert(job_id.clone(), Arc::clone(&cancel_token));
    }

    // Start the compression process in the background
    let app_handle_clone = app_handle.clone();
    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        let start_time = std::time::Instant::now();

        // Run the actual compression
        let result = compression_jobs::perform_compression(
            app_handle_clone.clone(),
            job_id_clone.clone(),
            release_dir_path.clone(),
            cancel_token,
        )
        .await;

        // Clean up the cancellation token
        if let Ok(mut compressions) = ACTIVE_COMPRESSIONS.lock() {
            compressions.remove(&job_id_clone);
        }

        // Send appropriate completion event
        match result {
            Ok((files, size, target_dir)) => {
                let elapsed = start_time.elapsed().as_secs_f64();
                CompressionStatus::Completed(CompletedStatus {
                    job_id: job_id_clone,
                    total_files: files,
                    total_size_kb: size,
                    elapsed_seconds: elapsed,
                    folder_path: target_dir.to_string_lossy().into_owned(),
                })
                .emit(&app_handle_clone)
                .ok();
            }
            // Match the variant, not the message text: error strings can
            // legitimately contain the word "cancelled" (e.g. in a path)
            Err(AppError::UserCancelled(_)) => {
                CompressionStatus::Cancelled(CancelledStatus {
                    job_id: job_id_clone,
                })
                .emit(&app_handle_clone)
                .ok();
            }
            Err(e) => {
                CompressionStatus::Failed(FailedStatus {
                    job_id: job_id_clone,
                    error: e.to_string(),
                })
                .emit(&app_handle_clone)
                .ok();
            }
        }
    });

    // Return the job ID immediately so the client can use it to cancel if needed
    Ok(job_id)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_compression(job_id: String) -> Result<(), AppError> {
    let compressions = ACTIVE_COMPRESSIONS.lock().map_err(|e| {
        AppError::ConfigError(format!("Failed to access compressions registry: {}", e))
    })?;

    if let Some(token) = compressions.get(&job_id) {
        // Signal cancellation
        token.store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    } else {
        Err(AppError::NotFoundError(format!(
            "No active compression job with ID: {}",
            job_id
        )))
    }
}
