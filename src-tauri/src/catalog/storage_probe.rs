//! What a storage location actually supports, answered by doing it.
//!
//! Network shares route every operation through a protocol whose support
//! is a matter of the server's configuration, the client implementation,
//! and how the share was mounted — none of which can be inferred from a
//! path, a filesystem name, or the host OS. A Synology share reached over
//! SMB refuses hardlinks from macOS while accepting symlinks and
//! rename-over-existing from the same mount, and Windows reaching the same
//! share may answer differently again.
//!
//! So the checks here run the real operation and report what happened,
//! including the error when something fails: "merging is unavailable
//! because the volume returned Operation not supported" is actionable,
//! and "merging didn't work" is not.

use serde::{Deserialize, Serialize};
use specta::Type;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use crate::error::AppError;

/// Bytes the duplicate scanner reads before deciding two files might
/// match — the unit of work worth timing on a slow share.
const PREFIX_BYTES: usize = 128 * 1024;
/// Writes sampled for the read-after-write check.
const VISIBILITY_SAMPLES: usize = 20;
const VISIBILITY_TIMEOUT_MS: u128 = 5_000;

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Type)]
pub enum ProbeStatus {
    /// The operation worked.
    Ok,
    /// The volume refused it, and Plinth has to work without it.
    Unsupported,
    /// It worked, but in a way worth knowing about.
    Warn,
    /// It should have worked and didn't.
    Failed,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct ProbeCheck {
    pub id: String,
    pub label: String,
    pub status: ProbeStatus,
    pub detail: String,
    /// Wall-clock cost, where the number means something.
    pub millis: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
pub struct StorageReport {
    pub path: String,
    pub checks: Vec<ProbeCheck>,
}

fn check(id: &str, label: &str, status: ProbeStatus, detail: impl Into<String>) -> ProbeCheck {
    ProbeCheck {
        id: id.into(),
        label: label.into(),
        status,
        detail: detail.into(),
        millis: None,
    }
}

/// Whether this volume can hardlink, and the reason when it cannot.
///
/// The bool form is what the duplicates panel asks for; the error is what
/// the probe reports. One implementation so the panel and the report can
/// never disagree about the same volume.
pub fn hardlink_support(dir: &Path) -> Result<(), String> {
    let base = dir.join(format!(".plinth-link-probe-{}", std::process::id()));
    let link = dir.join(format!(".plinth-link-probe-{}.link", std::process::id()));
    let outcome = std::fs::write(&base, b"probe")
        .map_err(|e| format!("could not write a probe file: {}", e))
        .and_then(|()| std::fs::hard_link(&base, &link).map_err(|e| e.to_string()));
    std::fs::remove_file(&link).ok();
    std::fs::remove_file(&base).ok();
    outcome
}

/// Run every check against `dir`, in a scratch folder of our own that is
/// removed before returning.
pub fn probe(dir: &Path) -> Result<StorageReport, AppError> {
    if !dir.is_dir() {
        return Err(AppError::NotFoundError(format!(
            "'{}' is not a folder",
            dir.display()
        )));
    }
    let scratch = dir.join(format!(".plinth-probe-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).map_err(|e| {
        AppError::IoError(format!(
            "Cannot write to '{}': {} — a read-only mount can be scanned, but not packed, \
             merged or rendered into",
            dir.display(),
            e
        ))
    })?;

    let mut checks = vec![check(
        "writable",
        "Writable",
        ProbeStatus::Ok,
        "created and removed a folder here",
    )];
    checks.push(hardlinks(&scratch));
    checks.push(identity(&scratch));
    checks.push(case_sensitivity(&scratch));
    checks.push(read_after_write(&scratch));
    checks.push(atomic_replace(&scratch));
    checks.push(prefix_read(&scratch));

    std::fs::remove_dir_all(&scratch).ok();
    Ok(StorageReport {
        path: dir.to_string_lossy().into_owned(),
        checks,
    })
}

fn hardlinks(scratch: &Path) -> ProbeCheck {
    match hardlink_support(scratch) {
        Ok(()) => check(
            "hardlinks",
            "Hardlinks",
            ProbeStatus::Ok,
            "duplicates can be merged into one copy on disk",
        ),
        Err(e) => check(
            "hardlinks",
            "Hardlinks",
            ProbeStatus::Unsupported,
            format!(
                "{} — duplicates can be found and deleted here, but not merged",
                e
            ),
        ),
    }
}

/// Physical-file identity is how a merged duplicate group is told apart
/// from a reclaimable one. Two distinct files reporting the same identity
/// would make real copies look already-merged, which is the dangerous
/// direction: reclaimable space would go unreported.
fn identity(scratch: &Path) -> ProbeCheck {
    let a = scratch.join("identity-a.bin");
    let b = scratch.join("identity-b.bin");
    if std::fs::write(&a, b"a").is_err() || std::fs::write(&b, b"b").is_err() {
        return check(
            "identity",
            "File identity",
            ProbeStatus::Failed,
            "could not write the probe files",
        );
    }
    let (id_a, id_b) = (super::dups::file_identity(&a), super::dups::file_identity(&b));
    let detail = match (&id_a, &id_b) {
        (None, _) | (_, None) => {
            return check(
                "identity",
                "File identity",
                ProbeStatus::Warn,
                "this volume reports no per-file identity — every path counts as its own \
                 copy, so merged duplicates will keep being offered for merging",
            )
        }
        (Some(x), Some(y)) if x == y => {
            return check(
                "identity",
                "File identity",
                ProbeStatus::Failed,
                format!(
                    "two different files share the identity {} — copies would be \
                     misreported as already merged",
                    x
                ),
            )
        }
        (Some(x), Some(_)) => format!("distinct and stable (e.g. {})", x),
    };
    // A re-stat has to agree with the first, or identity means nothing
    // across the gap between a scan and a merge.
    if super::dups::file_identity(&a) != id_a {
        return check(
            "identity",
            "File identity",
            ProbeStatus::Failed,
            "the same file reported two different identities moments apart",
        );
    }
    check("identity", "File identity", ProbeStatus::Ok, detail)
}

/// The catalog compares paths as strings, so a volume that treats two
/// spellings as one file can hold the same folder under two names.
fn case_sensitivity(scratch: &Path) -> ProbeCheck {
    let lower = scratch.join("case-probe.bin");
    if std::fs::write(&lower, b"probe").is_err() {
        return check(
            "case",
            "Case sensitivity",
            ProbeStatus::Failed,
            "could not write the probe file",
        );
    }
    let upper = scratch.join("CASE-PROBE.BIN");
    if upper.is_file() {
        check(
            "case",
            "Case sensitivity",
            ProbeStatus::Warn,
            "case-insensitive — the same folder added under two spellings would index twice",
        )
    } else {
        check(
            "case",
            "Case sensitivity",
            ProbeStatus::Ok,
            "case-sensitive, like the catalog's own path comparisons",
        )
    }
}

/// Rendering and preview promotion stat a file straight after writing it.
/// A share that reports it missing for a moment turns that into a spurious
/// failure.
fn read_after_write(scratch: &Path) -> ProbeCheck {
    let mut immediate = 0usize;
    let mut worst_ms = 0u128;
    for i in 0..VISIBILITY_SAMPLES {
        let path = scratch.join(format!("visibility-{}.bin", i));
        if std::fs::write(&path, vec![b'x'; 4096]).is_err() {
            return check(
                "read_after_write",
                "Read-after-write",
                ProbeStatus::Failed,
                "could not write the probe files",
            );
        }
        let started = Instant::now();
        let mut attempts = 0u32;
        loop {
            attempts += 1;
            let visible = std::fs::metadata(&path).is_ok_and(|m| m.len() == 4096);
            if visible || started.elapsed().as_millis() > VISIBILITY_TIMEOUT_MS {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        if attempts == 1 {
            immediate += 1;
        }
        worst_ms = worst_ms.max(started.elapsed().as_millis());
    }
    let detail = format!(
        "{}/{} writes were visible on the first check (worst wait {} ms)",
        immediate, VISIBILITY_SAMPLES, worst_ms
    );
    let status = if immediate == VISIBILITY_SAMPLES {
        ProbeStatus::Ok
    } else {
        ProbeStatus::Warn
    };
    check("read_after_write", "Read-after-write", status, detail)
}

/// Packing, merging and preview promotion all write to a temp name and
/// rename over the target, so no path is ever observed missing.
fn atomic_replace(scratch: &Path) -> ProbeCheck {
    let src = scratch.join("replace-src.bin");
    let dst = scratch.join("replace-dst.bin");
    if std::fs::write(&src, b"new").is_err() || std::fs::write(&dst, b"old").is_err() {
        return check(
            "atomic_replace",
            "Replace in place",
            ProbeStatus::Failed,
            "could not write the probe files",
        );
    }
    match std::fs::rename(&src, &dst) {
        Ok(()) if std::fs::read(&dst).is_ok_and(|bytes| bytes == b"new") => check(
            "atomic_replace",
            "Replace in place",
            ProbeStatus::Ok,
            "renaming over an existing file works",
        ),
        Ok(()) => check(
            "atomic_replace",
            "Replace in place",
            ProbeStatus::Failed,
            "the rename reported success but the old contents are still there",
        ),
        Err(e) => check(
            "atomic_replace",
            "Replace in place",
            ProbeStatus::Unsupported,
            format!("{} — packing and merging cannot finish safely here", e),
        ),
    }
}

/// The duplicate scanner's unit of work. On a library of any size this
/// number, not the catalog, is what a scan's runtime is made of.
fn prefix_read(scratch: &Path) -> ProbeCheck {
    let path = scratch.join("prefix.bin");
    let payload = vec![b'p'; PREFIX_BYTES * 2];
    let written = std::fs::File::create(&path).and_then(|mut f| {
        f.write_all(&payload)?;
        f.sync_all()
    });
    if written.is_err() {
        return check(
            "prefix_read",
            "Prefix read",
            ProbeStatus::Failed,
            "could not write the probe file",
        );
    }
    let started = Instant::now();
    let mut buffer = vec![0u8; PREFIX_BYTES];
    let read = std::fs::File::open(&path).and_then(|mut f| f.read_exact(&mut buffer));
    let elapsed = started.elapsed();
    match read {
        Ok(()) => ProbeCheck {
            millis: Some(elapsed.as_millis() as u32),
            ..check(
                "prefix_read",
                "Prefix read",
                ProbeStatus::Ok,
                format!(
                    "read {} KiB in {} ms — a duplicate scan pays this per candidate",
                    PREFIX_BYTES / 1024,
                    elapsed.as_millis()
                ),
            )
        },
        Err(e) => check(
            "prefix_read",
            "Prefix read",
            ProbeStatus::Failed,
            format!("{}", e),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_folder_passes_every_check_it_should() {
        let dir = std::env::temp_dir().join(format!("plinth_probe_test_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();

        let report = probe(&dir).unwrap();
        let status = |id: &str| {
            report
                .checks
                .iter()
                .find(|c| c.id == id)
                .unwrap_or_else(|| panic!("missing check {}", id))
                .status
        };
        assert_eq!(status("writable"), ProbeStatus::Ok);
        assert_eq!(status("hardlinks"), ProbeStatus::Ok);
        assert_eq!(status("identity"), ProbeStatus::Ok);
        assert_eq!(status("read_after_write"), ProbeStatus::Ok);
        assert_eq!(status("atomic_replace"), ProbeStatus::Ok);
        assert_eq!(status("prefix_read"), ProbeStatus::Ok);
        // case sensitivity is a property of the volume, not a pass/fail —
        // macOS ships case-insensitive by default, Linux does not
        assert!(matches!(
            status("case"),
            ProbeStatus::Ok | ProbeStatus::Warn
        ));

        // the probe cleans up after itself, on someone's real library
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(leftovers.is_empty(), "left behind: {:?}", leftovers);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_is_refused_rather_than_created() {
        let missing = std::env::temp_dir().join("plinth_probe_does_not_exist_xyz");
        std::fs::remove_dir_all(&missing).ok();
        assert!(probe(&missing).is_err());
        assert!(!missing.exists());
    }
}
