//! Provisioning a managed Blender: knowing which versions are good enough,
//! and (in later steps) downloading a pinned one when the user has none.
//!
//! Renders are look-locked on Blender 5.1 — older majors light and tone-map
//! the locked look differently. Anything >= 4.2 still *works*, so the gate
//! never blocks: it classifies, and the UI decides how loudly to suggest
//! the managed download.

use crate::error::AppError;
use crate::models::{BlenderCheck, BlenderInfo, BlenderVerdict};
use crate::render::engine;

/// The exact version the download pipeline installs. Bumping this is the
/// whole release procedure: the mirror's .sha256 sidecar is fetched at
/// download time, so no hashes live here.
pub const MANAGED_VERSION: &str = "5.1.2";

/// Below this (major, minor) rendering is unsupported outright.
pub const MIN_VERSION: (u32, u32) = (4, 2);

/// Below this the locked look renders differently; the UI suggests (but
/// never forces) the managed download.
pub const RECOMMENDED_VERSION: (u32, u32) = (5, 1);

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
            // Managed installs don't exist until the download pipeline lands
            is_managed: false,
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
