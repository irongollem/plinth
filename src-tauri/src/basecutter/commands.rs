//! Tauri commands for the Base Cutter job pipeline. Thin: validation +
//! spawning here, the actual child-process/stdout-parsing lives in job.rs
//! (kept process-free-testable per docs/BASECUTTER.md phase 3). Mirrors
//! render/commands.rs's start_render/cancel_render shape.

use crate::basecutter::job::{self, BaseCutJob, BaseCutToken};
use crate::error::AppError;
use crate::models::events::{
    BaseCutCutDoneStatus, BaseCutCutFailedStatus, BaseCutCutStartedStatus, BaseCutFailedStatus,
    BaseCutFinishedStatus, BaseCutStartedStatus, BaseCutStatus, BaseCutValidatedStatus,
    BaseCutValidatingStatus, BaseCutValidationReport,
};
use crate::render::engine;
use once_cell::sync::Lazy;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Notify;
use uuid::Uuid;

/// The single running base-cut job, if any (id + its cancel token). Unlike
/// render's ACTIVE_RENDERS map, only one base-cut job may run at a time
/// (docs/BASECUTTER.md "Job pipeline") — a plain Option is the simple guard
/// the doc calls for, no map needed.
static ACTIVE_BASE_CUT: Lazy<Mutex<Option<(String, Arc<Notify>)>>> = Lazy::new(|| Mutex::new(None));

#[tauri::command]
#[specta::specta]
pub async fn start_base_cut(app_handle: AppHandle, job: BaseCutJob) -> Result<String, AppError> {
    if job.placements.is_empty() {
        return Err(AppError::InvalidInput(
            "No placements in the base-cut job".to_string(),
        ));
    }
    if !Path::new(&job.landscape_path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("stl"))
    {
        return Err(AppError::InvalidInput(
            "The landscape must be an .stl file".to_string(),
        ));
    }
    if !Path::new(&job.landscape_path).is_file() {
        return Err(AppError::NotFoundError(format!(
            "Landscape not found: {}",
            job.landscape_path
        )));
    }

    let blender = engine::detect_blender_cached().await?;
    let script = job::materialize_base_cut_script(&app_handle)?;

    let job_id = Uuid::new_v4().to_string();
    let cancel_token = Arc::new(Notify::new());
    // Check-and-claim under ONE lock: a separate is-active check would let
    // two concurrent calls both pass it and spawn two Blender jobs.
    {
        let mut active = ACTIVE_BASE_CUT.lock().map_err(|e| {
            AppError::ConfigError(format!("Failed to access base-cut registry: {}", e))
        })?;
        if active.is_some() {
            return Err(AppError::InvalidInput(
                "A base-cut job is already running".to_string(),
            ));
        }
        *active = Some((job_id.clone(), Arc::clone(&cancel_token)));
    }

    let total = job.placements.len() as u32;
    BaseCutStatus::Started(BaseCutStartedStatus {
        job_id: job_id.clone(),
        total,
    })
    .emit(&app_handle)
    .ok();

    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        run_base_cut_job(app_handle, job_id_clone, blender, script, job, cancel_token).await;
    });

    Ok(job_id)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_base_cut(job_id: String) -> Result<(), AppError> {
    let active = ACTIVE_BASE_CUT
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Failed to access base-cut registry: {}", e)))?;
    match active.as_ref() {
        Some((active_id, token)) if *active_id == job_id => {
            token.notify_waiters();
            Ok(())
        }
        _ => Err(AppError::NotFoundError(format!(
            "No active base-cut job with ID: {}",
            job_id
        ))),
    }
}

/// Drive one job through job::spawn_and_parse, translating each
/// BaseCutToken into a BaseCutStatus event as it arrives, then emit the
/// terminal Finished/Failed event. Cancellation surfaces through Failed too
/// (docs/BASECUTTER.md's BaseCutStatus shape has no separate Cancelled
/// variant — see models/events.rs's doc comment on the enum).
async fn run_base_cut_job(
    app_handle: AppHandle,
    job_id: String,
    blender: crate::models::BlenderInfo,
    script: std::path::PathBuf,
    job: BaseCutJob,
    cancel_token: Arc<Notify>,
) {
    let total = job.placements.len() as u32;
    let script_dir = script
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    let result = match job::write_job_file(&script_dir, &job, &job_id) {
        Ok(job_path) => {
            let app_handle_ref = &app_handle;
            let job_id_ref = &job_id;
            let outcome = job::spawn_and_parse(
                &blender,
                &script,
                &job_path,
                &cancel_token,
                |token| handle_token(app_handle_ref, job_id_ref, token),
            )
            .await;
            std::fs::remove_file(&job_path).ok();
            outcome
        }
        Err(e) => Err((e, String::new())),
    };

    if let Ok(mut active) = ACTIVE_BASE_CUT.lock() {
        if active.as_ref().is_some_and(|(id, _)| id == &job_id) {
            *active = None;
        }
    }

    match result {
        Ok(ok_count) => {
            BaseCutStatus::Finished(BaseCutFinishedStatus {
                job_id,
                ok_count,
                total,
            })
            .emit(&app_handle)
            .ok();
        }
        Err((e, stdout_tail)) => {
            BaseCutStatus::Failed(BaseCutFailedStatus {
                job_id,
                message: e.to_string(),
                stdout_tail,
            })
            .emit(&app_handle)
            .ok();
        }
    }
}

/// Translate one BaseCutToken into its BaseCutStatus event and emit it.
fn handle_token(app_handle: &AppHandle, job_id: &str, token: &BaseCutToken) {
    match token {
        BaseCutToken::Validating => {
            BaseCutStatus::Validating(BaseCutValidatingStatus {
                job_id: job_id.to_string(),
            })
            .emit(app_handle)
            .ok();
        }
        // ValidationFailed also flows into the terminal Failed event
        // (spawn_and_parse kills the child and returns an error for it) —
        // surfacing it here too gives the frontend the report immediately
        // rather than only the flattened error string.
        BaseCutToken::Validated(report) | BaseCutToken::ValidationFailed(report) => {
            let report: BaseCutValidationReport =
                serde_json::from_value(report.clone()).unwrap_or_default();
            BaseCutStatus::Validated(BaseCutValidatedStatus {
                job_id: job_id.to_string(),
                report,
            })
            .emit(app_handle)
            .ok();
        }
        BaseCutToken::CutStart { index } => {
            BaseCutStatus::CutStarted(BaseCutCutStartedStatus {
                job_id: job_id.to_string(),
                index: *index,
            })
            .emit(app_handle)
            .ok();
        }
        BaseCutToken::CutDone {
            index,
            out,
            dims_mm,
            manifold,
        } => {
            BaseCutStatus::CutDone(BaseCutCutDoneStatus {
                job_id: job_id.to_string(),
                index: *index,
                out_path: out.clone(),
                dims_mm: *dims_mm,
                manifold: *manifold,
            })
            .emit(app_handle)
            .ok();
        }
        BaseCutToken::CutFailed { index, reason } => {
            BaseCutStatus::CutFailed(BaseCutCutFailedStatus {
                job_id: job_id.to_string(),
                index: *index,
                reason: reason.clone(),
            })
            .emit(app_handle)
            .ok();
        }
        // JOB_DONE carries the authoritative ok/total counts, but the
        // Finished event is emitted once by run_base_cut_job after
        // spawn_and_parse returns (it also needs to know whether the run
        // errored) — nothing to do here.
        BaseCutToken::JobDone { .. } => {}
    }
}
