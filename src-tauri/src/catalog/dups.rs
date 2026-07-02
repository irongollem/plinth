use crate::error::AppError;
use rusqlite::Connection;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use super::db;
use super::DuplicateGroup;

const PARTIAL_HASH_BYTES: usize = 128 * 1024;

/// Staged duplicate detection:
/// 1. same-size candidates come free from the index,
/// 2. partial (first 128 KiB) BLAKE3 hashes weed out most collisions,
/// 3. full-file hashes confirm — and are persisted so re-runs are cheap.
pub fn find_duplicates(
    conn: &Connection,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(u32, u32),
) -> Result<Vec<DuplicateGroup>, AppError> {
    let candidates = db::duplicate_size_candidates(conn)?;
    let total_candidates: u32 = candidates.iter().map(|(_, paths)| paths.len() as u32).sum();
    let mut processed: u32 = 0;

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

    db::duplicate_groups(conn)
}

fn hash_file(path: &Path, limit: Option<usize>) -> Result<String, AppError> {
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
        }];
        db::test_init(&conn);
        db::replace_catalog(&mut conn, &rows, &models, &[]).unwrap();

        let cancel = AtomicBool::new(false);
        let groups = find_duplicates(&conn, &cancel, |_, _| {}).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);
        assert!(groups[0]
            .paths
            .iter()
            .all(|p| p.ends_with("a.stl") || p.ends_with("b.stl")));

        fs::remove_dir_all(&dir).ok();
    }
}
