use crate::error::AppError;
use crate::models::{BlenderInfo, RenderOptions};
use crate::settings::SETTINGS_CACHE;
use once_cell::sync::Lazy;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use tokio::process::Command;

/// The Blender script ships INSIDE the binary. As a bundled resource it was
/// only re-copied next to the binary when the Rust code rebuilt, so pure
/// script edits silently kept rendering with a stale copy during dev.
const RENDER_SCRIPT: &str = include_str!("../../resources/render_mini.py");

/// Write the embedded script where Blender can read it. Always overwrites,
/// so the file on disk can never drift from the built app.
pub fn materialize_render_script(app_handle: &AppHandle) -> Result<PathBuf, AppError> {
    let dir = app_handle
        .path()
        .app_cache_dir()
        .or_else(|_| app_handle.path().app_data_dir())
        .map_err(|e| AppError::ConfigError(format!("No writable app dir: {}", e)))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| AppError::IoError(format!("Failed to create app dir: {}", e)))?;
    let path = dir.join("render_mini.py");
    std::fs::write(&path, RENDER_SCRIPT)
        .map_err(|e| AppError::IoError(format!("Failed to write render script: {}", e)))?;
    Ok(path)
}

/// (blender_path setting at detection time, detected install)
type CachedDetection = (Option<String>, BlenderInfo);

/// Last successful detection, keyed by the blender_path setting it was made
/// under. Detection spawns `blender --version` (a full Blender cold start),
/// far too expensive to repeat on every render.
static DETECTION_CACHE: Lazy<Mutex<Option<CachedDetection>>> = Lazy::new(|| Mutex::new(None));

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

    // A portable Blender shipped with (or downloaded next to) the app
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

pub fn new_command(binary: &Path) -> Command {
    let cmd = Command::new(binary);
    #[cfg(target_os = "windows")]
    let cmd = {
        let mut cmd = cmd;
        // CREATE_NO_WINDOW: don't flash a console per render
        cmd.creation_flags(0x08000000);
        cmd
    };
    cmd
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

/// Assemble the full headless render invocation for render_mini.py.
pub fn build_render_command(
    blender: &BlenderInfo,
    script: &Path,
    parts: &[String],
    options: &RenderOptions,
    output_path: &Path,
) -> Command {
    let mut cmd = new_command(Path::new(&blender.path));
    cmd.args(progress_args(&blender.version));
    cmd.arg("-b").arg("-P").arg(script).arg("--");
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
    cmd.arg("--out").arg(output_path);
    cmd
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

#[cfg(test)]
mod tests {
    use super::*;

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
