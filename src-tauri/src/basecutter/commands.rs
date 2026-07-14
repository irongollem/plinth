//! Tauri commands for the Base Cutter job pipeline. Thin: validation +
//! spawning here, the actual child-process/stdout-parsing lives in job.rs
//! (kept process-free-testable per docs/BASECUTTER.md phase 3). Mirrors
//! render/commands.rs's start_render/cancel_render shape.

use crate::basecutter::cutters::{top_face_of, CutterKind, Placement, PlinthParams};
use crate::basecutter::job::{self, BaseCutJob, BaseCutToken};
use crate::error::AppError;
use crate::models::events::{
    BaseCutCancelledStatus, BaseCutCutDoneStatus, BaseCutCutFailedStatus,
    BaseCutCutStartedStatus, BaseCutFailedStatus, BaseCutFinishedStatus, BaseCutStartedStatus,
    BaseCutStatus, BaseCutValidatedStatus, BaseCutValidatingStatus, BaseCutValidationReport,
};
use crate::render::engine;
use once_cell::sync::Lazy;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Notify;
use uuid::Uuid;

/// Below this, the plinth's own taper/height has eaten the entire cut
/// footprint (a degenerate or negative top face) — the cut would be a
/// sliver or nothing at all, so it's rejected before Blender ever runs
/// rather than surfacing as an inscrutable boolean failure mid-job.
const MIN_CUT_DIMENSION_MM: f64 = 1.0;

/// The smallest span of the derived cut footprint — the dimension the
/// taper inset shrinks fastest for non-square shapes (an oval's minor axis,
/// a rect's short side).
fn min_cut_dimension_mm(kind: &CutterKind) -> f64 {
    match kind {
        CutterKind::Circle { diameter_mm } => *diameter_mm,
        CutterKind::Ellipse { major_mm, minor_mm } => major_mm.min(*minor_mm),
        CutterKind::Rect { width_mm, depth_mm } => width_mm.min(*depth_mm),
    }
}

/// How a placement should be named in an error message: its own name if it
/// has one, else a 1-based ordinal (placements are user-facing, so
/// "placement 1" reads better than a 0-based index).
fn placement_label(placement: &Placement, index: usize) -> String {
    match &placement.name {
        Some(name) => format!("'{name}'"),
        None => format!("placement {}", index + 1),
    }
}

/// Input guards for `start_base_cut`, split out as a plain function (no
/// AppHandle/Blender detection) so both guards are unit-testable without
/// spawning a job:
///
/// - a placement whose derived cut footprint (`cutters::top_face_of`) has
///   any dimension at or under `MIN_CUT_DIMENSION_MM` is rejected — the
///   plinth taper/height has eaten the whole footprint, so the cut would
///   be degenerate;
/// - two placements sharing a (non-empty) name are rejected — base_cut.py
///   names each output STL after the placement, so a collision means one
///   cut silently overwrites the other's file.
pub fn validate_placements(placements: &[Placement], plinth: &PlinthParams) -> Result<(), AppError> {
    let mut seen_names: HashSet<&str> = HashSet::new();
    for (index, placement) in placements.iter().enumerate() {
        let cut = top_face_of(&placement.cutter, plinth);
        let dim = min_cut_dimension_mm(&cut);
        if dim <= MIN_CUT_DIMENSION_MM {
            return Err(AppError::InvalidInput(format!(
                "{}: the plinth taper/height eats the whole footprint (derived cut dimension {:.2}mm is at or under the {}mm minimum)",
                placement_label(placement, index),
                dim,
                MIN_CUT_DIMENSION_MM
            )));
        }
        if let Some(name) = &placement.name {
            if !seen_names.insert(name.as_str()) {
                return Err(AppError::InvalidInput(format!(
                    "Duplicate placement name '{name}' — two placements with the same name would overwrite each other's output STL"
                )));
            }
        }
    }
    Ok(())
}

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
    validate_placements(&job.placements, &job.plinth)?;

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
            // notify_one(), not notify_waiters(): notify_waiters() only
            // wakes a future that is ALREADY polling notified() and stores
            // no permit, so a cancel landing in the window between
            // start_base_cut spawning the job task and that task reaching
            // spawn_and_parse's select loop would be dropped on the floor.
            // notify_one() stores a permit for exactly this case — the
            // next notified().await resolves immediately instead of
            // hanging until Blender finishes on its own.
            token.notify_one();
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
/// terminal Finished/Cancelled/Failed event. A cancelled run comes back as
/// `Err((AppError::UserCancelled(_), _))` (spawn_and_parse's select loop),
/// which is matched out separately so the frontend sees Cancelled rather
/// than Failed — same distinction render/commands.rs::run_render_job makes.
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
        Err((AppError::UserCancelled(_), _stdout_tail)) => {
            BaseCutStatus::Cancelled(BaseCutCancelledStatus { job_id })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn placement(name: Option<&str>, cutter: CutterKind) -> Placement {
        Placement {
            cutter,
            x_mm: 0.0,
            y_mm: 0.0,
            rotation_deg: 0.0,
            magnet: None,
            name: name.map(str::to_string),
        }
    }

    #[test]
    fn accepts_a_normal_placement_set() {
        let placements = vec![
            placement(Some("round32"), CutterKind::Circle { diameter_mm: 32.0 }),
            placement(
                Some("square25"),
                CutterKind::Rect {
                    width_mm: 25.0,
                    depth_mm: 25.0,
                },
            ),
            placement(None, CutterKind::Circle { diameter_mm: 40.0 }),
        ];
        assert!(validate_placements(&placements, &PlinthParams::default()).is_ok());
    }

    #[test]
    fn rejects_a_footprint_the_taper_eats_entirely() {
        // A 2mm circle under the default plinth (inset ~= 0.991mm/side,
        // shrink ~= 1.983mm) derives to a footprint under the 1.0mm floor.
        let placements = vec![placement(
            Some("tiny"),
            CutterKind::Circle { diameter_mm: 2.0 },
        )];
        let err = validate_placements(&placements, &PlinthParams::default())
            .expect_err("a near-zero cut footprint must be rejected");
        let msg = err.to_string();
        assert!(msg.contains("'tiny'"), "error should name the placement: {msg}");
        assert!(
            msg.contains("taper") || msg.contains("footprint"),
            "error should explain why: {msg}"
        );
    }

    #[test]
    fn rejects_a_footprint_the_taper_eats_entirely_using_the_ordinal_when_unnamed() {
        let placements = vec![placement(None, CutterKind::Circle { diameter_mm: 2.0 })];
        let err = validate_placements(&placements, &PlinthParams::default())
            .expect_err("must still be rejected without a name");
        assert!(
            err.to_string().contains("placement 1"),
            "error should fall back to a 1-based ordinal: {err}"
        );
    }

    #[test]
    fn rejects_duplicate_placement_names() {
        let placements = vec![
            placement(Some("round32"), CutterKind::Circle { diameter_mm: 32.0 }),
            placement(Some("round32"), CutterKind::Circle { diameter_mm: 40.0 }),
        ];
        let err = validate_placements(&placements, &PlinthParams::default())
            .expect_err("duplicate names must be rejected");
        assert!(
            err.to_string().contains("round32"),
            "error should name the duplicate: {err}"
        );
    }

    #[test]
    fn allows_multiple_unnamed_placements() {
        // Unnamed placements get index-derived output names (base_0.stl,
        // base_1.stl, ...) in base_cut.py, so they never collide with each
        // other even though `name` is None for both.
        let placements = vec![
            placement(None, CutterKind::Circle { diameter_mm: 32.0 }),
            placement(None, CutterKind::Circle { diameter_mm: 32.0 }),
        ];
        assert!(validate_placements(&placements, &PlinthParams::default()).is_ok());
    }
}
