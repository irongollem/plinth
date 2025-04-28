use std::{
    fs::{self, File},
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Semaphore;

use crate::{
    error::AppError,
    models::events::{CompressionStatus, ProgressStatus, StartedStatus},
    settings::get_optimal_thread_count,
};

use super::{
    compressors,
    storage::{self, collect_files_for_compression},
    utils::calculate_total_size,
};

struct ProgressTracker {
    app_handle: AppHandle,
    total_files: u32,
    total_size: u32,
    processed_files: u32,
    processed_size: u32,
    cancel_token: Arc<AtomicBool>,
    current_file: String,
}

impl ProgressTracker {
    fn new(
        app_handle: AppHandle,
        total_files: u32,
        total_size: u32,
        cancel_token: Arc<AtomicBool>,
    ) -> Self {
        Self {
            app_handle,
            total_files,
            total_size,
            processed_files: 0,
            processed_size: 0,
            cancel_token,
            current_file: String::new(),
        }
    }

    fn create_callback(&mut self, file_path: &Path) -> impl FnMut(u32) + '_ {
        // Set current file being processed
        self.current_file = file_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        move |file_size: u32| {
            // Check for cancellation
            if self.cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                // Return early without updating progress
                return;
            }

            self.processed_size += file_size;
            self.processed_files += 1;
            let percent_size = (self.processed_size * 100) / self.total_size;
            let percent_files = (self.processed_files * 100) / self.total_files;

            CompressionStatus::Progress(ProgressStatus {
                processed_files: self.processed_files,
                total_files: self.total_files,
                processed_size: self.processed_size,
                total_size: self.total_size,
                percent_size,
                percent_files,
                current_file: self.current_file.clone(),
            })
            .emit(&self.app_handle)
            .ok();
        }
    }
}

pub async fn perform_compression(
    app_handle: AppHandle,
    release_dir_path: PathBuf,
    cancel_token: Arc<AtomicBool>,
) -> Result<(u32, u32), AppError> {
    let target_dir_path = storage::get_target_path(&app_handle)?;
    let extension = compressors::get_extension_for_compression_type();

    // Collect files for compression
    let (group_and_model_dirs, files_for_3pk, files_for_zip) =
        collect_files_for_compression(&release_dir_path)?;

    // Calculate sizes
    let (total_size, total_files) =
        calculate_total_size(&group_and_model_dirs, &files_for_3pk, &files_for_zip)?;

    // Emit started event
    CompressionStatus::Started(StartedStatus {
        total_files,
        total_size,
    })
    .emit(&app_handle)
    .ok();

    // Create progress tracking
    let progress_tracker = Arc::new(Mutex::new(ProgressTracker::new(
        app_handle.clone(),
        total_files,
        total_size,
        cancel_token.clone(),
    )));

    // Run the compression tasks with semaphore-controlled concurrency
    run_compression_tasks(
        progress_tracker,
        &target_dir_path,
        &extension,
        &group_and_model_dirs,
        &files_for_3pk,
        &files_for_zip,
        cancel_token.clone(),
    )
    .await?;

    // Check if cancelled before cleanup
    if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(AppError::UserCancelled(
            "Compression was cancelled by user".into(),
        ));
    }

    // Cleanup original files
    fs::remove_dir_all(&release_dir_path)
        .map_err(|e| AppError::IoError(format!("Failed to clean up release directory: {}", e)))?;

    Ok((total_files, total_size))
}

async fn run_compression_tasks(
    progress_tracker: Arc<Mutex<ProgressTracker>>,
    target_dir_path: &Path,
    extension: &str,
    group_and_model_dirs: &[PathBuf],
    files_for_3pk: &[PathBuf],
    files_for_zip: &[PathBuf],
    cancel_token: Arc<AtomicBool>,
) -> Result<(), AppError> {
    // Determine the optimal thread count
    let max_threads = get_optimal_thread_count() as usize;
    let semaphore = Arc::new(Semaphore::new(max_threads));

    // Create a vector to collect all compression tasks
    let mut compression_tasks = Vec::new();

    // Process model directories
    for path in group_and_model_dirs {
        // Skip if cancelled
        if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::UserCancelled(
                "Compression cancelled by user".into(),
            ));
        }

        let path_clone = path.clone();
        let target_dir_clone = target_dir_path.to_path_buf();
        let extension_clone = extension.to_string();
        let progress_clone = Arc::clone(&progress_tracker);
        let semaphore_clone = Arc::clone(&semaphore);
        let cancel_clone = Arc::clone(&cancel_token);

        let task = tokio::spawn(async move {
            // Acquire a permit from the semaphore
            let _permit = semaphore_clone.acquire().await.map_err(|e| {
                AppError::ConfigError(format!("Semaphore acquisition error: {}", e))
            })?;

            // Check if cancelled
            if cancel_clone.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(AppError::UserCancelled(
                    "Compression cancelled by user".into(),
                ));
            }

            let dir_name = path_clone
                .file_name()
                .ok_or_else(|| AppError::ConfigError("Invalid directory name".to_string()))?
                .to_string_lossy()
                .to_owned();

            let archive_path = target_dir_clone.join(format!("{}.{}", dir_name, extension_clone));

            // Execute compression in a blocking task
            tokio::task::spawn_blocking(move || -> Result<(), AppError> {
                // Check if cancelled before starting file
                if cancel_clone.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(AppError::UserCancelled(
                        "Compression cancelled by user".into(),
                    ));
                }

                let archive_file = File::create(&archive_path).map_err(|e| {
                    AppError::IoError(format!(
                        "Failed to create archive file '{}': {}",
                        archive_path.display(),
                        e
                    ))
                })?;

                let mut progress_guard = progress_clone.lock().map_err(|e| {
                    AppError::ConfigError(format!("Failed to lock progress state: {}", e))
                })?;

                compressors::compress_files(
                    &[path_clone.clone()],
                    archive_file,
                    Some(progress_guard.create_callback(&path_clone)),
                )
            })
            .await
            .map_err(|e| AppError::IoError(format!("Task panicked: {}", e)))?
        });

        compression_tasks.push(task);
    }

    // Compress release.3pk (similar pattern, extracted for brevity)
    if !files_for_3pk.is_empty() {
        compression_tasks.push(spawn_compression_task(
            files_for_3pk.to_vec(),
            target_dir_path.join("release.3pk"),
            Arc::clone(&progress_tracker),
            Arc::clone(&semaphore),
            Arc::clone(&cancel_token),
        ));
    }

    // Compress release.zip
    if !files_for_zip.is_empty() {
        compression_tasks.push(spawn_compression_task(
            files_for_zip.to_vec(),
            target_dir_path.join(format!("release.{}", extension)),
            Arc::clone(&progress_tracker),
            Arc::clone(&semaphore),
            Arc::clone(&cancel_token),
        ));
    }

    // Wait for all compression tasks to complete
    for task in compression_tasks {
        // Check for cancellation before waiting for each task
        if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::UserCancelled(
                "Compression cancelled by user".into(),
            ));
        }

        task.await
            .map_err(|e| AppError::IoError(format!("Task join error: {}", e)))??;
    }

    Ok(())
}

fn spawn_compression_task(
    files: Vec<PathBuf>,
    output_path: PathBuf,
    progress_tracker: Arc<Mutex<ProgressTracker>>,
    semaphore: Arc<Semaphore>,
    cancel_token: Arc<AtomicBool>,
) -> tokio::task::JoinHandle<Result<(), AppError>> {
    tokio::spawn(async move {
        // Acquire a permit from the semaphore
        let _permit = semaphore
            .acquire()
            .await
            .map_err(|e| AppError::ConfigError(format!("Semaphore acquisition error: {}", e)))?;

        // Check if cancelled
        if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(AppError::UserCancelled(
                "Compression cancelled by user".into(),
            ));
        }

        tokio::task::spawn_blocking(move || -> Result<(), AppError> {
            // Check if cancelled before starting file
            if cancel_token.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(AppError::UserCancelled(
                    "Compression cancelled by user".into(),
                ));
            }

            let output_file = File::create(&output_path).map_err(|e| {
                AppError::IoError(format!(
                    "Failed to create archive file '{}': {}",
                    output_path.display(),
                    e
                ))
            })?;

            let mut progress_guard = progress_tracker.lock().map_err(|e| {
                AppError::ConfigError(format!("Failed to lock progress state: {}", e))
            })?;

            // Use the first file path for the current file name, or output path if empty
            let file_path = files
                .first()
                .cloned()
                .unwrap_or_else(|| output_path.clone());

            compressors::compress_files(
                &files,
                output_file,
                Some(progress_guard.create_callback(&file_path)),
            )
        })
        .await
        .map_err(|e| AppError::IoError(format!("Task panicked: {}", e)))?
    })
}
