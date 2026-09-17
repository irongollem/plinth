use crate::error::AppError;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::db;
use super::geometry;
use super::DuplicateGroup;

const PARTIAL_HASH_BYTES: usize = 128 * 1024;
/// Sizes fetched from the index per page, and entries buffered before a
/// checkpoint flush. Together they cap what the scan holds at once, so
/// peak memory follows these constants rather than the catalog's size.
const SIZE_PAGE: u32 = 512;
const CHECKPOINT_BATCH: usize = 512;
const PROGRESS_STRIDE: u32 = 50;

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

/// What a duplicate scan is spending its time on. Reported because the
/// three kinds of work differ by orders of magnitude: answering from the
/// index costs nothing, a prefix read costs 128 KiB, and a full read costs
/// the whole file — a progress bar that calls all three "hashing" tells
/// the user nothing about how long the rest will take.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq, Type)]
pub enum DupPhase {
    #[default]
    Checking,
    PrefixHashing,
    FullHashing,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct DupProgress {
    pub processed: u32,
    pub total: u32,
    /// Candidates settled from stored hashes, without a disk read.
    pub cached: u32,
    pub prefix_hashed: u32,
    pub full_hashed: u32,
    pub phase: DupPhase,
}

/// A candidate in its prefix bucket. `confirmed` means the index already
/// holds its full hash, so stage 3 has nothing left to read for it.
fn cancelled() -> AppError {
    AppError::UserCancelled("Duplicate scan cancelled".into())
}

struct Bucketed {
    path: String,
    confirmed: bool,
}

/// Staged duplicate detection: same-size candidates come from the index,
/// partial (first 128 KiB) BLAKE3 hashes weed out most collisions, then
/// full-file hashes confirm and are persisted so re-runs are cheap.
///
/// Both stages checkpoint as they go — prefix hashes, and the physical
/// identity every candidate is stat'ed for along the way. An interrupted
/// scan therefore resumes from the index instead of rereading what it had
/// already read, which on a network library is the difference between
/// minutes and hours.
///
/// Stage 3's full `.stl` reads double as geometry mining — one pass over
/// the same bytes, capped by `edge_cap`.
pub fn find_duplicates(
    conn: &Connection,
    cancel: &AtomicBool,
    edge_cap: u32,
    on_progress: impl FnMut(DupProgress),
) -> Result<Vec<DuplicateGroup>, AppError> {
    let total = db::duplicate_candidate_count(conn)?;
    let mut scan = Scan {
        conn,
        cancel,
        edge_cap,
        progress: DupProgress {
            total,
            ..Default::default()
        },
        on_progress,
        prefixes: Vec::new(),
        identities: Vec::new(),
    };
    scan.run()?;
    db::duplicate_groups(conn)
}

struct Scan<'a, F: FnMut(DupProgress)> {
    conn: &'a Connection,
    cancel: &'a AtomicBool,
    edge_cap: u32,
    progress: DupProgress,
    on_progress: F,
    prefixes: Vec<db::PrefixHashRow>,
    identities: Vec<(String, String)>,
}

impl<F: FnMut(DupProgress)> Scan<'_, F> {
    /// Whatever ends the sweep — finished, cancelled, or a failed write —
    /// the reads already made are worth keeping. Only cancellation used to
    /// flush, so an error propagating out of stage 3 discarded up to a
    /// full batch of prefix hashes and identities.
    fn run(&mut self) -> Result<(), AppError> {
        let outcome = self.sweep();
        if outcome.is_err() {
            // the original failure is the one worth reporting; a flush
            // failing on top of it says the same thing twice
            self.flush().ok();
        }
        outcome
    }

    fn sweep(&mut self) -> Result<(), AppError> {
        let mut after = 0i64;
        loop {
            if self.cancelled() {
                return Err(cancelled());
            }
            let sizes = db::duplicate_candidate_sizes(self.conn, after, SIZE_PAGE)?;
            let Some(&last) = sizes.last() else { break };
            after = last;
            for size in sizes {
                for (prefix, bucket) in self.bucket_by_prefix(size)? {
                    // one candidate at this prefix: nothing to collide with
                    if bucket.len() > 1 {
                        self.confirm(&prefix, bucket, size)?;
                    }
                }
                self.checkpoint()?;
            }
        }
        self.flush()?;
        self.progress.processed = self.progress.total;
        self.progress.phase = DupPhase::Checking;
        self.emit();
        Ok(())
    }

    /// Stage 2 for one size: every candidate lands in a bucket keyed by the
    /// hash of its first 128 KiB, read only when neither this scan nor an
    /// earlier one already has that prefix.
    ///
    /// Files whose full hash is already stored are bucketed too, on the
    /// same key. Leaving them out would hide a duplicate whose partner had
    /// been hashed by an earlier run: the unhashed one would sit alone in
    /// its bucket and never be confirmed.
    fn bucket_by_prefix(&mut self, size: i64) -> Result<HashMap<String, Vec<Bucketed>>, AppError> {
        let mut buckets: HashMap<String, Vec<Bucketed>> = HashMap::new();
        for candidate in db::duplicate_candidates_for_size(self.conn, size)? {
            if self.cancelled() {
                return Err(cancelled());
            }
            self.progress.processed += 1;
            // merges and external swaps change a file's identity without
            // touching its content, so every candidate's is refreshed
            if let Some(identity) = file_identity(Path::new(&candidate.path)) {
                self.identities.push((candidate.path.clone(), identity));
            }
            let confirmed = candidate.content_hash.is_some();
            let prefix = match candidate.prefix_hash {
                Some(prefix) => {
                    self.progress.cached += 1;
                    self.progress.phase = DupPhase::Checking;
                    prefix
                }
                None => {
                    self.progress.phase = DupPhase::PrefixHashing;
                    match hash_file(Path::new(&candidate.path), Some(PARTIAL_HASH_BYTES)) {
                        Ok(prefix) => {
                            self.progress.prefix_hashed += 1;
                            self.prefixes.push(db::PrefixHashRow {
                                path: candidate.path.clone(),
                                prefix_hash: prefix.clone(),
                                size_bytes: size,
                                modified_at: candidate.modified_at,
                            });
                            prefix
                        }
                        Err(_) => continue, // unreadable file: not a duplicate candidate
                    }
                }
            };
            buckets.entry(prefix).or_default().push(Bucketed {
                path: candidate.path,
                confirmed,
            });
            self.tick();
            self.checkpoint()?;
        }
        Ok(buckets)
    }

    /// Stage 3: more than one candidate shares this prefix, so each member
    /// the index can't already vouch for is hashed in full.
    fn confirm(&mut self, prefix: &str, bucket: Vec<Bucketed>, size: i64) -> Result<(), AppError> {
        let beyond_prefix = size as usize > PARTIAL_HASH_BYTES;
        for candidate in bucket {
            if candidate.confirmed {
                continue;
            }
            if self.cancelled() {
                return Err(cancelled());
            }
            let path = Path::new(&candidate.path);
            let stl = is_stl(path);
            if stl || beyond_prefix {
                self.progress.phase = DupPhase::FullHashing;
                self.progress.full_hashed += 1;
                self.emit();
            }
            if stl {
                hash_and_mine(self.conn, &candidate.path, self.edge_cap)?;
                continue;
            }
            let hash = if beyond_prefix {
                match hash_file(path, None) {
                    Ok(hash) => hash,
                    Err(_) => continue,
                }
            } else {
                // the file ends inside the prefix, so that IS its full hash
                prefix.to_string()
            };
            db::store_hash(self.conn, &candidate.path, &hash)?;
        }
        Ok(())
    }

    /// Persist what's buffered. An interrupted scan keeps everything a
    /// flush has already committed, so this is also the granularity a
    /// resumed scan restarts at.
    fn flush(&mut self) -> Result<(), AppError> {
        if self.prefixes.is_empty() && self.identities.is_empty() {
            return Ok(());
        }
        db::store_dup_checkpoint(self.conn, &self.prefixes, &self.identities)?;
        self.prefixes.clear();
        self.identities.clear();
        Ok(())
    }

    fn checkpoint(&mut self) -> Result<(), AppError> {
        if self.prefixes.len() >= CHECKPOINT_BATCH || self.identities.len() >= CHECKPOINT_BATCH {
            self.flush()?;
        }
        Ok(())
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    fn emit(&mut self) {
        (self.on_progress)(self.progress);
    }

    fn tick(&mut self) {
        if self.progress.processed.is_multiple_of(PROGRESS_STRIDE) {
            self.emit();
        }
    }
}

/// Replace each duplicate path with a hardlink to `keep`, so every name
/// shares one physical copy. Contents are re-verified byte-for-byte right
/// before each replacement — the catalog's hashes date from the last scan,
/// and replacing a diverged file would destroy data. The swap is
/// link-to-hidden-temp then rename, so no path ever observes a missing
/// file. Returns merged paths + per-file errors.
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

/// Whether the volume holding `path` lets us create hardlinks — answered
/// by making one, not by guessing from filesystem names: NAS mounts route
/// the operation through a network protocol whose support is
/// config-dependent. The storage probe reports the same answer with the
/// volume's reason attached.
pub fn supports_links(path: &Path) -> bool {
    let dir = if path.is_dir() {
        path
    } else {
        match path.parent() {
            Some(parent) => parent,
            None => return false,
        }
    };
    super::storage_probe::hardlink_support(dir).is_ok()
}

fn is_stl(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("stl"))
}

/// A parse failure only skips the geometry write; the hash still lands,
/// so mining can never change which files dup-match.
fn hash_and_mine(conn: &Connection, path: &str, edge_cap: u32) -> Result<(), AppError> {
    let Ok((hash, parsed)) = geometry::stream_mine(path, edge_cap) else {
        return Ok(());
    };
    db::store_hash(conn, path, &hash)?;
    if let Ok(facts) = parsed {
        if !db::geometry_satisfies(conn, &hash, edge_cap)? {
            let derived_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            db::store_file_geometry(conn, &hash, &facts, derived_at)?;
        }
    }
    Ok(())
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
    use crate::catalog::stl_facts::EDGE_STATS_MAX_TRIS;
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
        let groups = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_| {}).unwrap();
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
        let groups = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_| {}).unwrap();
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

    fn candidate_row(path: &Path, dir: &Path, size: i64) -> FileRow {
        FileRow {
            path: path.to_string_lossy().into_owned(),
            dir_path: dir.to_string_lossy().into_owned(),
            file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
            extension: "bin".into(),
            size_bytes: size,
            modified_at: 1,
            ..Default::default()
        }
    }

    /// Two identical pairs, each file well past the 128 KiB prefix so both
    /// stages are real work, in a catalog that lives on disk and can be
    /// closed and reopened like the app's own.
    fn two_pairs(name: &str) -> (std::path::PathBuf, Vec<std::path::PathBuf>) {
        let dir = std::env::temp_dir().join(format!("plinth_{}_{}", name, std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let size = PARTIAL_HASH_BYTES + 20_000;
        let paths: Vec<std::path::PathBuf> = ["a", "b", "c", "d"]
            .iter()
            .map(|n| dir.join(format!("{}.bin", n)))
            .collect();
        // a == b and c == d; the two pairs differ inside the first 128 KiB
        for (path, fill) in paths.iter().zip([b'a', b'a', b'c', b'c']) {
            fs::write(path, vec![fill; size]).unwrap();
        }
        let mut conn = db::open(&dir.join("catalog.db")).unwrap();
        let rows: Vec<FileRow> = paths
            .iter()
            .map(|p| candidate_row(p, &dir, size as i64))
            .collect();
        db::replace_catalog(&mut conn, &dir.to_string_lossy(), &rows, &[], &[], &[], &[]).unwrap();
        (dir, paths)
    }

    fn reopen(dir: &Path) -> Connection {
        db::open(&dir.join("catalog.db")).unwrap()
    }

    #[test]
    fn cancelled_scan_keeps_its_reads_and_resumes_without_repeating_them() {
        let (dir, _paths) = two_pairs("dup_resume");

        // Cancel the moment the first full-file read starts: stage 2 has
        // read every prefix by then, which is exactly the work a restart
        // must not repeat.
        let cancel = AtomicBool::new(false);
        let mut first_pass = DupProgress::default();
        let conn = reopen(&dir);
        let interrupted = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |progress| {
            first_pass = progress;
            if progress.phase == DupPhase::FullHashing {
                cancel.store(true, Ordering::SeqCst);
            }
        });
        assert!(matches!(interrupted, Err(AppError::UserCancelled(_))));
        assert_eq!(first_pass.prefix_hashed, 4);
        drop(conn);

        // Reopening is the reboot: the prefixes and the identities stat'ed
        // alongside them were committed before the scan gave up.
        let conn = reopen(&dir);
        let stored_prefixes: u32 = conn
            .query_row("SELECT COUNT(*) FROM file_prefix_hashes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored_prefixes, 4);
        let identities: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE file_identity IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(identities, 4);

        let cancel = AtomicBool::new(false);
        let mut resumed = DupProgress::default();
        let groups = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |progress| {
            resumed = progress;
        })
        .unwrap();

        // Not one prefix reread: every candidate came back from the index
        assert_eq!(resumed.prefix_hashed, 0);
        assert_eq!(resumed.cached, 4);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| g.paths.len() == 2));

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_changed_file_gets_its_prefix_reread() {
        let (dir, paths) = two_pairs("dup_invalidate");
        let cancel = AtomicBool::new(false);
        let conn = reopen(&dir);
        find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_| {}).unwrap();

        // What a rescan does when a file's bytes changed under it: new
        // mtime, and the stale hash dropped. The cached prefix is keyed on
        // that mtime, so it stops matching too.
        conn.execute(
            "UPDATE files SET modified_at = 2, content_hash = NULL WHERE path = ?1",
            [paths[0].to_string_lossy()],
        )
        .unwrap();

        let mut second_pass = DupProgress::default();
        find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |progress| {
            second_pass = progress;
        })
        .unwrap();
        assert_eq!(second_pass.prefix_hashed, 1);
        assert_eq!(second_pass.cached, 3);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn confirms_a_candidate_whose_only_partner_was_hashed_by_an_earlier_run() {
        let (dir, paths) = two_pairs("dup_half_hashed");
        let conn = reopen(&dir);
        // The state a pack sidecar or a half-finished run leaves: one file
        // of the pair carries a full hash and nothing else does. Bucketing
        // only the unhashed files would leave b alone in its bucket, and
        // the pair would never be found.
        let hash = hash_file(&paths[0], None).unwrap();
        db::store_hash(&conn, &paths[0].to_string_lossy(), &hash).unwrap();

        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_| {}).unwrap();

        let pair = groups
            .iter()
            .find(|g| g.hash == hash)
            .expect("the half-hashed pair is still a duplicate group");
        assert_eq!(pair.paths.len(), 2);

        fs::remove_dir_all(&dir).ok();
    }

    /// Mirrors geometry::tests::build_binary_stl, kept local since that
    /// helper is private to its own module.
    fn build_binary_stl(triangles: &[[(f32, f32, f32); 3]]) -> Vec<u8> {
        let mut bytes = vec![0u8; 80];
        bytes.extend_from_slice(&(triangles.len() as u32).to_le_bytes());
        for tri in triangles {
            bytes.extend_from_slice(&[0u8; 12]); // normal, unused
            for &(x, y, z) in tri {
                bytes.extend_from_slice(&x.to_le_bytes());
                bytes.extend_from_slice(&y.to_le_bytes());
                bytes.extend_from_slice(&z.to_le_bytes());
            }
            bytes.extend_from_slice(&[0u8; 2]); // attribute byte count
        }
        bytes
    }

    fn one_triangle_stl() -> Vec<u8> {
        build_binary_stl(&[[(0.0, 0.0, 0.0), (5.0, 0.0, 0.0), (0.0, 5.0, 0.0)]])
    }

    fn stl_row(path: &Path, dir: &Path, size: i64) -> FileRow {
        FileRow {
            path: path.to_string_lossy().into_owned(),
            dir_path: dir.to_string_lossy().into_owned(),
            file_name: path.file_name().unwrap().to_string_lossy().into_owned(),
            extension: "stl".into(),
            size_bytes: size,
            modified_at: 1,
            ..Default::default()
        }
    }

    #[test]
    fn dup_scan_mines_geometry_from_its_own_full_read() {
        let dir = std::env::temp_dir().join(format!("stlpack_dup_mine_test_{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.stl");
        let b = dir.join("b.stl");
        let bytes = one_triangle_stl();
        fs::write(&a, &bytes).unwrap();
        fs::write(&b, &bytes).unwrap();

        let mut conn = Connection::open_in_memory().unwrap();
        let rows = vec![
            stl_row(&a, &dir, bytes.len() as i64),
            stl_row(&b, &dir, bytes.len() as i64),
        ];
        let model = ModelRow {
            dir_path: dir.to_string_lossy().into_owned(),
            name: "test".into(),
            source: "heuristic".into(),
            file_count: 2,
            ..Default::default()
        };
        db::test_init(&conn);
        db::replace_catalog(&mut conn, &dir.to_string_lossy(), &rows, &[model], &[], &[], &[])
            .unwrap();

        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_| {}).unwrap();
        assert_eq!(groups.len(), 1);

        let hash =
            db::known_hash(&conn, &a.to_string_lossy()).expect("hash stored by the dup scan");
        assert!(db::geometry_satisfies(&conn, &hash, EDGE_STATS_MAX_TRIS).unwrap());

        // Both files gone: a re-read on the mine run would surface as
        // `failed`, so `already_known` below proves the dup scan's stage-3
        // read was the only disk access these files ever got.
        fs::remove_file(&a).unwrap();
        fs::remove_file(&b).unwrap();

        let outcome = geometry::mine_geometry(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_, _| {}).unwrap();
        assert_eq!(
            outcome,
            geometry::GeometryOutcome {
                mined: 0,
                already_known: 2,
                failed: 0,
            }
        );

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn garbage_stl_pair_still_hashes_and_groups_despite_failing_to_parse() {
        let dir =
            std::env::temp_dir().join(format!("stlpack_dup_garbage_test_{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.stl");
        let b = dir.join("b.stl");
        // Past stream_mine's 84-byte header+count preamble, but the
        // triangle count it implies is nonsense — the read succeeds, the
        // parse doesn't, and dup detection must not tell the difference.
        let junk = vec![7u8; 200];
        fs::write(&a, &junk).unwrap();
        fs::write(&b, &junk).unwrap();

        let mut conn = Connection::open_in_memory().unwrap();
        let rows = vec![
            stl_row(&a, &dir, junk.len() as i64),
            stl_row(&b, &dir, junk.len() as i64),
        ];
        let model = ModelRow {
            dir_path: dir.to_string_lossy().into_owned(),
            name: "test".into(),
            source: "heuristic".into(),
            file_count: 2,
            ..Default::default()
        };
        db::test_init(&conn);
        db::replace_catalog(&mut conn, &dir.to_string_lossy(), &rows, &[model], &[], &[], &[])
            .unwrap();

        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&conn, &cancel, EDGE_STATS_MAX_TRIS, |_| {}).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);

        let hash = db::known_hash(&conn, &a.to_string_lossy())
            .expect("hash stored even though the bytes don't parse as STL");
        assert!(!db::geometry_satisfies(&conn, &hash, EDGE_STATS_MAX_TRIS).unwrap());

        fs::remove_dir_all(&dir).ok();
    }
}
