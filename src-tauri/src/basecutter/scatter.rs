//! Scatter job pipeline: embed scatter_landscape.py, build the job JSON,
//! spawn headless Blender, parse its stdout token protocol into
//! `ScatterStatus` events. See docs/SCATTER.md "Pinned interfaces" and
//! scatter_landscape.py's own docstring for the exact job JSON shape and
//! stdout protocol this file is the Rust side of.
//!
//! Mirrors the established pattern (this is the THIRD instance —
//! basecutter::job/commands for the cut pipeline, basecutter::generator for
//! the landscape bake, this for scatter): embedded script via
//! `include_str!` + `materialize_embedded_script`'s always-overwrite
//! materialization, `render::engine::run_blender_lines` as the process
//! harness, `--python-exit-code 1` so an uncaught script exception exits
//! Blender non-zero, a pure/process-free token parser, a check-and-claim
//! single-job guard under ONE lock, and `Notify::notify_one` for cancel
//! (permit semantics — see `cancel_scatter`'s doc comment).
//!
//! Shaped closer to `generator.rs` than to `job.rs`/`commands.rs`'s split:
//! scatter is "one script invocation, one job" (docs/SCATTER.md "The
//! architectural call: scatter is a LANDSCAPE TRANSFORMER"), not an N-item
//! batch with a mid-run validation-abort gate, so there's no reason to carry
//! two files' worth of indirection for it.

use crate::error::AppError;
use crate::models::BlenderInfo;
use crate::models::events::{
    ScatterCancelledStatus, ScatterFailedStatus, ScatterFinishedStatus, ScatterProgressStatus,
    ScatterStartedStatus, ScatterStatus,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::sync::Notify;
use uuid::Uuid;

/// The Blender script ships INSIDE the binary — same always-overwrite
/// materialization as base_cut.py/gen_landscape.py (see
/// engine::materialize_embedded_script for the stale-copy trap this avoids).
const SCATTER_SCRIPT: &str = include_str!("../../resources/scatter_landscape.py");

/// Write the embedded scatter script where Blender can read it. Always
/// overwrites, so the file on disk can never drift from the built app.
pub fn materialize_scatter_script(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    crate::render::engine::materialize_embedded_script(app_handle, "scatter_landscape.py", SCATTER_SCRIPT)
}

// ------------------------------------------------------------- piece types

/// A generated piece kind — the only source scatter can actually place
/// today (docs/SCATTER.md "Execution phases": bundled/user assets are S4).
/// Serializes lowercase ("pebble"/"rock") to match
/// scatter_landscape.py's `CANONICAL_MM` keys exactly.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
#[serde(rename_all = "lowercase")]
pub enum GeneratedPieceKind {
    Pebble,
    Rock,
}

/// One piece's source — externally tagged with NO `#[serde(tag = ...)]`
/// (Rust's default enum-with-struct-variants shape), matching
/// docs/SCATTER.md's pinned `PieceChoice.piece` shape verbatim:
/// `{"Generated": {"kind": "pebble"|"rock"}}` or `{"Asset": {"id": "..."}}`.
/// scatter_landscape.py's `validate_pieces` docstring calls this out by
/// name: "matches Rust's default serde derive (no #[serde(tag=...)])".
/// `Asset` is a recognized, well-formed part of the shape today even though
/// the script fails it gracefully (S4 not implemented yet) — see
/// `validate_pieces` in scatter_landscape.py.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
pub enum ScatterPieceSource {
    Generated { kind: GeneratedPieceKind },
    Asset { id: String },
}

fn default_weight() -> f64 {
    1.0
}

/// One entry in `ScatterParams.pieces`: a piece source plus its relative
/// pick weight (`scatter_landscape.py::pick_piece_kind` draws by weight from
/// the accepted, non-zero-weight entries). `weight` defaults to 1.0 when
/// omitted, mirroring the script's own `entry.get("weight", 1.0)`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
pub struct PieceChoice {
    pub piece: ScatterPieceSource,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

/// Where a bundled/user-library scatter asset lives, at scan time
/// (docs/SCATTER.md "Bundled assets" / "Scale anchor"). `footprint_mm` and
/// `height_mm` are measured once at curation/scan, not user input.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
#[serde(rename_all = "lowercase")]
pub enum ScatterAssetSource {
    Bundled,
    User,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
pub struct ScatterAsset {
    pub id: String,
    pub label: String,
    pub source: ScatterAssetSource,
    pub path: String,
    pub footprint_mm: f64,
    pub height_mm: f64,
}

/// Bundled scatter asset set — S4 work (docs/SCATTER.md "Execution phases":
/// curation from the scout list, manifold vetting, embedding, credits
/// panel). Returns an empty list for now so the frontend's piece picker can
/// wire up against the real command/return shape today (generated kinds
/// still work standalone) and light up automatically once curated assets
/// land, with no signature change needed.
///
/// `scan_scatter_library` (the user-library counterpart) is ALSO S4 and is
/// deliberately not stubbed here at all — see docs/SCATTER.md's phase list.
#[tauri::command]
#[specta::specta]
pub fn get_scatter_assets() -> Vec<ScatterAsset> {
    Vec::new()
}

// ----------------------------------------------------------------- params

fn default_scale() -> (f64, f64) {
    (0.85, 1.15)
}
fn default_scale_factor() -> f64 {
    1.0
}
fn default_sink_mm() -> (f64, f64) {
    (0.0, 0.6)
}
fn default_align_to_surface() -> bool {
    true
}
fn default_max_slope_deg() -> f64 {
    55.0
}
fn default_edge_margin_mm() -> f64 {
    2.0
}

/// Scatter placement parameters — see docs/SCATTER.md "Pinned interfaces"
/// and "Scale anchor: 28-32mm heroic". Defaults mirror
/// scatter_landscape.py's `scatter()`'s own `params.get(key, default)`
/// fallbacks exactly, so a partial JSON (from a preset or an older UI build)
/// behaves identically whether the default is applied here or in the
/// script. `seed`, `density_per_dm2`, and `pieces` have no script-side
/// default (missing keys raise `KeyError`/`ValueError` there), so they stay
/// required here too.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
pub struct ScatterParams {
    pub seed: u32,
    pub density_per_dm2: f64,
    #[serde(default = "default_scale")]
    pub scale: (f64, f64),
    #[serde(default = "default_scale_factor")]
    pub scale_factor: f64,
    #[serde(default = "default_sink_mm")]
    pub sink_mm: (f64, f64),
    #[serde(default = "default_align_to_surface")]
    pub align_to_surface: bool,
    #[serde(default = "default_max_slope_deg")]
    pub max_slope_deg: f64,
    #[serde(default = "default_edge_margin_mm")]
    pub edge_margin_mm: f64,
    pub pieces: Vec<PieceChoice>,
}

/// A scatter job, as sent from the frontend and forwarded to
/// scatter_landscape.py verbatim — unlike `BaseCutJob`, no field is renamed:
/// the script reads `job["landscape_path"]`, `job["out_path"]`,
/// `job["params"]` directly (see its module docstring's job JSON example).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Type)]
pub struct ScatterJob {
    pub landscape_path: String,
    pub out_path: String,
    pub params: ScatterParams,
}

// ------------------------------------------------------------ token parsing

/// One parsed line of scatter_landscape.py's stdout protocol (see its
/// docstring). Pure/process-free, same as `basecutter::job::parse_token` and
/// `generator::parse_landscape_token`, so the grammar is unit-testable
/// without spawning Blender. `SCATTER_PIECE` (the `--debug`-only per-piece
/// line) is deliberately not modeled here — S2 never passes `--debug`, so
/// the line never appears on this path.
#[derive(Debug, Clone, PartialEq)]
pub enum ScatterToken {
    Started,
    Progress { placed: u32, total: u32 },
    Done { out: String, placed: u32, manifold: bool },
    Failed { reason: String },
}

pub fn parse_scatter_token(line: &str) -> Option<ScatterToken> {
    #[derive(Deserialize)]
    struct ProgressPayload {
        placed: u32,
        total: u32,
    }
    #[derive(Deserialize)]
    struct DonePayload {
        out: String,
        placed: u32,
        manifold: bool,
    }
    #[derive(Deserialize)]
    struct FailedPayload {
        reason: String,
    }

    let line = line.trim();
    if line == "SCATTER_START" {
        return Some(ScatterToken::Started);
    }
    if let Some(json) = line.strip_prefix("SCATTER_PROGRESS ") {
        let p: ProgressPayload = serde_json::from_str(json).ok()?;
        return Some(ScatterToken::Progress {
            placed: p.placed,
            total: p.total,
        });
    }
    if let Some(json) = line.strip_prefix("SCATTER_DONE ") {
        let p: DonePayload = serde_json::from_str(json).ok()?;
        return Some(ScatterToken::Done {
            out: p.out,
            placed: p.placed,
            manifold: p.manifold,
        });
    }
    if let Some(json) = line.strip_prefix("SCATTER_FAILED ") {
        let p: FailedPayload = serde_json::from_str(json).ok()?;
        return Some(ScatterToken::Failed { reason: p.reason });
    }
    None
}

// ---------------------------------------------------------------- job file

/// Write the job JSON into `dir` (the materialized script's directory in
/// production; a scratch dir in tests) so Blender can read it via `--job`.
/// Unlike `job::write_job_file`, `ScatterJob` serializes directly to the
/// wire shape scatter_landscape.py expects — no derived-field injection.
pub fn write_job_file(dir: &Path, job: &ScatterJob, job_id: &str) -> Result<PathBuf, AppError> {
    let path = dir.join(format!("scatter_job_{job_id}.json"));
    let json = serde_json::to_string_pretty(job)
        .map_err(|e| AppError::JsonError(format!("Failed to encode scatter job: {}", e)))?;
    std::fs::write(&path, json)
        .map_err(|e| AppError::IoError(format!("Failed to write scatter job file: {}", e)))?;
    Ok(path)
}

/// Assemble the headless scatter invocation: `--background
/// --factory-startup --python-exit-code 1 --python <script> -- --job <json>`
/// — identical shape to `job::build_base_cut_command`, one job file rather
/// than per-run flags.
pub fn build_scatter_command(
    blender: &BlenderInfo,
    script: &Path,
    job_path: &Path,
) -> tokio::process::Command {
    let mut cmd = crate::render::engine::new_command(Path::new(&blender.path));
    cmd.arg("--background")
        .arg("--factory-startup")
        .arg("--python-exit-code")
        .arg("1")
        .arg("--python")
        .arg(script)
        .arg("--")
        .arg("--job")
        .arg(job_path);
    cmd
}

/// Spawn Blender against `job_path` and parse its stdout into
/// `ScatterToken`s, invoking `on_token` for each as it arrives. Returns the
/// `SCATTER_DONE` payload's fields on success, or `(error, stdout_tail)`.
///
/// Like `generator::spawn_and_parse` (and unlike `job::spawn_and_parse`'s
/// validation-abort gate), there is nothing to abort mid-run: one script
/// invocation makes exactly one decorated STL, so every token is handled the
/// same way regardless of content, and failure is entirely a non-zero exit
/// (the script's own `sys.exit(1)` after `SCATTER_FAILED`, or an uncaught
/// exception via `--python-exit-code 1`). The `Failed` token's `reason` is
/// captured so the error message can quote it instead of just "Blender
/// exited with exit status 1".
pub async fn spawn_and_parse<F>(
    blender: &BlenderInfo,
    script: &Path,
    job_path: &Path,
    cancel_token: &Notify,
    mut on_token: F,
) -> Result<(String, u32, bool), (AppError, String)>
where
    F: FnMut(&ScatterToken),
{
    let cmd = build_scatter_command(blender, script, job_path);
    let mut done: Option<(String, u32, bool)> = None;
    let mut failure_reason: Option<String> = None;

    let merge_tail = |out: String, err: String| {
        if err.is_empty() {
            out
        } else {
            format!("{}\n{}", out, err)
        }
    };

    let run_result = crate::render::engine::run_blender_lines(cmd, Some(cancel_token), |line| {
        if let Some(token) = parse_scatter_token(line) {
            match &token {
                ScatterToken::Done { out, placed, manifold } => {
                    done = Some((out.clone(), *placed, *manifold))
                }
                ScatterToken::Failed { reason } => failure_reason = Some(reason.clone()),
                ScatterToken::Started | ScatterToken::Progress { .. } => {}
            }
            on_token(&token);
        }
        ControlFlow::Continue(())
    })
    .await;

    use crate::render::engine::BlenderRunError::*;
    let run = match run_result {
        Ok(run) => run,
        Err(SpawnFailed(e)) => {
            return Err((
                AppError::IoError(format!("Failed to launch Blender: {}", e)),
                String::new(),
            ))
        }
        Err(StdoutCaptureFailed) => {
            return Err((
                AppError::IoError("Failed to capture Blender stdout".to_string()),
                String::new(),
            ))
        }
        Err(ReadFailed { source, stdout_tail, stderr_tail }) => {
            return Err((
                AppError::IoError(format!("Failed reading Blender output: {}", source)),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
        Err(WaitFailed { source, stdout_tail, stderr_tail }) => {
            return Err((
                AppError::IoError(format!("Failed waiting for Blender: {}", source)),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
        Err(Cancelled { stdout_tail, stderr_tail }) => {
            return Err((
                AppError::UserCancelled("Scatter job cancelled".to_string()),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
        Err(AbortedByCaller { stdout_tail, stderr_tail }) => {
            return Err((
                AppError::FileProcessingError("Scatter job aborted".to_string()),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
    };

    if !run.status.success() {
        let message = failure_reason
            .map(|reason| format!("Scatter failed: {}", reason))
            .unwrap_or_else(|| format!("Blender exited with {}", run.status));
        return Err((
            AppError::FileProcessingError(message),
            merge_tail(run.stdout_tail, run.stderr_tail),
        ));
    }

    done.ok_or_else(|| {
        (
            AppError::FileProcessingError(
                "Blender exited cleanly but never reported SCATTER_DONE".to_string(),
            ),
            merge_tail(run.stdout_tail, run.stderr_tail),
        )
    })
}

// -------------------------------------------------------------- validation

/// Input guards for `start_scatter`, split out as a plain function (no
/// `AppHandle`/Blender detection) so it's unit-testable without spawning a
/// job. Deliberately checks the same conditions scatter_landscape.py itself
/// would reject (bad density, bad scale range, no usable pieces) so the
/// error surfaces before a Blender launch instead of as an opaque
/// `SCATTER_FAILED`/non-zero-exit round trip.
pub fn validate_scatter_job(job: &ScatterJob) -> Result<(), AppError> {
    if !Path::new(&job.landscape_path).is_file() {
        return Err(AppError::NotFoundError(format!(
            "Landscape not found: {}",
            job.landscape_path
        )));
    }
    if let Some(parent) = Path::new(&job.out_path).parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(AppError::InvalidInput(format!(
                "Output directory does not exist: {}",
                parent.display()
            )));
        }
    }
    if job.params.density_per_dm2 <= 0.0 {
        return Err(AppError::InvalidInput(
            "density_per_dm2 must be > 0".to_string(),
        ));
    }
    let (scale_lo, scale_hi) = job.params.scale;
    if scale_lo <= 0.0 || scale_hi <= 0.0 || scale_lo > scale_hi {
        return Err(AppError::InvalidInput(format!(
            "scale range must be positive and ordered (min <= max), got ({}, {})",
            scale_lo, scale_hi
        )));
    }
    if job.params.pieces.is_empty() {
        return Err(AppError::InvalidInput(
            "params.pieces is empty — nothing to scatter".to_string(),
        ));
    }
    if !job.params.pieces.iter().any(|p| p.weight > 0.0) {
        return Err(AppError::InvalidInput(
            "every piece has weight <= 0 — nothing to scatter".to_string(),
        ));
    }
    Ok(())
}

// ------------------------------------------------------------------ guard

/// (job id, its cancel token) — the shape held by the active-job registry,
/// named so clippy's complex-type lint doesn't fire on the static below.
type ActiveJob = (String, Arc<Notify>);

/// The single running scatter job, if any — mirrors
/// `basecutter::commands::ACTIVE_BASE_CUT` /
/// `generator::ACTIVE_LANDSCAPE_GEN`: only one at a time, its own guard
/// (scatter is a distinct activity that merely shares the one Blender
/// process slot).
static ACTIVE_SCATTER: Lazy<Mutex<Option<ActiveJob>>> = Lazy::new(|| Mutex::new(None));

/// Atomically claim the single scatter slot under ONE lock — the fixed
/// pattern from the base-cut review: a separate is-active check before
/// claiming would let two concurrent calls both pass it and spawn two
/// Blender jobs. Extracted as its own function (taking the registry rather
/// than reaching for the static directly) so the race-free claim/release
/// behavior is unit-testable without a Tauri `AppHandle`.
fn claim_active_job(
    registry: &Mutex<Option<ActiveJob>>,
    job_id: &str,
    cancel_token: &Arc<Notify>,
) -> Result<(), AppError> {
    let mut active = registry
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Failed to access scatter registry: {}", e)))?;
    if active.is_some() {
        return Err(AppError::InvalidInput(
            "A scatter job is already running".to_string(),
        ));
    }
    *active = Some((job_id.to_string(), Arc::clone(cancel_token)));
    Ok(())
}

/// Release the slot, but only if it's still holding THIS job — a job that
/// already lost its slot (e.g. to a bug elsewhere) must not clobber a
/// different job's claim.
fn release_active_job(registry: &Mutex<Option<ActiveJob>>, job_id: &str) {
    if let Ok(mut active) = registry.lock() {
        if active.as_ref().is_some_and(|(id, _)| id == job_id) {
            *active = None;
        }
    }
}

// ---------------------------------------------------------------- commands

#[tauri::command]
#[specta::specta]
pub async fn start_scatter(app_handle: AppHandle, job: ScatterJob) -> Result<String, AppError> {
    validate_scatter_job(&job)?;

    let blender = crate::render::engine::detect_blender_cached().await?;
    let script = materialize_scatter_script(&app_handle)?;

    let job_id = Uuid::new_v4().to_string();
    let cancel_token = Arc::new(Notify::new());
    claim_active_job(&ACTIVE_SCATTER, &job_id, &cancel_token)?;

    ScatterStatus::Started(ScatterStartedStatus {
        job_id: job_id.clone(),
    })
    .emit(&app_handle)
    .ok();

    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        run_scatter_job(app_handle, job_id_clone, blender, script, job, cancel_token).await;
    });

    Ok(job_id)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_scatter(job_id: String) -> Result<(), AppError> {
    let active = ACTIVE_SCATTER
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Failed to access scatter registry: {}", e)))?;
    match active.as_ref() {
        Some((active_id, token)) if *active_id == job_id => {
            // notify_one(), not notify_waiters() — same reasoning as
            // basecutter::commands::cancel_base_cut: a cancel landing before
            // the spawned task reaches spawn_and_parse's select loop must
            // not be dropped on the floor. notify_one() stores a permit for
            // exactly this case.
            token.notify_one();
            Ok(())
        }
        _ => Err(AppError::NotFoundError(format!(
            "No active scatter job with ID: {}",
            job_id
        ))),
    }
}

/// Translate one ScatterToken into its ScatterStatus event and emit it.
/// `Started` is already emitted synchronously by `start_scatter` before the
/// child even spawns (so the frontend sees it immediately); `Done`/`Failed`
/// flow into the terminal Finished/Failed event `run_scatter_job` emits
/// after `spawn_and_parse` returns (it also needs to know whether the run
/// errored) — nothing to do here for either.
fn handle_token(app_handle: &AppHandle, job_id: &str, token: &ScatterToken) {
    if let ScatterToken::Progress { placed, total } = token {
        ScatterStatus::Progress(ScatterProgressStatus {
            job_id: job_id.to_string(),
            placed: *placed,
            total: *total,
        })
        .emit(app_handle)
        .ok();
    }
}

async fn run_scatter_job(
    app_handle: AppHandle,
    job_id: String,
    blender: BlenderInfo,
    script: PathBuf,
    job: ScatterJob,
    cancel_token: Arc<Notify>,
) {
    let script_dir = script
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let result = match write_job_file(&script_dir, &job, &job_id) {
        Ok(job_path) => {
            let app_handle_ref = &app_handle;
            let job_id_ref = &job_id;
            let outcome = spawn_and_parse(&blender, &script, &job_path, &cancel_token, |token| {
                handle_token(app_handle_ref, job_id_ref, token)
            })
            .await;
            std::fs::remove_file(&job_path).ok();
            outcome
        }
        Err(e) => Err((e, String::new())),
    };

    release_active_job(&ACTIVE_SCATTER, &job_id);

    match result {
        Ok((out_path, placed, manifold)) => {
            ScatterStatus::Finished(ScatterFinishedStatus {
                job_id,
                out_path,
                placed,
                manifold,
            })
            .emit(&app_handle)
            .ok();
        }
        Err((AppError::UserCancelled(_), _stdout_tail)) => {
            ScatterStatus::Cancelled(ScatterCancelledStatus { job_id })
                .emit(&app_handle)
                .ok();
        }
        Err((e, stdout_tail)) => {
            ScatterStatus::Failed(ScatterFailedStatus {
                job_id,
                message: e.to_string(),
                stdout_tail,
            })
            .emit(&app_handle)
            .ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pebble(weight: f64) -> PieceChoice {
        PieceChoice {
            piece: ScatterPieceSource::Generated {
                kind: GeneratedPieceKind::Pebble,
            },
            weight,
        }
    }

    fn valid_params() -> ScatterParams {
        ScatterParams {
            seed: 7,
            density_per_dm2: 25.0,
            scale: (0.85, 1.15),
            scale_factor: 1.0,
            sink_mm: (0.0, 0.6),
            align_to_surface: true,
            max_slope_deg: 55.0,
            edge_margin_mm: 3.0,
            pieces: vec![
                pebble(0.6),
                PieceChoice {
                    piece: ScatterPieceSource::Generated {
                        kind: GeneratedPieceKind::Rock,
                    },
                    weight: 0.4,
                },
            ],
        }
    }

    // ------------------------------------------------------ JSON shape --

    /// Pins the full job JSON against every key scatter_landscape.py's
    /// module docstring documents — the ground-truth shape check the task
    /// calls for.
    #[test]
    fn job_serializes_to_the_pinned_script_shape() {
        let job = ScatterJob {
            landscape_path: "/path/to/landscape.stl".to_string(),
            out_path: "/path/to/landscape-scattered.stl".to_string(),
            params: valid_params(),
        };
        let json = serde_json::to_value(&job).unwrap();

        assert_eq!(json["landscape_path"], "/path/to/landscape.stl");
        assert_eq!(json["out_path"], "/path/to/landscape-scattered.stl");
        assert!(json.get("landscape").is_none(), "must not use base_cut.py's renamed key");

        for key in [
            "seed",
            "density_per_dm2",
            "scale",
            "scale_factor",
            "sink_mm",
            "align_to_surface",
            "max_slope_deg",
            "edge_margin_mm",
            "pieces",
        ] {
            assert!(
                json["params"].get(key).is_some(),
                "params.{key} missing from wire JSON — scatter_landscape.py's docstring pins this key"
            );
        }
        assert_eq!(json["params"]["seed"], 7);
        assert_eq!(json["params"]["density_per_dm2"], 25.0);
        assert_eq!(json["params"]["scale"][0], 0.85);
        assert_eq!(json["params"]["scale"][1], 1.15);
        assert_eq!(json["params"]["sink_mm"][0], 0.0);
        assert_eq!(json["params"]["sink_mm"][1], 0.6);
        assert_eq!(json["params"]["align_to_surface"], true);
        assert_eq!(json["params"]["max_slope_deg"], 55.0);
        assert_eq!(json["params"]["edge_margin_mm"], 3.0);

        assert_eq!(json["params"]["pieces"][0]["piece"]["Generated"]["kind"], "pebble");
        assert_eq!(json["params"]["pieces"][0]["weight"], 0.6);
        assert_eq!(json["params"]["pieces"][1]["piece"]["Generated"]["kind"], "rock");
        assert_eq!(json["params"]["pieces"][1]["weight"], 0.4);

        let back: ScatterJob = serde_json::from_value(json).unwrap();
        assert_eq!(back.landscape_path, "/path/to/landscape.stl");
    }

    /// The `Asset` variant is a recognized, well-formed part of the pinned
    /// shape even though scatter_landscape.py fails it gracefully (S4 not
    /// implemented) — the wire shape itself must still round-trip cleanly.
    #[test]
    fn asset_piece_source_serializes_to_the_pinned_shape() {
        let choice = PieceChoice {
            piece: ScatterPieceSource::Asset {
                id: "skull-01".to_string(),
            },
            weight: 1.0,
        };
        let json = serde_json::to_value(&choice).unwrap();
        assert_eq!(json["piece"]["Asset"]["id"], "skull-01");
        assert!(json["piece"].get("Generated").is_none());

        let back: PieceChoice = serde_json::from_value(json).unwrap();
        assert_eq!(back, choice);
    }

    /// Every default here must match scatter_landscape.py's own
    /// `params.get(key, default)` fallbacks exactly — a JSON that omits an
    /// optional key must behave identically whether Rust or the script
    /// applies the default.
    #[test]
    fn params_defaults_match_the_scripts_own_fallbacks_when_omitted() {
        let json = serde_json::json!({
            "seed": 1,
            "density_per_dm2": 10.0,
            "pieces": [{"piece": {"Generated": {"kind": "pebble"}}}]
        });
        let params: ScatterParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.scale, (0.85, 1.15));
        assert_eq!(params.scale_factor, 1.0);
        assert_eq!(params.sink_mm, (0.0, 0.6));
        assert!(params.align_to_surface);
        assert_eq!(params.max_slope_deg, 55.0);
        assert_eq!(params.edge_margin_mm, 2.0);
        // entry.get("weight", 1.0)
        assert_eq!(params.pieces[0].weight, 1.0);
    }

    // ----------------------------------------------------- token parsing --

    #[test]
    fn parse_scatter_token_handles_every_token_type() {
        assert_eq!(parse_scatter_token("SCATTER_START"), Some(ScatterToken::Started));
        assert_eq!(
            parse_scatter_token(r#"SCATTER_PROGRESS {"placed": 3, "total": 10}"#),
            Some(ScatterToken::Progress { placed: 3, total: 10 })
        );
        assert_eq!(
            parse_scatter_token(
                r#"SCATTER_DONE {"out": "/l-scattered.stl", "placed": 10, "manifold": true}"#
            ),
            Some(ScatterToken::Done {
                out: "/l-scattered.stl".to_string(),
                placed: 10,
                manifold: true,
            })
        );
        assert_eq!(
            parse_scatter_token(r#"SCATTER_FAILED {"reason": "assets not supported yet (S4)"}"#),
            Some(ScatterToken::Failed {
                reason: "assets not supported yet (S4)".to_string(),
            })
        );
    }

    #[test]
    fn parse_scatter_token_rejects_garbage_and_blender_noise() {
        assert_eq!(parse_scatter_token(""), None);
        assert_eq!(parse_scatter_token("   "), None);
        assert_eq!(parse_scatter_token("SCATTER_START extra-garbage"), None);
        assert_eq!(parse_scatter_token("SCATTER_PROGRESS not-json"), None);
        assert_eq!(parse_scatter_token("random log line from Blender"), None);
        assert_eq!(
            parse_scatter_token("Blender 5.1.2 (hash abcdef1234 built 2025-01-01)"),
            None
        );
        // Missing a required field is not a token.
        assert_eq!(parse_scatter_token(r#"SCATTER_DONE {"out": "/l.stl"}"#), None);
        // The --debug-only per-piece line is not modeled — must not parse.
        assert_eq!(
            parse_scatter_token(r#"SCATTER_PIECE {"index": 0, "kind": "pebble"}"#),
            None
        );
    }

    // ------------------------------------------------------- command shape --

    #[test]
    fn scatter_command_has_expected_shape() {
        let blender = BlenderInfo {
            path: "/usr/bin/blender".to_string(),
            version: "Blender 5.1.2".to_string(),
        };
        let cmd = build_scatter_command(
            &blender,
            Path::new("/tmp/scatter_landscape.py"),
            Path::new("/tmp/job.json"),
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--background",
                "--factory-startup",
                "--python-exit-code",
                "1",
                "--python",
                "/tmp/scatter_landscape.py",
                "--",
                "--job",
                "/tmp/job.json",
            ]
        );
    }

    #[test]
    fn write_job_file_writes_readable_json() {
        let dir = std::env::temp_dir().join(format!("stlpack_scatter_unit_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let job = ScatterJob {
            landscape_path: "/l.stl".to_string(),
            out_path: "/out/l-scattered.stl".to_string(),
            params: valid_params(),
        };
        let path = write_job_file(&dir, &job, "abc123").unwrap();
        assert!(path.is_file());
        let contents = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["landscape_path"], "/l.stl");
        assert_eq!(value["out_path"], "/out/l-scattered.stl");
        std::fs::remove_dir_all(&dir).ok();
    }

    // --------------------------------------------------------- validation --

    #[test]
    fn validate_scatter_job_rejects_a_missing_landscape() {
        let job = ScatterJob {
            landscape_path: "/definitely/not/a/real/path.stl".to_string(),
            out_path: std::env::temp_dir().to_string_lossy().into_owned() + "/out.stl",
            params: valid_params(),
        };
        assert!(matches!(
            validate_scatter_job(&job),
            Err(AppError::NotFoundError(_))
        ));
    }

    #[test]
    fn validate_scatter_job_rejects_a_nonexistent_output_directory() {
        let dir = std::env::temp_dir().join(format!("stlpack_scatter_validate_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let landscape = dir.join("landscape.stl");
        std::fs::write(&landscape, b"not a real stl, just needs to exist").unwrap();

        let job = ScatterJob {
            landscape_path: landscape.to_string_lossy().into_owned(),
            out_path: dir
                .join("nonexistent-subdir")
                .join("out.stl")
                .to_string_lossy()
                .into_owned(),
            params: valid_params(),
        };
        assert!(matches!(
            validate_scatter_job(&job),
            Err(AppError::InvalidInput(_))
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_scatter_job_rejects_bad_density_and_scale_and_pieces() {
        let dir = std::env::temp_dir().join(format!("stlpack_scatter_validate2_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let landscape = dir.join("landscape.stl");
        std::fs::write(&landscape, b"not a real stl, just needs to exist").unwrap();
        let landscape_path = landscape.to_string_lossy().into_owned();
        let out_path = dir.join("out.stl").to_string_lossy().into_owned();

        let mut params = valid_params();
        params.density_per_dm2 = 0.0;
        let job = ScatterJob { landscape_path: landscape_path.clone(), out_path: out_path.clone(), params };
        assert!(matches!(validate_scatter_job(&job), Err(AppError::InvalidInput(_))));

        let mut params = valid_params();
        params.scale = (1.2, 0.8); // min > max
        let job = ScatterJob { landscape_path: landscape_path.clone(), out_path: out_path.clone(), params };
        assert!(matches!(validate_scatter_job(&job), Err(AppError::InvalidInput(_))));

        let mut params = valid_params();
        params.scale = (0.0, 1.15); // not > 0
        let job = ScatterJob { landscape_path: landscape_path.clone(), out_path: out_path.clone(), params };
        assert!(matches!(validate_scatter_job(&job), Err(AppError::InvalidInput(_))));

        let mut params = valid_params();
        params.pieces = vec![];
        let job = ScatterJob { landscape_path: landscape_path.clone(), out_path: out_path.clone(), params };
        assert!(matches!(validate_scatter_job(&job), Err(AppError::InvalidInput(_))));

        let mut params = valid_params();
        params.pieces = vec![pebble(0.0)];
        let job = ScatterJob { landscape_path: landscape_path.clone(), out_path: out_path.clone(), params };
        assert!(matches!(validate_scatter_job(&job), Err(AppError::InvalidInput(_))));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn validate_scatter_job_accepts_a_well_formed_job() {
        let dir = std::env::temp_dir().join(format!("stlpack_scatter_validate3_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let landscape = dir.join("landscape.stl");
        std::fs::write(&landscape, b"not a real stl, just needs to exist").unwrap();

        let job = ScatterJob {
            landscape_path: landscape.to_string_lossy().into_owned(),
            out_path: dir.join("out.stl").to_string_lossy().into_owned(),
            params: valid_params(),
        };
        assert!(validate_scatter_job(&job).is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    // -------------------------------------------------------------- guard --

    /// The check-and-claim race the base-cut review flagged: a claim must
    /// happen under the SAME lock acquisition as the is-active check, or two
    /// concurrent callers could both observe "empty" and both claim.
    /// Exercised directly against a fresh, non-global registry so it's
    /// deterministic and doesn't touch the shared ACTIVE_SCATTER static.
    #[test]
    fn claim_active_job_rejects_a_second_claim_while_one_is_active() {
        let registry: Mutex<Option<ActiveJob>> = Mutex::new(None);
        let token_a = Arc::new(Notify::new());
        claim_active_job(&registry, "job-a", &token_a).expect("first claim should succeed");

        let token_b = Arc::new(Notify::new());
        let err = claim_active_job(&registry, "job-b", &token_b)
            .expect_err("second claim while job-a is active must be rejected");
        assert!(matches!(err, AppError::InvalidInput(_)));

        release_active_job(&registry, "job-a");
        claim_active_job(&registry, "job-b", &token_b).expect("claim after release should succeed");
    }

    #[test]
    fn release_active_job_only_clears_its_own_claim() {
        let registry: Mutex<Option<ActiveJob>> = Mutex::new(None);
        let token = Arc::new(Notify::new());
        claim_active_job(&registry, "job-a", &token).unwrap();

        // Releasing a different job id must not clobber job-a's claim.
        release_active_job(&registry, "job-b");
        assert!(registry.lock().unwrap().is_some());

        release_active_job(&registry, "job-a");
        assert!(registry.lock().unwrap().is_none());
    }

    // --------------------------------------------------------------- misc --

    #[test]
    fn get_scatter_assets_returns_empty_until_s4() {
        assert!(get_scatter_assets().is_empty());
    }

    // ------------------------------------------------------- integration --

    /// End-to-end: generate a small watertight landscape with Blender itself
    /// (see basecutter::job's identical helper for why an imported/hand-
    /// authored mesh is avoided — junk meshes fake unrelated symptoms), run
    /// a scatter job through spawn_and_parse (NOT the tauri command layer),
    /// and assert Finished-shaped output: a decorated STL exists, is
    /// manifold, and placed at least one piece.
    ///
    /// Run with: cargo test -- --ignored scatters_end_to_end_with_real_blender
    #[tokio::test]
    #[ignore = "requires a local Blender install and ~30s"]
    async fn scatters_end_to_end_with_real_blender() {
        let blender = crate::render::engine::detect_blender()
            .await
            .expect("Blender not found — install it or set BLENDER_BIN");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/scatter_landscape.py");

        let dir = std::env::temp_dir().join(format!("stlpack_scatter_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let landscape = dir.join("landscape.stl");
        generate_test_landscape(&blender, &dir, &landscape).await;
        assert!(landscape.is_file(), "landscape generation failed");

        let out_path = dir.join("landscape-scattered.stl");
        let job = ScatterJob {
            landscape_path: landscape.to_string_lossy().into_owned(),
            out_path: out_path.to_string_lossy().into_owned(),
            params: ScatterParams {
                seed: 7,
                density_per_dm2: 8.0,
                scale: (0.85, 1.15),
                scale_factor: 1.0,
                sink_mm: (0.0, 0.6),
                align_to_surface: true,
                max_slope_deg: 55.0,
                edge_margin_mm: 3.0,
                pieces: vec![pebble(1.0)],
            },
        };
        let job_path = write_job_file(&dir, &job, "test-job").expect("write job file");

        let cancel_token = Notify::new();
        let mut tokens: Vec<ScatterToken> = Vec::new();
        let result = spawn_and_parse(&blender, &script, &job_path, &cancel_token, |token| {
            tokens.push(token.clone());
        })
        .await;

        let (out, placed, manifold) = match result {
            Ok(v) => v,
            Err((e, tail)) => panic!("scatter job failed: {e}\nstdout tail:\n{tail}"),
        };

        assert!(Path::new(&out).is_file(), "expected a scattered STL at {:?}", out);
        assert!(manifold, "scattered landscape is not manifold");
        assert!(
            placed > 0,
            "expected at least one piece placed on a ~40x40mm plate at density 8/dm2"
        );
        assert!(tokens.iter().any(|t| matches!(t, ScatterToken::Started)));
        assert!(tokens.iter().any(|t| matches!(t, ScatterToken::Progress { .. })));
        assert!(
            matches!(tokens.last(), Some(ScatterToken::Done { .. })),
            "expected the token sequence to end with SCATTER_DONE, got: {:?}",
            tokens
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Builds a small closed (watertight) bumpy blob with Blender itself —
    /// a subdivided cube with per-vertex jitter — and exports it as the
    /// landscape STL the end-to-end test scatters onto. Deliberately its own
    /// copy rather than a shared helper: basecutter::job's
    /// `generate_test_landscape` is private to that module's own test mod,
    /// and each embedded-script test suite builds its own throwaway fixture
    /// (same non-shared convention as the scripts themselves).
    async fn generate_test_landscape(blender: &BlenderInfo, dir: &Path, out: &Path) {
        let gen_script = dir.join("gen_landscape.py");
        let py = r#"
import bpy
import os
import random

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.mesh.primitive_cube_add(size=40)
obj = bpy.context.object
bpy.ops.object.mode_set(mode='EDIT')
bpy.ops.mesh.subdivide(number_cuts=4)
bpy.ops.object.mode_set(mode='OBJECT')

random.seed(7)
for v in obj.data.vertices:
    v.co.z += random.uniform(-2.0, 2.0)
obj.data.update()

bpy.ops.object.select_all(action='DESELECT')
obj.select_set(True)
bpy.context.view_layer.objects.active = obj
out_path = os.environ["STLPACK_TEST_LANDSCAPE_OUT"]
bpy.ops.wm.stl_export(filepath=out_path, export_selected_objects=True)
"#;
        std::fs::write(&gen_script, py).unwrap();

        let mut cmd = crate::render::engine::new_command(Path::new(&blender.path));
        cmd.arg("--background")
            .arg("--factory-startup")
            .arg("--python")
            .arg(&gen_script)
            .env("STLPACK_TEST_LANDSCAPE_OUT", out.to_string_lossy().into_owned());
        let output = cmd
            .output()
            .await
            .expect("failed to launch blender for landscape generation");
        assert!(
            output.status.success(),
            "landscape generation failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
