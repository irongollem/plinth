use crate::error::AppError;
use crate::models::{BlenderInfo, RenderOptions};
use crate::settings::SETTINGS_CACHE;
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::future::Future;
use std::ops::ControlFlow;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Notify;

/// The Blender script ships INSIDE the binary. As a bundled resource it was
/// only re-copied next to the binary when the Rust code rebuilt, so pure
/// script edits silently kept rendering with a stale copy during dev.
const RENDER_SCRIPT: &str = include_str!("../../resources/render_mini.py");

/// Write an embedded Blender script where Blender can read it. Always
/// overwrites, so the file on disk can never drift from the built app — the
/// trap this avoids: as a bundled resource the script was only re-copied
/// next to the binary when the Rust code rebuilt, so pure script edits
/// silently kept running a stale copy during dev. Shared by every embedded
/// script (render_mini.py, base_cut.py, ...) so the fix lives in one place.
pub(crate) fn materialize_embedded_script(
    app_handle: &AppHandle,
    file_name: &str,
    contents: &str,
) -> Result<PathBuf, AppError> {
    let dir = app_handle
        .path()
        .app_cache_dir()
        .or_else(|_| app_handle.path().app_data_dir())
        .map_err(|e| AppError::ConfigError(format!("No writable app dir: {}", e)))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::IoError(format!("Failed to create app dir: {}", e)))?;
    let path = dir.join(file_name);
    std::fs::write(&path, contents)
        .map_err(|e| AppError::IoError(format!("Failed to write {}: {}", file_name, e)))?;
    Ok(path)
}

/// Write the embedded render script where Blender can read it. Always
/// overwrites, so the file on disk can never drift from the built app.
pub fn materialize_render_script(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    materialize_embedded_script(app_handle, "render_mini.py", RENDER_SCRIPT)
}

/// (blender_path setting at detection time, detected install)
type CachedDetection = (Option<String>, BlenderInfo);

/// Last successful detection, keyed by the blender_path setting it was made
/// under. Detection spawns `blender --version` (a full Blender cold start),
/// far too expensive to repeat on every render.
static DETECTION_CACHE: Lazy<Mutex<Option<CachedDetection>>> = Lazy::new(|| Mutex::new(None));

/// Forget the cached detection — a managed install just landed and should
/// win the next probe even though the blender_path setting didn't change.
pub fn invalidate_detection_cache() {
    if let Ok(mut cache) = DETECTION_CACHE.lock() {
        *cache = None;
    }
}

/// Resolve the Blender binary: explicit setting -> BLENDER_BIN env -> PATH -> platform defaults.
/// Returns the first candidate that actually runs and reports a version.
/// Always probes fresh (and refreshes the cache) — use detect_blender_cached
/// on hot paths.
pub async fn detect_blender() -> Result<BlenderInfo, AppError> {
    let configured = configured_blender_path();
    for candidate in candidate_paths() {
        let candidate = normalize_binary(candidate);
        if let Some(version) = blender_version(&candidate).await {
            let info = BlenderInfo {
                path: candidate.to_string_lossy().into_owned(),
                version,
            };
            if let Ok(mut cache) = DETECTION_CACHE.lock() {
                *cache = Some((configured, info.clone()));
            }
            return Ok(info);
        }
    }
    Err(AppError::NotFoundError(
        "Blender not found. Install Blender 4.x+ or set its location in Settings.".to_string(),
    ))
}

/// Cached detection for per-render use. Re-detects when the configured
/// blender_path setting changed or the cached binary vanished.
pub async fn detect_blender_cached() -> Result<BlenderInfo, AppError> {
    let configured = configured_blender_path();
    if let Ok(cache) = DETECTION_CACHE.lock() {
        if let Some((setting, info)) = cache.as_ref() {
            let binary_still_there =
                !info.path.contains(std::path::MAIN_SEPARATOR) || Path::new(&info.path).is_file();
            if *setting == configured && binary_still_there {
                return Ok(info.clone());
            }
        }
    }
    detect_blender().await
}

fn configured_blender_path() -> Option<String> {
    SETTINGS_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.blender_path.clone())
}

fn candidate_paths() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    if let Ok(cache) = SETTINGS_CACHE.lock() {
        if let Some(path) = &cache.blender_path {
            if !path.is_empty() {
                candidates.push(PathBuf::from(path));
            }
        }
    }

    if let Ok(path) = std::env::var("BLENDER_BIN") {
        candidates.push(PathBuf::from(path));
    }

    // A Blender the app downloaded itself (app data dir). Ranks above the
    // ambient installs below on purpose: the whole point of downloading one
    // is to outvote an older Blender sitting on PATH or in /Applications.
    if let Some(managed) = crate::render::provision::managed_binary() {
        candidates.push(managed);
    }

    // A portable Blender shipped inside the app bundle
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            #[cfg(target_os = "macos")]
            candidates
                .push(exe_dir.join("../Resources/blender/Blender.app/Contents/MacOS/Blender"));
            #[cfg(target_os = "linux")]
            candidates.push(exe_dir.join("blender/blender"));
            #[cfg(target_os = "windows")]
            candidates.push(exe_dir.join("blender").join("blender.exe"));
        }
    }

    // Bare name resolves through PATH when spawned
    candidates.push(PathBuf::from("blender"));

    #[cfg(target_os = "macos")]
    {
        candidates.push(PathBuf::from(
            "/Applications/Blender.app/Contents/MacOS/Blender",
        ));
        if let Some(home) = std::env::var_os("HOME") {
            candidates
                .push(PathBuf::from(home).join("Applications/Blender.app/Contents/MacOS/Blender"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        candidates.push(PathBuf::from("/usr/bin/blender"));
        candidates.push(PathBuf::from("/usr/local/bin/blender"));
        candidates.push(PathBuf::from("/snap/bin/blender"));
    }

    #[cfg(target_os = "windows")]
    {
        let install_root = PathBuf::from("C:\\Program Files\\Blender Foundation");
        if let Ok(entries) = std::fs::read_dir(&install_root) {
            let mut versioned: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path().join("blender.exe"))
                .filter(|p| p.is_file())
                .collect();
            versioned.sort();
            versioned.reverse(); // newest version first
            candidates.extend(versioned);
        }
    }

    candidates
}

/// Allow users to point at Blender.app on macOS instead of the inner binary.
fn normalize_binary(path: PathBuf) -> PathBuf {
    if path.extension().is_some_and(|ext| ext == "app") {
        path.join("Contents/MacOS/Blender")
    } else {
        path
    }
}

async fn blender_version(binary: &Path) -> Option<String> {
    let output = new_command(binary).arg("--version").output().await.ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .find(|line| line.trim_start().starts_with("Blender"))
        .map(|line| line.trim().to_string())
}

/// All Blender spawns share the no-console-flash guarantee (crate::process).
pub fn new_command(binary: &Path) -> Command {
    crate::process::new_async_command(binary)
}

/// Blender 5+ moved render stats ("Fra: 1 | ... | Sample 8/96") behind the
/// new logging system; ask for them so progress is parseable. 4.x prints
/// them by default and predates the named log levels.
pub fn progress_args(version: &str) -> &'static [&'static str] {
    let major = version
        .trim_start_matches("Blender")
        .trim_start()
        .split('.')
        .next()
        .and_then(|v| v.trim().parse::<u32>().ok())
        .unwrap_or(0);
    if major >= 5 {
        &["--log-level", "info"]
    } else {
        &[]
    }
}

// --------------------------- shared Blender run harness --------------------

/// A Blender child process that ran to completion (not cancelled, not
/// aborted by `on_line`): its exit status plus the stdout/stderr tail ring
/// buffers (last 10 lines each) for post-mortem error messages. Whether a
/// given `status` counts as success is the caller's call — some callers
/// additionally require an output file to exist, some fold the tails into
/// their own error string, some don't need the tails at all.
#[derive(Debug)]
pub(crate) struct BlenderRun {
    pub status: ExitStatus,
    pub stdout_tail: String,
    pub stderr_tail: String,
}

/// Why `run_blender_lines` did not reach a `BlenderRun`. Deliberately more
/// granular than "spawn / io / cancelled": job.rs, run_blender, and
/// run_batch_child each had their own exact wording per failure stage before
/// this dedup, and this enum keeps each stage distinguishable so every
/// caller can still produce its own exact text (see each call site's error
/// mapping — the messages are unchanged from before the refactor).
#[derive(Debug)]
pub(crate) enum BlenderRunError {
    /// `Command::spawn` itself failed (bad binary path, no exec perms, ...).
    /// No process ever existed, so there is nothing to put in the tails.
    SpawnFailed(std::io::Error),
    /// `child.stdout` was `None` — the API returns an `Option` even though
    /// we always request `Stdio::piped()`. No output was read yet either.
    StdoutCaptureFailed,
    /// A line read off Blender's stdout failed at the OS level.
    ReadFailed {
        source: std::io::Error,
        stdout_tail: String,
        stderr_tail: String,
    },
    /// `child.wait()` (after stdout closed) failed at the OS level.
    WaitFailed {
        source: std::io::Error,
        stdout_tail: String,
        stderr_tail: String,
    },
    /// `cancel`'s `Notify` fired mid-run: the child was killed WITHOUT
    /// waiting for it — a killed process's exit status is noise, matching
    /// every caller's pre-refactor cancel handling.
    Cancelled {
        stdout_tail: String,
        stderr_tail: String,
    },
    /// `on_line` returned `ControlFlow::Break` — basecutter's
    /// validation-failure gate is the only current user of this (the
    /// validation pass must kill the child immediately rather than let it
    /// keep cutting against a landscape it just rejected). Given the same
    /// kill-without-waiting treatment as `Cancelled` but kept as a distinct
    /// variant so a caller can give it its own error text.
    AbortedByCaller {
        stdout_tail: String,
        stderr_tail: String,
    },
}

/// The shared Blender child-process harness, extracted out of job.rs,
/// render/commands.rs, and render/batch.rs, which each copy-pasted it
/// verbatim: pipe stdout/stderr + `stdin(null)` + `kill_on_drop`, spawn, tee
/// stderr into a 10-line ring buffer on a background task, read stdout
/// line-by-line into another 10-line ring buffer while invoking `on_line`
/// for each raw line, race that against `cancel` (if given) with
/// kill-on-cancel, then `wait()` for the exit status.
///
/// `on_line` receives each stdout line exactly as read (untrimmed — callers
/// already trim where they need to). Returning `ControlFlow::Break(())`
/// asks for an immediate kill-and-stop (see `BlenderRunError::AbortedByCaller`);
/// `Continue` keeps reading. `cancel` is optional so a caller with no
/// cancellation source (there is none today, but the harness shouldn't
/// assume one) can pass `None` and simply never race against it.
///
/// This is pure process plumbing. Per-line parsing/side-effects, success
/// criteria (exit status, output-file checks, terminal-token bookkeeping),
/// event emissions, and error text are ALL caller-side — see
/// job.rs::spawn_and_parse, render/commands.rs::run_blender, and
/// render/batch.rs::run_batch_child for how each keeps its own behavior.
pub(crate) async fn run_blender_lines<'a>(
    mut cmd: Command,
    cancel: Option<&'a Notify>,
    mut on_line: impl FnMut(&str) -> ControlFlow<()>,
) -> Result<BlenderRun, BlenderRunError> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(BlenderRunError::SpawnFailed)?;

    let stdout = child
        .stdout
        .take()
        .ok_or(BlenderRunError::StdoutCaptureFailed)?;
    let stderr = child.stderr.take();

    let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    if let Some(stderr) = stderr {
        let tail = Arc::clone(&stderr_tail);
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

    let tail_snapshot = |stdout_tail: &VecDeque<String>| -> (String, String) {
        let out = stdout_tail.iter().cloned().collect::<Vec<_>>().join("\n");
        let err = stderr_tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        (out, err)
    };

    // Registered ONCE, right here, and kept alive across every loop
    // iteration: a fresh `notified()` per iteration would drop a cancel that
    // lands between iterations (notify_one()'s stored wake-up permit only
    // helps a waiter that is ALREADY registered when it fires). Boxed rather
    // than `tokio::pin!`-ed because `cancel` is optional: with no Notify to
    // wait on we still need a future to race against, one that simply never
    // resolves.
    let mut cancelled: Pin<Box<dyn Future<Output = ()> + Send + 'a>> = match cancel {
        Some(notify) => Box::pin(notify.notified()),
        None => Box::pin(std::future::pending()),
    };

    loop {
        tokio::select! {
            _ = &mut cancelled => {
                child.kill().await.ok();
                let (stdout_tail, stderr_tail) = tail_snapshot(&stdout_tail);
                return Err(BlenderRunError::Cancelled { stdout_tail, stderr_tail });
            }
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if stdout_tail.len() >= 10 {
                            stdout_tail.pop_front();
                        }
                        stdout_tail.push_back(line.clone());

                        if on_line(&line).is_break() {
                            child.kill().await.ok();
                            let (stdout_tail, stderr_tail) = tail_snapshot(&stdout_tail);
                            return Err(BlenderRunError::AbortedByCaller { stdout_tail, stderr_tail });
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        let (stdout_tail, stderr_tail) = tail_snapshot(&stdout_tail);
                        return Err(BlenderRunError::ReadFailed { source: e, stdout_tail, stderr_tail });
                    }
                }
            }
        }
    }

    let status = child.wait().await.map_err(|e| {
        let (stdout_tail, stderr_tail) = tail_snapshot(&stdout_tail);
        BlenderRunError::WaitFailed { source: e, stdout_tail, stderr_tail }
    })?;
    let (stdout_tail, stderr_tail) = tail_snapshot(&stdout_tail);
    Ok(BlenderRun { status, stdout_tail, stderr_tail })
}

/// The shared prefix every headless render_mini.py invocation starts with —
/// `build_render_command` and `build_batch_render_command` were otherwise
/// byte-for-byte identical up to their mode-specific flags. `--python-exit-code
/// 1` makes an uncaught script exception exit Blender non-zero instead of
/// Blender's default of exiting 0 after a Python traceback (the same gap
/// `basecutter::job::build_base_cut_command` closed). For single-render this
/// catches e.g. `import_join`'s "File not found"/"STL import produced no
/// mesh" `SystemExit`s (previously only caught indirectly, by `run_blender`
/// noticing no output file appeared). For batch mode it only matters for a
/// crash BEFORE run_batch's per-entry `try/except` (a bad manifest file,
/// ...) — a failure inside the loop is already caught there and reported as
/// a machine-readable `BATCH_DONE {"ok":false}` line, so Blender still exits
/// 0 (correctly) after a batch with individual failures.
fn render_script_invocation(blender: &BlenderInfo, script: &Path) -> Command {
    let mut cmd = new_command(Path::new(&blender.path));
    cmd.args(progress_args(&blender.version));
    cmd.arg("-b")
        .arg("--python-exit-code")
        .arg("1")
        .arg("-P")
        .arg(script)
        .arg("--");
    cmd
}

/// Assemble the full headless render invocation for render_mini.py.
pub fn build_render_command(
    blender: &BlenderInfo,
    script: &Path,
    parts: &[String],
    options: &RenderOptions,
    output_path: &Path,
) -> Command {
    let mut cmd = render_script_invocation(blender, script);
    for part in parts {
        cmd.arg(part);
    }
    let (rx, ry, rz) = options.rotate;
    cmd.arg("--rotate").arg(format!("{},{},{}", rx, ry, rz));
    if let Some((r, g, b)) = options.color {
        cmd.arg("--color").arg(format!("{},{},{}", r, g, b));
    }
    if let Some(azimuth) = options.azimuth {
        cmd.arg("--azimuth").arg(azimuth.to_string());
    }
    if let Some(elevation) = options.elevation {
        cmd.arg("--elev").arg(elevation.to_string());
    }
    if let Some(zoom) = options.zoom {
        cmd.arg("--zoom").arg(zoom.to_string());
    }
    if let Some(resolution) = options.resolution {
        cmd.arg("--res").arg(resolution.to_string());
    }
    if let Some(samples) = options.samples {
        cmd.arg("--samples").arg(samples.to_string());
    }
    if let Some(look) = &options.look {
        cmd.arg("--look").arg(look);
    }
    if options.align_parts {
        cmd.arg("--align-parts");
    }
    // Inline JSON as ONE argv element — no shell is involved, so no quoting
    // or length worries, and concurrent jobs share no temp-file state
    if let Some(config) = options
        .look_config
        .as_deref()
        .filter(|c| !c.trim().is_empty())
    {
        cmd.arg("--config").arg(config);
    }
    if options.scale_reference {
        if let Some((path, height)) = configured_scale_reference() {
            cmd.arg("--scale-ref").arg(path);
            cmd.arg("--scale-ref-height").arg(height.to_string());
        }
    }
    if let Some(nominal) = &options.base {
        cmd.arg("--base").arg(base_arg_json(nominal));
    }
    cmd.arg("--out").arg(output_path);
    cmd
}

/// `--base` payload: the user's NOMINAL footprint plus Rust's already-
/// derived cut (top-face) footprint and the plinth profile — same ownership
/// rule as the base cutter job (docs/BASECUTTER.md "The plinth": Rust owns
/// the nominal->cut derivation via `top_face_of`, the script only lofts
/// between the two footprints it's handed, never re-deriving). Inline JSON
/// as ONE argv element, same reasoning as `--config` above.
///
/// `PlinthParams::default()` here — not the user's Base Cutter profile — is
/// deliberate, not a stub: per docs/BASECUTTER.md "Synergy: standard bases
/// in the Render tool", a rendered mini stood on a base only reads as a
/// scale cue if it's the industry-standard measured profile every hobbyist
/// already recognizes. A user's customized taper/wall/hollow settings are a
/// Base Cutter *cutting* concern (what actually gets printed) and must not
/// leak into this shared reference, or the same "32 mm round" would render
/// differently per user and stop being a legible scale reference at all.
fn base_arg_json(nominal: &crate::basecutter::cutters::CutterKind) -> String {
    use crate::basecutter::cutters::{top_face_of, PlinthParams};
    let plinth = PlinthParams::default();
    let cut = top_face_of(nominal, &plinth);
    serde_json::json!({
        "nominal": nominal,
        "cut": cut,
        "height_mm": plinth.height_mm,
        "taper_deg": plinth.taper_deg,
    })
    .to_string()
}

/// The user's scale-reference figure, when one is configured. Read from the
/// settings cache like the blender path: the toggle in RenderOptions only
/// says "include it", the asset itself is a settings-level choice.
fn configured_scale_reference() -> Option<(String, f64)> {
    let cache = SETTINGS_CACHE.lock().ok()?;
    let path = cache
        .scale_reference_path
        .clone()
        .filter(|p| !p.trim().is_empty())?;
    Some((path, cache.scale_reference_height_mm.unwrap_or(28.0)))
}

/// Extract "Sample 32/96" style progress from a Cycles stdout line.
pub fn parse_sample_progress(line: &str) -> Option<(u32, u32)> {
    let idx = line.rfind("Sample ")?;
    let rest = line[idx + "Sample ".len()..].split_whitespace().next()?;
    let rest = rest.trim_end_matches(',');
    let (current, total) = rest.split_once('/')?;
    let current: u32 = current.trim().parse().ok()?;
    let total: u32 = total.trim().parse().ok()?;
    if total == 0 {
        return None;
    }
    Some((current, total))
}

// ----------------------------- batch mode ---------------------------------

/// One model in a batch manifest — the script renders these sequentially in
/// a single Blender launch (startup cost paid once for the whole sweep).
/// Deliberately a struct, not positional args: a future scale-reference
/// figure ("banana for scale") is one more optional field here plus a script
/// flag, no pipeline redesign.
#[derive(serde::Serialize, Debug, Clone)]
pub struct BatchEntry {
    pub parts: Vec<String>,
    pub out: String,
    pub rotate: (f64, f64, f64),
}

#[derive(serde::Serialize, Debug, Clone)]
pub struct BatchManifest {
    pub entries: Vec<BatchEntry>,
}

/// Assemble the headless batch invocation: `--batch <manifest>` and NO
/// positional parts — in single-model mode positionals all join into one
/// mini, which is exactly wrong for a batch.
pub fn build_batch_render_command(
    blender: &BlenderInfo,
    script: &Path,
    manifest_path: &Path,
) -> Command {
    let mut cmd = render_script_invocation(blender, script);
    cmd.arg("--batch").arg(manifest_path);
    cmd.arg("--look").arg("flat");
    cmd
}

/// Machine-readable lines run_batch prints between Cycles' own output.
#[derive(Debug, Clone, PartialEq)]
pub enum BatchLine {
    Start { total: u32 },
    Model { index: u32 },
    Measured { index: u32, dims_mm: [f64; 3], parts: u32 },
    Done { index: u32, ok: bool, error: Option<String> },
}

/// Parse one stdout line into a BatchLine (None for everything else,
/// including Cycles' Sample lines — parse_sample_progress handles those).
pub fn parse_batch_line(line: &str) -> Option<BatchLine> {
    #[derive(serde::Deserialize)]
    struct Start {
        total: u32,
    }
    #[derive(serde::Deserialize)]
    struct Model {
        index: u32,
    }
    #[derive(serde::Deserialize)]
    struct Measured {
        index: u32,
        dims_mm: [f64; 3],
        parts: u32,
    }
    #[derive(serde::Deserialize)]
    struct Done {
        index: u32,
        ok: bool,
        #[serde(default)]
        error: Option<String>,
    }
    let line = line.trim();
    if let Some(json) = line.strip_prefix("BATCH_START ") {
        let s: Start = serde_json::from_str(json).ok()?;
        return Some(BatchLine::Start { total: s.total });
    }
    if let Some(json) = line.strip_prefix("BATCH_MODEL ") {
        let m: Model = serde_json::from_str(json).ok()?;
        return Some(BatchLine::Model { index: m.index });
    }
    if let Some(json) = line.strip_prefix("MEASURED ") {
        let m: Measured = serde_json::from_str(json).ok()?;
        return Some(BatchLine::Measured {
            index: m.index,
            dims_mm: m.dims_mm,
            parts: m.parts,
        });
    }
    if let Some(json) = line.strip_prefix("BATCH_DONE ") {
        let d: Done = serde_json::from_str(json).ok()?;
        return Some(BatchLine::Done {
            index: d.index,
            ok: d.ok,
            error: d.error,
        });
    }
    None
}

/// Write the manifest where the batch job's Blender can read it. Scratch
/// space per job id; the job deletes the dir when it ends.
pub fn batch_scratch_dir(app_handle: &AppHandle, job_id: &str) -> Result<PathBuf, AppError> {
    let dir = app_handle
        .path()
        .app_cache_dir()
        .or_else(|_| app_handle.path().app_data_dir())
        .map_err(|e| AppError::ConfigError(format!("No writable app dir: {}", e)))?
        .join("batch_renders")
        .join(job_id);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::IoError(format!("Failed to create batch dir: {}", e)))?;
    Ok(dir)
}

pub fn write_batch_manifest(dir: &Path, manifest: &BatchManifest) -> Result<PathBuf, AppError> {
    let path = dir.join("manifest.json");
    let json = serde_json::to_string_pretty(manifest)
        .map_err(|e| AppError::ConfigError(format!("Failed to encode batch manifest: {}", e)))?;
    std::fs::write(&path, json)
        .map_err(|e| AppError::IoError(format!("Failed to write batch manifest: {}", e)))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_batch_lines() {
        assert_eq!(
            parse_batch_line(r#"BATCH_START {"total": 3}"#),
            Some(BatchLine::Start { total: 3 })
        );
        assert_eq!(
            parse_batch_line(r#"BATCH_MODEL {"index": 1, "out": "/tmp/1.png"}"#),
            Some(BatchLine::Model { index: 1 })
        );
        assert_eq!(
            parse_batch_line(r#"MEASURED {"index": 0, "dims_mm": [60.2, 35.1, 88.7], "parts": 3}"#),
            Some(BatchLine::Measured {
                index: 0,
                dims_mm: [60.2, 35.1, 88.7],
                parts: 3
            })
        );
        assert_eq!(
            parse_batch_line(r#"BATCH_DONE {"index": 2, "ok": false, "error": "File not found: x.stl"}"#),
            Some(BatchLine::Done {
                index: 2,
                ok: false,
                error: Some("File not found: x.stl".to_string())
            })
        );
        assert_eq!(
            parse_batch_line(r#"BATCH_DONE {"index": 0, "ok": true}"#),
            Some(BatchLine::Done {
                index: 0,
                ok: true,
                error: None
            })
        );
        // Cycles' own lines and noise are not batch lines
        assert_eq!(parse_batch_line("Fra:1 | Sample 32/96"), None);
        assert_eq!(parse_batch_line("BATCH_START not-json"), None);
        assert_eq!(parse_batch_line("[render_mini] done."), None);
    }

    #[test]
    fn batch_command_has_manifest_and_no_positionals() {
        let blender = BlenderInfo {
            path: "/usr/bin/blender".into(),
            version: "Blender 5.1.2".into(),
        };
        let cmd = build_batch_render_command(
            &blender,
            Path::new("/tmp/render_mini.py"),
            Path::new("/tmp/manifest.json"),
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w[0] == "--batch" && w[1] == "/tmp/manifest.json"));
        assert!(!args.iter().any(|a| a.ends_with(".stl")), "no positional parts");
        assert!(args.iter().any(|a| a == "--log-level"), "5.x progress args present");
        assert!(
            args.windows(2).any(|w| w[0] == "--python-exit-code" && w[1] == "1"),
            "batch mode needs the exit-code guard too (pre-loop crashes, e.g. a bad manifest)"
        );
    }

    /// Same gap basecutter's build_base_cut_command closed: without this
    /// flag Blender exits 0 even after an uncaught Python exception (e.g.
    /// import_join's "File not found" SystemExit), so a pre-render crash
    /// could only be caught indirectly via the missing-output-file check.
    #[test]
    fn render_command_has_python_exit_code_guard() {
        let blender = BlenderInfo {
            path: "/usr/bin/blender".into(),
            version: "Blender 5.1.2".into(),
        };
        let options = RenderOptions {
            rotate: (90.0, 0.0, 0.0),
            color: None,
            azimuth: None,
            elevation: None,
            zoom: None,
            resolution: None,
            samples: None,
            look: None,
            output_path: None,
            overwrite: false,
            align_parts: false,
            look_config: None,
            scale_reference: false,
            base: None,
        };
        let cmd = build_render_command(
            &blender,
            Path::new("render_mini.py"),
            &["model.stl".to_string()],
            &options,
            Path::new("out.png"),
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.windows(2).any(|w| w[0] == "--python-exit-code" && w[1] == "1"));
        // Must come before the script is invoked (a global Blender option,
        // not a script argv flag after "--").
        let exit_code_idx = args.iter().position(|a| a == "--python-exit-code").unwrap();
        let separator_idx = args.iter().position(|a| a == "--").unwrap();
        assert!(exit_code_idx < separator_idx);
    }

    #[test]
    fn batch_manifest_serializes_the_script_contract() {
        let manifest = BatchManifest {
            entries: vec![BatchEntry {
                parts: vec!["/lib/a.stl".into()],
                out: "/tmp/0.png".into(),
                rotate: (90.0, 0.0, 0.0),
            }],
        };
        let json = serde_json::to_string(&manifest).unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(value["entries"][0]["parts"][0], "/lib/a.stl");
        assert_eq!(value["entries"][0]["out"], "/tmp/0.png");
        assert_eq!(value["entries"][0]["rotate"][0], 90.0);
    }

    #[test]
    fn parses_cycles_sample_lines() {
        assert_eq!(
            parse_sample_progress(
                "Fra:1 Mem:129.92M (Peak 137.85M) | Time:00:01.83 | Sample 32/96"
            ),
            Some((32, 96))
        );
        assert_eq!(
            parse_sample_progress("Sample 96/96, denoising"),
            Some((96, 96))
        );
        // Blender 5.x logging format
        assert_eq!(
            parse_sample_progress(
                "00:01.643  render           | Fra: 1 | Remaining: 00:09.21 | Mem: 2M | Sample 1/16"
            ),
            Some((1, 16))
        );
        assert_eq!(parse_sample_progress("Saved: 'out.png'"), None);
        assert_eq!(parse_sample_progress("Sample x/y"), None);
        assert_eq!(
            parse_sample_progress("Adaptive sampling: automatic min samples = 64"),
            None
        );
    }

    /// Full pipeline against a real Blender install: detection, command
    /// construction, headless render, progress parsing, output file.
    /// Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires a local Blender install and ~30s"]
    async fn renders_end_to_end_with_real_blender() {
        let blender = detect_blender()
            .await
            .expect("Blender not found — install it or set BLENDER_BIN");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/render_mini.py");
        let dir = std::env::temp_dir().join(format!("stlpack_render_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stl = dir.join("tetra.stl");
        write_test_stl(&stl);
        let out = dir.join("tetra.png");

        let options = RenderOptions {
            rotate: (90.0, 0.0, 0.0),
            color: None,
            azimuth: None,
            elevation: None,
            zoom: None,
            resolution: Some(128),
            samples: Some(8),
            look: Some("rich".to_string()),
            output_path: None,
            overwrite: true,
            align_parts: false,
            look_config: Some(r#"{"key":{"energy":1500}}"#.to_string()),
            scale_reference: false,
            base: None,
        };
        let mut cmd = build_render_command(
            &blender,
            &script,
            &[stl.to_string_lossy().into_owned()],
            &options,
            &out,
        );
        let output = cmd.output().await.expect("failed to launch blender");
        let stdout = String::from_utf8_lossy(&output.stdout);

        assert!(output.status.success(), "blender failed:\n{}", stdout);
        assert!(out.is_file(), "no output image written");
        assert!(
            stdout
                .lines()
                .any(|line| parse_sample_progress(line).is_some()),
            "no parseable Sample progress lines:\n{}",
            stdout
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Drives the actual shared harness (`run_blender_lines`), not just
    /// command construction — the render pipeline had no fast integration
    /// test exercising the refactored-out plumbing itself (spawn, stdout
    /// piping, cancel-select loop, wait). `run_blender` in
    /// render/commands.rs needs an `AppHandle` to test the same way, which
    /// is impractical to construct headless; `run_blender_lines` needing
    /// only a `Command` + closure is exactly the "factor the testable core"
    /// escape hatch — it's the part worth testing against a real Blender
    /// anyway, since render/commands.rs's `run_blender` is now a thin
    /// wrapper (event emission + output-file check) around it.
    /// Run with: cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires a local Blender install and ~30s"]
    async fn run_blender_lines_end_to_end_with_real_blender() {
        let blender = detect_blender()
            .await
            .expect("Blender not found — install it or set BLENDER_BIN");
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/render_mini.py");
        let dir = std::env::temp_dir().join(format!("stlpack_run_lines_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let stl = dir.join("tetra.stl");
        write_test_stl(&stl);
        let out = dir.join("tetra.png");

        let options = RenderOptions {
            rotate: (90.0, 0.0, 0.0),
            color: None,
            azimuth: None,
            elevation: None,
            zoom: None,
            resolution: Some(128),
            samples: Some(8),
            look: Some("rich".to_string()),
            output_path: None,
            overwrite: true,
            align_parts: false,
            look_config: None,
            scale_reference: false,
            base: None,
        };
        let cmd = build_render_command(
            &blender,
            &script,
            &[stl.to_string_lossy().into_owned()],
            &options,
            &out,
        );

        let mut saw_progress = false;
        let run = run_blender_lines(cmd, None, |line| {
            if parse_sample_progress(line).is_some() {
                saw_progress = true;
            }
            ControlFlow::Continue(())
        })
        .await
        .unwrap_or_else(|e| panic!("run_blender_lines failed: {:?}", e));

        assert!(run.status.success(), "blender exited with {}", run.status);
        assert!(out.is_file(), "no output image written");
        assert!(
            saw_progress,
            "no parseable Sample progress lines observed via on_line; stdout tail:\n{}",
            run.stdout_tail
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    fn write_test_stl(path: &Path) {
        // Minimal binary STL: one tetrahedron. Blender recomputes the
        // zeroed normals on import.
        let verts: [[f32; 3]; 4] = [
            [0.0, 0.0, 0.0],
            [10.0, 0.0, 0.0],
            [0.0, 10.0, 0.0],
            [0.0, 0.0, 10.0],
        ];
        let faces = [[0usize, 1, 2], [0, 1, 3], [0, 2, 3], [1, 2, 3]];
        let mut buf = vec![0u8; 80];
        buf.extend_from_slice(&(faces.len() as u32).to_le_bytes());
        for face in faces {
            for _ in 0..3 {
                buf.extend_from_slice(&0f32.to_le_bytes());
            }
            for vi in face {
                for c in verts[vi] {
                    buf.extend_from_slice(&c.to_le_bytes());
                }
            }
            buf.extend_from_slice(&0u16.to_le_bytes());
        }
        std::fs::write(path, buf).unwrap();
    }

    /// The look overrides must survive as EXACTLY one argv element — if the
    /// JSON ever got split on whitespace or shell-quoted, Python would see
    /// garbage paths instead of a config.
    #[test]
    fn look_config_passes_as_single_arg() {
        let blender = BlenderInfo {
            path: "blender".to_string(),
            version: "Blender 4.2.1".to_string(),
        };
        let json = r#"{"key": {"energy": 5000}, "sss_radius": [1, 0.5, 0.25]}"#;
        let options = RenderOptions {
            rotate: (90.0, 0.0, 0.0),
            color: None,
            azimuth: None,
            elevation: None,
            zoom: None,
            resolution: None,
            samples: None,
            look: None,
            output_path: None,
            overwrite: false,
            align_parts: false,
            look_config: Some(json.to_string()),
            scale_reference: false,
            base: None,
        };
        let cmd = build_render_command(
            &blender,
            Path::new("render_mini.py"),
            &["model.stl".to_string()],
            &options,
            Path::new("out.png"),
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let idx = args
            .iter()
            .position(|a| a == "--config")
            .expect("--config missing");
        assert_eq!(args[idx + 1], json);

        // None and blank configs add no flag at all
        for empty in [None, Some("  ".to_string())] {
            let options = RenderOptions {
                look_config: empty,
                ..options.clone()
            };
            let cmd = build_render_command(
                &blender,
                Path::new("render_mini.py"),
                &["model.stl".to_string()],
                &options,
                Path::new("out.png"),
            );
            assert!(!cmd.as_std().get_args().any(|a| a == "--config"));
        }
    }

    /// `--base` must carry BOTH the user's nominal footprint and Rust's
    /// already-derived cut (top-face) footprint — the script must never
    /// re-derive the taper shrink itself (docs/BASECUTTER.md "The plinth").
    #[test]
    fn base_arg_carries_nominal_and_derived_cut() {
        use crate::basecutter::cutters::CutterKind;

        let blender = BlenderInfo {
            path: "blender".to_string(),
            version: "Blender 4.2.1".to_string(),
        };
        let options = RenderOptions {
            rotate: (90.0, 0.0, 0.0),
            color: None,
            azimuth: None,
            elevation: None,
            zoom: None,
            resolution: None,
            samples: None,
            look: None,
            output_path: None,
            overwrite: false,
            align_parts: false,
            look_config: None,
            scale_reference: false,
            base: Some(CutterKind::Circle { diameter_mm: 32.0 }),
        };
        let cmd = build_render_command(
            &blender,
            Path::new("render_mini.py"),
            &["model.stl".to_string()],
            &options,
            Path::new("out.png"),
        );
        let args: Vec<String> = cmd
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let idx = args.iter().position(|a| a == "--base").expect("--base missing");
        let payload: serde_json::Value = serde_json::from_str(&args[idx + 1]).unwrap();
        assert_eq!(payload["nominal"]["kind"], "circle");
        assert_eq!(payload["nominal"]["diameter_mm"], 32.0);
        assert_eq!(payload["cut"]["kind"], "circle");
        // 32 - 2*3.7*tan(15deg) = 30.017, same derivation
        // top_face_of_circle_matches_measured_taper pins in basecutter::cutters
        let cut_diameter = payload["cut"]["diameter_mm"].as_f64().unwrap();
        assert!((cut_diameter - 30.017).abs() < 0.01);
        assert_eq!(payload["height_mm"], 3.7);
        assert_eq!(payload["taper_deg"], 15.0);

        // None = no flag at all
        let options = RenderOptions { base: None, ..options };
        let cmd = build_render_command(
            &blender,
            Path::new("render_mini.py"),
            &["model.stl".to_string()],
            &options,
            Path::new("out.png"),
        );
        assert!(!cmd.as_std().get_args().any(|a| a == "--base"));
    }

    #[test]
    fn progress_args_by_version() {
        assert_eq!(
            progress_args("Blender 5.1.2"),
            &["--log-level", "info"] as &[&str]
        );
        assert_eq!(
            progress_args("Blender 6.0.0"),
            &["--log-level", "info"] as &[&str]
        );
        assert!(progress_args("Blender 4.2.1 LTS").is_empty());
        assert!(progress_args("garbage").is_empty());
    }
}
