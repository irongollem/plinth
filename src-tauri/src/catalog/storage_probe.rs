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
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::error::AppError;

/// Bytes the duplicate scanner reads before deciding two files might
/// match — the unit of work worth timing on a slow share.
const PREFIX_BYTES: usize = 128 * 1024;
/// How far the prefix-read sample will look for a real file before
/// falling back to one of its own.
const SAMPLE_DIR_BUDGET: usize = 24;
const SAMPLE_ENTRY_BUDGET: usize = 256;
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
    // create_dir, not create_dir_all: the latter succeeds on a directory
    // that already exists, which would pass the writability check without
    // writing anything. The name carries a timestamp so a leftover folder
    // from a crashed run cannot make this a false pass either.
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let scratch = dir.join(format!(".plinth-probe-{}-{}", std::process::id(), stamp));
    std::fs::create_dir(&scratch).map_err(|e| {
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
        "created a folder here",
    )];
    checks.push(hardlinks(&scratch));
    checks.push(identity(&scratch));
    checks.push(case_sensitivity(&scratch));
    checks.push(read_after_write(&scratch));
    checks.push(atomic_replace(&scratch));
    checks.push(prefix_read(dir, &scratch));

    // Reported, not discarded: this runs inside someone's library, and a
    // probe folder left behind is litter they would have to find.
    if let Err(e) = std::fs::remove_dir_all(&scratch) {
        checks.push(check(
            "cleanup",
            "Cleanup",
            ProbeStatus::Failed,
            format!("could not remove {}: {}", scratch.display(), e),
        ));
    }
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

/// Rendering and preview promotion stat a file straight after it is
/// written.
///
/// What this measures is narrower than it looks, and the difference
/// matters: the write and the stat happen in ONE process, so a client
/// that caches its own metadata answers from that cache and the share is
/// never asked. A clean result here therefore rules out nothing about the
/// case #41 actually describes — Blender writing a PNG that Plinth then
/// stats, which crosses a process boundary and can miss the cache
/// entirely. Only a delay visible even to the writing process shows up
/// here, and that is the loudest possible version of the problem.
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
        if !std::fs::metadata(&path).is_ok_and(|m| m.len() == 4096) {
            return check(
                "read_after_write",
                "Read-after-write",
                ProbeStatus::Failed,
                format!(
                    "a file was still not readable back {} ms after being written —                      renders and previews will fail here",
                    VISIBILITY_TIMEOUT_MS
                ),
            );
        }
        worst_ms = worst_ms.max(started.elapsed().as_millis());
    }
    let detail = format!(
        "{}/{} visible to the writing process on the first check (worst wait \
         {} ms) — says nothing about another process reading them",
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
///
/// Timed against a file that was already on the volume rather than one
/// just written: reading back your own fresh write measures the page
/// cache and the `open()` round trip, not what a scan pays per candidate.
/// The scratch file is the fallback when the folder holds nothing big
/// enough, and the detail says which was used, because the two numbers
/// mean different things.
fn prefix_read(dir: &Path, scratch: &Path) -> ProbeCheck {
    // A file the volume already held, if one can be read: its prefix has
    // not just passed through this machine's cache on the way in.
    if let Some(elapsed) = existing_sample(dir).as_deref().and_then(time_prefix) {
        return check(
            "prefix_read",
            "Prefix read",
            ProbeStatus::Ok,
            format!(
                "read {} KiB from a file already on this volume in {} ms — a duplicate \
                 scan pays this per candidate",
                PREFIX_BYTES / 1024,
                elapsed.as_millis()
            ),
        );
    }

    // Nothing here to sample, or the sample would not open — neither says
    // anything about the volume's speed, so time one of our own and say so.
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
    match time_prefix(&path) {
        Some(elapsed) => check(
            "prefix_read",
            "Prefix read",
            ProbeStatus::Warn,
            format!(
                "read {} KiB in {} ms, but from a file written moments ago — a real \
                 scan reads colder data than this",
                PREFIX_BYTES / 1024,
                elapsed.as_millis()
            ),
        ),
        None => check(
            "prefix_read",
            "Prefix read",
            ProbeStatus::Failed,
            "could not read back a file this probe had just written",
        ),
    }
}

fn time_prefix(path: &Path) -> Option<std::time::Duration> {
    let started = Instant::now();
    let mut buffer = vec![0u8; PREFIX_BYTES];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut buffer))
        .ok()
        .map(|()| started.elapsed())
}

/// A file already sitting on the volume, big enough to read a full prefix
/// from. Shallow and bounded: this is a timing sample, not a search, and
/// it runs against libraries with hundreds of thousands of files.
fn existing_sample(dir: &Path) -> Option<PathBuf> {
    let mut queue = std::collections::VecDeque::from([dir.to_path_buf()]);
    let mut seen_dirs = 0usize;
    while let Some(next) = queue.pop_front() {
        seen_dirs += 1;
        if seen_dirs > SAMPLE_DIR_BUDGET {
            return None;
        }
        let entries = std::fs::read_dir(&next).ok()?;
        for entry in entries.flatten().take(SAMPLE_ENTRY_BUDGET) {
            // Hidden entries are skipped on both sides. A macOS-touched
            // share carries a .DS_Store big enough to qualify as a sample
            // and unreadable when you try — a property of that file, not
            // of the volume, so timing it would report a false failure.
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_file() && meta.len() >= PREFIX_BYTES as u64 {
                return Some(path);
            }
            if meta.is_dir() {
                queue.push_back(path);
            }
        }
    }
    None
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
        // nothing on this volume to sample, so the timing is flagged as
        // measured against the probe's own fresh write
        assert_eq!(status("prefix_read"), ProbeStatus::Warn);
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

    /// A real file on the volume is the sample worth timing — reading back
    /// a write from moments ago measures this machine's cache.
    #[test]
    fn a_file_already_present_is_preferred_over_a_fresh_write() {
        let dir = std::env::temp_dir().join(format!("plinth_probe_sample_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.stl"), vec![b's'; PREFIX_BYTES + 1]).unwrap();
        // a macOS-touched share carries one of these, big enough to qualify
        // and unreadable when you try
        std::fs::write(dir.join(".DS_Store"), vec![b'd'; PREFIX_BYTES + 1]).unwrap();

        let report = probe(&dir).unwrap();
        let prefix = report
            .checks
            .iter()
            .find(|c| c.id == "prefix_read")
            .unwrap();
        assert_eq!(prefix.status, ProbeStatus::Ok);
        assert!(
            prefix.detail.contains("already on this volume"),
            "{}",
            prefix.detail
        );

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
