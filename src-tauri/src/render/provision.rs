//! Provisioning a managed Blender: knowing which versions are good enough,
//! and (in later steps) downloading a pinned one when the user has none.
//!
//! Renders are look-locked on Blender 5.1 — older majors light and tone-map
//! the locked look differently. Anything >= 4.2 still *works*, so the gate
//! never blocks: it classifies, and the UI decides how loudly to suggest
//! the managed download.

use crate::error::AppError;
use crate::models::{BlenderCheck, BlenderVerdict};
use crate::render::engine;
use once_cell::sync::OnceCell;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

/// The exact version the download pipeline installs. Bumping this is the
/// whole release procedure: the mirror's .sha256 sidecar is fetched at
/// download time, so no hashes live here.
pub const MANAGED_VERSION: &str = "5.1.2";

/// Below this (major, minor) rendering is unsupported outright.
pub const MIN_VERSION: (u32, u32) = (4, 2);

/// Below this the locked look renders differently; the UI suggests (but
/// never forces) the managed download.
pub const RECOMMENDED_VERSION: (u32, u32) = (5, 1);

/// Where managed installs live. Seeded once at startup because the
/// detector's candidate_paths() is synchronous and handle-less — the same
/// reason SETTINGS_CACHE exists. NOT the exe-relative Resources dir the
/// detector also probes: that one is for a Blender *bundled into* the app,
/// and writing there at runtime would break macOS code signing.
static APP_DATA_DIR: OnceCell<PathBuf> = OnceCell::new();

/// Seed APP_DATA_DIR and sweep staging leftovers from crashed downloads.
pub fn init_app_data_dir(app: &AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = APP_DATA_DIR.set(dir);
    }
    if let Some(root) = managed_root() {
        sweep_stale_staging(&root);
    }
}

/// `<app_data>/blender` — one version dir per managed install inside.
pub fn managed_root() -> Option<PathBuf> {
    APP_DATA_DIR.get().map(|dir| dir.join("blender"))
}

/// The platform binary inside a managed version dir, mirroring the layout
/// each official archive extracts to.
fn binary_in_version_dir(dir: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    return dir.join("Blender.app/Contents/MacOS/Blender");
    #[cfg(target_os = "linux")]
    return dir.join("blender");
    #[cfg(target_os = "windows")]
    return dir.join("blender.exe");
}

/// The binary of the highest-versioned managed install that actually has
/// one — a half-written dir (no binary) never wins.
pub fn managed_binary() -> Option<PathBuf> {
    managed_binary_in(&managed_root()?)
}

fn managed_binary_in(root: &Path) -> Option<PathBuf> {
    let mut versions: Vec<(u32, u32, u32, PathBuf)> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let (major, minor, patch) = parse_blender_version(&name.to_string_lossy())?;
            let binary = binary_in_version_dir(&entry.path());
            binary.is_file().then_some((major, minor, patch, binary))
        })
        .collect();
    versions.sort();
    versions.pop().map(|(_, _, _, binary)| binary)
}

/// Downloads assemble under dot-prefixed staging dirs and only get renamed
/// into place whole; anything still staging-named at startup is debris from
/// a crash mid-download.
fn sweep_stale_staging(root: &Path) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(".staging-") {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Pull (major, minor, patch) out of a `--version` banner such as
/// "Blender 4.2.1 LTS" or "Blender 5.1.2". Missing components default to 0;
/// a banner with no leading number at all is None.
pub fn parse_blender_version(banner: &str) -> Option<(u32, u32, u32)> {
    let numbers = banner
        .trim_start_matches("Blender")
        .trim_start()
        .split_whitespace()
        .next()?;
    let mut parts = numbers.split('.');
    let major: u32 = parts.next()?.trim().parse().ok()?;
    let minor: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let patch: u32 = parts.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// Verdict for a *found* install — the caller maps detection failure to
/// Missing. An unparseable banner passes as Ok: never block a working
/// Blender on a banner format change (progress_args takes the same stance).
pub fn classify(banner: &str) -> BlenderVerdict {
    match parse_blender_version(banner) {
        Some((major, minor, _)) if (major, minor) < MIN_VERSION => BlenderVerdict::TooOld,
        Some((major, minor, _)) if (major, minor) < RECOMMENDED_VERSION => BlenderVerdict::Outdated,
        _ => BlenderVerdict::Ok,
    }
}

/// Detection plus the version verdict, in one call the first-run dialog,
/// Settings, and the Render view all share.
#[tauri::command]
#[specta::specta]
pub async fn check_blender() -> Result<BlenderCheck, AppError> {
    match engine::detect_blender().await {
        Ok(info) => Ok(BlenderCheck {
            verdict: classify(&info.version),
            is_managed: managed_root()
                .is_some_and(|root| Path::new(&info.path).starts_with(&root)),
            info: Some(info),
            managed_version: MANAGED_VERSION.to_string(),
        }),
        Err(AppError::NotFoundError(_)) => Ok(BlenderCheck {
            verdict: BlenderVerdict::Missing,
            info: None,
            is_managed: false,
            managed_version: MANAGED_VERSION.to_string(),
        }),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_banners() {
        assert_eq!(parse_blender_version("Blender 5.1.2"), Some((5, 1, 2)));
        assert_eq!(parse_blender_version("Blender 4.2.1 LTS"), Some((4, 2, 1)));
        assert_eq!(parse_blender_version("Blender 4.5"), Some((4, 5, 0)));
        assert_eq!(parse_blender_version("garbage"), None);
        assert_eq!(parse_blender_version("Blender Foundation"), None);
    }

    #[test]
    fn highest_complete_managed_version_wins() {
        let root = std::env::temp_dir().join(format!("stl-pack-managed-{}", uuid::Uuid::new_v4()));

        // No root dir at all -> no managed install
        assert_eq!(managed_binary_in(&root), None);

        // 4.2.0 complete, 5.1.2 complete, 6.0.0 half-written (no binary)
        for version in ["4.2.0", "5.1.2"] {
            let binary = binary_in_version_dir(&root.join(version));
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            std::fs::write(&binary, "stub").unwrap();
        }
        std::fs::create_dir_all(root.join("6.0.0")).unwrap();

        let winner = managed_binary_in(&root).expect("a managed binary");
        assert!(winner.starts_with(root.join("5.1.2")));

        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn classifies_against_the_gate() {
        assert!(matches!(classify("Blender 4.1.1"), BlenderVerdict::TooOld));
        assert!(matches!(classify("Blender 3.6.5 LTS"), BlenderVerdict::TooOld));
        assert!(matches!(classify("Blender 4.2.0"), BlenderVerdict::Outdated));
        assert!(matches!(classify("Blender 5.0.9"), BlenderVerdict::Outdated));
        assert!(matches!(classify("Blender 5.1.0"), BlenderVerdict::Ok));
        assert!(matches!(classify("Blender 6.0.0"), BlenderVerdict::Ok));
        // Lenient on the unknown: a working Blender is never blocked on
        // a banner we can't read
        assert!(matches!(classify("Blender next"), BlenderVerdict::Ok));
    }
}
