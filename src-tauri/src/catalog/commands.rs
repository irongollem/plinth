use crate::error::AppError;
use crate::models::events::{
    DuplicateCancelledStatus, DuplicateCompletedStatus, DuplicateFailedStatus,
    DuplicateProgressStatus, DuplicateStartedStatus, DuplicateStatus, ScanCancelledStatus,
    ScanCompletedStatus, ScanFailedStatus, ScanProgressStatus, ScanStartedStatus, ScanStatus,
};
use once_cell::sync::Lazy;
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use uuid::Uuid;

use super::{
    db, dups, scanner, CatalogFile, CatalogSearchResult, CatalogStats, DuplicateGroup,
    ReleaseSummary, TagCount,
};

/// Scan and duplicate jobs share one registry; both cancel through
/// cancel_catalog_job.
static ACTIVE_CATALOG_JOBS: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

fn db_path(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    Ok(app_handle
        .path()
        .app_data_dir()
        .map_err(|e| AppError::ConfigError(format!("No app data dir: {}", e)))?
        .join("catalog.db"))
}

fn open_db(app_handle: &AppHandle) -> Result<Connection, AppError> {
    db::open(&db_path(app_handle)?)
}

fn register_job(job_id: &str) -> Result<Arc<AtomicBool>, AppError> {
    let cancel = Arc::new(AtomicBool::new(false));
    ACTIVE_CATALOG_JOBS
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Job registry unavailable: {}", e)))?
        .insert(job_id.to_string(), Arc::clone(&cancel));
    Ok(cancel)
}

fn unregister_job(job_id: &str) {
    if let Ok(mut jobs) = ACTIVE_CATALOG_JOBS.lock() {
        jobs.remove(job_id);
    }
}

#[tauri::command]
#[specta::specta]
pub async fn start_catalog_scan(app_handle: AppHandle, root: String) -> Result<String, AppError> {
    if !Path::new(&root).is_dir() {
        return Err(AppError::NotFoundError(format!(
            "Catalog root '{}' is not a directory",
            root
        )));
    }

    let job_id = Uuid::new_v4().to_string();
    let cancel = register_job(&job_id)?;
    let job_id_clone = job_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        ScanStatus::Started(ScanStartedStatus {
            job_id: job_id_clone.clone(),
            root: root.clone(),
        })
        .emit(&app_handle)
        .ok();

        let result = (|| -> Result<(u32, u32), AppError> {
            let mut last_emit = Instant::now();
            let progress_app = app_handle.clone();
            let progress_job = job_id_clone.clone();
            let outcome =
                scanner::scan(Path::new(&root), &cancel, |files_indexed, current_dir| {
                    if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                        last_emit = Instant::now();
                        ScanStatus::Progress(ScanProgressStatus {
                            job_id: progress_job.clone(),
                            files_indexed,
                            current_dir: current_dir.to_string(),
                        })
                        .emit(&progress_app)
                        .ok();
                    }
                })?;

            let mut conn = open_db(&app_handle)?;
            db::replace_catalog(
                &mut conn,
                &outcome.files,
                &outcome.models,
                &outcome.metadata_tags,
            )?;
            Ok((outcome.files.len() as u32, outcome.models.len() as u32))
        })();

        unregister_job(&job_id_clone);
        match result {
            Ok((total_files, total_models)) => {
                ScanStatus::Completed(ScanCompletedStatus {
                    job_id: job_id_clone,
                    total_files,
                    total_models,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                })
                .emit(&app_handle)
                .ok();
            }
            Err(AppError::UserCancelled(_)) => {
                ScanStatus::Cancelled(ScanCancelledStatus {
                    job_id: job_id_clone,
                })
                .emit(&app_handle)
                .ok();
            }
            Err(e) => {
                ScanStatus::Failed(ScanFailedStatus {
                    job_id: job_id_clone,
                    error: e.to_string(),
                })
                .emit(&app_handle)
                .ok();
            }
        }
    });

    Ok(job_id)
}

#[tauri::command]
#[specta::specta]
pub async fn start_duplicate_scan(app_handle: AppHandle) -> Result<String, AppError> {
    let job_id = Uuid::new_v4().to_string();
    let cancel = register_job(&job_id)?;
    let job_id_clone = job_id.clone();

    tauri::async_runtime::spawn_blocking(move || {
        DuplicateStatus::Started(DuplicateStartedStatus {
            job_id: job_id_clone.clone(),
        })
        .emit(&app_handle)
        .ok();

        let result = (|| -> Result<Vec<DuplicateGroup>, AppError> {
            let conn = open_db(&app_handle)?;
            let mut last_emit = Instant::now();
            let progress_app = app_handle.clone();
            let progress_job = job_id_clone.clone();
            dups::find_duplicates(&conn, &cancel, |processed, total| {
                if last_emit.elapsed() >= PROGRESS_EMIT_INTERVAL {
                    last_emit = Instant::now();
                    DuplicateStatus::Progress(DuplicateProgressStatus {
                        job_id: progress_job.clone(),
                        processed,
                        total,
                    })
                    .emit(&progress_app)
                    .ok();
                }
            })
        })();

        unregister_job(&job_id_clone);
        match result {
            Ok(groups) => {
                let wasted: f64 = groups
                    .iter()
                    .map(|g| g.size_bytes * (g.paths.len().saturating_sub(1)) as f64)
                    .sum();
                DuplicateStatus::Completed(DuplicateCompletedStatus {
                    job_id: job_id_clone,
                    group_count: groups.len() as u32,
                    wasted_bytes: wasted,
                })
                .emit(&app_handle)
                .ok();
            }
            Err(AppError::UserCancelled(_)) => {
                DuplicateStatus::Cancelled(DuplicateCancelledStatus {
                    job_id: job_id_clone,
                })
                .emit(&app_handle)
                .ok();
            }
            Err(e) => {
                DuplicateStatus::Failed(DuplicateFailedStatus {
                    job_id: job_id_clone,
                    error: e.to_string(),
                })
                .emit(&app_handle)
                .ok();
            }
        }
    });

    Ok(job_id)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_catalog_job(job_id: String) -> Result<(), AppError> {
    let jobs = ACTIVE_CATALOG_JOBS
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Job registry unavailable: {}", e)))?;
    match jobs.get(&job_id) {
        Some(cancel) => {
            cancel.store(true, Ordering::SeqCst);
            Ok(())
        }
        None => Err(AppError::NotFoundError(format!(
            "No active catalog job with ID: {}",
            job_id
        ))),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn search_catalog(
    app_handle: AppHandle,
    query: String,
    tags: Vec<String>,
    limit: u32,
    offset: u32,
) -> Result<CatalogSearchResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        let page = db::search(&conn, &query, &tags, limit.min(200), offset)?;
        Ok(CatalogSearchResult {
            entries: page.entries,
            total: page.total,
        })
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Search task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_catalog_tags(app_handle: AppHandle) -> Result<Vec<TagCount>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        Ok(db::list_tags(&conn)?
            .into_iter()
            .map(|(tag, count)| TagCount { tag, count })
            .collect())
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Tag task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn add_catalog_tag(
    app_handle: AppHandle,
    dir_path: String,
    tag: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::add_tag(&conn, &dir_path, &tag)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Tag task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn remove_catalog_tag(
    app_handle: AppHandle,
    dir_path: String,
    tag: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::remove_tag(&conn, &dir_path, &tag)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Tag task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_catalog_model_files(
    app_handle: AppHandle,
    dir_path: String,
) -> Result<Vec<CatalogFile>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::model_files(&conn, &dir_path)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("File task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_catalog_stats(app_handle: AppHandle) -> Result<CatalogStats, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::stats(&conn)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Stats task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_duplicate_groups(app_handle: AppHandle) -> Result<Vec<DuplicateGroup>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::duplicate_groups(&conn)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Duplicate task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_catalog_releases(app_handle: AppHandle) -> Result<Vec<ReleaseSummary>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::list_releases(&conn)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Release listing task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn update_model_metadata(
    app_handle: AppHandle,
    dir_path: String,
    pose: Option<String>,
    scale: Option<String>,
    support_status: Option<String>,
    release_date: Option<String>,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::update_model_metadata(&conn, &dir_path, pose, scale, support_status, release_date)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Metadata update failed: {}", e)))?
}
