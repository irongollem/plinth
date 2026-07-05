use super::engine;
use crate::error::AppError;
use crate::models::events::{
    RenderCancelledStatus, RenderCompletedStatus, RenderFailedStatus, RenderProgressStatus,
    RenderStartedStatus, RenderStatus,
};
use crate::models::{BlenderInfo, RenderOptions};
use once_cell::sync::Lazy;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;
use tauri_specta::Event;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Notify;
use uuid::Uuid;

static ACTIVE_RENDERS: Lazy<Mutex<HashMap<String, Arc<Notify>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[tauri::command]
#[specta::specta]
pub async fn detect_blender() -> Result<BlenderInfo, AppError> {
    engine::detect_blender().await
}

#[tauri::command]
#[specta::specta]
pub async fn start_render(
    app_handle: AppHandle,
    parts: Vec<String>,
    options: RenderOptions,
) -> Result<String, AppError> {
    // The Blender script imports STL only — slicer scenes (.lys, .chitubox)
    // and other sidecar files that ride along in model folders would make
    // the import step die mid-render with an opaque Blender error. Filter
    // here so every caller gets the same guarantee.
    let parts: Vec<String> = parts
        .into_iter()
        .filter(|p| {
            Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("stl"))
        })
        .collect();
    if parts.is_empty() {
        return Err(AppError::InvalidInput(
            "No renderable files: the render engine imports .stl parts only".to_string(),
        ));
    }
    for part in &parts {
        if !Path::new(part).is_file() {
            return Err(AppError::NotFoundError(format!("STL not found: {}", part)));
        }
    }

    let blender = engine::detect_blender_cached().await?;
    let script = engine::materialize_render_script(&app_handle)?;

    let output_path = match &options.output_path {
        Some(out) => PathBuf::from(out),
        None => PathBuf::from(&parts[0]).with_extension("png"),
    };
    // Never clobber silently: without explicit overwrite consent an
    // existing file gets a -1/-2/... suffix (the Started/Completed events
    // carry the real path, so the UI always shows where it went)
    let output_path = if options.overwrite {
        output_path
    } else {
        unique_path(output_path)
    };

    let job_id = Uuid::new_v4().to_string();
    let cancel_token = Arc::new(Notify::new());
    {
        let mut renders = ACTIVE_RENDERS.lock().map_err(|e| {
            AppError::ConfigError(format!("Failed to access render registry: {}", e))
        })?;
        renders.insert(job_id.clone(), Arc::clone(&cancel_token));
    }

    let job_id_clone = job_id.clone();
    tokio::spawn(async move {
        run_render_job(
            app_handle,
            job_id_clone,
            blender,
            script,
            parts,
            options,
            output_path,
            cancel_token,
        )
        .await;
    });

    Ok(job_id)
}

/// First non-existing variant of `path`: name.png, name-1.png, name-2.png...
fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "render".to_string());
    let extension = path
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_else(|| "png".to_string());
    let parent = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for n in 1.. {
        let candidate = parent.join(format!("{}-{}.{}", stem, n, extension));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!("ran out of integers before file names")
}

/// Read an image as a data URL for the branding bake. The webview draws
/// the finished render (plus logo) onto a canvas — but images loaded via
/// the asset: protocol are cross-origin and TAINT the canvas, making
/// toBlob() throw. Data URLs don't, so the bytes take this detour.
#[tauri::command]
#[specta::specta]
pub async fn read_image_base64(path: String) -> Result<String, AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path)
            .map_err(|e| AppError::IoError(format!("Failed to read image {}: {}", path, e)))?;
        let mime = match Path::new(&path)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref()
        {
            Some("jpg") | Some("jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            Some("svg") => "image/svg+xml",
            _ => "image/png",
        };
        Ok(format!("data:{};base64,{}", mime, STANDARD.encode(&bytes)))
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Image read task failed: {}", e)))?
}

/// Overwrite an existing render with the branding-composited PNG the
/// webview produced. Three guards keep a bad bake from eating a good
/// render: the target must already exist (this only ever re-writes a
/// finished render), the bytes must carry the PNG magic (a blank/failed
/// canvas export dies here, not on disk), and the write goes through
/// temp + rename so a crash can't leave a half-written file.
#[tauri::command]
#[specta::specta]
pub async fn write_png_base64(path: String, data: String) -> Result<(), AppError> {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    tauri::async_runtime::spawn_blocking(move || {
        let target = PathBuf::from(&path);
        if !target.is_file() {
            return Err(AppError::NotFoundError(format!(
                "Refusing to write composited image: {} does not exist",
                path
            )));
        }
        // Accept a full data URL or bare base64 — everything after the
        // last comma is the payload either way
        let payload = data.rsplit(',').next().unwrap_or(&data);
        let bytes = STANDARD
            .decode(payload)
            .map_err(|e| AppError::InvalidInput(format!("Invalid image data: {}", e)))?;
        if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
            return Err(AppError::InvalidInput(
                "Composited data is not a PNG — keeping the original render".to_string(),
            ));
        }
        let tmp = target.with_extension("png.tmp");
        std::fs::write(&tmp, &bytes)
            .map_err(|e| AppError::IoError(format!("Failed to write composited image: {}", e)))?;
        std::fs::rename(&tmp, &target).map_err(|e| {
            std::fs::remove_file(&tmp).ok();
            AppError::IoError(format!("Failed to replace render with composite: {}", e))
        })
    })
    .await
    .map_err(|e| AppError::ConfigError(format!("Image write task failed: {}", e)))?
}

#[tauri::command]
#[specta::specta]
pub async fn cancel_render(job_id: String) -> Result<(), AppError> {
    let renders = ACTIVE_RENDERS
        .lock()
        .map_err(|e| AppError::ConfigError(format!("Failed to access render registry: {}", e)))?;

    if let Some(token) = renders.get(&job_id) {
        token.notify_waiters();
        Ok(())
    } else {
        Err(AppError::NotFoundError(format!(
            "No active render job with ID: {}",
            job_id
        )))
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_render_job(
    app_handle: AppHandle,
    job_id: String,
    blender: BlenderInfo,
    script: PathBuf,
    parts: Vec<String>,
    options: RenderOptions,
    output_path: PathBuf,
    cancel_token: Arc<Notify>,
) {
    let start_time = std::time::Instant::now();

    RenderStatus::Started(RenderStartedStatus {
        job_id: job_id.clone(),
        output_path: output_path.to_string_lossy().into_owned(),
    })
    .emit(&app_handle)
    .ok();

    let result = run_blender(
        &app_handle,
        &job_id,
        &blender,
        &script,
        &parts,
        &options,
        &output_path,
        &cancel_token,
    )
    .await;

    if let Ok(mut renders) = ACTIVE_RENDERS.lock() {
        renders.remove(&job_id);
    }

    match result {
        Ok(()) => {
            RenderStatus::Completed(RenderCompletedStatus {
                job_id,
                output_path: output_path.to_string_lossy().into_owned(),
                elapsed_seconds: start_time.elapsed().as_secs_f64(),
            })
            .emit(&app_handle)
            .ok();
        }
        Err(AppError::UserCancelled(_)) => {
            RenderStatus::Cancelled(RenderCancelledStatus { job_id })
                .emit(&app_handle)
                .ok();
        }
        Err(e) => {
            RenderStatus::Failed(RenderFailedStatus {
                job_id,
                error: e.to_string(),
            })
            .emit(&app_handle)
            .ok();
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_blender(
    app_handle: &AppHandle,
    job_id: &str,
    blender: &BlenderInfo,
    script: &Path,
    parts: &[String],
    options: &RenderOptions,
    output_path: &Path,
    cancel_token: &Notify,
) -> Result<(), AppError> {
    let mut cmd = engine::build_render_command(blender, script, parts, options, output_path);

    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| AppError::IoError(format!("Failed to launch Blender: {}", e)))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| AppError::IoError("Failed to capture Blender stdout".to_string()))?;
    let stderr = child.stderr.take();

    // Collect stderr in the background for error reporting
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
    let mut last_percent: u32 = 0;

    // Register cancellation interest ONCE and keep the future alive across
    // loop iterations: notify_waiters() stores no permit, so a fresh
    // notified() per iteration would drop a cancel that lands while the
    // loop body is processing a line.
    let cancelled = cancel_token.notified();
    tokio::pin!(cancelled);

    loop {
        tokio::select! {
            _ = &mut cancelled => {
                child.kill().await.ok();
                return Err(AppError::UserCancelled("Render cancelled".to_string()));
            }
            line = stdout_lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        if stdout_tail.len() >= 10 {
                            stdout_tail.pop_front();
                        }
                        stdout_tail.push_back(line.clone());

                        if let Some((current, total)) = engine::parse_sample_progress(&line) {
                            let percent = (current * 100) / total;
                            if percent != last_percent {
                                last_percent = percent;
                                RenderStatus::Progress(RenderProgressStatus {
                                    job_id: job_id.to_string(),
                                    current_sample: current,
                                    total_samples: total,
                                    percent,
                                })
                                .emit(app_handle)
                                .ok();
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(e) => return Err(AppError::IoError(format!("Failed reading Blender output: {}", e))),
                }
            }
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| AppError::IoError(format!("Failed waiting for Blender: {}", e)))?;

    if !status.success() {
        let stderr_lines = stderr_tail
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        let stdout_lines = stdout_tail.iter().cloned().collect::<Vec<_>>().join("\n");
        return Err(AppError::FileProcessingError(format!(
            "Blender exited with {}\n{}\n{}",
            status, stdout_lines, stderr_lines
        )));
    }

    if !output_path.is_file() {
        return Err(AppError::FileProcessingError(format!(
            "Blender finished but no image was written to {}",
            output_path.display()
        )));
    }

    Ok(())
}
