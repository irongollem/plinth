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
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};
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
    if parts.is_empty() {
        return Err(AppError::InvalidInput(
            "At least one STL part is required".to_string(),
        ));
    }
    for part in &parts {
        if !Path::new(part).is_file() {
            return Err(AppError::NotFoundError(format!("STL not found: {}", part)));
        }
    }

    let blender = engine::detect_blender_cached().await?;
    let script = app_handle
        .path()
        .resolve("resources/render_mini.py", BaseDirectory::Resource)
        .map_err(|e| AppError::ConfigError(format!("Render script not found: {}", e)))?;

    let output_path = match &options.output_path {
        Some(out) => PathBuf::from(out),
        None => PathBuf::from(&parts[0]).with_extension("png"),
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
