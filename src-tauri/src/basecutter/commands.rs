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
use std::path::{Path, PathBuf};
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

/// Windows reserved device names — case-insensitive, and reserved with or
/// without an extension (`NUL.stl` is just as unusable as `NUL`). The
/// userbase is mostly Windows (per docs), so this matters in practice, not
/// just in theory.
const WINDOWS_RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Whether `segment` is safe to use as a single bare filename component —
/// shared by `validate_placements` (placement.name -> `{name}.stl` in
/// base_cut.py's unique_out_path) and generator::start_landscape_generation
/// (preset_id -> `{slug}-{seed}.stl`). Both values cross the Tauri IPC
/// boundary from an untrusted frontend and land in an `os.path.join`/
/// `PathBuf::join` call, where an absolute path or `..` component escapes
/// the intended output directory rather than staying inside it — this is
/// the actual trust boundary, the frontend's own char-blocklist
/// (src/utils/placementName.ts) is not enough on its own.
///
/// Rejects: empty (after trim), any path separator (`/` or `\` — both, not
/// just the host OS's, since the userbase is mostly Windows regardless of
/// where the app happens to run), `.`/`..` or a `..` component, a
/// `Component::Prefix` (drive letter / UNC prefix), any of the Windows
/// forbidden characters `< > : " | ? *`, control characters, or a Windows
/// reserved device name (CON, NUL, COM1, ...).
pub(crate) fn is_safe_filename_segment(segment: &str) -> bool {
    let trimmed = segment.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return false;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return false;
    }
    const FORBIDDEN_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*'];
    if trimmed.chars().any(|c| FORBIDDEN_CHARS.contains(&c) || c.is_control()) {
        return false;
    }
    // Belt-and-braces: Path::components() catches anything the manual
    // checks above missed (e.g. a `Component::Prefix` a raw char scan
    // wouldn't recognize as one on a non-Windows build host).
    let component_count = Path::new(trimmed).components().count();
    if component_count != 1 || !matches!(Path::new(trimmed).components().next(), Some(std::path::Component::Normal(_)))
    {
        return false;
    }
    let base = trimmed.split('.').next().unwrap_or(trimmed);
    if WINDOWS_RESERVED_NAMES
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(base))
    {
        return false;
    }
    true
}

/// Rust-side sanity bound on `BaseCutJob.topper_mm` — deliberately looser
/// than base_cut.py's [1.0, 3.0] clamp (docs/BASECUTTER.md "Pinned
/// interfaces": "the script clamps 1..3 anyway — Rust just guards
/// nonsense"). This only rejects values that can't possibly be a sane
/// request (non-finite, zero/negative, or wildly oversized); the script
/// remains the single source of truth for the actual usable range and
/// echoes back a clamp in CUT_DONE when it adjusts the value.
const MAX_SANE_TOPPER_MM: f64 = 10.0;

/// Guard for `BaseCutJob.topper_mm`, split out as a plain function so it's
/// unit-testable without spawning a job (same shape as `validate_placements`
/// below). `None` (normal seat-on-plinth mode) always passes.
fn validate_topper_mm(topper_mm: Option<f64>) -> Result<(), AppError> {
    match topper_mm {
        None => Ok(()),
        Some(t) if !t.is_finite() => Err(AppError::InvalidInput(
            "topper_mm must be a finite number".to_string(),
        )),
        Some(t) if t <= 0.0 => Err(AppError::InvalidInput(format!(
            "topper_mm must be positive, got {t}"
        ))),
        Some(t) if t > MAX_SANE_TOPPER_MM => Err(AppError::InvalidInput(format!(
            "topper_mm {t} is unreasonably large (max {MAX_SANE_TOPPER_MM}mm) — base_cut.py clamps the usable range to 1.0-3.0mm anyway"
        ))),
        Some(_) => Ok(()),
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
///   cut silently overwrites the other's file;
/// - a placement.name that isn't a safe single filename segment is
///   rejected — it flows unsanitized into base_cut.py's `unique_out_path`
///   as `os.path.join(out_dir, f"{name}.stl")`, and this IPC command is the
///   real trust boundary (the frontend's char-blocklist in
///   src/utils/placementName.ts is not enough on its own): a caller of the
///   Tauri bridge can send any string, and a `..`/absolute/drive-prefixed
///   name would write the STL outside out_dir entirely.
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
            if !is_safe_filename_segment(name) {
                return Err(AppError::InvalidInput(format!(
                    "{}: name '{name}' is not a valid output filename — it must not contain a path separator, '..', a drive/UNC prefix, or the characters < > : \" | ? *",
                    placement_label(placement, index)
                )));
            }
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
    validate_topper_mm(job.topper_mm)?;

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
            fused,
            shells,
            topper_mm_clamped,
            magnet_ignored,
            glb,
        } => {
            BaseCutStatus::CutDone(BaseCutCutDoneStatus {
                job_id: job_id.to_string(),
                index: *index,
                out_path: out.clone(),
                dims_mm: *dims_mm,
                manifold: *manifold,
                fused: *fused,
                shells: *shells,
                topper_mm_clamped: *topper_mm_clamped,
                magnet_ignored: *magnet_ignored,
                glb_path: glb.clone(),
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

/// The pseudo-designer folder cut output lands under (docs/BASECUTTER.md
/// phase 5, "export-into-catalog"). A real studio name would misattribute
/// bases the user cut themselves; "Plinth Bases" reads as its own shelf and
/// — per catalog::layout's designer/release/model tiers — the group name
/// and cut date stay in SEPARATE segments (see export_cuts's doc comment
/// for why that separation is the whole reason this parses cleanly).
const PLINTH_DESIGNER: &str = "Plinth Bases";

/// Proleptic-Gregorian (year, month, day) from a Unix timestamp, UTC. No
/// date/time crate is in Cargo.toml (elsewhere in the codebase a raw
/// SystemTime/UNIX_EPOCH duration is the calendar-free norm, e.g.
/// catalog/pack.rs's packed_at) — this is Howard Hinnant's well-known
/// division-only civil-from-days conversion:
/// http://howardhinnant.github.io/date_algorithms.html
fn civil_from_unix_seconds(secs: u64) -> (i64, u32, u32) {
    let days = (secs / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

/// "This month" in UTC — stamps the export folder's release date with when
/// the copy actually happened.
fn current_year_month_utc() -> (i64, u32) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (year, month, _day) = civil_from_unix_seconds(secs);
    (year, month)
}

/// Copy a finished job's successful cuts into a catalog root, process-free
/// and settings-free so the folder-layout decision is unit-testable without
/// an AppHandle (same split as `validate_placements` above). The exported
/// tauri command below only adds the settings lookup and the current date.
///
/// Layout: `{root}/Plinth Bases/{YYYY-MM group_name}/{cut file stem}/file`.
/// This is deliberately THREE tiers deep, reusing catalog::layout::model_dir
/// verbatim (the exact function the release builder/normalizer write
/// through) rather than dropping files straight into the release folder:
/// scanner::infer_model_identity climbs from the leaf directory up and stops
/// naming the model at the FIRST non-generic, non-pose, non-support
/// segment — so if the date-bearing "YYYY-MM group_name" folder held the
/// files directly, that same segment would supply both the release date
/// AND the model's display name, baking "2026-07 " into every card's
/// title. A per-cut model folder underneath keeps the leaf segment
/// (the cut's own name) clean, and the date is still recovered one level up
/// by scanner::date_from_segment. See catalog/scanner.rs's
/// infer_model_identity and date_from_segment for the exact climb this
/// relies on.
///
/// Never a move: cut output stays local/catalog-bound per docs/BASECUTTER.md
/// "Risks" (licensing covers personal printing, not redistribution) — this
/// function only ever copies into a configured catalog root and has no path
/// into file::commands' release/share pipeline.
///
/// The per-cut folder is keyed on the cut's file stem, but the stem alone
/// is not a reliable identity: base_cut.py names an unnamed placement off
/// its index within ONE job's out_dir (unique_out_path), so two SEPARATE
/// job runs each independently start back at the same bare stem (a
/// 28.5mm round cutter cut in two different sessions is "round285.stl"
/// both times). Landing both in the same folder would silently merge two
/// unrelated bases into one catalog model with the second read as an
/// extra PART of the first — see `cut_dest_dir`, which only reuses an
/// existing per-cut folder when it already holds this exact cut (a true
/// re-export), and gives any other stem collision its own folder instead.
///
/// Each per-cut model folder also gets a minimal `model.json` sidecar naming
/// PLINTH_DESIGNER (see `write_export_model_json`): the scanner's designer
/// resolution is model.json designer -> release.json designer -> a
/// known-designers folder-name lexicon, in that order, and "Plinth Bases" is
/// in none of them — without the sidecar an export would scan back in as an
/// undesignered heuristic model instead of a Plinth Bases one.
pub fn export_cuts(
    paths: &[String],
    root: &str,
    group_name: &str,
    catalog_roots: &[String],
    year_month: (i64, u32),
) -> Result<String, AppError> {
    if paths.is_empty() {
        return Err(AppError::InvalidInput(
            "No cut STLs to export".to_string(),
        ));
    }
    let group_name = group_name.trim();
    if group_name.is_empty() {
        return Err(AppError::InvalidInput(
            "A group name is required".to_string(),
        ));
    }

    let root_norm = crate::catalog::commands::normalized_root(root);
    if !catalog_roots
        .iter()
        .any(|r| crate::catalog::commands::normalized_root(r) == root_norm)
    {
        return Err(AppError::InvalidInput(format!(
            "'{}' is not a configured catalog folder — add it in Settings first",
            root
        )));
    }
    let root_path = Path::new(&root_norm);
    if !root_path.is_dir() {
        return Err(AppError::NotFoundError(format!(
            "Catalog folder not found: {}",
            root_norm
        )));
    }

    // Exact-path duplicates in the input list are a caller bug (the same
    // cut named twice), distinct from a destination already holding a file
    // of the same name (handled below with a -N suffix, never an error).
    let mut seen: HashSet<&str> = HashSet::new();
    for path in paths {
        if !seen.insert(path.as_str()) {
            return Err(AppError::InvalidInput(format!(
                "'{}' is listed twice in the export",
                path
            )));
        }
    }

    // Validate every source up front so a missing file fails clearly before
    // any copying starts, rather than leaving a partial export behind.
    for path in paths {
        let source = Path::new(path);
        if !source.is_file() {
            return Err(AppError::NotFoundError(format!(
                "Cut STL not found: {}",
                path
            )));
        }
        if !source
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("stl"))
        {
            return Err(AppError::InvalidInput(format!(
                "Not an STL file: {}",
                path
            )));
        }
    }

    let date = format!("{:04}-{:02}", year_month.0, year_month.1);

    for path in paths {
        let source = Path::new(path);
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cut".to_string());
        let dest_dir = cut_dest_dir(root_path, group_name, &date, &stem, source);
        std::fs::create_dir_all(&dest_dir)?;
        // The sidecar's name must match the folder THIS cut actually landed
        // in, not the raw stem: cut_dest_dir may have pushed a stem-collided
        // cut to "{stem} 2" to keep it a separate model, and naming it
        // "{stem}" there would hand it the same group_name as the folder it
        // was disambiguated away from, undoing the split at the metadata
        // layer.
        let dir_stem = dest_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| stem.clone());
        let file_name = source
            .file_name()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("{stem}.stl")));
        let dest_file = crate::file::utils::unique_path(dest_dir.join(file_name));
        std::fs::copy(source, &dest_file).map_err(|e| {
            AppError::IoError(format!(
                "Failed to copy {} to {}: {}",
                path,
                dest_file.display(),
                e
            ))
        })?;
        // VTT GLB export design doc "Base cut": a glb-mode cut writes a
        // `.glb` twin right next to its STL (same stem). Copy it alongside
        // under the SAME naming the STL just landed under — `dest_file`
        // already carries whatever unique_path suffix it got, so the
        // sidecar mirrors that exactly rather than re-deriving its own.
        // Not every cut has one (glb:false jobs, or STLs that never came
        // from base_cut.py at all), so this is silently skipped when the
        // source doesn't carry a sidecar.
        let source_glb = source.with_extension("glb");
        if source_glb.is_file() {
            let dest_glb = dest_file.with_extension("glb");
            std::fs::copy(&source_glb, &dest_glb).map_err(|e| {
                AppError::IoError(format!(
                    "Failed to copy {} to {}: {}",
                    source_glb.display(),
                    dest_glb.display(),
                    e
                ))
            })?;
        }
        write_export_model_json(&dest_dir, &dir_stem)?;
    }

    let release_dir =
        crate::catalog::layout::release_dir(root_path, PLINTH_DESIGNER, group_name, Some(&date));
    Ok(release_dir.to_string_lossy().into_owned())
}

/// The per-cut model folder for `source`, disambiguated against a stem
/// collision with an UNRELATED cut (see export_cuts's doc comment for why
/// that's a real scenario, not a hypothetical one). A folder is reused
/// only when it already holds a model file byte-identical to `source` —
/// the genuine re-export/versioning case (`export_suffixes_instead_of_
/// overwriting_on_a_second_export`). Anything else occupying the stem is a
/// different base and gets pushed to "{stem} 2", "{stem} 3", ... until a
/// free or matching folder is found — mirroring normalize::numbered_name's
/// pattern, but at the folder tier instead of the file tier.
fn cut_dest_dir(root_path: &Path, group_name: &str, date: &str, stem: &str, source: &Path) -> PathBuf {
    let mut candidate_stem = stem.to_string();
    for n in 2.. {
        let dir = crate::catalog::layout::model_dir(
            root_path,
            PLINTH_DESIGNER,
            Some(group_name),
            Some(date),
            &candidate_stem,
        );
        if !dir.is_dir() || dir_holds_the_same_cut(&dir, source) {
            return dir;
        }
        candidate_stem = format!("{stem} {n}");
    }
    unreachable!("ran out of integers before a landing spot")
}

/// Whether `dir` already contains a model file with the exact bytes of
/// `source` — the signal that this folder is a re-export of the SAME cut
/// rather than a different one that happens to share a stem.
fn dir_holds_the_same_cut(dir: &Path, source: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"))
            && crate::catalog::normalize::same_content(&path, source)
    })
}

/// Write a minimal `model.json` naming PLINTH_DESIGNER into a per-cut model
/// folder, matching catalog::scanner::ModelJson's shape (`name` is its only
/// required field). Never overwrites an existing sidecar: a re-export into
/// the same folder (`file::utils::unique_path`'s -N suffix case, above) adds
/// a second STL beside the first, but the folder's designer/name were
/// already settled by the first export — and the user may have hand-edited
/// that sidecar since, which a blind rewrite here would silently discard.
fn write_export_model_json(dest_dir: &Path, stem: &str) -> Result<(), AppError> {
    let sidecar_path = dest_dir.join("model.json");
    if sidecar_path.exists() {
        return Ok(());
    }
    let sidecar = serde_json::json!({
        "name": stem,
        "designer": PLINTH_DESIGNER,
    });
    let contents = serde_json::to_string_pretty(&sidecar)
        .map_err(|e| AppError::ConfigError(format!("Failed to encode model.json: {}", e)))?;
    std::fs::write(&sidecar_path, contents).map_err(|e| {
        AppError::IoError(format!(
            "Failed to write {}: {}",
            sidecar_path.display(),
            e
        ))
    })
}

#[tauri::command]
#[specta::specta]
pub async fn export_cuts_to_catalog(
    app_handle: AppHandle,
    paths: Vec<String>,
    root: String,
    group_name: String,
) -> Result<String, AppError> {
    let settings = crate::settings::get_settings(app_handle.clone())
        .await
        .map_err(AppError::ConfigError)?;
    let catalog_roots = settings.catalog_roots.unwrap_or_default();
    let year_month = current_year_month_utc();
    tauri::async_runtime::spawn_blocking(move || {
        export_cuts(&paths, &root, &group_name, &catalog_roots, year_month)
    })
    .await
    .map_err(|e| AppError::IoError(format!("Export task panicked: {}", e)))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::scanner::ModelJson;

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

    // ---- validate_topper_mm (docs/BASECUTTER.md's BaseCutJob.topper_mm) ----

    #[test]
    fn topper_mm_none_is_always_fine() {
        assert!(validate_topper_mm(None).is_ok());
    }

    #[test]
    fn topper_mm_accepts_any_sane_positive_value() {
        // Rust's guard is deliberately looser than base_cut.py's [1.0, 3.0]
        // clamp — values outside that range are still valid REQUESTS, the
        // script just clamps and echoes back the adjustment.
        assert!(validate_topper_mm(Some(1.5)).is_ok());
        assert!(validate_topper_mm(Some(0.1)).is_ok());
        assert!(validate_topper_mm(Some(10.0)).is_ok());
    }

    #[test]
    fn topper_mm_rejects_zero_and_negative() {
        for bad in [0.0, -1.0, -0.001] {
            let err = validate_topper_mm(Some(bad)).expect_err("must reject non-positive");
            assert!(err.to_string().contains("positive"), "{err}");
        }
    }

    #[test]
    fn topper_mm_rejects_absurdly_large_values() {
        let err = validate_topper_mm(Some(10.001)).expect_err("must reject > 10mm");
        assert!(err.to_string().contains("unreasonably large"), "{err}");
    }

    #[test]
    fn topper_mm_rejects_non_finite() {
        let err = validate_topper_mm(Some(f64::NAN)).expect_err("must reject NaN");
        assert!(err.to_string().contains("finite"), "{err}");
        let err = validate_topper_mm(Some(f64::INFINITY)).expect_err("must reject infinity");
        assert!(err.to_string().contains("finite"), "{err}");
    }

    // ---- placement.name path-traversal guard (IPC is the trust boundary:
    // a caller of the Tauri bridge can send any string, regardless of what
    // the frontend's char-blocklist would allow through a UI) ----

    #[test]
    fn rejects_placement_names_that_escape_out_dir() {
        for bad in ["../evil", "a/b", "a\\b", "C:evil", "/etc/passwd", "..", "."] {
            let placements = vec![placement(Some(bad), CutterKind::Circle { diameter_mm: 32.0 })];
            let err = validate_placements(&placements, &PlinthParams::default())
                .expect_err(&format!("'{bad}' must be rejected as a placement name"));
            assert!(
                err.to_string().contains("not a valid output filename"),
                "error should explain why '{bad}' was rejected: {err}"
            );
        }
    }

    #[test]
    fn accepts_an_ordinary_placement_name() {
        let placements = vec![placement(Some("round32"), CutterKind::Circle { diameter_mm: 32.0 })];
        assert!(validate_placements(&placements, &PlinthParams::default()).is_ok());
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

    // ---- export_cuts_to_catalog (docs/BASECUTTER.md phase 5) ----

    /// civil_from_unix_seconds pinned against `date -u -r <secs>` reference
    /// points (epoch, a Y2K leap day, a 2024 leap day, and a 2026 date) so a
    /// future edit to the conversion can't silently drift the calendar.
    #[test]
    fn civil_from_unix_seconds_matches_known_dates() {
        assert_eq!(civil_from_unix_seconds(0), (1970, 1, 1));
        assert_eq!(civil_from_unix_seconds(951_782_400), (2000, 2, 29));
        assert_eq!(civil_from_unix_seconds(1_709_164_800), (2024, 2, 29));
        assert_eq!(civil_from_unix_seconds(1_784_073_600), (2026, 7, 15));
        assert_eq!(civil_from_unix_seconds(1_767_225_600), (2026, 1, 1));
        assert_eq!(civil_from_unix_seconds(946_598_400), (1999, 12, 31));
    }

    /// A temp dir standing in for one catalog root, cleaned up on drop via
    /// its own Drop impl — the same manual-cleanup style scanner.rs's tests
    /// use (this crate has no tempfile dependency).
    struct TempRoot(PathBuf);
    impl TempRoot {
        fn new(label: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "stlpack_basecutter_export_{}_{}",
                label,
                std::process::id()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TempRoot {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }

    fn write_stub_stl(dir: &Path, name: &str) -> String {
        let path = dir.join(name);
        std::fs::write(&path, b"solid stub\nendsolid stub\n").unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Pins the folder shape this module deliberately chose: three tiers
    /// (Designer/Release/Model) so the release-date segment never has to
    /// double as the model's name segment — see export_cuts's doc comment.
    #[test]
    fn export_places_each_cut_under_its_own_model_folder() {
        let root = TempRoot::new("layout");
        let src = TempRoot::new("layout_src");
        let round = write_stub_stl(src.path(), "round32.stl");
        let square = write_stub_stl(src.path(), "square25.stl");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        let dest = export_cuts(
            &[round, square],
            &root.path().to_string_lossy(),
            "Test Regiment",
            &roots,
            (2026, 7),
        )
        .expect("export should succeed");

        assert_eq!(
            Path::new(&dest),
            root.path().join("Plinth Bases").join("2026-07 Test Regiment")
        );
        assert!(root
            .path()
            .join("Plinth Bases/2026-07 Test Regiment/round32/round32.stl")
            .is_file());
        assert!(root
            .path()
            .join("Plinth Bases/2026-07 Test Regiment/square25/square25.stl")
            .is_file());
    }

    // ---- .glb sidecar copy (VTT GLB export design doc "Base cut":
    // "commands.rs: export_cuts_to_catalog copies the .glb sidecar when it
    // exists next to a cut STL") ----

    /// A glb-mode cut's `.glb` twin (same stem, next to the STL — exactly
    /// what base_cut.py writes) rides along into the catalog under the
    /// same name the STL itself landed under.
    #[test]
    fn export_copies_a_glb_sidecar_when_present() {
        let root = TempRoot::new("glb_sidecar");
        let src = TempRoot::new("glb_sidecar_src");
        let stl = write_stub_stl(src.path(), "round32.stl");
        let glb_contents = b"glTF\x02\x00\x00\x00stub-binary-glb-payload";
        std::fs::write(src.path().join("round32.glb"), glb_contents).unwrap();
        let roots = vec![root.path().to_string_lossy().into_owned()];

        export_cuts(&[stl], &root.path().to_string_lossy(), "Group", &roots, (2026, 7))
            .expect("export should succeed");

        let dest_glb = root.path().join("Plinth Bases/2026-07 Group/round32/round32.glb");
        assert!(dest_glb.is_file(), "expected the .glb sidecar to be copied alongside the STL");
        assert_eq!(
            std::fs::read(&dest_glb).unwrap(),
            glb_contents,
            "the copied .glb must be byte-identical to the source sidecar"
        );
    }

    /// glb:false cuts (and any STL that never had a base_cut.py sidecar at
    /// all) must not spuriously grow a `.glb` file in the catalog.
    #[test]
    fn export_skips_glb_copy_when_no_sidecar_exists() {
        let root = TempRoot::new("no_glb_sidecar");
        let src = TempRoot::new("no_glb_sidecar_src");
        let stl = write_stub_stl(src.path(), "round32.stl");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        export_cuts(&[stl], &root.path().to_string_lossy(), "Group", &roots, (2026, 7))
            .expect("export should succeed");

        let dest_glb = root.path().join("Plinth Bases/2026-07 Group/round32/round32.glb");
        assert!(!dest_glb.is_file(), "no sidecar existed on disk, so none should have been copied");
    }

    /// The sidecar's destination name mirrors whatever unique_path suffix
    /// the STL itself got on a re-export — same collision handling, same
    /// stem, so a re-exported glb-mode cut's twin lands beside the
    /// -N-suffixed STL it belongs to, not the first export's.
    #[test]
    fn export_glb_sidecar_mirrors_the_stl_suffix_on_a_second_export() {
        let root = TempRoot::new("glb_sidecar_suffix");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        let src1 = TempRoot::new("glb_sidecar_suffix_src1");
        let first = write_stub_stl(src1.path(), "round32.stl");
        std::fs::write(src1.path().join("round32.glb"), b"first-glb").unwrap();
        export_cuts(&[first], &root.path().to_string_lossy(), "Group", &roots, (2026, 7)).unwrap();

        let src2 = TempRoot::new("glb_sidecar_suffix_src2");
        let second = write_stub_stl(src2.path(), "round32.stl");
        std::fs::write(src2.path().join("round32.glb"), b"second-glb").unwrap();
        export_cuts(&[second], &root.path().to_string_lossy(), "Group", &roots, (2026, 7)).unwrap();

        let model_dir = root.path().join("Plinth Bases/2026-07 Group/round32");
        assert!(model_dir.join("round32.glb").is_file());
        assert_eq!(std::fs::read(model_dir.join("round32.glb")).unwrap(), b"first-glb");
        assert!(
            model_dir.join("round32-1.glb").is_file(),
            "the second export's sidecar must mirror its STL's -1 suffix"
        );
        assert_eq!(std::fs::read(model_dir.join("round32-1.glb")).unwrap(), b"second-glb");
    }

    /// The scanner's designer resolution is model.json designer ->
    /// release.json -> a known-designers folder lexicon, in that order, and
    /// "Plinth Bases" is in none of them — without a sidecar an export would
    /// scan back in as an undesignered heuristic model. Parses the written
    /// file into the scanner's own ModelJson type, not just a raw JSON blob,
    /// so this fails if the shape the scanner expects ever changes underfoot.
    #[test]
    fn export_writes_a_model_json_sidecar_the_scanner_can_read() {
        let root = TempRoot::new("sidecar");
        let src = TempRoot::new("sidecar_src");
        let round = write_stub_stl(src.path(), "round32.stl");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        export_cuts(
            &[round],
            &root.path().to_string_lossy(),
            "Test Regiment",
            &roots,
            (2026, 7),
        )
        .expect("export should succeed");

        let sidecar_path = root
            .path()
            .join("Plinth Bases/2026-07 Test Regiment/round32/model.json");
        let contents = std::fs::read_to_string(&sidecar_path).expect("sidecar written");
        let parsed: ModelJson =
            serde_json::from_str(&contents).expect("sidecar must parse as the scanner's ModelJson");
        assert_eq!(parsed.name, "round32");
        assert_eq!(parsed.designer.as_deref(), Some(PLINTH_DESIGNER));
    }

    #[test]
    fn export_rejects_a_root_not_in_catalog_roots() {
        let root = TempRoot::new("unconfigured");
        let src = TempRoot::new("unconfigured_src");
        let stl = write_stub_stl(src.path(), "round32.stl");

        let err = export_cuts(
            &[stl],
            &root.path().to_string_lossy(),
            "Group",
            &[], // nothing configured
            (2026, 7),
        )
        .expect_err("an unconfigured root must be rejected");
        assert!(
            err.to_string().contains("not a configured catalog folder"),
            "error should explain why: {err}"
        );
    }

    #[test]
    fn export_rejects_a_missing_source() {
        let root = TempRoot::new("missing_src");
        let roots = vec![root.path().to_string_lossy().into_owned()];
        let missing = root.path().join("nope.stl").to_string_lossy().into_owned();

        let err = export_cuts(&[missing.clone()], &root.path().to_string_lossy(), "Group", &roots, (2026, 7))
            .expect_err("a missing source must be rejected");
        assert!(
            err.to_string().contains(&missing),
            "error should name the missing source: {err}"
        );
    }

    #[test]
    fn export_rejects_the_same_path_listed_twice() {
        let root = TempRoot::new("dup_input");
        let src = TempRoot::new("dup_input_src");
        let stl = write_stub_stl(src.path(), "round32.stl");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        let err = export_cuts(
            &[stl.clone(), stl],
            &root.path().to_string_lossy(),
            "Group",
            &roots,
            (2026, 7),
        )
        .expect_err("the same source listed twice must be rejected");
        assert!(
            err.to_string().contains("listed twice"),
            "error should explain why: {err}"
        );
    }

    /// Re-exporting into the same group never overwrites the earlier copy —
    /// the file gets a -N suffix instead (file::utils::unique_path, shared
    /// with render/commands.rs), never silent data loss.
    #[test]
    fn export_suffixes_instead_of_overwriting_on_a_second_export() {
        let root = TempRoot::new("collision");
        let src = TempRoot::new("collision_src");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        let first = write_stub_stl(src.path(), "round32.stl");
        let dest1 = export_cuts(
            &[first],
            &root.path().to_string_lossy(),
            "Group",
            &roots,
            (2026, 7),
        )
        .unwrap();

        // A second cut that happens to produce the same file name (a fresh
        // temp dir, so this isn't the "same path twice" guard above).
        let second_src = TempRoot::new("collision_src2");
        let second = write_stub_stl(second_src.path(), "round32.stl");
        let dest2 = export_cuts(&[second], &root.path().to_string_lossy(), "Group", &roots, (2026, 7)).unwrap();

        assert_eq!(dest1, dest2, "same group -> same release dir");
        let model_dir = Path::new(&dest1).join("round32");
        assert!(model_dir.join("round32.stl").is_file());
        assert!(
            model_dir.join("round32-1.stl").is_file(),
            "the second export must land beside the first, not over it"
        );
    }

    /// Two DIFFERENT bases that happen to land on the same default stem
    /// ("round285" from a 28.5mm round cutter, cut in two separate job
    /// runs — each run's own out_dir independently starts base_cut.py's
    /// unique_out_path numbering back at the bare name) must NOT collapse
    /// into one catalog model. Before the fix, export_cuts keyed the
    /// per-cut folder on stem alone and reused it on any name collision,
    /// and write_export_model_json's never-clobber guard then froze the
    /// FIRST cut's sidecar over the folder — so the scanner read the
    /// second (unrelated) cut as a second FILE of the first cut's model,
    /// producing one card with the two bases as overlapping "parts"
    /// instead of two cards. Distinguishing content (not the byte-for-byte
    /// stub `export_suffixes_instead_of_overwriting_on_a_second_export`
    /// uses) is what makes this a genuinely different base, not a re-export.
    #[test]
    fn distinct_bases_sharing_a_default_stem_scan_as_separate_models() {
        let root = TempRoot::new("stem_collision");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        let src1 = TempRoot::new("stem_collision_src1");
        let first = src1.path().join("round285.stl");
        std::fs::write(&first, b"solid base one\nendsolid base one\n").unwrap();
        export_cuts(
            &[first.to_string_lossy().into_owned()],
            &root.path().to_string_lossy(),
            "Sunday Batch",
            &roots,
            (2026, 7),
        )
        .expect("first export should succeed");

        // A second, later job run: a DIFFERENT base, but base_cut.py's
        // per-job unique_out_path independently starts back at the bare
        // "round285.stl" name in this run's own out_dir.
        let src2 = TempRoot::new("stem_collision_src2");
        let second = src2.path().join("round285.stl");
        std::fs::write(&second, b"solid base two, totally different geometry\nendsolid base two\n")
            .unwrap();
        export_cuts(
            &[second.to_string_lossy().into_owned()],
            &root.path().to_string_lossy(),
            "Sunday Batch",
            &roots,
            (2026, 7),
        )
        .expect("second export should succeed");

        let cancel = std::sync::atomic::AtomicBool::new(false);
        let outcome =
            crate::catalog::scanner::scan(root.path(), &cancel, &[], |_, _| {}).unwrap();

        assert_eq!(
            outcome.models.len(),
            2,
            "two unrelated bases must scan as two models, not one multi-part model: {:?}",
            outcome
                .models
                .iter()
                .map(|m| (m.dir_path.clone(), m.file_count))
                .collect::<Vec<_>>()
        );
        for model in &outcome.models {
            assert_eq!(
                model.file_count, 1,
                "each base must own its files, not absorb a sibling cut's file"
            );
        }
    }

    /// A second export into an already-sidecar'd folder must not clobber a
    /// sidecar the user has since hand-edited — write_export_model_json
    /// checks existence first, unconditionally, every time.
    #[test]
    fn export_does_not_overwrite_an_existing_model_json_sidecar() {
        let root = TempRoot::new("sidecar_reexport");
        let src = TempRoot::new("sidecar_reexport_src");
        let roots = vec![root.path().to_string_lossy().into_owned()];

        let first = write_stub_stl(src.path(), "round32.stl");
        export_cuts(
            &[first],
            &root.path().to_string_lossy(),
            "Group",
            &roots,
            (2026, 7),
        )
        .unwrap();

        let sidecar_path = root
            .path()
            .join("Plinth Bases/2026-07 Group/round32/model.json");
        // simulate a user hand-edit landing between the two exports
        std::fs::write(&sidecar_path, r#"{"name":"Round 32 (renamed)"}"#).unwrap();

        let second_src = TempRoot::new("sidecar_reexport_src2");
        let second = write_stub_stl(second_src.path(), "round32.stl");
        export_cuts(&[second], &root.path().to_string_lossy(), "Group", &roots, (2026, 7)).unwrap();

        let contents = std::fs::read_to_string(&sidecar_path).unwrap();
        assert!(
            contents.contains("renamed"),
            "re-export must not clobber the user's hand-edited sidecar: {contents}"
        );
    }
}
