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
    db, dups, scanner, BatchOutcome, CatalogEntry, CatalogFile, CatalogGroupResult,
    CatalogSearchResult, CatalogStats, DuplicateGroup, FileVariant, MoveOperation, ReleaseSummary,
    TagCount,
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

    // Resolve the designer lexicon up front (async store read) so the
    // blocking scan can borrow it; fall back to the built-in defaults.
    let designers = crate::settings::get_settings(app_handle.clone())
        .await
        .ok()
        .and_then(|s| s.known_designers)
        .filter(|list| !list.is_empty())
        .unwrap_or_else(crate::settings::default_designers);

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
                scanner::scan(Path::new(&root), &cancel, &designers, |files_indexed, current_dir| {
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
pub async fn search_catalog_groups(
    app_handle: AppHandle,
    query: String,
    tags: Vec<String>,
    limit: u32,
    offset: u32,
) -> Result<CatalogGroupResult, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        let page = db::search_groups(&conn, &query, &tags, limit.min(200), offset)?;
        Ok(CatalogGroupResult {
            groups: page.groups,
            total: page.total,
        })
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Group search task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn get_catalog_group_members(
    app_handle: AppHandle,
    group_name: String,
) -> Result<Vec<CatalogEntry>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::group_members(&conn, &group_name)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Group member task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn rename_catalog_group(
    app_handle: AppHandle,
    group_name: String,
    new_name: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::rename_group(&conn, &group_name, &new_name)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Group rename task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn combine_catalog_groups(
    app_handle: AppHandle,
    group_names: Vec<String>,
    target_name: String,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = open_db(&app_handle)?;
        db::combine_groups(&mut conn, &group_names, &target_name)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Group combine task failed: {}", e)))?
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
    variant_key: Option<String>,
) -> Result<Vec<CatalogFile>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::model_files(&conn, &dir_path, variant_key.as_deref())
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
    custom_name: Option<String>,
    pose: Option<String>,
    scale: Option<String>,
    support_status: Option<String>,
    release_date: Option<String>,
    designer: Option<String>,
    sculptor: Option<String>,
    release_name: Option<String>,
) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::update_model_user_meta(
            &conn,
            &dir_path,
            custom_name,
            pose,
            scale,
            support_status,
            release_date,
            designer,
            sculptor,
            release_name,
        )
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Metadata update failed: {}", e)))?
}

/// Assign files to a pose (with optional per-file support) so a dump
/// folder can be split into pose members. Metadata only — nothing moves on
/// disk. Returns the number of known files assigned.
#[tauri::command]
#[specta::specta]
pub async fn assign_files_to_pose(
    app_handle: AppHandle,
    paths: Vec<String>,
    pose: Option<String>,
    support_status: Option<String>,
) -> Result<u32, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = open_db(&app_handle)?;
        db::set_file_variants(&mut conn, &paths, pose, support_status)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Assign task failed: {}", e)))?
}

/// Revert files to plain members of their folder.
#[tauri::command]
#[specta::specta]
pub async fn clear_file_pose(app_handle: AppHandle, paths: Vec<String>) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::clear_file_variants(&conn, &paths)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Clear task failed: {}", e)))?
}

/// The pose assignments under a folder, for the split UI.
#[tauri::command]
#[specta::specta]
pub async fn get_file_variants(
    app_handle: AppHandle,
    dir_path: String,
) -> Result<Vec<FileVariant>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let conn = open_db(&app_handle)?;
        db::get_file_variants(&conn, &dir_path)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Assignment read task failed: {}", e)))?
}

/// Copy an image into the app's previews dir and point the model at it.
/// The copy (not a reference) is deliberate: render outputs and picked
/// images live wherever the user left them and may be cleaned up; the
/// catalog preview must not die with them. The filename is a stable
/// per-model hash plus a timestamp — a fresh URL each time, because the
/// webview caches aggressively by URL, with older copies swept first.
#[tauri::command]
#[specta::specta]
pub async fn set_model_preview(
    app_handle: AppHandle,
    dir_path: String,
    image_path: String,
) -> Result<String, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        if !Path::new(&image_path).is_file() {
            return Err(AppError::NotFoundError(format!(
                "Image not found: {}",
                image_path
            )));
        }
        let previews_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| AppError::ConfigError(format!("No app data dir: {}", e)))?
            .join("previews");
        std::fs::create_dir_all(&previews_dir)
            .map_err(|e| AppError::IoError(format!("Failed to create previews dir: {}", e)))?;

        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        dir_path.hash(&mut hasher);
        let prefix = format!("{:016x}", hasher.finish());

        if let Ok(entries) = std::fs::read_dir(&previews_dir) {
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with(&prefix) {
                    std::fs::remove_file(entry.path()).ok();
                }
            }
        }

        let extension = Path::new(&image_path)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_else(|| "png".to_string());
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let dest = previews_dir.join(format!("{}-{}.{}", prefix, stamp, extension));
        std::fs::copy(&image_path, &dest)
            .map_err(|e| AppError::IoError(format!("Failed to copy preview: {}", e)))?;

        let dest_str = dest.to_string_lossy().into_owned();
        let conn = open_db(&app_handle)?;
        db::set_model_preview(&conn, &dir_path, &dest_str)?;
        Ok(dest_str)
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Preview task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn delete_duplicate_files(
    app_handle: AppHandle,
    file_paths: Vec<String>,
) -> Result<BatchOutcome, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut removed: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();
        for path in file_paths {
            match std::fs::remove_file(&path) {
                Ok(()) => removed.push(path),
                // Already gone from disk still means gone from the catalog
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => removed.push(path),
                Err(e) => errors.push(format!("{}: {}", path, e)),
            }
        }
        // Prune the index too, or the duplicate groups keep showing the
        // deleted copies until the next full rescan
        if !removed.is_empty() {
            let mut conn = open_db(&app_handle)?;
            db::remove_files(&mut conn, &removed)?;
        }
        Ok(BatchOutcome {
            succeeded: removed.len() as u32,
            errors,
        })
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("File deletion task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn batch_move_models(
    app_handle: AppHandle,
    operations: Vec<MoveOperation>,
) -> Result<BatchOutcome, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut conn = open_db(&app_handle)?;
        let mut succeeded = 0u32;
        let mut errors: Vec<String> = Vec::new();

        for op in operations {
            let from_path = PathBuf::from(&op.from);
            let to_path = PathBuf::from(&op.to);

            if !from_path.exists() {
                errors.push(format!("Source not found: {}", op.from));
                continue;
            }
            // rename() onto an existing path is platform-dependent (may
            // clobber a file, may fail on a dir) — refuse up front instead
            if to_path.exists() {
                errors.push(format!("Destination already exists: {}", op.to));
                continue;
            }
            if let Some(parent) = to_path.parent() {
                if let Err(e) = std::fs::create_dir_all(parent) {
                    errors.push(format!("Failed to create parent dirs for {}: {}", op.to, e));
                    continue;
                }
            }
            if let Err(e) = std::fs::rename(&from_path, &to_path) {
                errors.push(format!("Failed to move {} to {}: {}", op.from, op.to, e));
                continue;
            }
            // Disk and index must move together: a stale dir_path drops the
            // model's user tags on the next rescan (see db::move_model)
            match db::move_model(&mut conn, &op.from, &op.to) {
                Ok(()) => succeeded += 1,
                Err(e) => errors.push(format!(
                    "Moved {} on disk but failed to update the catalog (rescan to fix): {}",
                    op.to, e
                )),
            }
        }

        Ok(BatchOutcome { succeeded, errors })
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Batch move task failed: {}", e)))?
}
