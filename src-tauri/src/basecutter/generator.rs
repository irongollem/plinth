//! Parametric landscape generator — see docs/BASECUTTER.md "The landscape
//! generator (phase 6)". Mirrors job.rs's shape (embedded script, job JSON
//! file, run_blender_lines harness, single-job guard, typed events) but for
//! a single-shot bake instead of an N-cut batch: one Blender launch produces
//! one heightfield STL, deterministic from a seed.
//!
//! PRESETS LIVE HERE, NOT IN THE SCRIPT: gen_landscape.py only knows
//! parameters (docs/BASECUTTER.md again) — a new preset is a new row in
//! `get_landscape_presets`, never a script change.

use crate::error::AppError;
use crate::models::BlenderInfo;
use crate::models::events::{
    LandscapeGenCancelledStatus, LandscapeGenFailedStatus, LandscapeGenFinishedStatus,
    LandscapeGenStartedStatus, LandscapeGenStatus,
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;
use tokio::sync::Notify;
use uuid::Uuid;

/// The Blender script ships INSIDE the binary — same always-overwrite
/// materialization as base_cut.py/render_mini.py (see
/// engine::materialize_embedded_script for the stale-copy trap this avoids).
const GEN_LANDSCAPE_SCRIPT: &str = include_str!("../../resources/gen_landscape.py");

/// Grid step floor (docs/BASECUTTER.md phase 6: "resolution_mm ... floor
/// 0.4") — pinned here too so a too-fine request from the frontend is
/// clamped before it ever reaches the script, not just inside it.
pub const MIN_RESOLUTION_MM: f64 = 0.4;

fn default_resolution_mm() -> f64 {
    0.75
}
fn default_carrier_mm() -> f64 {
    2.0
}
fn default_amount() -> f64 {
    1.0
}

/// Base terrain: stacked-octave noise, optionally ridged for sharp crests.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct NoiseLayer {
    #[serde(default)]
    pub enabled: bool,
    /// Frequency multiplier — bigger scale = smaller, more numerous
    /// features (matches Blender's own Noise Texture node convention).
    #[serde(default)]
    pub scale: f64,
    #[serde(default)]
    pub octaves: u32,
    /// abs/1-abs transform per octave for sharp mountain crests instead of
    /// rolling hills.
    #[serde(default)]
    pub ridged: bool,
    #[serde(default = "default_amount")]
    pub amount: f64,
}

impl Default for NoiseLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            scale: 0.05,
            octaves: 4,
            ridged: false,
            amount: 1.0,
        }
    }
}

/// Windswept sand: a directional sine wave, its phase distorted by a slow
/// noise field when `waviness` > 0 so ripples meander instead of ruling
/// dead-straight lines.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct RipplesLayer {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub wavelength_mm: f64,
    #[serde(default)]
    pub direction_deg: f64,
    #[serde(default = "default_amount")]
    pub amount: f64,
    #[serde(default)]
    pub waviness: f64,
}

impl Default for RipplesLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            wavelength_mm: 8.0,
            direction_deg: 0.0,
            amount: 1.0,
            waviness: 0.3,
        }
    }
}

/// Cobblestones: a hashed, jittered 2D Voronoi — domed stones separated by
/// a recessed mortar gap.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct StonesLayer {
    #[serde(default)]
    pub enabled: bool,
    pub cell_mm: f64,
    pub gap_mm: f64,
    /// 0 = flat-topped setts, 1 = fully rounded cobbles.
    pub dome: f64,
    /// Per-stone height variance (0 = every stone the same height).
    pub jitter: f64,
    #[serde(default = "default_amount")]
    pub amount: f64,
}

impl Default for StonesLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            cell_mm: 12.0,
            gap_mm: 1.2,
            dome: 0.6,
            jitter: 0.15,
            amount: 1.0,
        }
    }
}

/// N seeded gaussian bumps — loose rock/rubble, combined by max (not sum)
/// so overlapping boulders read as touching domes rather than a stacked
/// tower (see gen_landscape.py's _boulders_layer docstring).
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct BouldersLayer {
    #[serde(default)]
    pub enabled: bool,
    pub count: u32,
    pub min_mm: f64,
    pub max_mm: f64,
    #[serde(default = "default_amount")]
    pub amount: f64,
}

impl Default for BouldersLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            count: 6,
            min_mm: 8.0,
            max_mm: 20.0,
            amount: 1.0,
        }
    }
}

/// Lava/river channels: low-frequency noise, absolute-valued so its
/// zero-crossings become winding channel centerlines and its peaks become
/// raised crusted banks.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct FlowLayer {
    #[serde(default)]
    pub enabled: bool,
    pub channel_width_mm: f64,
    pub meander_scale: f64,
    /// Sharpens the channel-to-bank transition (a power curve), not an
    /// overall scale — `amount` is still the one knob every layer shares
    /// for that.
    pub bank_height: f64,
    #[serde(default = "default_amount")]
    pub amount: f64,
}

impl Default for FlowLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            channel_width_mm: 10.0,
            meander_scale: 0.3,
            bank_height: 1.0,
            amount: 1.0,
        }
    }
}

/// Parabolic crown across the plate's width — cobblestone streets are
/// highest at the centerline, sloping to the gutters at the edges.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct CamberLayer {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_amount")]
    pub amount: f64,
}

impl Default for CamberLayer {
    fn default() -> Self {
        Self {
            enabled: false,
            amount: 1.0,
        }
    }
}

/// All style layers, every one individually optional and summable. Field-
/// level `#[serde(default)]` (each layer type's own `Default`, which sets
/// `enabled: false`) means a preset JSON can omit whole layers outright.
#[derive(Serialize, Deserialize, Clone, Debug, Default, Type)]
pub struct LandscapeLayers {
    #[serde(default)]
    pub noise: NoiseLayer,
    #[serde(default)]
    pub ripples: RipplesLayer,
    #[serde(default)]
    pub stones: StonesLayer,
    #[serde(default)]
    pub boulders: BouldersLayer,
    #[serde(default)]
    pub flow: FlowLayer,
    #[serde(default)]
    pub camber: CamberLayer,
}

/// The frontend-facing (and preset-carried) parameter set. Deliberately has
/// no `out` field — `start_landscape_generation` derives the output path
/// (app data dir) and injects it into the wire JSON the same way
/// `job::job_json_with_cut_footprints` injects "cut" for base_cut.py.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct LandscapeParams {
    pub seed: u32,
    pub width_mm: f64,
    pub depth_mm: f64,
    #[serde(default = "default_resolution_mm")]
    pub resolution_mm: f64,
    #[serde(default = "default_carrier_mm")]
    pub carrier_mm: f64,
    pub relief_mm: f64,
    #[serde(default)]
    pub layers: LandscapeLayers,
}

/// A named, ready-to-generate parameter set (docs/BASECUTTER.md: "Presets
/// are parameter sets" — the cutter-library move again, a new terrain
/// style is a new row here, not a new pipeline).
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct GeneratorPreset {
    pub id: String,
    pub label: String,
    pub params: LandscapeParams,
}

/// The four seed presets (docs/BASECUTTER.md phase 6), tuned for a
/// 120x80mm plate by actually running gen_landscape.py and eyeballing the
/// bake (see the phase's verification renders) — not just numbers that
/// happened to compile.
///
/// - **cobblestone-street**: stones (12mm cells, tight 1.2mm mortar) +
///   camber (the street crown) + a little base noise so the plate isn't
///   perfectly flat between stones.
/// - **sandy**: directional ripples (windswept dune ridges) over soft,
///   low-amplitude rolling noise (the dune body itself).
/// - **rocky**: ridged noise (fine jagged detail) + a handful of chunky
///   boulders. The noise amount is kept LOW (0.18) and its scale HIGH
///   (0.1, i.e. small/fine features) relative to the boulders (amount 1.0,
///   14-30mm) — an earlier tuning pass at noise amount 0.35/scale 0.15 (a
///   similar wavelength to the boulders themselves) visually swamped the
///   boulder domes into the noise texture; separating the two features by
///   both amplitude AND frequency is what made the boulders read as
///   boulders again (see the phase's verification renders).
/// - **lava-flow**: the flow channel field + a light ridged-noise crust so
///   the banks aren't perfectly smooth.
pub fn seed_presets() -> Vec<GeneratorPreset> {
    vec![
        GeneratorPreset {
            id: "cobblestone-street".to_string(),
            label: "Cobblestone street".to_string(),
            params: LandscapeParams {
                seed: 1,
                width_mm: 120.0,
                depth_mm: 80.0,
                resolution_mm: 0.75,
                carrier_mm: 2.0,
                relief_mm: 3.0,
                layers: LandscapeLayers {
                    stones: StonesLayer {
                        enabled: true,
                        cell_mm: 12.0,
                        gap_mm: 1.2,
                        dome: 0.6,
                        jitter: 0.2,
                        amount: 1.0,
                    },
                    camber: CamberLayer {
                        enabled: true,
                        amount: 0.35,
                    },
                    noise: NoiseLayer {
                        enabled: true,
                        scale: 0.2,
                        octaves: 2,
                        ridged: false,
                        amount: 0.1,
                    },
                    ..Default::default()
                },
            },
        },
        GeneratorPreset {
            id: "sandy".to_string(),
            label: "Sandy dunes".to_string(),
            params: LandscapeParams {
                seed: 2,
                width_mm: 120.0,
                depth_mm: 80.0,
                resolution_mm: 0.75,
                carrier_mm: 2.0,
                relief_mm: 4.0,
                layers: LandscapeLayers {
                    ripples: RipplesLayer {
                        enabled: true,
                        wavelength_mm: 9.0,
                        direction_deg: 20.0,
                        amount: 1.0,
                        waviness: 0.5,
                    },
                    noise: NoiseLayer {
                        enabled: true,
                        scale: 0.025,
                        octaves: 3,
                        ridged: false,
                        amount: 0.5,
                    },
                    ..Default::default()
                },
            },
        },
        GeneratorPreset {
            id: "rocky".to_string(),
            label: "Rocky ground".to_string(),
            params: LandscapeParams {
                seed: 3,
                width_mm: 120.0,
                depth_mm: 80.0,
                resolution_mm: 0.75,
                carrier_mm: 2.0,
                relief_mm: 10.0,
                layers: LandscapeLayers {
                    noise: NoiseLayer {
                        enabled: true,
                        scale: 0.1,
                        octaves: 4,
                        ridged: true,
                        amount: 0.18,
                    },
                    boulders: BouldersLayer {
                        enabled: true,
                        count: 7,
                        min_mm: 14.0,
                        max_mm: 30.0,
                        amount: 1.0,
                    },
                    ..Default::default()
                },
            },
        },
        GeneratorPreset {
            id: "lava-flow".to_string(),
            label: "Lava flow".to_string(),
            params: LandscapeParams {
                seed: 4,
                width_mm: 120.0,
                depth_mm: 80.0,
                resolution_mm: 0.75,
                carrier_mm: 2.0,
                relief_mm: 8.0,
                layers: LandscapeLayers {
                    flow: FlowLayer {
                        enabled: true,
                        channel_width_mm: 14.0,
                        meander_scale: 0.35,
                        bank_height: 1.6,
                        amount: 1.0,
                    },
                    noise: NoiseLayer {
                        enabled: true,
                        scale: 0.09,
                        octaves: 3,
                        ridged: true,
                        amount: 0.25,
                    },
                    ..Default::default()
                },
            },
        },
    ]
}

#[tauri::command]
#[specta::specta]
pub fn get_landscape_presets() -> Vec<GeneratorPreset> {
    seed_presets()
}

/// Write the embedded generator script where Blender can read it. Always
/// overwrites, so the file on disk can never drift from the built app.
pub fn materialize_gen_landscape_script(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    crate::render::engine::materialize_embedded_script(
        app_handle,
        "gen_landscape.py",
        GEN_LANDSCAPE_SCRIPT,
    )
}

/// One parsed line of gen_landscape.py's stdout protocol (see its
/// docstring). Kept pure/process-free, same as basecutter::job's
/// `parse_token`, so the grammar is unit-testable without spawning Blender.
#[derive(Debug, Clone, PartialEq)]
pub enum LandscapeToken {
    Generating {
        seed: u32,
    },
    Generated {
        out: String,
        dims_mm: [f64; 3],
        verts: u32,
        manifold: bool,
    },
    GenerationFailed {
        reason: String,
    },
}

pub fn parse_landscape_token(line: &str) -> Option<LandscapeToken> {
    #[derive(Deserialize)]
    struct GeneratingPayload {
        seed: u32,
    }
    #[derive(Deserialize)]
    struct GeneratedPayload {
        out: String,
        dims_mm: [f64; 3],
        verts: u32,
        manifold: bool,
    }
    #[derive(Deserialize)]
    struct FailedPayload {
        reason: String,
    }

    let line = line.trim();
    if let Some(json) = line.strip_prefix("GENERATING ") {
        let p: GeneratingPayload = serde_json::from_str(json).ok()?;
        return Some(LandscapeToken::Generating { seed: p.seed });
    }
    if let Some(json) = line.strip_prefix("GENERATED ") {
        let p: GeneratedPayload = serde_json::from_str(json).ok()?;
        return Some(LandscapeToken::Generated {
            out: p.out,
            dims_mm: p.dims_mm,
            verts: p.verts,
            manifold: p.manifold,
        });
    }
    if let Some(json) = line.strip_prefix("GENERATION_FAILED ") {
        let p: FailedPayload = serde_json::from_str(json).ok()?;
        return Some(LandscapeToken::GenerationFailed { reason: p.reason });
    }
    None
}

/// Clamp resolution to the floor and inject the computed output path — the
/// wire JSON gen_landscape.py actually reads. Mirrors
/// `job::job_json_with_cut_footprints`'s "Rust derives, script consumes"
/// split.
fn params_json_with_out(params: &LandscapeParams, out_path: &Path) -> Result<serde_json::Value, AppError> {
    let mut clamped = params.clone();
    if clamped.resolution_mm < MIN_RESOLUTION_MM {
        clamped.resolution_mm = MIN_RESOLUTION_MM;
    }
    let mut value = serde_json::to_value(&clamped)
        .map_err(|e| AppError::JsonError(format!("Failed to encode landscape params: {}", e)))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "out".to_string(),
            serde_json::Value::String(out_path.to_string_lossy().into_owned()),
        );
    }
    Ok(value)
}

/// Write the job params JSON into `dir` so Blender can read it via
/// `--params`.
pub fn write_params_file(
    dir: &Path,
    params: &LandscapeParams,
    out_path: &Path,
    job_id: &str,
) -> Result<PathBuf, AppError> {
    let path = dir.join(format!("gen_landscape_params_{job_id}.json"));
    let value = params_json_with_out(params, out_path)?;
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| AppError::JsonError(format!("Failed to encode landscape params: {}", e)))?;
    std::fs::write(&path, json)
        .map_err(|e| AppError::IoError(format!("Failed to write landscape params file: {}", e)))?;
    Ok(path)
}

/// Assemble the headless generation invocation — same `--background
/// --factory-startup --python-exit-code 1 --python <script> --` convention
/// as base_cut.py/render_mini.py, `--params <json>` instead of `--job`.
pub fn build_gen_landscape_command(
    blender: &BlenderInfo,
    script: &Path,
    params_path: &Path,
) -> tokio::process::Command {
    let mut cmd = crate::render::engine::new_command(Path::new(&blender.path));
    cmd.arg("--background")
        .arg("--factory-startup")
        .arg("--python-exit-code")
        .arg("1")
        .arg("--python")
        .arg(script)
        .arg("--")
        .arg("--params")
        .arg(params_path);
    cmd
}

/// Spawn Blender against `params_path` and parse its stdout into
/// `LandscapeToken`s, invoking `on_token` for each as it arrives. Returns
/// the `Generated` payload's fields on success, or `(error, stdout_tail)`.
///
/// Unlike `job::spawn_and_parse`'s validation gate, there is nothing to
/// abort mid-run here — one script invocation makes exactly one mesh, so
/// every token is handled the same way regardless of content; failure is
/// entirely a non-zero exit (the script's own `sys.exit(1)` after
/// `GENERATION_FAILED`, or an uncaught exception via `--python-exit-code
/// 1`). The `GenerationFailed` token's `reason` is captured so the error
/// message can quote it instead of just "Blender exited with exit status 1".
pub async fn spawn_and_parse<F>(
    blender: &BlenderInfo,
    script: &Path,
    params_path: &Path,
    cancel_token: &Notify,
    mut on_token: F,
) -> Result<(String, [f64; 3], u32, bool), (AppError, String)>
where
    F: FnMut(&LandscapeToken),
{
    let cmd = build_gen_landscape_command(blender, script, params_path);
    let mut generated: Option<(String, [f64; 3], u32, bool)> = None;
    let mut failure_reason: Option<String> = None;

    let merge_tail = |out: String, err: String| {
        if err.is_empty() {
            out
        } else {
            format!("{}\n{}", out, err)
        }
    };

    let run_result = crate::render::engine::run_blender_lines(cmd, Some(cancel_token), |line| {
        if let Some(token) = parse_landscape_token(line) {
            match &token {
                LandscapeToken::Generated {
                    out,
                    dims_mm,
                    verts,
                    manifold,
                } => generated = Some((out.clone(), *dims_mm, *verts, *manifold)),
                LandscapeToken::GenerationFailed { reason } => {
                    failure_reason = Some(reason.clone())
                }
                LandscapeToken::Generating { .. } => {}
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
                AppError::UserCancelled("Landscape generation cancelled".to_string()),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
        Err(AbortedByCaller { stdout_tail, stderr_tail }) => {
            return Err((
                AppError::FileProcessingError("Landscape generation aborted".to_string()),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
    };

    if !run.status.success() {
        let message = failure_reason
            .map(|reason| format!("Landscape generation failed: {}", reason))
            .unwrap_or_else(|| format!("Blender exited with {}", run.status));
        return Err((
            AppError::FileProcessingError(message),
            merge_tail(run.stdout_tail, run.stderr_tail),
        ));
    }

    generated.ok_or_else(|| {
        (
            AppError::FileProcessingError(
                "Blender exited cleanly but never reported GENERATED".to_string(),
            ),
            merge_tail(run.stdout_tail, run.stderr_tail),
        )
    })
}

/// (job id, its cancel token) — the shape held by `ACTIVE_LANDSCAPE_GEN`,
/// named so clippy's complex-type lint doesn't fire on the static below.
type ActiveJob = (String, Arc<Notify>);

/// The single running landscape-generation job, if any — mirrors
/// basecutter::commands::ACTIVE_BASE_CUT (only one at a time; generation
/// and cutting are different activities but share the one Blender process
/// slot, so each gets its own simple guard rather than a shared map).
static ACTIVE_LANDSCAPE_GEN: Lazy<Mutex<Option<ActiveJob>>> = Lazy::new(|| Mutex::new(None));

fn landscape_output_dir(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| AppError::ConfigError(format!("No app data dir: {}", e)))?
        .join("landscapes");
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::IoError(format!("Failed to create landscapes dir: {}", e)))?;
    Ok(dir)
}

/// `preset_id` is the preset chip's id ("cobblestone-street", ...) when
/// starting from a chip, None for a from-scratch custom bake — only used to
/// name the output file (docs/BASECUTTER.md:
/// "landscapes/<preset-or-custom>-<seed>.stl").
#[tauri::command]
#[specta::specta]
pub async fn start_landscape_generation(
    app_handle: AppHandle,
    params: LandscapeParams,
    preset_id: Option<String>,
) -> Result<String, AppError> {
    if params.width_mm <= 0.0 || params.depth_mm <= 0.0 {
        return Err(AppError::InvalidInput(
            "Landscape width/depth must be positive".to_string(),
        ));
    }
    if params.relief_mm < 0.0 {
        return Err(AppError::InvalidInput(
            "Relief height must not be negative".to_string(),
        ));
    }

    let blender = crate::render::engine::detect_blender_cached().await?;
    let script = materialize_gen_landscape_script(&app_handle)?;

    let slug = preset_id.as_deref().unwrap_or("custom");
    let out_dir = landscape_output_dir(&app_handle)?;
    let out_path = out_dir.join(format!("{slug}-{}.stl", params.seed));

    let job_id = Uuid::new_v4().to_string();
    let cancel_token = Arc::new(Notify::new());
    {
        let mut active = ACTIVE_LANDSCAPE_GEN.lock().map_err(|e| {
            AppError::ConfigError(format!("Failed to access landscape-gen registry: {}", e))
        })?;
        if active.is_some() {
            return Err(AppError::InvalidInput(
                "A landscape generation job is already running".to_string(),
            ));
        }
        *active = Some((job_id.clone(), Arc::clone(&cancel_token)));
    }

    LandscapeGenStatus::Started(LandscapeGenStartedStatus {
        job_id: job_id.clone(),
        seed: params.seed,
    })
    .emit(&app_handle)
    .ok();

    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        run_landscape_gen_job(
            app_handle,
            job_id_clone,
            blender,
            script,
            params,
            out_path,
            cancel_token,
        )
        .await;
    });

    Ok(job_id)
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_landscape_generation(job_id: String) -> Result<(), AppError> {
    let active = ACTIVE_LANDSCAPE_GEN.lock().map_err(|e| {
        AppError::ConfigError(format!("Failed to access landscape-gen registry: {}", e))
    })?;
    match active.as_ref() {
        Some((active_id, token)) if *active_id == job_id => {
            // notify_one(), not notify_waiters() — same reasoning as
            // basecutter::commands::cancel_base_cut: a cancel landing before
            // the spawned task reaches spawn_and_parse's select loop must
            // not be dropped on the floor.
            token.notify_one();
            Ok(())
        }
        _ => Err(AppError::NotFoundError(format!(
            "No active landscape generation job with ID: {}",
            job_id
        ))),
    }
}

async fn run_landscape_gen_job(
    app_handle: AppHandle,
    job_id: String,
    blender: BlenderInfo,
    script: PathBuf,
    params: LandscapeParams,
    out_path: PathBuf,
    cancel_token: Arc<Notify>,
) {
    let script_dir = script
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let result = match write_params_file(&script_dir, &params, &out_path, &job_id) {
        Ok(params_path) => {
            let outcome = spawn_and_parse(&blender, &script, &params_path, &cancel_token, |_| {}).await;
            std::fs::remove_file(&params_path).ok();
            outcome
        }
        Err(e) => Err((e, String::new())),
    };

    if let Ok(mut active) = ACTIVE_LANDSCAPE_GEN.lock() {
        if active.as_ref().is_some_and(|(id, _)| id == &job_id) {
            *active = None;
        }
    }

    match result {
        Ok((out, dims_mm, _verts, manifold)) => {
            LandscapeGenStatus::Finished(LandscapeGenFinishedStatus {
                job_id,
                out_path: out,
                dims_mm,
                manifold,
            })
            .emit(&app_handle)
            .ok();
        }
        Err((AppError::UserCancelled(_), _stdout_tail)) => {
            LandscapeGenStatus::Cancelled(LandscapeGenCancelledStatus { job_id })
                .emit(&app_handle)
                .ok();
        }
        Err((e, stdout_tail)) => {
            LandscapeGenStatus::Failed(LandscapeGenFailedStatus {
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

    #[test]
    fn landscape_params_serialize_to_the_script_shape() {
        let params = LandscapeParams {
            seed: 7,
            width_mm: 120.0,
            depth_mm: 80.0,
            resolution_mm: 0.75,
            carrier_mm: 2.0,
            relief_mm: 6.0,
            layers: LandscapeLayers {
                noise: NoiseLayer {
                    enabled: true,
                    scale: 0.05,
                    octaves: 4,
                    ridged: false,
                    amount: 1.0,
                },
                ..Default::default()
            },
        };
        let json = serde_json::to_value(&params).unwrap();
        assert_eq!(json["seed"], 7);
        assert_eq!(json["width_mm"], 120.0);
        assert_eq!(json["layers"]["noise"]["enabled"], true);
        assert_eq!(json["layers"]["noise"]["scale"], 0.05);
        assert_eq!(json["layers"]["ripples"]["enabled"], false);
        assert_eq!(json["layers"]["stones"]["enabled"], false);
        assert_eq!(json["layers"]["boulders"]["enabled"], false);
        assert_eq!(json["layers"]["flow"]["enabled"], false);
        assert_eq!(json["layers"]["camber"]["enabled"], false);
        // No "out" on the frontend-facing type — that's injected only into
        // the wire JSON (see params_json_with_out / write_params_file).
        assert!(json.get("out").is_none());
    }

    #[test]
    fn a_preset_json_can_omit_disabled_layers_entirely() {
        let json = serde_json::json!({
            "seed": 1,
            "width_mm": 120.0,
            "depth_mm": 80.0,
            "relief_mm": 6.0,
            "layers": { "noise": { "enabled": true, "scale": 0.05, "octaves": 4, "amount": 1.0 } }
        });
        let params: LandscapeParams = serde_json::from_value(json).unwrap();
        assert_eq!(params.resolution_mm, default_resolution_mm());
        assert_eq!(params.carrier_mm, default_carrier_mm());
        assert!(params.layers.noise.enabled);
        assert!(!params.layers.stones.enabled);
        assert!(!params.layers.boulders.enabled);
    }

    #[test]
    fn params_json_with_out_injects_the_out_key_and_clamps_resolution() {
        let params = LandscapeParams {
            seed: 1,
            width_mm: 120.0,
            depth_mm: 80.0,
            resolution_mm: 0.1, // below the floor
            carrier_mm: 2.0,
            relief_mm: 6.0,
            layers: LandscapeLayers::default(),
        };
        let value = params_json_with_out(&params, Path::new("/out/landscape.stl")).unwrap();
        assert_eq!(value["out"], "/out/landscape.stl");
        assert_eq!(value["resolution_mm"], MIN_RESOLUTION_MM);
    }

    #[test]
    fn get_landscape_presets_has_the_four_seed_presets() {
        let presets = seed_presets();
        let ids: Vec<&str> = presets.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["cobblestone-street", "sandy", "rocky", "lava-flow"]
        );
        for preset in &presets {
            assert!(preset.params.width_mm > 0.0);
            assert!(preset.params.depth_mm > 0.0);
            assert!(preset.params.relief_mm > 0.0);
        }
    }

    #[test]
    fn cobblestone_preset_enables_stones_and_camber() {
        let preset = seed_presets()
            .into_iter()
            .find(|p| p.id == "cobblestone-street")
            .unwrap();
        assert!(preset.params.layers.stones.enabled);
        assert!(preset.params.layers.camber.enabled);
        assert!(!preset.params.layers.boulders.enabled);
    }

    #[test]
    fn sandy_preset_enables_ripples() {
        let preset = seed_presets()
            .into_iter()
            .find(|p| p.id == "sandy")
            .unwrap();
        assert!(preset.params.layers.ripples.enabled);
        assert!(!preset.params.layers.stones.enabled);
    }

    #[test]
    fn rocky_preset_enables_ridged_noise_and_boulders() {
        let preset = seed_presets()
            .into_iter()
            .find(|p| p.id == "rocky")
            .unwrap();
        assert!(preset.params.layers.noise.enabled);
        assert!(preset.params.layers.noise.ridged);
        assert!(preset.params.layers.boulders.enabled);
    }

    #[test]
    fn lava_flow_preset_enables_flow() {
        let preset = seed_presets()
            .into_iter()
            .find(|p| p.id == "lava-flow")
            .unwrap();
        assert!(preset.params.layers.flow.enabled);
    }

    #[test]
    fn get_landscape_presets_matches_seed_presets() {
        assert_eq!(
            get_landscape_presets().len(),
            seed_presets().len()
        );
    }

    #[test]
    fn parse_landscape_token_handles_every_token_type() {
        assert_eq!(
            parse_landscape_token(r#"GENERATING {"seed": 7}"#),
            Some(LandscapeToken::Generating { seed: 7 })
        );
        assert_eq!(
            parse_landscape_token(
                r#"GENERATED {"out": "/l.stl", "dims_mm": [120.0, 80.0, 8.0], "verts": 100, "manifold": true}"#
            ),
            Some(LandscapeToken::Generated {
                out: "/l.stl".to_string(),
                dims_mm: [120.0, 80.0, 8.0],
                verts: 100,
                manifold: true,
            })
        );
        assert_eq!(
            parse_landscape_token(r#"GENERATION_FAILED {"reason": "bad params"}"#),
            Some(LandscapeToken::GenerationFailed {
                reason: "bad params".to_string(),
            })
        );
    }

    #[test]
    fn parse_landscape_token_rejects_garbage_and_blender_noise() {
        assert_eq!(parse_landscape_token(""), None);
        assert_eq!(parse_landscape_token("   "), None);
        assert_eq!(parse_landscape_token("GENERATING not-json"), None);
        assert_eq!(parse_landscape_token("random log line from Blender"), None);
        assert_eq!(
            parse_landscape_token("Blender 5.1.2 (hash abcdef1234 built 2025-01-01)"),
            None
        );
        assert_eq!(parse_landscape_token(r#"GENERATED {"out": "/l.stl"}"#), None);
    }

    #[test]
    fn gen_landscape_command_has_expected_shape() {
        let blender = BlenderInfo {
            path: "/usr/bin/blender".to_string(),
            version: "Blender 5.1.2".to_string(),
        };
        let cmd = build_gen_landscape_command(
            &blender,
            Path::new("/tmp/gen_landscape.py"),
            Path::new("/tmp/params.json"),
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
                "/tmp/gen_landscape.py",
                "--",
                "--params",
                "/tmp/params.json",
            ]
        );
    }

    #[test]
    fn write_params_file_writes_readable_json_with_out() {
        let dir = std::env::temp_dir().join(format!("stlpack_landscape_unit_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let params = LandscapeParams {
            seed: 5,
            width_mm: 120.0,
            depth_mm: 80.0,
            resolution_mm: 0.75,
            carrier_mm: 2.0,
            relief_mm: 6.0,
            layers: LandscapeLayers::default(),
        };
        let out_path = dir.join("landscape.stl");
        let path = write_params_file(&dir, &params, &out_path, "abc123").unwrap();
        assert!(path.is_file());
        let contents = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["seed"], 5);
        assert_eq!(value["out"], out_path.to_string_lossy().into_owned());
        std::fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------- integration --

    /// End-to-end: bake every seed preset for real through
    /// spawn_and_parse (NOT the tauri command layer), assert each comes
    /// back GENERATED + manifold, and print dims/verts so a human can
    /// sanity-check the numbers alongside the rendered PNGs from the
    /// phase's verification step.
    ///
    /// Run with: cargo test -- --ignored bakes_every_seed_preset
    #[tokio::test]
    #[ignore = "requires a local Blender install and ~30s"]
    async fn bakes_every_seed_preset() {
        let blender = crate::render::engine::detect_blender()
            .await
            .expect("Blender not found — install it or set BLENDER_BIN");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/gen_landscape.py");
        let dir = std::env::temp_dir().join(format!("stlpack_landscape_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();

        for preset in seed_presets() {
            let out_path = dir.join(format!("{}.stl", preset.id));
            let params_path =
                write_params_file(&dir, &preset.params, &out_path, &preset.id).expect("write params file");

            let cancel_token = Notify::new();
            let mut tokens: Vec<LandscapeToken> = Vec::new();
            let result = spawn_and_parse(&blender, &script, &params_path, &cancel_token, |token| {
                tokens.push(token.clone());
            })
            .await;

            let (out, dims_mm, verts, manifold) = match result {
                Ok(v) => v,
                Err((e, tail)) => panic!("preset '{}' failed: {e}\nstdout tail:\n{tail}", preset.id),
            };
            println!(
                "preset '{}': out={out} dims_mm={:?} verts={verts} manifold={manifold}",
                preset.id, dims_mm
            );
            assert!(manifold, "preset '{}' produced a non-manifold mesh", preset.id);
            assert!(Path::new(&out).is_file(), "expected an STL at {:?}", out);
            assert!(
                tokens.iter().any(|t| matches!(t, LandscapeToken::Generating { .. })),
                "preset '{}': expected a GENERATING token",
                preset.id
            );
        }

        std::fs::remove_dir_all(&dir).ok();
    }
}
