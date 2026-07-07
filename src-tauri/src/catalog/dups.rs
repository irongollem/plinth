use crate::error::AppError;
use rusqlite::Connection;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::db;
use super::DuplicateGroup;

const PARTIAL_HASH_BYTES: usize = 128 * 1024;

/// Opaque physical-file identity: "device:inode" on Unix, volume:index on
/// Windows. Two paths sharing it are one file on disk (hardlinks), which is
/// how a merged duplicate group is told apart from a reclaimable one.
pub fn file_identity(path: &Path) -> Option<String> {
    file_id::get_file_id(path).ok().map(|id| match id {
        file_id::FileId::Inode {
            device_id,
            inode_number,
        } => format!("{}:{}", device_id, inode_number),
        file_id::FileId::LowRes {
            volume_serial_number,
            file_index,
        } => format!("{}:{}", volume_serial_number, file_index),
        file_id::FileId::HighRes {
            volume_serial_number,
            file_id,
        } => format!("{}:{}", volume_serial_number, file_id),
    })
}

/// Staged duplicate detection:
/// 1. same-size candidates come free from the index,
/// 2. partial (first 128 KiB) BLAKE3 hashes weed out most collisions,
/// 3. full-file hashes confirm — and are persisted so re-runs are cheap.
///
/// Every candidate's physical identity is refreshed along the way (a stat,
/// nearly free next to hashing): merges and external file swaps change
/// identity without touching content, so a stale value would misreport
/// what's reclaimable.
pub fn find_duplicates(
    conn: &Connection,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u32, u32),
) -> Result<Vec<DuplicateGroup>, AppError> {
    let candidates = db::duplicate_size_candidates(conn)?;
    let total_candidates: u32 = candidates.iter().map(|(_, paths)| paths.len() as u32).sum();
    let mut processed: u32 = 0;
    let mut identities: Vec<(String, String)> = Vec::new();

    for (size, paths) in candidates {
        if cancel.load(Ordering::SeqCst) {
            return Err(AppError::UserCancelled("Duplicate scan cancelled".into()));
        }

        // Files small enough that the partial hash IS the full hash skip
        // straight to stage 3
        let needs_two_stages = size as usize > PARTIAL_HASH_BYTES;

        // Stage 2: group by partial hash
        let mut partial_groups: HashMap<String, Vec<String>> = HashMap::new();
        for path in paths {
            processed += 1;
            if processed.is_multiple_of(50) {
                on_progress(processed, total_candidates);
            }
            if cancel.load(Ordering::SeqCst) {
                return Err(AppError::UserCancelled("Duplicate scan cancelled".into()));
            }
            if let Some(identity) = file_identity(Path::new(&path)) {
                identities.push((path.clone(), identity));
            }
            // A stored full hash makes both stages unnecessary
            if db::known_hash(conn, &path).is_some() {
                partial_groups.entry("known".into()).or_default().push(path);
                continue;
            }
            match hash_file(Path::new(&path), Some(PARTIAL_HASH_BYTES)) {
                Ok(partial) => partial_groups.entry(partial).or_default().push(path),
                Err(_) => continue, // unreadable file: not a duplicate candidate
            }
        }

        // Stage 3: full hash where partials collide (or trust stored hashes)
        for (key, group) in partial_groups {
            let confirmable = key == "known" || group.len() > 1;
            if !confirmable {
                continue;
            }
            for path in group {
                if db::known_hash(conn, &path).is_some() {
                    continue;
                }
                if cancel.load(Ordering::SeqCst) {
                    return Err(AppError::UserCancelled("Duplicate scan cancelled".into()));
                }
                let full = if needs_two_stages {
                    hash_file(Path::new(&path), None)
                } else {
                    // partial covered the whole file; rehash cheaply anyway
                    hash_file(Path::new(&path), Some(PARTIAL_HASH_BYTES))
                };
                if let Ok(hash) = full {
                    db::store_hash(conn, &path, &hash)?;
                }
            }
        }
    }
    on_progress(total_candidates, total_candidates);
    db::store_identities(conn, &identities)?;

    db::duplicate_groups(conn)
}

/// Replace each duplicate path with a hardlink to `keep`, so every name
/// shares one physical copy and the difference is reclaimed. Contents are
/// re-verified byte-for-byte right before each replacement: the catalog's
/// hashes date from the last scan, and replacing a file that has since
/// diverged would destroy data. The swap itself is link-to-hidden-temp then
/// rename, so no path ever observes a missing file — a crash leaves at worst
/// a dot-file the scanner ignores. Returns merged paths + per-file errors.
pub fn merge_duplicates(
    keep: &Path,
    duplicates: &[String],
) -> Result<(Vec<String>, Vec<String>), AppError> {
    let keep_hash = hash_file(keep, None)?;
    let keep_identity = file_identity(keep);
    let mut merged: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (n, dup) in duplicates.iter().enumerate() {
        let dup_path = Path::new(dup);
        // Already one file on disk (e.g. merged in an earlier run): done
        if keep_identity.is_some() && file_identity(dup_path) == keep_identity {
            merged.push(dup.clone());
            continue;
        }
        match hash_file(dup_path, None) {
            Ok(hash) if hash == keep_hash => {}
            Ok(_) => {
                errors.push(format!(
                    "{}: contents changed since the last scan — rescan duplicates first",
                    dup
                ));
                continue;
            }
            Err(e) => {
                errors.push(format!("{}: {}", dup, e));
                continue;
            }
        }
        let Some(parent) = dup_path.parent() else {
            errors.push(format!("{}: has no parent directory", dup));
            continue;
        };
        let temp = parent.join(format!(".plinth-merge-{}-{}.tmp", std::process::id(), n));
        // Cross-volume or link-less filesystems (exFAT, some SMB mounts)
        // fail here, before anything is touched
        if let Err(e) = std::fs::hard_link(keep, &temp) {
            errors.push(format!(
                "{}: this location doesn't support merging ({})",
                dup, e
            ));
            continue;
        }
        match std::fs::rename(&temp, dup_path) {
            Ok(()) => merged.push(dup.clone()),
            Err(e) => {
                std::fs::remove_file(&temp).ok();
                errors.push(format!("{}: {}", dup, e));
            }
        }
    }
    Ok((merged, errors))
}

/// Whether the volume holding `path` lets us create hardlinks — answered by
/// making one, not by guessing from filesystem names: NAS mounts route the
/// operation through a network protocol whose support is config-dependent.
pub fn supports_links(path: &Path) -> bool {
    let dir = if path.is_dir() {
        path
    } else {
        match path.parent() {
            Some(parent) => parent,
            None => return false,
        }
    };
    let base = dir.join(format!(".plinth-probe-{}", std::process::id()));
    let link = dir.join(format!(".plinth-probe-{}.link", std::process::id()));
    let supported =
        std::fs::write(&base, b"probe").is_ok() && std::fs::hard_link(&base, &link).is_ok();
    std::fs::remove_file(&link).ok();
    std::fs::remove_file(&base).ok();
    supported
}

pub(crate) fn hash_file(path: &Path, limit: Option<usize>) -> Result<String, AppError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AppError::IoError(format!("Cannot open {}: {}", path.display(), e)))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut remaining = limit.unwrap_or(usize::MAX);
    loop {
        let want = buffer.len().min(remaining);
        if want == 0 {
            break;
        }
        let read = file
            .read(&mut buffer[..want])
            .map_err(|e| AppError::IoError(format!("Read failed for {}: {}", path.display(), e)))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        remaining -= read;
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{db, FileRow, ModelRow};
    use std::fs;

    #[test]
    fn finds_true_duplicates_and_skips_same_size_different_content() {
        let dir = std::env::temp_dir().join(format!("stlpack_dup_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.stl");
        let b = dir.join("b.stl");
        let c = dir.join("c.stl");
        fs::write(&a, b"identical-content!").unwrap();
        fs::write(&b, b"identical-content!").unwrap();
        fs::write(&c, b"different-content!").unwrap(); // same length as a/b

        let mut conn = Connection::open_in_memory().unwrap();
        // reuse the public schema init through open()? open needs a path;
        // use the crate-internal init via a throwaway on-disk db instead
        let rows: Vec<FileRow> = [&a, &b, &c]
            .iter()
            .map(|p| FileRow {
                path: p.to_string_lossy().into_owned(),
                dir_path: dir.to_string_lossy().into_owned(),
                file_name: p.file_name().unwrap().to_string_lossy().into_owned(),
                extension: "stl".into(),
                size_bytes: 18,
                modified_at: 1,
                ..Default::default()
            })
            .collect();
        let models = vec![ModelRow {
            dir_path: dir.to_string_lossy().into_owned(),
            name: "test".into(),
            description: None,
            designer: None,
            release_name: None,
            preview_path: None,
            source: "heuristic".into(),
            uuid: None,
            file_count: 3,
            total_size_bytes: 54,
            variant: None,
            pose: None,
            scale: None,
            support_status: None,
            release_date: None,
            sculptor: None,
            base_round_mm: None,
            base_square_mm: None,
            group_name: None,
            ..Default::default()
        }];
        db::test_init(&conn);
        db::replace_catalog(&mut conn, &dir.to_string_lossy(), &rows, &models, &[], &[], &[]).unwrap();

        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&conn, &cancel, |_, _| {}).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);
        // a and b are separate files on disk: both copies are real
        assert_eq!(groups[0].distinct_copies, 2);
        assert!(groups[0]
            .paths
            .iter()
            .all(|p| p.ends_with("a.stl") || p.ends_with("b.stl")));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn hardlinked_copies_count_as_one_physical_copy() {
        let dir = std::env::temp_dir().join(format!("stlpack_link_test_{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.stl");
        let b = dir.join("b.stl"); // hardlink of a: same bytes, same inode
        let c = dir.join("c.stl"); // plain copy: same bytes, own inode
        fs::write(&a, b"shared-base-part").unwrap();
        fs::hard_link(&a, &b).unwrap();
        fs::write(&c, b"shared-base-part").unwrap();

        let mut conn = Connection::open_in_memory().unwrap();
        let rows: Vec<FileRow> = [&a, &b, &c]
            .iter()
            .map(|p| FileRow {
                path: p.to_string_lossy().into_owned(),
                dir_path: dir.to_string_lossy().into_owned(),
                file_name: p.file_name().unwrap().to_string_lossy().into_owned(),
                extension: "stl".into(),
                size_bytes: 16,
                modified_at: 1,
                ..Default::default()
            })
            .collect();
        db::test_init(&conn);
        db::replace_catalog(&mut conn, &dir.to_string_lossy(), &rows, &[], &[], &[], &[]).unwrap();

        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&conn, &cancel, |_, _| {}).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 3);
        // Three names, but a+b share one inode: only c is a reclaimable copy
        assert_eq!(groups[0].distinct_copies, 2);
        // Headline stats report disk usage, not the sum of names: 3×16 minus
        // the 16 bytes the hardlink doesn't actually occupy
        assert_eq!(db::stats(&conn).unwrap().total_size_bytes, 32.0);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn merge_links_identical_files_and_refuses_changed_ones() {
        let dir = std::env::temp_dir().join(format!("stlpack_merge_test_{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(dir.join("variant_b")).unwrap();
        let keep = dir.join("base.stl");
        let same = dir.join("variant_b").join("base.stl");
        let changed = dir.join("edited.stl");
        fs::write(&keep, b"unicorn-base-bytes").unwrap();
        fs::write(&same, b"unicorn-base-bytes").unwrap();
        // Same length, different bytes — must be refused, not clobbered
        fs::write(&changed, b"unicorn-EDIT-bytes").unwrap();

        let (merged, errors) = merge_duplicates(
            &keep,
            &[
                same.to_string_lossy().into_owned(),
                changed.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();

        assert_eq!(merged.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("contents changed"));
        // The merged path is now the same physical file as the keeper…
        assert_eq!(file_identity(&keep), file_identity(&same));
        // …and the diverged file kept its own bytes
        assert_eq!(fs::read(&changed).unwrap(), b"unicorn-EDIT-bytes");
        // Merging again is a no-op success, not an error
        let (again, again_errors) =
            merge_duplicates(&keep, &[same.to_string_lossy().into_owned()]).unwrap();
        assert_eq!(again.len(), 1);
        assert!(again_errors.is_empty());

        assert!(supports_links(&keep));

        fs::remove_dir_all(&dir).ok();
    }
}
