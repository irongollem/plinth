//! Base Cutter job pipeline: embed the script, build the job JSON,
//! spawn headless Blender, parse its stdout token protocol into
//! `BaseCutStatus` events. Mirrors `render/batch.rs` — one Blender launch,
//! N cuts, incremental progress, kill-on-cancel, a stdout tail ring buffer
//! for post-mortems. See docs/BASECUTTER.md "The cut pipeline" and
//! "Pinned interfaces", and base_cut.py's own docstring for the exact job
//! JSON shape and stdout protocol this file is the Rust side of.

use crate::basecutter::cutters::{Placement, PlinthParams};
use crate::error::AppError;
use crate::models::BlenderInfo;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
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

/// Write the job JSON into `dir` (the materialized script's directory in
/// production; a scratch dir in tests) so Blender can read it via `--job`.
pub fn write_job_file(dir: &Path, job: &BaseCutJob, job_id: &str) -> Result<PathBuf, AppError> {
    let path = dir.join(format!("base_cut_job_{job_id}.json"));
    let json = serde_json::to_string_pretty(job)
        .map_err(|e| AppError::JsonError(format!("Failed to encode base-cut job: {}", e)))?;
    std::fs::write(&path, json)
        .map_err(|e| AppError::IoError(format!("Failed to write base-cut job file: {}", e)))?;
    Ok(path)
}

/// Assemble the headless base-cut invocation: `--background
/// --factory-startup --python <script> -- --job <json>` (see
/// docs/BASECUTTER.md "Pinned interfaces" — same `--` convention as
/// render_mini.py, but base_cut.py takes one job file, not per-cut flags).
pub fn build_base_cut_command(
    blender: &BlenderInfo,
    script: &Path,
    job_path: &Path,
) -> tokio::process::Command {
    let mut cmd = crate::render::engine::new_command(Path::new(&blender.path));
    cmd.arg("--background")
        .arg("--factory-startup")
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
    let mut cmd = build_base_cut_command(blender, script, job_path);
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| (AppError::IoError(format!("Failed to launch Blender: {}", e)), String::new()))?;

    let stdout = child.stdout.take().ok_or_else(|| {
        (
            AppError::IoError("Failed to capture Blender stdout".to_string()),
            String::new(),
        )
    })?;
    let stderr = child.stderr.take();

    let stderr_tail: std::sync::Arc<std::sync::Mutex<VecDeque<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(VecDeque::new()));
    if let Some(stderr) = stderr {
        let tail = std::sync::Arc::clone(&stderr_tail);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(mut tail) = tail.lock() {
                    if tail.len() >= 10 {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            }
        });
    }

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stdout_tail: VecDeque<String> = VecDeque::new();
    let mut local_ok: u32 = 0;
    let mut job_done_ok: Option<u32> = None;

    let tail_snapshot = |stdout_tail: &VecDeque<String>, stderr_tail: &std::sync::Mutex<VecDeque<String>>| {
        let out = stdout_tail.iter().cloned().collect::<Vec<_>>().join("\n");
        let err = stderr_tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        if err.is_empty() {
            out
        } else {
            format!("{}\n{}", out, err)
        }
    };

    // Registered ONCE and kept alive across iterations — notify_waiters()
    // stores no permit (see render/commands.rs::run_blender for the
    // original rationale).
    let cancelled = cancel_token.notified();
    tokio::pin!(cancelled);

    loop {
        tokio::select! {
            _ = &mut cancelled => {
                child.kill().await.ok();
                return Err((
                    AppError::UserCancelled("Base cut job cancelled".to_string()),
                    tail_snapshot(&stdout_tail, &stderr_tail),
                ));
            }
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if stdout_tail.len() >= 10 {
                            stdout_tail.pop_front();
                        }
                        stdout_tail.push_back(line.clone());

                        if let Some(token) = parse_token(&line) {
                            match &token {
                                BaseCutToken::CutDone { .. } => local_ok += 1,
                                BaseCutToken::JobDone { ok, .. } => job_done_ok = Some(*ok),
                                _ => {}
                            }
                            let is_validation_failure = matches!(token, BaseCutToken::ValidationFailed(_));
                            on_token(&token);
                            if is_validation_failure {
                                child.kill().await.ok();
                                return Err((
                                    AppError::FileProcessingError(
                                        "Landscape failed validation".to_string(),
                                    ),
                                    tail_snapshot(&stdout_tail, &stderr_tail),
                                ));
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        return Err((
                            AppError::IoError(format!("Failed reading Blender output: {}", e)),
                            tail_snapshot(&stdout_tail, &stderr_tail),
                        ))
                    }
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| {
        (
            AppError::IoError(format!("Failed waiting for Blender: {}", e)),
            tail_snapshot(&stdout_tail, &stderr_tail),
        )
    })?;
    if !status.success() {
        return Err((
            AppError::FileProcessingError(format!("Blender exited with {}", status)),
            tail_snapshot(&stdout_tail, &stderr_tail),
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

        let back: BaseCutJob = serde_json::from_value(json).unwrap();
        assert_eq!(back.landscape_path, "/path/to/landscape.stl");
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
            placements: vec![],
        };
        let path = write_job_file(&dir, &job, "abc123").unwrap();
        assert!(path.is_file());
        let contents = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(value["landscape"], "/l.stl");
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
}
