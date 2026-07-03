use crate::error::AppError;
use rusqlite::{params, Connection};
use std::path::Path;

use super::{
    CatalogEntry, CatalogFile, CatalogGroup, CatalogStats, DuplicateGroup, ExtensionStat, FileRow,
    ModelRow, ReleaseSummary,
};

const SCHEMA_VERSION: i64 = 4;

/// Open (and if needed initialize) the catalog database.
pub fn open(db_path: &Path) -> Result<Connection, AppError> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::IoError(format!("Failed to create catalog dir: {}", e)))?;
    }
    let conn = Connection::open(db_path)
        .map_err(|e| AppError::ConfigError(format!("Failed to open catalog db: {}", e)))?;
    // WAL lets the scanner write while searches read
    conn.pragma_update(None, "journal_mode", "WAL").ok();
    conn.busy_timeout(std::time::Duration::from_secs(10)).ok();
    init_schema(&conn)?;
    Ok(conn)
}

fn init_schema(conn: &Connection) -> Result<(), AppError> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0);
    // The base CREATEs are all IF NOT EXISTS and run on EVERY open — only
    // the versioned migrations below are gated. Gating the base batch once
    // burned us: a build stamped user_version before a newly-coded table
    // existed, and the version check then guaranteed it could never appear
    // ("no such table" with no way out short of deleting the db).
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS files (
            path        TEXT PRIMARY KEY,
            dir_path    TEXT NOT NULL,
            file_name   TEXT NOT NULL,
            extension   TEXT NOT NULL,
            size_bytes  INTEGER NOT NULL,
            modified_at INTEGER NOT NULL,
            content_hash TEXT,
            indexed_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_files_dir ON files(dir_path);
        CREATE INDEX IF NOT EXISTS idx_files_size ON files(size_bytes);

        CREATE TABLE IF NOT EXISTS models (
            dir_path     TEXT PRIMARY KEY,
            name         TEXT NOT NULL,
            description  TEXT,
            designer     TEXT,
            release_name TEXT,
            preview_path TEXT,
            source       TEXT NOT NULL DEFAULT 'heuristic',
            uuid         TEXT,
            file_count   INTEGER NOT NULL DEFAULT 0,
            total_size_bytes INTEGER NOT NULL DEFAULT 0,
            -- The logical model this row is a variant of ("galeb duhr" for
            -- galeb duhr/unsupported/A). Scanner-derived; rows sharing a
            -- group_name (case-insensitive) render as ONE catalog card.
            group_name   TEXT,
            indexed_at   INTEGER NOT NULL
        );

        -- Keyed by dir_path + tag (not scan-generated ids) so user tags
        -- survive full rescans; source distinguishes metadata imports.
        CREATE TABLE IF NOT EXISTS model_tags (
            dir_path TEXT NOT NULL,
            tag      TEXT NOT NULL,
            source   TEXT NOT NULL DEFAULT 'user',
            PRIMARY KEY (dir_path, tag)
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS models_fts USING fts5(
            name, description, tags, dir_path
        );

        CREATE TABLE IF NOT EXISTS meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        -- User-editable metadata lives OUTSIDE models on purpose: rescans
        -- rebuild models wholesale (replace_catalog), and anything stored
        -- there is lost. Keyed by dir_path like model_tags, surviving the
        -- same way. Scanner-inferred values stay in models; a row here
        -- overrides them (COALESCE in search).
        CREATE TABLE IF NOT EXISTS model_user_meta (
            dir_path       TEXT PRIMARY KEY,
            custom_name    TEXT,
            pose           TEXT,
            scale          TEXT,
            support_status TEXT,
            release_date   TEXT,
            preview_path   TEXT
        );

        -- Group display-name overrides, keyed by the SCANNER's group name
        -- so they survive rescans (folder names are stable; the override
        -- rides on top). Renaming two groups to the same display name
        -- merges them — that's the manual "combine under one model" tool.
        CREATE TABLE IF NOT EXISTS group_renames (
            source_group TEXT PRIMARY KEY COLLATE NOCASE,
            display_name TEXT NOT NULL
        );
        "#,
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to init catalog schema: {}", e)))?;

    // Column migrations are shape-checked, NOT version-gated: during dev
    // iteration a build can stamp user_version before an ALTER exists in
    // code, and a version gate then locks that ALTER out forever ("no such
    // column" with no way back). Asking the table what it actually has
    // makes the check idempotent and self-healing on every open.
    let existing_columns: Vec<String> = conn
        .prepare("PRAGMA table_info(models)")
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .and_then(|rows| rows.collect())
        })
        .map_err(|e| AppError::ConfigError(format!("Failed to inspect catalog schema: {}", e)))?;
    for column in [
        "pose",
        "scale",
        "support_status",
        "release_date",
        "group_name",
    ] {
        if !existing_columns.iter().any(|c| c == column) {
            conn.execute(
                &format!("ALTER TABLE models ADD COLUMN {} TEXT", column),
                [],
            )
            .map_err(|e| {
                AppError::ConfigError(format!(
                    "Failed to migrate catalog schema (add {}): {}",
                    column, e
                ))
            })?;
        }
    }

    if version >= SCHEMA_VERSION {
        return Ok(());
    }

    // v3: rescue metadata that v2 stored in models — those values were
    // silently wiped by every rescan, so anything a user typed in a v2 build
    // moves to the rescan-safe table before it can be lost again
    if version < 3 {
        conn.execute(
            "INSERT OR IGNORE INTO model_user_meta
                 (dir_path, pose, scale, support_status, release_date)
             SELECT dir_path, pose, scale, support_status, release_date FROM models
             WHERE pose IS NOT NULL OR scale IS NOT NULL
                OR support_status IS NOT NULL OR release_date IS NOT NULL",
            [],
        )
        .map_err(|e| AppError::ConfigError(format!("Failed to migrate user metadata: {}", e)))?;
    }

    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|e| AppError::ConfigError(format!("Failed to set schema version: {}", e)))?;
    Ok(())
}

/// Replace the indexed catalog in one transaction. User tags survive;
/// metadata tags are refreshed from the scan.
pub fn replace_catalog(
    conn: &mut Connection,
    files: &[FileRow],
    models: &[ModelRow],
    metadata_tags: &[(String, String)],
) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Catalog write failed: {}", e));
    let tx = conn.transaction().map_err(map_err)?;
    {
        // Preserve known content hashes across the rebuild — hashing is the
        // expensive part of duplicate detection
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS old_hashes AS
                 SELECT path, size_bytes, modified_at, content_hash
                 FROM files WHERE content_hash IS NOT NULL;",
        )
        .map_err(map_err)?;

        tx.execute("DELETE FROM files", []).map_err(map_err)?;
        tx.execute("DELETE FROM models", []).map_err(map_err)?;
        tx.execute("DELETE FROM model_tags WHERE source = 'metadata'", [])
            .map_err(map_err)?;

        let mut insert_file = tx
            .prepare(
                "INSERT OR REPLACE INTO files
                 (path, dir_path, file_name, extension, size_bytes, modified_at, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, strftime('%s','now'))",
            )
            .map_err(map_err)?;
        for f in files {
            insert_file
                .execute(params![
                    f.path,
                    f.dir_path,
                    f.file_name,
                    f.extension,
                    f.size_bytes,
                    f.modified_at
                ])
                .map_err(map_err)?;
        }
        drop(insert_file);

        // Restore hashes for files that didn't change
        tx.execute(
            "UPDATE files SET content_hash = (
                 SELECT oh.content_hash FROM old_hashes oh
                 WHERE oh.path = files.path
                   AND oh.size_bytes = files.size_bytes
                   AND oh.modified_at = files.modified_at
             )",
            [],
        )
        .map_err(map_err)?;
        tx.execute("DROP TABLE old_hashes", []).map_err(map_err)?;

        let mut insert_model = tx
            .prepare(
                "INSERT OR REPLACE INTO models
                 (dir_path, name, description, designer, release_name, preview_path,
                  source, uuid, file_count, total_size_bytes, pose, scale, support_status,
                  release_date, group_name, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                  strftime('%s','now'))",
            )
            .map_err(map_err)?;
        for m in models {
            insert_model
                .execute(params![
                    m.dir_path,
                    m.name,
                    m.description,
                    m.designer,
                    m.release_name,
                    m.preview_path,
                    m.source,
                    m.uuid,
                    m.file_count,
                    m.total_size_bytes,
                    m.pose,
                    m.scale,
                    m.support_status,
                    m.release_date,
                    m.group_name
                ])
                .map_err(map_err)?;
        }
        drop(insert_model);

        let mut insert_tag = tx
            .prepare(
                "INSERT OR IGNORE INTO model_tags (dir_path, tag, source)
                 VALUES (?1, ?2, 'metadata')",
            )
            .map_err(map_err)?;
        for (dir_path, tag) in metadata_tags {
            insert_tag
                .execute(params![dir_path, tag])
                .map_err(map_err)?;
        }
        drop(insert_tag);

        // Drop user tags and user metadata whose model no longer exists on
        // disk — both tables are keyed by dir_path and survive rescans
        tx.execute(
            "DELETE FROM model_tags
             WHERE dir_path NOT IN (SELECT dir_path FROM models)",
            [],
        )
        .map_err(map_err)?;
        tx.execute(
            "DELETE FROM model_user_meta
             WHERE dir_path NOT IN (SELECT dir_path FROM models)",
            [],
        )
        .map_err(map_err)?;
        tx.execute(
            "DELETE FROM group_renames
             WHERE lower(source_group) NOT IN
                 (SELECT DISTINCT lower(COALESCE(group_name, name)) FROM models)",
            [],
        )
        .map_err(map_err)?;

        rebuild_fts(&tx).map_err(map_err)?;

        tx.execute(
            "INSERT OR REPLACE INTO meta (key, value)
             VALUES ('last_scan', strftime('%s','now'))",
            [],
        )
        .map_err(map_err)?;
    }
    tx.commit().map_err(map_err)?;
    Ok(())
}

// The group's display name is folded into the tags text so a search for a
// RENAMED group ("Stone Guardian") still finds its member rows, whose own
// names may say something else entirely ("galeb duhr A").
const FTS_INSERT_SELECT: &str = "INSERT INTO models_fts (name, description, tags, dir_path)
         SELECT COALESCE(u.custom_name, m.name),
                COALESCE(m.description, ''),
                COALESCE((SELECT group_concat(t.tag, ' ') FROM model_tags t
                          WHERE t.dir_path = m.dir_path), '')
                    || ' ' || COALESCE(r.display_name, ''),
                m.dir_path
         FROM models m
         LEFT JOIN model_user_meta u ON u.dir_path = m.dir_path
         LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name)";

fn rebuild_fts(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM models_fts", [])?;
    conn.execute(FTS_INSERT_SELECT, [])?;
    Ok(())
}

/// Refresh the FTS row for one model after a tag or user-meta change.
fn refresh_fts_row(conn: &Connection, dir_path: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM models_fts WHERE dir_path = ?1", [dir_path])?;
    conn.execute(
        &format!("{} WHERE m.dir_path = ?1", FTS_INSERT_SELECT),
        [dir_path],
    )?;
    Ok(())
}

/// Build an FTS5 prefix query from free text: each token becomes "tok"*.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|token| format!("\"{}\"*", token.replace('"', "")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct SearchPage {
    pub entries: Vec<CatalogEntry>,
    pub total: u32,
}

/// FTS + tag filters shared by the flat and grouped searches; both operate
/// on `models m` so the clauses are interchangeable.
fn build_search_filter(
    query: &str,
    tags: &[String],
) -> (String, Vec<Box<dyn rusqlite::types::ToSql>>) {
    let mut where_clauses: Vec<String> = Vec::new();
    let mut bound: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

    let trimmed = query.trim();
    if !trimmed.is_empty() {
        where_clauses.push(
            "m.dir_path IN (SELECT dir_path FROM models_fts WHERE models_fts MATCH ?)".to_string(),
        );
        bound.push(Box::new(fts_query(trimmed)));
    }
    for tag in tags {
        where_clauses.push(
            "EXISTS (SELECT 1 FROM model_tags mt WHERE mt.dir_path = m.dir_path AND mt.tag = ?)"
                .to_string(),
        );
        bound.push(Box::new(tag.clone()));
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };
    (where_sql, bound)
}

/// The one SELECT that yields CatalogEntry rows. name/preview/details
/// resolve user overrides over scanner values; custom_name additionally
/// travels raw so the UI can tell an override apart from an inferred name
/// (and clear it to revert).
fn entry_select_sql(where_sql: &str, tail_sql: &str) -> String {
    format!(
        "SELECT m.dir_path, COALESCE(u.custom_name, m.name), m.description, m.designer,
                m.release_name, COALESCE(u.preview_path, m.preview_path),
                m.file_count, m.total_size_bytes,
                COALESCE((SELECT group_concat(t.tag, char(31)) FROM model_tags t
                          WHERE t.dir_path = m.dir_path), ''),
                COALESCE(u.pose, m.pose), COALESCE(u.scale, m.scale),
                COALESCE(u.support_status, m.support_status),
                COALESCE(u.release_date, m.release_date),
                u.custom_name
         FROM models m LEFT JOIN model_user_meta u ON u.dir_path = m.dir_path {} {}",
        where_sql, tail_sql
    )
}

fn map_entry_row(row: &rusqlite::Row) -> rusqlite::Result<CatalogEntry> {
    let tags_joined: String = row.get(8)?;
    Ok(CatalogEntry {
        dir_path: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        designer: row.get(3)?,
        release_name: row.get(4)?,
        preview_path: row.get(5)?,
        file_count: row.get(6)?,
        total_size_bytes: row.get::<_, i64>(7)? as f64,
        tags: if tags_joined.is_empty() {
            Vec::new()
        } else {
            tags_joined.split('\u{1f}').map(String::from).collect()
        },
        pose: row.get(9)?,
        scale: row.get(10)?,
        support_status: row.get(11)?,
        release_date: row.get(12)?,
        custom_name: row.get(13)?,
    })
}

pub fn search(
    conn: &Connection,
    query: &str,
    tags: &[String],
    limit: u32,
    offset: u32,
) -> Result<SearchPage, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Catalog search failed: {}", e));
    let (where_sql, bound) = build_search_filter(query, tags);
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = bound.iter().map(|b| b.as_ref()).collect();

    let total: u32 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM models m {}", where_sql),
            params_ref.as_slice(),
            |row| row.get(0),
        )
        .map_err(map_err)?;

    let sql = entry_select_sql(
        &where_sql,
        &format!(
            "ORDER BY COALESCE(u.custom_name, m.name) COLLATE NOCASE LIMIT {} OFFSET {}",
            limit, offset
        ),
    );
    let mut stmt = conn.prepare(&sql).map_err(map_err)?;
    let entries = stmt
        .query_map(params_ref.as_slice(), map_entry_row)
        .map_err(map_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_err)?;

    Ok(SearchPage { entries, total })
}

pub struct GroupPage {
    pub groups: Vec<CatalogGroup>,
    pub total: u32,
}

/// One row per LOGICAL model: variants sharing a group_name (supported/
/// unsupported builds, poses A/B/C) collapse into a single group with
/// aggregate counts. Rows scanned before v4 have no group_name and fall
/// back to their own name — a group of one, i.e. the old behavior.
pub fn search_groups(
    conn: &Connection,
    query: &str,
    tags: &[String],
    limit: u32,
    offset: u32,
) -> Result<GroupPage, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Catalog group search failed: {}", e));
    let (where_sql, bound) = build_search_filter(query, tags);
    let params_ref: Vec<&dyn rusqlite::types::ToSql> = bound.iter().map(|b| b.as_ref()).collect();

    // Effective group = rename override > scanner group > own name. The
    // rename join keys on the scanner name so it survives rescans, and two
    // groups renamed alike collapse into one (deliberate merge tool).
    let total: u32 = conn
        .query_row(
            &format!(
                "SELECT COUNT(DISTINCT lower(COALESCE(r.display_name, m.group_name, m.name)))
                 FROM models m
                 LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name) {}",
                where_sql
            ),
            params_ref.as_slice(),
            |row| row.get(0),
        )
        .map_err(map_err)?;

    // MAX(preview) = any variant's image is better than none;
    // MAX(designer) likewise picks an arbitrary non-null representative
    let sql = format!(
        "SELECT COALESCE(r.display_name, m.group_name, m.name) AS gname,
                MAX(m.designer),
                COUNT(*),
                COUNT(DISTINCT COALESCE(u.pose, m.pose)),
                group_concat(DISTINCT COALESCE(u.support_status, m.support_status)),
                SUM(m.file_count),
                SUM(m.total_size_bytes),
                MAX(COALESCE(u.preview_path, m.preview_path))
         FROM models m
         LEFT JOIN model_user_meta u ON u.dir_path = m.dir_path
         LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name) {}
         GROUP BY lower(gname)
         ORDER BY gname COLLATE NOCASE
         LIMIT {} OFFSET {}",
        where_sql, limit, offset
    );
    let mut stmt = conn.prepare(&sql).map_err(map_err)?;
    let groups = stmt
        .query_map(params_ref.as_slice(), |row| {
            let supports: Option<String> = row.get(4)?;
            Ok(CatalogGroup {
                group_name: row.get(0)?,
                designer: row.get(1)?,
                variant_count: row.get(2)?,
                pose_count: row.get(3)?,
                support_statuses: supports
                    .map(|s| s.split(',').map(String::from).collect())
                    .unwrap_or_default(),
                file_count: row.get::<_, i64>(5)? as u32,
                total_size_bytes: row.get::<_, i64>(6)? as f64,
                preview_path: row.get(7)?,
            })
        })
        .map_err(map_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_err)?;

    Ok(GroupPage { groups, total })
}

/// All variants of one logical model, ordered for the drawer: support
/// status first (alphabetical puts supported before unsupported, unknowns
/// last), then pose.
pub fn group_members(conn: &Connection, group_name: &str) -> Result<Vec<CatalogEntry>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Group member query failed: {}", e));
    let sql = entry_select_sql(
        "LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name)
         WHERE lower(COALESCE(r.display_name, m.group_name, m.name)) = lower(?)",
        "ORDER BY COALESCE(u.support_status, m.support_status) IS NULL,
                  COALESCE(u.support_status, m.support_status),
                  COALESCE(u.pose, m.pose) IS NULL,
                  COALESCE(u.pose, m.pose),
                  m.dir_path",
    );
    let mut stmt = conn.prepare(&sql).map_err(map_err)?;
    let entries = stmt
        .query_map([group_name], map_entry_row)
        .map_err(map_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_err)?;
    Ok(entries)
}

/// Rename the group shown as `group_name` to `new_name` — stored against
/// the scanner-level source groups so it survives rescans. An empty
/// new_name clears the override(s), reverting to the folder-derived name.
/// Renaming a group to another group's name merges the two.
pub fn rename_group(conn: &Connection, group_name: &str, new_name: &str) -> Result<(), AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Group rename failed: {}", e));
    let new_name = new_name.trim();
    if new_name.is_empty() {
        conn.execute(
            "DELETE FROM group_renames
             WHERE lower(display_name) = lower(?1) OR lower(source_group) = lower(?1)",
            [group_name],
        )
        .map_err(map_err)?;
    } else {
        let changed = conn
            .execute(
                "INSERT INTO group_renames (source_group, display_name)
                 SELECT DISTINCT COALESCE(m.group_name, m.name), ?2
                 FROM models m
                 LEFT JOIN group_renames r
                     ON r.source_group = COALESCE(m.group_name, m.name)
                 WHERE lower(COALESCE(r.display_name, m.group_name, m.name)) = lower(?1)
                 ON CONFLICT(source_group) DO UPDATE SET display_name = excluded.display_name",
                params![group_name, new_name],
            )
            .map_err(map_err)?;
        if changed == 0 {
            return Err(AppError::NotFoundError(format!(
                "No catalog group named '{}'",
                group_name
            )));
        }
    }
    // renamed groups must be findable by their new name
    rebuild_fts(conn).map_err(map_err)?;
    Ok(())
}

pub fn list_tags(conn: &Connection) -> Result<Vec<(String, u32)>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT tag, COUNT(*) FROM model_tags GROUP BY tag
             ORDER BY COUNT(*) DESC, tag COLLATE NOCASE",
        )
        .map_err(|e| AppError::ConfigError(format!("Tag listing failed: {}", e)))?;
    let rows = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| AppError::ConfigError(format!("Tag listing failed: {}", e)))?;
    Ok(rows)
}

pub fn add_tag(conn: &Connection, dir_path: &str, tag: &str) -> Result<(), AppError> {
    let tag = tag.trim().to_lowercase().replace(' ', "_");
    if tag.is_empty() {
        return Err(AppError::InvalidInput("Empty tag".to_string()));
    }
    conn.execute(
        "INSERT OR IGNORE INTO model_tags (dir_path, tag, source) VALUES (?1, ?2, 'user')",
        params![dir_path, tag],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to add tag: {}", e)))?;
    refresh_fts_row(conn, dir_path)
        .map_err(|e| AppError::ConfigError(format!("Failed to update search index: {}", e)))?;
    Ok(())
}

pub fn remove_tag(conn: &Connection, dir_path: &str, tag: &str) -> Result<(), AppError> {
    // Metadata tags reappear on the next scan by design — the metadata
    // file is their source of truth
    conn.execute(
        "DELETE FROM model_tags WHERE dir_path = ?1 AND tag = ?2",
        params![dir_path, tag],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to remove tag: {}", e)))?;
    refresh_fts_row(conn, dir_path)
        .map_err(|e| AppError::ConfigError(format!("Failed to update search index: {}", e)))?;
    Ok(())
}

pub fn model_files(conn: &Connection, dir_path: &str) -> Result<Vec<CatalogFile>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT path, file_name, extension, size_bytes FROM files
             WHERE dir_path = ?1 ORDER BY file_name COLLATE NOCASE",
        )
        .map_err(|e| AppError::ConfigError(format!("File listing failed: {}", e)))?;
    let rows = stmt
        .query_map([dir_path], |row| {
            Ok(CatalogFile {
                path: row.get(0)?,
                file_name: row.get(1)?,
                extension: row.get(2)?,
                size_bytes: row.get::<_, i64>(3)? as f64,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|e| AppError::ConfigError(format!("File listing failed: {}", e)))?;
    Ok(rows)
}

pub fn stats(conn: &Connection) -> Result<CatalogStats, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Stats query failed: {}", e));
    let (total_files, total_size): (u32, i64) = conn
        .query_row(
            "SELECT COUNT(*), COALESCE(SUM(size_bytes), 0) FROM files",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(map_err)?;
    let total_models: u32 = conn
        .query_row("SELECT COUNT(*) FROM models", [], |row| row.get(0))
        .map_err(map_err)?;
    let last_scan: Option<f64> = conn
        .query_row(
            "SELECT CAST(value AS REAL) FROM meta WHERE key = 'last_scan'",
            [],
            |row| row.get(0),
        )
        .ok();

    let mut stmt = conn
        .prepare(
            "SELECT extension, COUNT(*), SUM(size_bytes) FROM files
             GROUP BY extension ORDER BY SUM(size_bytes) DESC",
        )
        .map_err(map_err)?;
    let extensions = stmt
        .query_map([], |row| {
            Ok(ExtensionStat {
                extension: row.get(0)?,
                file_count: row.get(1)?,
                total_size_bytes: row.get::<_, i64>(2)? as f64,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;

    Ok(CatalogStats {
        total_models,
        total_files,
        total_size_bytes: total_size as f64,
        extensions,
        last_scan_epoch: last_scan,
    })
}

/// Sizes that occur more than once — the free prefilter for duplicate
/// detection.
pub fn duplicate_size_candidates(conn: &Connection) -> Result<Vec<(i64, Vec<String>)>, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Dup query failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT size_bytes FROM files WHERE size_bytes > 0
             GROUP BY size_bytes HAVING COUNT(*) > 1",
        )
        .map_err(map_err)?;
    let sizes: Vec<i64> = stmt
        .query_map([], |row| row.get(0))
        .and_then(|rows| rows.collect())
        .map_err(map_err)?;

    let mut result = Vec::with_capacity(sizes.len());
    let mut path_stmt = conn
        .prepare("SELECT path FROM files WHERE size_bytes = ?1 ORDER BY path")
        .map_err(map_err)?;
    for size in sizes {
        let paths: Vec<String> = path_stmt
            .query_map([size], |row| row.get(0))
            .and_then(|rows| rows.collect())
            .map_err(map_err)?;
        result.push((size, paths));
    }
    Ok(result)
}

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

/// Assemble confirmed duplicate groups from stored hashes.
pub fn duplicate_groups(conn: &Connection) -> Result<Vec<DuplicateGroup>, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Dup grouping failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT content_hash, size_bytes, group_concat(path, char(31))
             FROM files
             WHERE content_hash IS NOT NULL
             GROUP BY content_hash HAVING COUNT(*) > 1
             ORDER BY size_bytes * (COUNT(*) - 1) DESC",
        )
        .map_err(map_err)?;
    let groups = stmt
        .query_map([], |row| {
            let joined: String = row.get(2)?;
            Ok(DuplicateGroup {
                hash: row.get(0)?,
                size_bytes: row.get::<_, i64>(1)? as f64,
                paths: joined.split('\u{1f}').map(String::from).collect(),
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(groups)
}

/// Distinct release_name groups found across scanned models, most-models
/// first. Purely a read over already-indexed columns — see ReleaseSummary
/// for why this isn't a "publish log".
pub fn list_releases(conn: &Connection) -> Result<Vec<ReleaseSummary>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Release listing failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT release_name,
                    -- designer isn't guaranteed uniform across a release's
                    -- models (heuristic entries may lack one); take the
                    -- first non-null value as a representative label
                    (SELECT designer FROM models m2
                     WHERE m2.release_name = m1.release_name AND designer IS NOT NULL
                     LIMIT 1),
                    COUNT(*), COALESCE(SUM(total_size_bytes), 0)
             FROM models m1
             WHERE release_name IS NOT NULL AND release_name != ''
             GROUP BY release_name
             ORDER BY COUNT(*) DESC",
        )
        .map_err(map_err)?;
    let releases = stmt
        .query_map([], |row| {
            Ok(ReleaseSummary {
                release_name: row.get(0)?,
                designer: row.get(1)?,
                model_count: row.get(2)?,
                total_size_bytes: row.get::<_, i64>(3)? as f64,
            })
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(releases)
}

fn require_model(conn: &Connection, dir_path: &str) -> Result<(), AppError> {
    conn.query_row(
        "SELECT 1 FROM models WHERE dir_path = ?1",
        [dir_path],
        |_| Ok(()),
    )
    .map_err(|_| AppError::NotFoundError(format!("No cataloged model at '{}'", dir_path)))
}

/// Upsert the user-editable fields (rescan-safe, see model_user_meta).
/// A None custom_name clears the override, reverting to the scanner name.
pub fn update_model_user_meta(
    conn: &Connection,
    dir_path: &str,
    custom_name: Option<String>,
    pose: Option<String>,
    scale: Option<String>,
    support_status: Option<String>,
    release_date: Option<String>,
) -> Result<(), AppError> {
    require_model(conn, dir_path)?;
    conn.execute(
        "INSERT INTO model_user_meta
             (dir_path, custom_name, pose, scale, support_status, release_date)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(dir_path) DO UPDATE SET
             custom_name = excluded.custom_name,
             pose = excluded.pose,
             scale = excluded.scale,
             support_status = excluded.support_status,
             release_date = excluded.release_date",
        params![
            dir_path,
            custom_name,
            pose,
            scale,
            support_status,
            release_date
        ],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to update metadata: {}", e)))?;
    // custom_name feeds search — keep the FTS row in step
    refresh_fts_row(conn, dir_path)
        .map_err(|e| AppError::ConfigError(format!("Failed to update search index: {}", e)))?;
    Ok(())
}

/// Point a model at a user-chosen or rendered preview image. Stored in
/// model_user_meta so it survives rescans and beats the scanner's pick.
pub fn set_model_preview(
    conn: &Connection,
    dir_path: &str,
    preview_path: &str,
) -> Result<(), AppError> {
    require_model(conn, dir_path)?;
    conn.execute(
        "INSERT INTO model_user_meta (dir_path, preview_path) VALUES (?1, ?2)
         ON CONFLICT(dir_path) DO UPDATE SET preview_path = excluded.preview_path",
        params![dir_path, preview_path],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to set preview: {}", e)))?;
    Ok(())
}

/// Prune file rows after an on-disk delete. Duplicate groups and stats
/// both derive from `files`, so this is what makes a dedup delete visible
/// immediately instead of only after the next full rescan. Per-model
/// counters are recomputed for the affected dirs so the UI stays honest.
pub fn remove_files(conn: &mut Connection, paths: &[String]) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Catalog prune failed: {}", e));
    let tx = conn.transaction().map_err(map_err)?;
    {
        let mut affected_dirs: Vec<String> = Vec::new();
        let mut dir_stmt = tx
            .prepare("SELECT dir_path FROM files WHERE path = ?1")
            .map_err(map_err)?;
        let mut delete_stmt = tx
            .prepare("DELETE FROM files WHERE path = ?1")
            .map_err(map_err)?;
        for path in paths {
            if let Ok(dir) = dir_stmt.query_row([path], |row| row.get::<_, String>(0)) {
                if !affected_dirs.contains(&dir) {
                    affected_dirs.push(dir);
                }
            }
            delete_stmt.execute([path]).map_err(map_err)?;
        }
        drop(dir_stmt);
        drop(delete_stmt);

        let mut recount_stmt = tx
            .prepare(
                "UPDATE models SET
                     file_count = (SELECT COUNT(*) FROM files WHERE dir_path = ?1),
                     total_size_bytes =
                         (SELECT COALESCE(SUM(size_bytes), 0) FROM files WHERE dir_path = ?1)
                 WHERE dir_path = ?1",
            )
            .map_err(map_err)?;
        for dir in &affected_dirs {
            recount_stmt.execute([dir]).map_err(map_err)?;
        }
    }
    tx.commit().map_err(map_err)?;
    Ok(())
}

/// Repoint every indexed path after a model directory moves on disk.
/// model_tags is keyed by dir_path, and replace_catalog deletes tags whose
/// dir_path no longer matches a model — so skipping this doesn't just leave
/// the catalog stale, it silently loses user tags on the next rescan.
pub fn move_model(conn: &mut Connection, from: &str, to: &str) -> Result<(), AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Catalog move failed: {}", e));
    let tx = conn.transaction().map_err(map_err)?;
    {
        // substr comparison instead of LIKE: paths may contain % or _
        tx.execute(
            "UPDATE models SET
                 preview_path = CASE
                     WHEN substr(preview_path, 1, length(?1) + 1) = ?1 || '/'
                     THEN ?2 || substr(preview_path, length(?1) + 1)
                     ELSE preview_path END,
                 dir_path = ?2
             WHERE dir_path = ?1",
            params![from, to],
        )
        .map_err(map_err)?;
        tx.execute(
            "UPDATE files SET
                 path = ?2 || substr(path, length(?1) + 1),
                 dir_path = ?2
             WHERE dir_path = ?1",
            params![from, to],
        )
        .map_err(map_err)?;
        // OR IGNORE + sweep: if the destination somehow already carries the
        // same tag, the PK collision shouldn't abort the whole move
        tx.execute(
            "UPDATE OR IGNORE model_tags SET dir_path = ?2 WHERE dir_path = ?1",
            params![from, to],
        )
        .map_err(map_err)?;
        tx.execute("DELETE FROM model_tags WHERE dir_path = ?1", [from])
            .map_err(map_err)?;
        tx.execute(
            "UPDATE OR IGNORE model_user_meta SET dir_path = ?2 WHERE dir_path = ?1",
            params![from, to],
        )
        .map_err(map_err)?;
        tx.execute("DELETE FROM model_user_meta WHERE dir_path = ?1", [from])
            .map_err(map_err)?;

        tx.execute("DELETE FROM models_fts WHERE dir_path = ?1", [from])
            .map_err(map_err)?;
        refresh_fts_row(&tx, to).map_err(map_err)?;
    }
    tx.commit().map_err(map_err)?;
    Ok(())
}

/// Schema init for in-memory test databases in sibling modules.
#[cfg(test)]
pub(crate) fn test_init(conn: &Connection) {
    init_schema(conn).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn
    }

    fn sample_rows() -> (Vec<FileRow>, Vec<ModelRow>, Vec<(String, String)>) {
        let files = vec![
            FileRow {
                path: "/lib/newt/GiantNewt_v02.stl".into(),
                dir_path: "/lib/newt".into(),
                file_name: "GiantNewt_v02.stl".into(),
                extension: "stl".into(),
                size_bytes: 2048,
                modified_at: 100,
            },
            FileRow {
                path: "/lib/bugbear/Bugbear.stl".into(),
                dir_path: "/lib/bugbear".into(),
                file_name: "Bugbear.stl".into(),
                extension: "stl".into(),
                size_bytes: 4096,
                modified_at: 100,
            },
        ];
        let models = vec![
            ModelRow {
                dir_path: "/lib/newt".into(),
                name: "Giant Newt".into(),
                description: Some("A very large newt".into()),
                designer: Some("DTL".into()),
                release_name: Some("Critterfolk".into()),
                preview_path: None,
                source: "metadata".into(),
                uuid: None,
                file_count: 1,
                total_size_bytes: 2048,
                pose: None,
                scale: None,
                support_status: None,
                release_date: None,
                group_name: Some("Giant Newt".into()),
            },
            ModelRow {
                dir_path: "/lib/bugbear".into(),
                name: "Bugbear".into(),
                description: None,
                designer: None,
                release_name: None,
                preview_path: None,
                source: "heuristic".into(),
                uuid: None,
                file_count: 1,
                total_size_bytes: 4096,
                pose: None,
                scale: None,
                support_status: None,
                release_date: None,
                group_name: Some("Bugbear".into()),
            },
        ];
        let tags = vec![("/lib/newt".to_string(), "amphibian".to_string())];
        (files, models, tags)
    }

    #[test]
    fn fts_prefix_search_finds_models() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();

        // prefix match on name
        let page = search(&conn, "new", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].name, "Giant Newt");
        assert_eq!(page.entries[0].tags, vec!["amphibian"]);

        // tag search through FTS
        let page = search(&conn, "amphib", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1);

        // empty query lists everything
        let page = search(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(page.total, 2);

        // tag filter
        let page = search(&conn, "", &["amphibian".to_string()], 10, 0).unwrap();
        assert_eq!(page.total, 1);

        // no match
        let page = search(&conn, "dragon", &[], 10, 0).unwrap();
        assert_eq!(page.total, 0);
    }

    #[test]
    fn user_tags_survive_rescan_and_metadata_tags_refresh() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();

        add_tag(&conn, "/lib/newt", "painted").unwrap();
        // searchable immediately
        assert_eq!(search(&conn, "painted", &[], 10, 0).unwrap().total, 1);

        // rescan with metadata tags gone: user tag survives, metadata tag drops
        replace_catalog(&mut conn, &files, &models, &[]).unwrap();
        let page = search(&conn, "", &["painted".to_string()], 10, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(
            search(&conn, "", &["amphibian".to_string()], 10, 0)
                .unwrap()
                .total,
            0
        );

        // a model that disappeared from disk takes its user tags with it
        let (files, models, _) = sample_rows();
        let only_bugbear_files: Vec<_> = files
            .into_iter()
            .filter(|f| f.dir_path == "/lib/bugbear")
            .collect();
        let only_bugbear_models: Vec<_> = models
            .into_iter()
            .filter(|m| m.dir_path == "/lib/bugbear")
            .collect();
        replace_catalog(&mut conn, &only_bugbear_files, &only_bugbear_models, &[]).unwrap();
        let remaining: u32 = conn
            .query_row("SELECT COUNT(*) FROM model_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn hashes_survive_rescan_when_file_unchanged() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        store_hash(&conn, "/lib/newt/GiantNewt_v02.stl", "abc123").unwrap();

        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        assert_eq!(
            known_hash(&conn, "/lib/newt/GiantNewt_v02.stl"),
            Some("abc123".to_string())
        );

        // changed mtime invalidates the stored hash
        let mut changed = files.clone();
        changed[0].modified_at = 999;
        replace_catalog(&mut conn, &changed, &models, &tags).unwrap();
        assert_eq!(known_hash(&conn, "/lib/newt/GiantNewt_v02.stl"), None);
    }

    #[test]
    fn stats_and_duplicate_candidates() {
        let mut conn = test_conn();
        let (mut files, models, tags) = sample_rows();
        // make the two files the same size -> duplicate candidates
        files[1].size_bytes = 2048;
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();

        let stats = stats(&conn).unwrap();
        assert_eq!(stats.total_files, 2);
        assert_eq!(stats.total_models, 2);
        assert_eq!(stats.total_size_bytes, 4096.0);

        let candidates = duplicate_size_candidates(&conn).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].1.len(), 2);

        store_hash(&conn, &files[0].path, "same").unwrap();
        store_hash(&conn, &files[1].path, "same").unwrap();
        let groups = duplicate_groups(&conn).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].paths.len(), 2);
    }

    #[test]
    fn remove_files_prunes_dups_and_recounts_models() {
        let mut conn = test_conn();
        let (mut files, models, tags) = sample_rows();
        // two identical-content files -> one duplicate group
        files[1].size_bytes = 2048;
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        store_hash(&conn, &files[0].path, "same").unwrap();
        store_hash(&conn, &files[1].path, "same").unwrap();
        assert_eq!(duplicate_groups(&conn).unwrap().len(), 1);

        remove_files(&mut conn, &[files[1].path.clone()]).unwrap();

        // group dissolves without a rescan, and the model's counters follow
        assert!(duplicate_groups(&conn).unwrap().is_empty());
        let (count, size): (u32, i64) = conn
            .query_row(
                "SELECT file_count, total_size_bytes FROM models WHERE dir_path = '/lib/bugbear'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 0);
        assert_eq!(size, 0);
    }

    #[test]
    fn move_model_repoints_index_and_keeps_tags_through_rescan() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        add_tag(&conn, "/lib/newt", "painted").unwrap();

        move_model(&mut conn, "/lib/newt", "/lib/amphibians/newt").unwrap();

        // model, files and search index all follow the new path
        let page = search(&conn, "newt", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].dir_path, "/lib/amphibians/newt");
        assert!(page.entries[0].tags.contains(&"painted".to_string()));
        let moved_file: String = conn
            .query_row(
                "SELECT path FROM files WHERE dir_path = '/lib/amphibians/newt'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(moved_file, "/lib/amphibians/newt/GiantNewt_v02.stl");

        // the regression this guards: a rescan reflecting the new location
        // must not drop the user tag (model_tags is keyed by dir_path)
        let (mut files, mut models, _) = sample_rows();
        files[0].path = "/lib/amphibians/newt/GiantNewt_v02.stl".into();
        files[0].dir_path = "/lib/amphibians/newt".into();
        models[0].dir_path = "/lib/amphibians/newt".into();
        replace_catalog(&mut conn, &files, &models, &[]).unwrap();
        assert_eq!(
            search(&conn, "", &["painted".to_string()], 10, 0)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn user_meta_edits_survive_rescan_and_reject_unknown_models() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();

        update_model_user_meta(
            &conn,
            "/lib/newt",
            Some("Newt, Giant (repose)".into()),
            Some("A".into()),
            Some("32mm".into()),
            Some("supported".into()),
            None,
        )
        .unwrap();
        set_model_preview(&conn, "/lib/newt", "/appdata/previews/abc.png").unwrap();

        // the whole point of model_user_meta: a full rescan keeps user edits
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        let page = search(&conn, "repose", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1, "custom name is searchable after rescan");
        let entry = &page.entries[0];
        assert_eq!(entry.name, "Newt, Giant (repose)");
        assert_eq!(entry.custom_name.as_deref(), Some("Newt, Giant (repose)"));
        assert_eq!(entry.pose.as_deref(), Some("A"));
        assert_eq!(entry.scale.as_deref(), Some("32mm"));
        assert_eq!(
            entry.preview_path.as_deref(),
            Some("/appdata/previews/abc.png")
        );

        // clearing the override reverts to the scanner name
        update_model_user_meta(&conn, "/lib/newt", None, None, None, None, None).unwrap();
        let page = search(&conn, "newt", &[], 10, 0).unwrap();
        assert_eq!(page.entries[0].name, "Giant Newt");
        // ...but the preview set separately is untouched by a metadata save
        assert_eq!(
            page.entries[0].preview_path.as_deref(),
            Some("/appdata/previews/abc.png")
        );

        assert!(update_model_user_meta(&conn, "/nope", None, None, None, None, None).is_err());
        assert!(set_model_preview(&conn, "/nope", "/x.png").is_err());
    }

    #[test]
    fn base_tables_self_heal_on_a_version_stamped_db() {
        // The exact failure this guards: a dev build stamped user_version=4
        // before group_renames existed in the code, so the versioned early
        // return skipped its CREATE forever ("no such table: group_renames")
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute_batch("DROP TABLE group_renames").unwrap();

        init_schema(&conn).unwrap();
        let count: u32 = conn
            .query_row("SELECT COUNT(*) FROM group_renames", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0, "table recreated despite current user_version");
    }

    #[test]
    fn model_columns_self_heal_on_a_version_stamped_db() {
        // Sibling failure: user_version stamped before the group_name ALTER
        // existed in code — the version gate then skipped it forever ("no
        // such column: m.group_name"). Columns are now shape-checked.
        let mut conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        conn.execute_batch("ALTER TABLE models DROP COLUMN group_name")
            .unwrap();

        init_schema(&conn).unwrap();

        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(page.total, 2, "grouped search works after self-heal");
    }

    #[test]
    fn groups_collapse_variants_and_members_come_back_ordered() {
        let mut conn = test_conn();
        // one logical model, four variant dirs: 2 supports x 2 poses
        let variant = |support: &str, pose: &str| ModelRow {
            dir_path: format!("/lib/galeb duhr/{}/{}", support, pose),
            name: format!("galeb duhr {}", pose),
            description: None,
            designer: None,
            release_name: None,
            preview_path: None,
            source: "heuristic".into(),
            uuid: None,
            file_count: 2,
            total_size_bytes: 100,
            pose: Some(pose.into()),
            scale: None,
            support_status: Some(support.into()),
            release_date: None,
            group_name: Some("galeb duhr".into()),
        };
        let models = vec![
            variant("unsupported", "B"),
            variant("unsupported", "A"),
            variant("supported", "A"),
            variant("supported", "B"),
        ];
        replace_catalog(&mut conn, &[], &models, &[]).unwrap();

        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1, "four variants, one card");
        let group = &page.groups[0];
        assert_eq!(group.group_name, "galeb duhr");
        assert_eq!(group.variant_count, 4);
        assert_eq!(group.pose_count, 2);
        assert_eq!(group.file_count, 8);
        let mut supports = group.support_statuses.clone();
        supports.sort();
        assert_eq!(supports, vec!["supported", "unsupported"]);

        // FTS still finds the group through any variant's name
        let page = search_groups(&conn, "galeb", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1);

        // members ordered: supported A, supported B, unsupported A, ...
        let members = group_members(&conn, "GALEB DUHR").unwrap();
        assert_eq!(members.len(), 4, "lookup is case-insensitive");
        let order: Vec<_> = members
            .iter()
            .map(|m| (m.support_status.clone().unwrap(), m.pose.clone().unwrap()))
            .collect();
        assert_eq!(
            order,
            vec![
                ("supported".to_string(), "A".to_string()),
                ("supported".to_string(), "B".to_string()),
                ("unsupported".to_string(), "A".to_string()),
                ("unsupported".to_string(), "B".to_string()),
            ]
        );
    }

    #[test]
    fn group_renames_survive_rescans_and_merge_when_named_alike() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();

        rename_group(&conn, "Giant Newt", "Stone Guardian").unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert!(page.groups.iter().any(|g| g.group_name == "Stone Guardian"));
        assert!(!page.groups.iter().any(|g| g.group_name == "Giant Newt"));

        // findable by the new name, both in FTS and member lookup
        assert_eq!(
            search_groups(&conn, "guardian", &[], 10, 0).unwrap().total,
            1
        );
        assert_eq!(group_members(&conn, "stone guardian").unwrap().len(), 1);

        // a rescan keeps the rename (keyed on the scanner's group name)
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        assert_eq!(
            search_groups(&conn, "guardian", &[], 10, 0).unwrap().total,
            1
        );

        // renaming another group to the same display name merges them
        rename_group(&conn, "Bugbear", "Stone Guardian").unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1, "two groups now share one card");
        assert_eq!(group_members(&conn, "Stone Guardian").unwrap().len(), 2);

        // empty name reverts every override displaying that name
        rename_group(&conn, "Stone Guardian", "").unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(page.total, 2);
        assert!(page.groups.iter().any(|g| g.group_name == "Giant Newt"));

        assert!(rename_group(&conn, "no such group", "x").is_err());
    }

    #[test]
    fn user_meta_follows_a_model_move() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();
        update_model_user_meta(
            &conn,
            "/lib/newt",
            Some("Shiny Newt".into()),
            None,
            None,
            None,
            None,
        )
        .unwrap();

        move_model(&mut conn, "/lib/newt", "/lib/amphibians/newt").unwrap();

        let page = search(&conn, "shiny", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.entries[0].dir_path, "/lib/amphibians/newt");
    }

    #[test]
    fn lists_releases_grouped_from_scanned_models() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        // sample_rows' bugbear model has no release_name (heuristic, no
        // metadata) — only the newt's "Critterfolk" should surface
        replace_catalog(&mut conn, &files, &models, &tags).unwrap();

        let releases = list_releases(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].release_name, "Critterfolk");
        assert_eq!(releases[0].designer.as_deref(), Some("DTL"));
        assert_eq!(releases[0].model_count, 1);
    }
}
