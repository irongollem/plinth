//! Base Cutter job pipeline: embed the script, build the job JSON,
//! spawn headless Blender, parse its stdout token protocol into
//! `BaseCutStatus` events. Mirrors `render/batch.rs` — one Blender launch,
//! N cuts, incremental progress, kill-on-cancel, a stdout tail ring buffer
//! for post-mortems. See docs/BASECUTTER.md "The cut pipeline" and
//! "Pinned interfaces", and base_cut.py's own docstring for the exact job
//! JSON shape and stdout protocol this file is the Rust side of.

use crate::basecutter::cutters::{top_face_of, Placement, PlinthParams};
use crate::error::AppError;
use crate::models::BlenderInfo;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use tauri::AppHandle;
use tokio::sync::Notify;

/// The Blender script ships INSIDE the binary — same always-overwrite
/// materialization as render_mini.py (see engine::materialize_embedded_script
/// for the stale-copy trap this avoids).
const BASE_CUT_SCRIPT: &str = include_str!("../../resources/base_cut.py");

/// Write the embedded base-cut script where Blender can read it. Always
/// overwrites, so the file on disk can never drift from the built app.
pub fn materialize_base_cut_script(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    crate::render::engine::materialize_embedded_script(app_handle, "base_cut.py", BASE_CUT_SCRIPT)
}

/// A base-cut job, as sent from the frontend and forwarded to base_cut.py.
/// Field names/renames match the script's job JSON verbatim (see its
/// top docstring and docs/BASECUTTER.md "Pinned interfaces") — `landscape`
/// and `out_dir` are the script's exact keys.
#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct BaseCutJob {
    #[serde(rename = "landscape")]
    pub landscape_path: String,
    pub placements: Vec<Placement>,
    pub plinth: PlinthParams,
    pub out_dir: String,
    /// `Some(t)` = BASE TOPPER mode: no plinth at all — the plug is
    /// flat-trimmed `t` mm below its lowest sculpted point and exported as a
    /// glue-on terrain slab for hard plastic bases. `None` = the normal
    /// seat-on-plinth flow. See docs/BASECUTTER.md "Pinned interfaces" for
    /// the full contract (clamp range, magnet handling). `serde(default)` so
    /// old frontends/job files without the field keep working.
    #[serde(default)]
    pub topper_mm: Option<f64>,
}

/// One parsed line of base_cut.py's stdout protocol (see its docstring and
/// docs/BASECUTTER.md "Pinned interfaces"). `Validated`/`ValidationFailed`
/// carry the raw JSON report rather than a fixed struct — the script's
/// report dict is free to grow fields without this parser needing to know
/// about every one of them; job.rs's caller decides how to shape it into
/// an event payload.
#[derive(Debug, Clone, PartialEq)]
pub enum BaseCutToken {
    Validating,
    Validated(serde_json::Value),
    ValidationFailed(serde_json::Value),
    CutStart {
        index: u32,
    },
    CutDone {
        index: u32,
        out: String,
        dims_mm: [f64; 3],
        manifold: bool,
        /// `Some(false)` = the plug/plinth union left more than one loose
        /// shell behind (the silent-fuse-failure tripwire) — `None` in
        /// topper mode (nothing to fuse) or when the union fused cleanly.
        fused: Option<bool>,
        /// Loose-shell count backing `fused`, present alongside it.
        shells: Option<u32>,
        /// The effective `topper_mm` the script clamped to, present only
        /// when the requested value fell outside base_cut.py's [1.0, 3.0]
        /// clamp range.
        topper_mm_clamped: Option<f64>,
        /// `Some(true)` = this placement carried a magnet spec that topper
        /// mode ignored (nothing to pocket without a plinth).
        magnet_ignored: Option<bool>,
    },
    CutFailed {
        index: u32,
        reason: String,
    },
    JobDone {
        total: u32,
        ok: u32,
    },
}

/// Parse one stdout line into a `BaseCutToken` (None for everything else —
/// Blender's own log noise, blank lines, or garbage). Pure and
/// process-free so the token grammar is unit-testable without spawning
/// Blender.
pub fn parse_token(line: &str) -> Option<BaseCutToken> {
    #[derive(Deserialize)]
    struct Indexed {
        index: u32,
    }
    #[derive(Deserialize)]
    struct CutDonePayload {
        index: u32,
        out: String,
        dims_mm: [f64; 3],
        manifold: bool,
        #[serde(default)]
        fused: Option<bool>,
        #[serde(default)]
        shells: Option<u32>,
        #[serde(default)]
        topper_mm_clamped: Option<f64>,
        #[serde(default)]
        magnet_ignored: Option<bool>,
    }
    #[derive(Deserialize)]
    struct CutFailedPayload {
        index: u32,
        reason: String,
    }
    #[derive(Deserialize)]
    struct JobDonePayload {
        total: u32,
        ok: u32,
    }

    let line = line.trim();
    if line == "VALIDATING" {
        return Some(BaseCutToken::Validating);
    }
    if let Some(json) = line.strip_prefix("VALIDATED ") {
        return serde_json::from_str(json).ok().map(BaseCutToken::Validated);
    }
    if let Some(json) = line.strip_prefix("VALIDATION_FAILED ") {
        return serde_json::from_str(json)
            .ok()
            .map(BaseCutToken::ValidationFailed);
    }
    if let Some(json) = line.strip_prefix("CUT_START ") {
        let p: Indexed = serde_json::from_str(json).ok()?;
        return Some(BaseCutToken::CutStart { index: p.index });
    }
    if let Some(json) = line.strip_prefix("CUT_DONE ") {
        let p: CutDonePayload = serde_json::from_str(json).ok()?;
        return Some(BaseCutToken::CutDone {
            index: p.index,
            out: p.out,
            dims_mm: p.dims_mm,
            manifold: p.manifold,
            fused: p.fused,
            shells: p.shells,
            topper_mm_clamped: p.topper_mm_clamped,
            magnet_ignored: p.magnet_ignored,
        });
    }
    if let Some(json) = line.strip_prefix("CUT_FAILED ") {
        let p: CutFailedPayload = serde_json::from_str(json).ok()?;
        return Some(BaseCutToken::CutFailed {
            index: p.index,
            reason: p.reason,
        });
    }
    if let Some(json) = line.strip_prefix("JOB_DONE ") {
        let p: JobDonePayload = serde_json::from_str(json).ok()?;
        return Some(BaseCutToken::JobDone {
            total: p.total,
            ok: p.ok,
        });
    }
    None
}

/// Serialize `job` and inject each placement's derived cut footprint under a
/// "cut" key (same tagged `CutterKind` shape as `cutter`) — Rust stays the
/// single owner of the nominal->cut derivation (docs/BASECUTTER.md "The
/// plinth": `top_face_of`), so base_cut.py consumes "cut" directly instead
/// of re-deriving it from taper/height itself. Does not touch `BaseCutJob`
/// (the frontend-facing type), only the JSON handed to the script.
fn job_json_with_cut_footprints(job: &BaseCutJob) -> Result<serde_json::Value, AppError> {
    let mut value = serde_json::to_value(job)
        .map_err(|e| AppError::JsonError(format!("Failed to encode base-cut job: {}", e)))?;
    if let Some(placements) = value.get_mut("placements").and_then(|p| p.as_array_mut()) {
        for (placement_value, placement) in placements.iter_mut().zip(&job.placements) {
            let cut = top_face_of(&placement.cutter, &job.plinth);
            let cut_json = serde_json::to_value(&cut)
                .map_err(|e| AppError::JsonError(format!("Failed to encode cut footprint: {}", e)))?;
            if let Some(obj) = placement_value.as_object_mut() {
                obj.insert("cut".to_string(), cut_json);
            }
        }
    }
    Ok(value)
}

/// Write the job JSON into `dir` (the materialized script's directory in
/// production; a scratch dir in tests) so Blender can read it via `--job`.
pub fn write_job_file(dir: &Path, job: &BaseCutJob, job_id: &str) -> Result<PathBuf, AppError> {
    let path = dir.join(format!("base_cut_job_{job_id}.json"));
    let value = job_json_with_cut_footprints(job)?;
    let json = serde_json::to_string_pretty(&value)
        .map_err(|e| AppError::JsonError(format!("Failed to encode base-cut job: {}", e)))?;
    std::fs::write(&path, json)
        .map_err(|e| AppError::IoError(format!("Failed to write base-cut job file: {}", e)))?;
    Ok(path)
}

/// Assemble the headless base-cut invocation: `--background
/// --factory-startup --python-exit-code 1 --python <script> -- --job <json>`
/// (see docs/BASECUTTER.md "Pinned interfaces" — same `--` convention as
/// render_mini.py, but base_cut.py takes one job file, not per-cut flags).
/// `--python-exit-code 1` makes an uncaught script exception (bad job JSON,
/// a multi-object STL, an unwritable out_dir — anything before the per-cut
/// try/except in main()'s loop) exit Blender non-zero; without it Blender's
/// default behaviour is to exit 0 even after a Python traceback, so a
/// pre-loop crash would otherwise be reported as `Finished{ok_count:0}`
/// instead of a failure.
pub fn build_base_cut_command(
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
/// `BaseCutToken`s, invoking `on_token` for each one as it arrives (so a
/// caller can emit incremental progress events without this function
/// knowing anything about `AppHandle` or specta events — that's what keeps
/// it directly testable, per docs/BASECUTTER.md phase 3's done-when).
///
/// `VALIDATION_FAILED` is treated as fatal: the validation pass is meant to
/// gate the whole job (docs/BASECUTTER.md "The cut pipeline"), so the child
/// is killed rather than left to keep cutting against a landscape it just
/// rejected.
///
/// Returns the number of cuts that succeeded (from the script's own
/// `JOB_DONE` count when present, else a local tally of `CUT_DONE` tokens)
/// on a clean exit, or `(error, stdout_tail)` otherwise.
pub async fn spawn_and_parse<F>(
    blender: &BlenderInfo,
    script: &Path,
    job_path: &Path,
    cancel_token: &Notify,
    mut on_token: F,
) -> Result<u32, (AppError, String)>
where
    F: FnMut(&BaseCutToken),
{
    let cmd = build_base_cut_command(blender, script, job_path);
    let mut local_ok: u32 = 0;
    let mut job_done_ok: Option<u32> = None;

    // Unlike run_blender/run_batch_child, every error path here carries a
    // tail (the (AppError, String) return shape), so the merge happens once
    // below rather than being baked into the harness.
    let merge_tail = |out: String, err: String| {
        if err.is_empty() {
            out
        } else {
            format!("{}\n{}", out, err)
        }
    };

    let run_result = crate::render::engine::run_blender_lines(cmd, Some(cancel_token), |line| {
        if let Some(token) = parse_token(line) {
            match &token {
                BaseCutToken::CutDone { .. } => local_ok += 1,
                BaseCutToken::JobDone { ok, .. } => job_done_ok = Some(*ok),
                _ => {}
            }
            let is_validation_failure = matches!(token, BaseCutToken::ValidationFailed(_));
            on_token(&token);
            if is_validation_failure {
                // The validation pass gates the whole job (see
                // docs/BASECUTTER.md "The cut pipeline"): kill the child
                // rather than let it keep cutting against a landscape it
                // just rejected.
                return ControlFlow::Break(());
            }
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
                AppError::UserCancelled("Base cut job cancelled".to_string()),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
        Err(AbortedByCaller { stdout_tail, stderr_tail }) => {
            return Err((
                AppError::FileProcessingError("Landscape failed validation".to_string()),
                merge_tail(stdout_tail, stderr_tail),
            ))
        }
    };

    if !run.status.success() {
        return Err((
            AppError::FileProcessingError(format!("Blender exited with {}", run.status)),
            merge_tail(run.stdout_tail, run.stderr_tail),
        ));
    }

    Ok(job_done_ok.unwrap_or(local_ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::basecutter::cutters::{CutterKind, MagnetSpec};

    #[test]
    fn job_serializes_to_the_script_shape() {
        let job = BaseCutJob {
            landscape_path: "/path/to/landscape.stl".to_string(),
            out_dir: "/dir".to_string(),
            plinth: PlinthParams {
                height_mm: 3.7,
                taper_deg: 15.0,
                hollow: true,
                wall_mm: 1.2,
                top_mm: 1.2,
                magnet_clearance_mm: 0.15,
            },
            placements: vec![Placement {
                cutter: CutterKind::Circle { diameter_mm: 32.0 },
                x_mm: 0.0,
                y_mm: 0.0,
                rotation_deg: 0.0,
                magnet: Some(MagnetSpec {
                    diameter_mm: 5.0,
                    height_mm: 1.0,
                    count: 1,
                }),
                name: Some("round32".to_string()),
            }],
            topper_mm: None,
        };
        let json = serde_json::to_value(&job).unwrap();

        // Matches base_cut.py's docstring example verbatim on the two
        // renamed keys and the nested placement shape.
        assert_eq!(json["landscape"], "/path/to/landscape.stl");
        assert_eq!(json["out_dir"], "/dir");
        assert!(json.get("landscape_path").is_none(), "must not leak the Rust field name");

        assert_eq!(json["plinth"]["height_mm"], 3.7);
        assert_eq!(json["plinth"]["taper_deg"], 15.0);
        assert_eq!(json["plinth"]["hollow"], true);
        assert_eq!(json["plinth"]["wall_mm"], 1.2);
        assert_eq!(json["plinth"]["top_mm"], 1.2);
        assert_eq!(json["plinth"]["magnet_clearance_mm"], 0.15);

        let placement = &json["placements"][0];
        assert_eq!(placement["name"], "round32");
        assert_eq!(placement["cutter"]["kind"], "circle");
        assert_eq!(placement["cutter"]["diameter_mm"], 32.0);
        assert_eq!(placement["x_mm"], 0.0);
        assert_eq!(placement["y_mm"], 0.0);
        assert_eq!(placement["rotation_deg"], 0.0);
        assert_eq!(placement["magnet"]["diameter_mm"], 5.0);
        assert_eq!(placement["magnet"]["height_mm"], 1.0);
        assert_eq!(json["topper_mm"], serde_json::Value::Null);

        let back: BaseCutJob = serde_json::from_value(json).unwrap();
        assert_eq!(back.landscape_path, "/path/to/landscape.stl");
        assert_eq!(back.topper_mm, None);
    }

    /// Pinned interface: `BaseCutJob.topper_mm` serializes verbatim as the
    /// key `topper_mm` (no rename), and round-trips through the same
    /// job_json_with_cut_footprints path a normal job takes — topper mode
    /// still gets the derived "cut" footprint injected per placement (the
    /// cut footprint stays the TOP face in topper mode too, see
    /// docs/BASECUTTER.md's BaseCutJob.topper_mm note).
    #[test]
    fn job_with_topper_mm_serializes_the_key() {
        let job = BaseCutJob {
            landscape_path: "/l.stl".to_string(),
            out_dir: "/out".to_string(),
            plinth: PlinthParams::default(),
            placements: vec![Placement {
                cutter: CutterKind::Circle { diameter_mm: 32.0 },
                x_mm: 0.0,
                y_mm: 0.0,
                rotation_deg: 0.0,
                magnet: None,
                name: Some("topper32".to_string()),
            }],
            topper_mm: Some(1.5),
        };
        let json = serde_json::to_value(&job).unwrap();
        assert_eq!(json["topper_mm"], 1.5);

        let back: BaseCutJob = serde_json::from_value(json).unwrap();
        assert_eq!(back.topper_mm, Some(1.5));

        let wire = job_json_with_cut_footprints(&job).unwrap();
        assert_eq!(wire["topper_mm"], 1.5);
        assert_eq!(wire["placements"][0]["cut"]["kind"], "circle");
    }

    /// Old job JSON (pre-topper_mm) still deserializes — `#[serde(default)]`
    /// backfills `None` rather than erroring on the missing key.
    #[test]
    fn job_without_topper_mm_key_defaults_to_none() {
        let json = serde_json::json!({
            "landscape": "/l.stl",
            "out_dir": "/out",
            "plinth": PlinthParams::default(),
            "placements": [],
        });
        let job: BaseCutJob = serde_json::from_value(json).unwrap();
        assert_eq!(job.topper_mm, None);
    }

    /// The wire JSON (what actually reaches base_cut.py, via
    /// job_json_with_cut_footprints/write_job_file) carries a "cut" key per
    /// placement — the derived top-face footprint — so Rust stays the one
    /// owner of the nominal->cut derivation instead of the script
    /// re-deriving it. 32mm circle + default plinth -> 30.017mm (see
    /// cutters::top_face_of_circle_matches_measured_taper for the math).
    #[test]
    fn wire_json_carries_the_derived_cut_footprint() {
        let job = BaseCutJob {
            landscape_path: "/l.stl".to_string(),
            out_dir: "/out".to_string(),
            plinth: PlinthParams::default(),
            placements: vec![Placement {
                cutter: CutterKind::Circle { diameter_mm: 32.0 },
                x_mm: 0.0,
                y_mm: 0.0,
                rotation_deg: 0.0,
                magnet: None,
                name: Some("round32".to_string()),
            }],
            topper_mm: None,
        };
        let wire = job_json_with_cut_footprints(&job).unwrap();
        let cut = &wire["placements"][0]["cut"];
        assert_eq!(cut["kind"], "circle");
        let diameter_mm = cut["diameter_mm"].as_f64().unwrap();
        assert!(
            (diameter_mm - 30.017).abs() < 0.01,
            "got {diameter_mm}, want 30.017 +/- 0.01"
        );

        // The frontend-facing BaseCutJob type itself must not gain a "cut"
        // field — only the wire JSON does.
        let plain = serde_json::to_value(&job).unwrap();
        assert!(plain["placements"][0].get("cut").is_none());
    }

    #[test]
    fn job_without_magnet_or_name_omits_them_as_null() {
        let job = BaseCutJob {
            landscape_path: "/l.stl".to_string(),
            out_dir: "/out".to_string(),
            plinth: PlinthParams {
                height_mm: 3.7,
                taper_deg: 15.0,
                hollow: true,
                wall_mm: 1.2,
                top_mm: 1.2,
                magnet_clearance_mm: 0.15,
            },
            placements: vec![Placement {
                cutter: CutterKind::Rect {
                    width_mm: 25.0,
                    depth_mm: 25.0,
                },
                x_mm: 10.0,
                y_mm: -10.0,
                rotation_deg: 45.0,
                magnet: None,
                name: None,
            }],
            topper_mm: None,
        };
        let json = serde_json::to_value(&job).unwrap();
        assert_eq!(json["placements"][0]["magnet"], serde_json::Value::Null);
        assert_eq!(json["placements"][0]["name"], serde_json::Value::Null);
    }

    #[test]
    fn parse_token_handles_every_token_type() {
        assert_eq!(parse_token("VALIDATING"), Some(BaseCutToken::Validating));
        assert_eq!(
            parse_token(r#"VALIDATED {"non_manifold_edges": 0, "dims_mm": [40.0, 40.0, 4.0], "verts": 100}"#),
            Some(BaseCutToken::Validated(serde_json::json!({
                "non_manifold_edges": 0,
                "dims_mm": [40.0, 40.0, 4.0],
                "verts": 100
            })))
        );
        assert_eq!(
            parse_token(r#"VALIDATION_FAILED {"non_manifold_edges": 12, "dims_mm": [1.0,1.0,1.0], "verts": 8}"#),
            Some(BaseCutToken::ValidationFailed(serde_json::json!({
                "non_manifold_edges": 12,
                "dims_mm": [1.0, 1.0, 1.0],
                "verts": 8
            })))
        );
        assert_eq!(
            parse_token(r#"CUT_START {"index": 0}"#),
            Some(BaseCutToken::CutStart { index: 0 })
        );
        assert_eq!(
            parse_token(
                r#"CUT_DONE {"index": 0, "out": "/dir/round32.stl", "dims_mm": [32.0, 32.0, 8.5], "manifold": true}"#
            ),
            Some(BaseCutToken::CutDone {
                index: 0,
                out: "/dir/round32.stl".to_string(),
                dims_mm: [32.0, 32.0, 8.5],
                manifold: true,
                fused: None,
                shells: None,
                topper_mm_clamped: None,
                magnet_ignored: None,
            })
        );
        // The additive fields (fused/shells/topper_mm_clamped/
        // magnet_ignored) all parse when present, independent of one
        // another — a topper-mode cut with an ignored magnet, and a
        // normal-mode cut whose union didn't fully fuse.
        assert_eq!(
            parse_token(
                r#"CUT_DONE {"index": 1, "out": "/dir/topper.stl", "dims_mm": [32.0, 32.0, 3.0], "manifold": true, "topper_mm_clamped": 3.0, "magnet_ignored": true}"#
            ),
            Some(BaseCutToken::CutDone {
                index: 1,
                out: "/dir/topper.stl".to_string(),
                dims_mm: [32.0, 32.0, 3.0],
                manifold: true,
                fused: None,
                shells: None,
                topper_mm_clamped: Some(3.0),
                magnet_ignored: Some(true),
            })
        );
        assert_eq!(
            parse_token(
                r#"CUT_DONE {"index": 2, "out": "/dir/round40.stl", "dims_mm": [40.0, 40.0, 9.0], "manifold": true, "fused": false, "shells": 2}"#
            ),
            Some(BaseCutToken::CutDone {
                index: 2,
                out: "/dir/round40.stl".to_string(),
                dims_mm: [40.0, 40.0, 9.0],
                manifold: true,
                fused: Some(false),
                shells: Some(2),
                topper_mm_clamped: None,
                magnet_ignored: None,
            })
        );
        assert_eq!(
            parse_token(r#"CUT_FAILED {"index": 1, "reason": "cut is empty"}"#),
            Some(BaseCutToken::CutFailed {
                index: 1,
                reason: "cut is empty".to_string(),
            })
        );
        assert_eq!(
            parse_token(r#"JOB_DONE {"total": 2, "ok": 1}"#),
            Some(BaseCutToken::JobDone { total: 2, ok: 1 })
        );
    }

    #[test]
    fn parse_token_rejects_garbage_and_blender_noise() {
        assert_eq!(parse_token(""), None);
        assert_eq!(parse_token("   "), None);
        assert_eq!(parse_token("CUT_START not-json"), None);
        assert_eq!(parse_token("VALIDATED"), None); // missing payload
        assert_eq!(parse_token("random log line from Blender"), None);
        assert_eq!(
            parse_token("Blender 5.1.2 (hash abcdef1234 built 2025-01-01)"),
            None
        );
        assert_eq!(
            parse_token("Read prefs: /Users/x/Library/Application Support/Blender/5.1/config/userpref.blend"),
            None
        );
        assert_eq!(parse_token("Info: Deleted 1 object(s)"), None);
        // A CUT_DONE-shaped line missing a required field is not a token.
        assert_eq!(parse_token(r#"CUT_DONE {"index": 0}"#), None);
    }

    #[test]
    fn base_cut_command_has_expected_shape() {
        let blender = BlenderInfo {
            path: "/usr/bin/blender".to_string(),
            version: "Blender 5.1.2".to_string(),
        };
        let cmd = build_base_cut_command(
            &blender,
            Path::new("/tmp/base_cut.py"),
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
                "/tmp/base_cut.py",
                "--",
                "--job",
                "/tmp/job.json",
            ]
        );
    }

    #[test]
    fn write_job_file_writes_readable_json() {
        let dir = std::env::temp_dir().join(format!("stlpack_basecut_unit_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let job = BaseCutJob {
            landscape_path: "/l.stl".to_string(),
            out_dir: "/out".to_string(),
            plinth: PlinthParams {
                height_mm: 3.7,
                taper_deg: 15.0,
                hollow: true,
                wall_mm: 1.2,
                top_mm: 1.2,
                magnet_clearance_mm: 0.15,
            },
            placements: vec![Placement {
                cutter: CutterKind::Circle { diameter_mm: 32.0 },
                x_mm: 0.0,
                y_mm: 0.0,
                rotation_deg: 0.0,
                magnet: None,
                name: Some("round32".to_string()),
            }],
            topper_mm: None,
        };
        let path = write_job_file(&dir, &job, "abc123").unwrap();
        assert!(path.is_file());
        let contents = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["landscape"], "/l.stl");
        // The file on disk (what Blender actually reads) carries the
        // derived "cut" footprint, not just the raw BaseCutJob fields.
        assert_eq!(value["placements"][0]["cut"]["kind"], "circle");
        assert!(value["placements"][0]["cut"]["diameter_mm"].as_f64().unwrap() < 32.0);
        std::fs::remove_dir_all(&dir).ok();
    }

    // ------------------------------------------------------- integration --

    /// End-to-end: generate a tiny watertight landscape with Blender itself
    /// (imported junk meshes fake unrelated symptoms — see the doc's
    /// "inverted-normals incident" risk note), run a 1-placement job
    /// through spawn_and_parse (NOT the tauri command layer), and assert
    /// an STL appears and the token sequence ends with JOB_DONE.
    ///
    /// Run with: cargo test -- --ignored cuts_end_to_end_with_real_blender
    #[tokio::test]
    #[ignore = "requires a local Blender install and ~30s"]
    async fn cuts_end_to_end_with_real_blender() {
        let blender = crate::render::engine::detect_blender()
            .await
            .expect("Blender not found — install it or set BLENDER_BIN");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/base_cut.py");

        let dir = std::env::temp_dir().join(format!("stlpack_basecut_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let landscape = dir.join("landscape.stl");
        generate_test_landscape(&blender, &dir, &landscape).await;
        assert!(landscape.is_file(), "landscape generation failed");

        let out_dir = dir.join("out");
        std::fs::create_dir_all(&out_dir).unwrap();

        let job = BaseCutJob {
            landscape_path: landscape.to_string_lossy().into_owned(),
            out_dir: out_dir.to_string_lossy().into_owned(),
            plinth: PlinthParams::default(),
            placements: vec![Placement {
                cutter: CutterKind::Circle { diameter_mm: 32.0 },
                x_mm: 0.0,
                y_mm: 0.0,
                rotation_deg: 0.0,
                magnet: None,
                name: Some("round32".to_string()),
            }],
            topper_mm: None,
        };
        let job_path = write_job_file(&dir, &job, "test-job").expect("write job file");

        let cancel_token = Notify::new();
        let mut tokens: Vec<BaseCutToken> = Vec::new();
        let result = spawn_and_parse(&blender, &script, &job_path, &cancel_token, |token| {
            tokens.push(token.clone());
        })
        .await;

        let ok_count = match result {
            Ok(ok) => ok,
            Err((e, tail)) => panic!("base-cut job failed: {e}\nstdout tail:\n{tail}"),
        };

        assert_eq!(ok_count, 1, "expected 1 successful cut, tokens: {:?}", tokens);
        assert!(
            matches!(tokens.last(), Some(BaseCutToken::JobDone { total: 1, ok: 1 })),
            "expected the token sequence to end with JOB_DONE, got: {:?}",
            tokens
        );
        assert!(
            tokens.iter().any(|t| matches!(t, BaseCutToken::Validating)),
            "expected a VALIDATING token"
        );
        // The generated test landscape is a clean, watertight Blender
        // primitive, so the (now real, not a dead protocol arm — see
        // base_cut.py's validate()) gate must pass it: VALIDATED, not
        // VALIDATION_FAILED.
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, BaseCutToken::Validated(_))),
            "expected a VALIDATED token, got: {:?}",
            tokens
        );
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, BaseCutToken::CutStart { index: 0 })),
            "expected a CUT_START token"
        );

        let stl = out_dir.join("round32.stl");
        assert!(stl.is_file(), "expected an STL at {:?}", stl);
        assert!(std::fs::metadata(&stl).unwrap().len() > 84, "STL looks empty");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Builds a small closed (watertight) bumpy blob with Blender itself —
    /// a subdivided cube with per-vertex jitter — and exports it as the
    /// landscape STL the end-to-end test cuts from. Deliberately NOT an
    /// imported/hand-authored mesh: see the doc's note that junk meshes
    /// fake unrelated symptoms (the inverted-normals incident).
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

    // ---- cross-language drift tripwire (docs/BASECUTTER.md "The plinth") --
    //
    // src/utils/magnetSuggest.ts mirrors three pieces of base_cut.py's magnet
    // math in TypeScript, since the frontend suggests/fits magnets without a
    // round-trip to Blender: the r_boss formula, the _magnet_positions
    // spacing expression, and MAX_MAGNET_COUNT. Unlike cutFootprint.ts's
    // mirror of the taper shrink (pinned by cutters.rs's own Rust tests
    // against the same numbers), nothing here fails if base_cut.py's magnet
    // math changes underfoot — this test is that failure.
    //
    // If this test fails: base_cut.py's magnet geometry changed.
    // src/utils/magnetSuggest.ts (and its magnetSuggest.test.ts) must be
    // re-mirrored to match, and the pinned strings below updated to the new
    // source lines.

    #[test]
    fn magnet_boss_formula_is_still_the_string_magnet_suggest_ts_mirrors() {
        assert!(
            BASE_CUT_SCRIPT.contains(
                r#"r_boss = magnet["diameter_mm"] / 2.0 + clearance + wall"#
            ),
            "base_cut.py's boss-radius formula changed — re-mirror \
             bossOuterDiameterMm in src/utils/magnetSuggest.ts"
        );
    }

    #[test]
    fn magnet_positions_spacing_is_still_the_expression_magnet_suggest_ts_mirrors() {
        assert!(
            BASE_CUT_SCRIPT.contains("spacing = long_dim / (count + 1)"),
            "base_cut.py's _magnet_positions spacing changed — re-mirror \
             magnetPositionsMm in src/utils/magnetSuggest.ts"
        );
        assert!(
            BASE_CUT_SCRIPT.contains(
                "return [direction * ((i - (count - 1) / 2.0) * spacing) for i in range(count)]"
            ),
            "base_cut.py's _magnet_positions offset formula changed — re-mirror \
             magnetPositionsMm in src/utils/magnetSuggest.ts"
        );
    }

    #[test]
    fn max_magnet_count_is_still_the_value_magnet_suggest_ts_mirrors() {
        assert!(
            BASE_CUT_SCRIPT.contains("MAX_MAGNET_COUNT = 4"),
            "base_cut.py's MAX_MAGNET_COUNT changed — update the mirrored \
             constant of the same name in src/utils/magnetSuggest.ts"
        );
    }
}
