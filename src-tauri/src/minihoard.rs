//! Easter-egg integration with minihoard, Plinth's sibling CLI for
//! fetching a MyMiniFactory library. If the binary is installed on this
//! machine a "Minihoard" entry quietly appears in the sidebar; if not,
//! Plinth never mentions it. Deliberately undocumented on Plinth's side —
//! only minihoard's own docs describe the integration.
//!
//! The integration is a thin console, not an API: minihoard speaks human
//! text (no JSON output mode), so Plinth runs a WHITELISTED subcommand and
//! streams its stdout/stderr lines to the webview verbatim. Parsing that
//! text would couple us to a tool that versions independently; showing it
//! doesn't.

use crate::error::AppError;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use specta::Type;
use crate::process::new_command;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::AppHandle;
use tauri_specta::Event;
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug, Type)]
pub struct MinihoardInfo {
    pub path: String,
    pub version: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type, Event)]
pub enum MinihoardStatus {
    Line(MinihoardLine),
    Finished(MinihoardFinished),
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct MinihoardLine {
    pub job_id: String,
    pub line: String,
    /// stderr lines render dimmer — minihoard uses stderr for progress
    /// chatter, stdout for the actual answers.
    pub is_err: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct MinihoardFinished {
    pub job_id: String,
    pub success: bool,
    /// Present when the process couldn't run at all (spawn failure).
    pub error: Option<String>,
}

/// One run at a time: minihoard owns per-machine state (credentials,
/// download dirs), and the console UI shows a single stream anyway.
static ACTIVE_RUN: Lazy<Mutex<Option<(String, Child)>>> = Lazy::new(|| Mutex::new(None));

/// Subcommands the console may launch. Everything here is either read-only
/// or downloads into minihoard's own configured directories; account
/// mutation (login/logout/configure) stays in the real terminal where
/// minihoard can prompt interactively.
const ALLOWED_SUBCOMMANDS: &[&str] = &["list", "download", "get", "config", "where", "whoami"];

fn binary_candidates() -> Vec<PathBuf> {
    let name = if cfg!(windows) {
        "minihoard.exe"
    } else {
        "minihoard"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    // PATH first — wherever the user's shell would find it
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(name)));
    }
    // then minihoard's own install destinations, which a GUI app launched
    // from Finder/Explorer often does NOT have on its PATH
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin").join(name));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("minihoard/bin").join(name));
    }
    candidates
}

/// Find minihoard and prove it runs (`--version`). None simply means the
/// egg stays hidden.
#[tauri::command]
#[specta::specta]
pub async fn detect_minihoard() -> Option<MinihoardInfo> {
    tauri::async_runtime::spawn_blocking(|| {
        for candidate in binary_candidates() {
            if !candidate.is_file() {
                continue;
            }
            let Ok(output) = new_command(&candidate).arg("--version").output() else {
                continue;
            };
            if !output.status.success() {
                continue;
            }
            // "minihoard 0.3.1" -> "0.3.1"
            let stdout = String::from_utf8_lossy(&output.stdout);
            let version = stdout.split_whitespace().nth(1).unwrap_or("?").to_string();
            return Some(MinihoardInfo {
                path: candidate.to_string_lossy().into_owned(),
                version,
            });
        }
        None
    })
    .await
    .ok()
    .flatten()
}

/// Launch one whitelisted minihoard run and stream its output as events.
/// Returns the job id immediately; MinihoardStatus::Finished closes it.
#[tauri::command]
#[specta::specta]
pub async fn run_minihoard(
    app_handle: AppHandle,
    binary_path: String,
    args: Vec<String>,
) -> Result<String, AppError> {
    let subcommand = validated_subcommand(&args)?;
    // The binary path round-trips through the webview (detect → run); only
    // accept something detect could actually have produced.
    let binary = PathBuf::from(&binary_path);
    if !binary_candidates().contains(&binary) || !binary.is_file() {
        return Err(AppError::InvalidInput(
            "Unrecognized minihoard binary path".to_string(),
        ));
    }

    let mut active = ACTIVE_RUN
        .lock()
        .map_err(|e| AppError::ConfigError(format!("minihoard registry poisoned: {}", e)))?;
    if let Some((_, child)) = active.as_mut() {
        // reap a finished child instead of refusing forever
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => *active = None,
            Ok(None) => {
                return Err(AppError::InvalidInput(
                    "minihoard is already running — wait or cancel it first".to_string(),
                ))
            }
        }
    }

    let mut command = new_command(&binary);
    command
        .args(&args)
        // batch downloads confirm interactively; there is no terminal here
        .arg_if(subcommand == "download" || subcommand == "get", "-y")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|e| AppError::IoError(format!("Failed to launch minihoard: {}", e)))?;

    let job_id = Uuid::new_v4().to_string();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    *active = Some((job_id.clone(), child));
    drop(active);

    // One reader thread per pipe: interleaving preserves rough real-time
    // order, and a filled stderr pipe can't deadlock stdout (or vice versa).
    if let Some(stdout) = stdout {
        spawn_line_reader(app_handle.clone(), job_id.clone(), stdout, false);
    }
    if let Some(stderr) = stderr {
        spawn_line_reader(app_handle.clone(), job_id.clone(), stderr, true);
    }

    // Waiter: emits Finished and frees the single-run slot.
    let waiter_app = app_handle.clone();
    let waiter_job = job_id.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let status = loop {
            let mut active = match ACTIVE_RUN.lock() {
                Ok(active) => active,
                Err(_) => return,
            };
            match active.as_mut() {
                Some((id, child)) if *id == waiter_job => match child.try_wait() {
                    Ok(Some(status)) => {
                        *active = None;
                        break Ok(status);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        *active = None;
                        break Err(e);
                    }
                },
                // cancelled (slot cleared) or replaced — nothing left to report
                _ => return,
            }
            drop(active);
            std::thread::sleep(std::time::Duration::from_millis(150));
        };
        let (success, error) = match status {
            Ok(status) => (status.success(), None),
            Err(e) => (false, Some(e.to_string())),
        };
        MinihoardStatus::Finished(MinihoardFinished {
            job_id: waiter_job,
            success,
            error,
        })
        .emit(&waiter_app)
        .ok();
    });

    Ok(job_id)
}

/// Kill the active run (the "stop" button). Emits Finished(success=false).
#[tauri::command]
#[specta::specta]
pub async fn cancel_minihoard(app_handle: AppHandle, job_id: String) -> Result<(), AppError> {
    let mut active = ACTIVE_RUN
        .lock()
        .map_err(|e| AppError::ConfigError(format!("minihoard registry poisoned: {}", e)))?;
    match active.take() {
        Some((id, mut child)) if id == job_id => {
            child.kill().ok();
            child.wait().ok(); // reap — no zombie
            MinihoardStatus::Finished(MinihoardFinished {
                job_id,
                success: false,
                error: None,
            })
            .emit(&app_handle)
            .ok();
            Ok(())
        }
        other => {
            *active = other; // put back a different run untouched
            Err(AppError::NotFoundError(format!(
                "No active minihoard run with ID: {}",
                job_id
            )))
        }
    }
}

fn spawn_line_reader<R: std::io::Read + Send + 'static>(
    app_handle: AppHandle,
    job_id: String,
    pipe: R,
    is_err: bool,
) {
    tauri::async_runtime::spawn_blocking(move || {
        for line in BufReader::new(pipe).lines() {
            let Ok(line) = line else { break };
            // lines() only strips \n — Windows children emit \r\n
            let line = line.trim_end_matches('\r').to_string();
            MinihoardStatus::Line(MinihoardLine {
                job_id: job_id.clone(),
                line,
                is_err,
            })
            .emit(&app_handle)
            .ok();
        }
    });
}

/// Tiny ergonomic shim: `Command` has no conditional-arg builder.
trait ArgIf {
    fn arg_if(&mut self, condition: bool, arg: &str) -> &mut Self;
}
impl ArgIf for Command {
    fn arg_if(&mut self, condition: bool, arg: &str) -> &mut Self {
        if condition {
            self.arg(arg);
        }
        self
    }
}

fn validated_subcommand(args: &[String]) -> Result<String, AppError> {
    let subcommand = args.first().cloned().unwrap_or_default();
    if ALLOWED_SUBCOMMANDS.contains(&subcommand.as_str()) {
        Ok(subcommand)
    } else {
        Err(AppError::InvalidInput(format!(
            "'{}' isn't available from Plinth — run it in a terminal",
            subcommand
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The console must stay read-only + downloads: credential mutation
    /// (login/logout/configure) and self-update need a real terminal, and
    /// nothing the webview sends may smuggle another subcommand through.
    #[test]
    fn only_safe_subcommands_pass_the_whitelist() {
        for allowed in ["list", "download", "get", "config", "where", "whoami"] {
            assert!(validated_subcommand(&[allowed.to_string()]).is_ok());
        }
        for refused in ["login", "logout", "configure", "upgrade", "set-cookie", ""] {
            assert!(
                validated_subcommand(&[refused.to_string()]).is_err(),
                "'{}' must be refused",
                refused
            );
        }
        assert!(validated_subcommand(&[]).is_err());
    }
}
