use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};

use crate::catalog::stl_facts::{BaseShape, StlFacts};
use crate::catalog::{BaseSuggestion, DuplicateGroup, ModelFileGeometry};

/// One duplicate candidate, with whatever the index already knows about
/// its bytes: `content_hash` means stage 3 is done for it, `prefix_hash`
/// means stage 2 is. Both NULL is a candidate nothing has read yet.
pub struct DupCandidate {
    pub path: String,
    pub modified_at: i64,
    pub content_hash: Option<String>,
    pub prefix_hash: Option<String>,
}

/// How many files a duplicate scan will examine — the denominator for its
/// progress, counted in SQL so no path list is held to produce it.
pub fn duplicate_candidate_count(conn: &Connection) -> Result<u32, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Dup query failed: {}", e));
    let count: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(n), 0) FROM
                 (SELECT COUNT(*) AS n FROM files WHERE size_bytes > 0
                  GROUP BY size_bytes HAVING COUNT(*) > 1)",
            [],
            |row| row.get(0),
        )
        .map_err(map_err)?;
    Ok(count as u32)
}

/// One page of sizes that occur more than once — the free prefilter for
/// duplicate detection. Paged on the size itself (`after` is the last size
/// of the previous page, 0 to start) rather than OFFSET, so the scan holds
/// one page at a time instead of the whole catalog's path set.
pub fn duplicate_candidate_sizes(
    conn: &Connection,
    after: i64,
    limit: u32,
) -> Result<Vec<i64>, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Dup query failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT size_bytes FROM files WHERE size_bytes > ?1
             GROUP BY size_bytes HAVING COUNT(*) > 1
             ORDER BY size_bytes LIMIT ?2",
        )
        .map_err(map_err)?;
    let sizes = stmt
        .query_map(params![after, limit], |row| row.get(0))
        .and_then(|rows| rows.collect())
        .map_err(map_err)?;
    Ok(sizes)
}

/// The candidates of one size, each carrying the stored hashes a scan can
/// reuse instead of reading. A cached prefix hash counts only while the
/// file it was taken from still has the size and mtime it had then — the
/// join conditions ARE the invalidation rule, so a changed file silently
/// reads as "no cached prefix" rather than matching on stale bytes.
pub fn duplicate_candidates_for_size(
    conn: &Connection,
    size: i64,
) -> Result<Vec<DupCandidate>, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Dup query failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT f.path, f.modified_at, f.content_hash, p.prefix_hash
             FROM files f
             LEFT JOIN file_prefix_hashes p
               ON p.path = f.path
              AND p.size_bytes = f.size_bytes
              AND p.modified_at = f.modified_at
             WHERE f.size_bytes = ?1
             ORDER BY f.path",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map([size], |row| {
            Ok(DupCandidate {
                path: row.get(0)?,
                modified_at: row.get(1)?,
                content_hash: row.get(2)?,
                prefix_hash: row.get(3)?,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(rows)
}

/// The stored full hash of one path. The duplicate scanner reads hashes
/// through its candidate query instead, so this survives as the assertion
/// accessor its tests reach for.
#[cfg(test)]
pub fn known_hash(conn: &Connection, path: &str) -> Option<String> {
    conn.query_row(
        "SELECT content_hash FROM files WHERE path = ?1",
        [path],
        |row| row.get(0),
    )
    .ok()
    .flatten()
}

pub fn store_hash(conn: &Connection, path: &str, hash: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE files SET content_hash = ?2 WHERE path = ?1",
        params![path, hash],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to store hash: {}", e)))?;
    Ok(())
}

/// Loose (unpacked) STL files a geometry mining pass should consider —
/// archive_path IS NOT NULL rows are excluded because their bytes live
/// inside a model.plinthpack, not loose on disk, so mining has nothing to
/// fs::read (see catalog::geometry's module doc). Each candidate's
/// content_hash rides along so the mining loop can skip a hash it already
/// has (set by an earlier scan/dup-run/pack) straight to the geometry-known
/// check without opening the file.
pub fn stl_geometry_candidates(
    conn: &Connection,
) -> Result<Vec<(String, Option<String>)>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Geometry candidate query failed: {}", e));
    let mut stmt = conn
        .prepare("SELECT path, content_hash FROM files WHERE extension = 'stl' AND archive_path IS NULL")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(rows)
}

/// False (re-mine) only for a row whose open_edges is NULL while its
/// tri_count now fits under `edge_cap` — raising the cap backfills
/// instead of skipping forever. Also false for a pre-#18 row (base_checked
/// still 0), so every such row re-streams once and backfills its base facts.
pub fn geometry_satisfies(
    conn: &Connection,
    content_hash: &str,
    edge_cap: u32,
) -> Result<bool, AppError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM file_geometry
             WHERE content_hash = ?1 AND (open_edges IS NOT NULL OR tri_count > ?2)
               AND base_checked = 1",
            params![content_hash, edge_cap],
            |row| row.get(0),
        )
        .map_err(|e| AppError::ConfigError(format!("Geometry lookup failed: {}", e)))?;
    Ok(count > 0)
}

fn base_shape_str(shape: BaseShape) -> &'static str {
    match shape {
        BaseShape::Round => "round",
        BaseShape::Square => "square",
    }
}

/// Stores bbox extents (max − min per axis), not the raw min/max. Always
/// sets base_checked = 1, even when facts.base is None — that's what tells
/// geometry_satisfies this row has already been through base detection.
pub fn store_file_geometry(
    conn: &Connection,
    content_hash: &str,
    facts: &StlFacts,
    derived_at: i64,
) -> Result<(), AppError> {
    let x_mm = (facts.max.0 - facts.min.0) as f64;
    let y_mm = (facts.max.1 - facts.min.1) as f64;
    let z_mm = (facts.max.2 - facts.min.2) as f64;
    let (base_shape, base_mm) = match facts.base {
        Some((shape, mm)) => (Some(base_shape_str(shape)), Some(mm)),
        None => (None, None),
    };
    conn.execute(
        "INSERT INTO file_geometry
             (content_hash, tri_count, x_mm, y_mm, z_mm, volume_mm3, open_edges,
              base_shape, base_mm, base_checked, derived_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1, ?10)
         ON CONFLICT(content_hash) DO UPDATE SET
             tri_count = excluded.tri_count,
             x_mm = excluded.x_mm, y_mm = excluded.y_mm, z_mm = excluded.z_mm,
             volume_mm3 = excluded.volume_mm3,
             open_edges = excluded.open_edges,
             base_shape = excluded.base_shape,
             base_mm = excluded.base_mm,
             base_checked = excluded.base_checked,
             derived_at = excluded.derived_at",
        params![
            content_hash,
            facts.tri_count,
            x_mm,
            y_mm,
            z_mm,
            facts.volume_mm3,
            facts.open_edge_count,
            base_shape,
            base_mm,
            derived_at
        ],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to store geometry: {}", e)))?;
    Ok(())
}

/// A model-level base suggestion: every one of the dir's mined files that
/// detected a base must agree (same shape, mm within ±1.0) — any
/// disagreement, or zero detections, yields no suggestion. Suppressed (per
/// curation precedence) when the matching effective curated field is
/// already set, or the model dismissed suggestions — both checked here so
/// the drawer only ever sees a suggestion worth showing.
pub fn model_base_suggestion(
    conn: &Connection,
    dir_path: &str,
) -> Result<Option<BaseSuggestion>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Base suggestion query failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT g.base_shape, g.base_mm
             FROM files f JOIN file_geometry g ON g.content_hash = f.content_hash
             WHERE f.dir_path = ?1 AND g.base_checked = 1 AND g.base_shape IS NOT NULL",
        )
        .map_err(map_err)?;
    let detections: Vec<(String, f64)> = stmt
        .query_map(params![dir_path], |row| Ok((row.get(0)?, row.get(1)?)))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;

    let Some((shape, first_mm)) = detections.first().cloned() else {
        return Ok(None);
    };
    let mut min_mm = first_mm;
    let mut max_mm = first_mm;
    for (other_shape, mm) in &detections[1..] {
        if *other_shape != shape {
            return Ok(None);
        }
        min_mm = min_mm.min(*mm);
        max_mm = max_mm.max(*mm);
    }
    if max_mm - min_mm > 1.0 {
        return Ok(None);
    }
    let mm = (min_mm + max_mm) / 2.0;
    let mm = (mm * 10.0).round() / 10.0;

    let (curated_round, curated_square, dismissed): (Option<String>, Option<String>, i64) = conn
        .query_row(
            "SELECT NULLIF(COALESCE(u.base_round, m.base_round), ''),
                    NULLIF(COALESCE(u.base_square, m.base_square), ''),
                    COALESCE(u.base_suggestion_dismissed, 0)
             FROM models m LEFT JOIN model_user_meta u ON u.dir_path = m.dir_path
             WHERE m.dir_path = ?1",
            [dir_path],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(map_err)?
        .unwrap_or((None, None, 0));
    if dismissed != 0 {
        return Ok(None);
    }
    let already_curated = match shape.as_str() {
        "round" => curated_round.is_some(),
        "square" => curated_square.is_some(),
        _ => false,
    };
    if already_curated {
        return Ok(None);
    }

    Ok(Some(BaseSuggestion { shape, mm }))
}

/// Inner join on content_hash: un-mined files simply don't appear.
pub fn model_geometry(conn: &Connection, dir_path: &str) -> Result<Vec<ModelFileGeometry>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Geometry listing failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT f.file_name, g.tri_count, g.x_mm, g.y_mm, g.z_mm, g.volume_mm3, g.open_edges
             FROM files f JOIN file_geometry g ON g.content_hash = f.content_hash
             WHERE f.dir_path = ?1
             ORDER BY f.file_name COLLATE NOCASE",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map(params![dir_path], |row| {
            Ok(ModelFileGeometry {
                file_name: row.get(0)?,
                tri_count: row.get(1)?,
                x_mm: row.get(2)?,
                y_mm: row.get(3)?,
                z_mm: row.get(4)?,
                volume_mm3: row.get(5)?,
                open_edges: row.get(6)?,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(rows)
}

/// Commit one batch of duplicate-scan progress: the prefix hashes read
/// since the last flush and the physical identities stat'ed alongside
/// them, in a single transaction.
///
/// Both halves used to be held until the scan finished, so an interrupted
/// run threw away everything it had learned. Flushing in bounded batches
/// keeps that work — and one transaction per batch is also what keeps
/// thousands of cheap stats from becoming thousands of fsyncs.
pub fn store_dup_checkpoint(
    conn: &Connection,
    prefixes: &[PrefixHashRow],
    identities: &[(String, String)],
) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Failed to store scan progress: {}", e));
    let tx = conn.unchecked_transaction().map_err(map_err)?;
    {
        let mut identity_stmt = tx
            .prepare("UPDATE files SET file_identity = ?2 WHERE path = ?1")
            .map_err(map_err)?;
        for (path, identity) in identities {
            identity_stmt.execute(params![path, identity]).map_err(map_err)?;
        }
        let mut prefix_stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO file_prefix_hashes
                 (path, prefix_hash, size_bytes, modified_at, hashed_at)
                 VALUES (?1, ?2, ?3, ?4, strftime('%s','now'))",
            )
            .map_err(map_err)?;
        for row in prefixes {
            prefix_stmt
                .execute(params![
                    row.path,
                    row.prefix_hash,
                    row.size_bytes,
                    row.modified_at
                ])
                .map_err(map_err)?;
        }
    }
    tx.commit().map_err(map_err)?;
    Ok(())
}

/// A prefix hash and the file attributes that make it trustworthy later.
pub struct PrefixHashRow {
    pub path: String,
    pub prefix_hash: String,
    pub size_bytes: i64,
    pub modified_at: i64,
}

/// Post-merge bookkeeping: identity AND modified_at, in one transaction.
/// The mtime matters because replacing a duplicate with a hardlink gives the
/// path the keeper's timestamp — if the index kept the old one, the next
/// rescan's changed-file check would fail and silently drop the stored hash
/// and identity, making the merged group vanish and reappear across scans.
pub fn store_merge_results(
    conn: &Connection,
    entries: &[(String, String, i64)],
) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Failed to record merge: {}", e));
    let tx = conn.unchecked_transaction().map_err(map_err)?;
    {
        let mut stmt = tx
            .prepare("UPDATE files SET file_identity = ?2, modified_at = ?3 WHERE path = ?1")
            .map_err(map_err)?;
        for (path, identity, modified_at) in entries {
            stmt.execute(params![path, identity, modified_at])
                .map_err(map_err)?;
        }
    }
    tx.commit().map_err(map_err)?;
    Ok(())
}

/// Assemble confirmed duplicate groups from stored hashes. Paths that are
/// hardlinks of each other share a file_identity and cost the disk only one
/// copy, so reclaimable space is driven by DISTINCT identities, not path
/// count. A missing identity falls back to the path — i.e. it's assumed to
/// be its own copy — so unscanned rows never hide reclaimable bytes.
pub fn duplicate_groups(conn: &Connection) -> Result<Vec<DuplicateGroup>, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Dup grouping failed: {}", e));
    let mut stmt = conn
        .prepare(
            // group_concat skips NULLs, so the CASE gives just the packed
            // subset (NULL when none)
            "SELECT content_hash, size_bytes, group_concat(path, char(31)),
                    COUNT(DISTINCT COALESCE(file_identity, path)),
                    group_concat(CASE WHEN archive_path IS NOT NULL THEN path END, char(31))
             FROM files
             WHERE content_hash IS NOT NULL
             GROUP BY content_hash HAVING COUNT(*) > 1
             ORDER BY size_bytes * (COUNT(DISTINCT COALESCE(file_identity, path)) - 1) DESC,
                      size_bytes DESC",
        )
        .map_err(map_err)?;
    let groups = stmt
        .query_map([], |row| {
            let joined: String = row.get(2)?;
            let packed_joined: Option<String> = row.get(4)?;
            Ok(DuplicateGroup {
                hash: row.get(0)?,
                size_bytes: row.get::<_, i64>(1)? as f64,
                paths: joined.split('\u{1f}').map(String::from).collect(),
                distinct_copies: row.get(3)?,
                packed_paths: packed_joined
                    .map(|p| p.split('\u{1f}').map(String::from).collect())
                    .unwrap_or_default(),
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::db::test_util::*;
    use crate::catalog::db::*;
    use crate::catalog::FileRow;

    #[test]
    fn candidate_sizes_page_through_every_duplicate_size() {
        let mut conn = test_conn();
        // three sizes that occur twice, one that occurs once
        let files: Vec<FileRow> = [
            ("/lib/a1.stl", 100),
            ("/lib/a2.stl", 100),
            ("/lib/b1.stl", 200),
            ("/lib/b2.stl", 200),
            ("/lib/c1.stl", 300),
            ("/lib/c2.stl", 300),
            ("/lib/lonely.stl", 400),
        ]
        .iter()
        .map(|(path, size)| file_row(path, "/lib", *size))
        .collect();
        replace_catalog(&mut conn, "/lib", &files, &[], &[], &[], &[]).unwrap();

        // a page at a time, the way the scanner walks them
        let mut seen = Vec::new();
        let mut after = 0;
        while let Some(&last) = duplicate_candidate_sizes(&conn, after, 1)
            .unwrap()
            .last()
        {
            seen.push(last);
            after = last;
        }
        assert_eq!(seen, vec![100, 200, 300]);
        assert_eq!(duplicate_candidate_count(&conn).unwrap(), 6);
    }

    #[test]
    fn a_cached_prefix_hash_only_counts_while_size_and_mtime_still_match() {
        let mut conn = test_conn();
        let files = vec![
            file_row("/lib/a.stl", "/lib", 100),
            file_row("/lib/b.stl", "/lib", 100),
        ];
        replace_catalog(&mut conn, "/lib", &files, &[], &[], &[], &[]).unwrap();
        store_dup_checkpoint(
            &conn,
            &[PrefixHashRow {
                path: "/lib/a.stl".into(),
                prefix_hash: "prefix-of-a".into(),
                size_bytes: 100,
                modified_at: 100,
            }],
            &[("/lib/b.stl".into(), "dev:1".into())],
        )
        .unwrap();

        let cached = duplicate_candidates_for_size(&conn, 100).unwrap();
        assert_eq!(cached[0].prefix_hash.as_deref(), Some("prefix-of-a"));
        assert_eq!(cached[1].prefix_hash, None);

        // the file changed under us: the cached prefix stops being offered
        conn.execute("UPDATE files SET modified_at = 101 WHERE path = '/lib/a.stl'", [])
            .unwrap();
        let stale = duplicate_candidates_for_size(&conn, 100).unwrap();
        assert_eq!(stale[0].prefix_hash, None);
    }

    fn stl_facts_with_base(base: Option<(BaseShape, f64)>) -> StlFacts {
        StlFacts {
            tri_count: 12,
            min: (0.0, 0.0, 0.0),
            max: (10.0, 10.0, 10.0),
            volume_mm3: 1000.0,
            open_edge_count: Some(0),
            base,
        }
    }

    #[test]
    fn model_base_suggestion_requires_agreement() {
        let mut conn = test_conn();
        let files = vec![
            FileRow {
                content_hash: Some("hash-a".into()),
                ..file_row("/lib/newt/a.stl", "/lib/newt", 1024)
            },
            FileRow {
                content_hash: Some("hash-b".into()),
                ..file_row("/lib/newt/b.stl", "/lib/newt", 1024)
            },
        ];
        let model = model_row("/lib/newt", "Giant Newt");
        replace_catalog(&mut conn, "/lib", &files, &[model], &[], &[], &[]).unwrap();

        // Agreeing round bases within the ±1mm window suggest their average.
        store_file_geometry(&conn, "hash-a", &stl_facts_with_base(Some((BaseShape::Round, 32.0))), 100).unwrap();
        store_file_geometry(&conn, "hash-b", &stl_facts_with_base(Some((BaseShape::Round, 32.4))), 100).unwrap();
        let suggestion = model_base_suggestion(&conn, "/lib/newt")
            .unwrap()
            .expect("agreeing files should suggest");
        assert_eq!(suggestion.shape, "round");
        assert!((suggestion.mm - 32.2).abs() < 0.01, "got {}", suggestion.mm);

        // A disagreeing shape yields no suggestion at all.
        store_file_geometry(&conn, "hash-b", &stl_facts_with_base(Some((BaseShape::Square, 32.0))), 100).unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_none());

        // Same shape, but mm too far apart (> 1.0mm) also disagrees.
        store_file_geometry(&conn, "hash-b", &stl_facts_with_base(Some((BaseShape::Round, 34.0))), 100).unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_none());

        // Zero detections (neither file found a base) suggests nothing.
        store_file_geometry(&conn, "hash-a", &stl_facts_with_base(None), 100).unwrap();
        store_file_geometry(&conn, "hash-b", &stl_facts_with_base(None), 100).unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_none());
    }

    #[test]
    fn model_base_suggestion_respects_curation_precedence_and_dismissal() {
        let mut conn = test_conn();
        let files = vec![FileRow {
            content_hash: Some("hash-a".into()),
            ..file_row("/lib/newt/a.stl", "/lib/newt", 1024)
        }];
        let model = model_row("/lib/newt", "Giant Newt");
        replace_catalog(&mut conn, "/lib", &files, &[model], &[], &[], &[]).unwrap();
        store_file_geometry(&conn, "hash-a", &stl_facts_with_base(Some((BaseShape::Round, 32.0))), 100).unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_some());

        // Already curated for the DETECTED shape: no suggestion, even
        // though the mined facts still agree with themselves.
        update_model_user_meta(
            &conn, "/lib/newt", None, None, None, None, None, None, None, None, None,
            Some("32".into()), None,
        )
        .unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_none());

        // Clearing the curated field brings the suggestion back...
        update_model_user_meta(
            &conn, "/lib/newt", None, None, None, None, None, None, None, None, None, None, None,
        )
        .unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_some());

        // ...but a dismissal suppresses it regardless of curation.
        dismiss_base_suggestion(&conn, "/lib/newt").unwrap();
        assert!(model_base_suggestion(&conn, "/lib/newt").unwrap().is_none());
    }
}
