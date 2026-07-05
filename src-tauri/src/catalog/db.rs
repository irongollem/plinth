use crate::error::AppError;
use rusqlite::{params, Connection};
use std::path::Path;

use super::{
    CatalogEntry, CatalogFile, CatalogGroup, CatalogStats, DuplicateGroup, ExtensionStat, FileRow,
    FileVariant, FileVariantRow, ModelRow, ReleaseSummary,
};

const SCHEMA_VERSION: i64 = 5;

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
            -- Opaque physical-file id ("device:inode" on Unix, volume:index
            -- on Windows), captured during duplicate scans. Paths sharing it
            -- are hardlinks — one copy on disk — so equal-hash groups with
            -- one distinct identity are already deduplicated, not reclaimable.
            file_identity TEXT,
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

        -- trigram tokenizer: substring + fuzzy-ish matching ("ermaid" finds
        -- "Mermaid"), not just whole-token prefix. Punctuation is folded out
        -- on the way in (see fts_insert_select) so a query typed without an
        -- apostrophe still hits a possessive designer name.
        CREATE VIRTUAL TABLE IF NOT EXISTS models_fts USING fts5(
            name, description, tags, dir_path,
            tokenize = 'trigram'
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
            preview_path   TEXT,
            -- designer (the studio/brand) rides on the release for scanned
            -- models but is overridable per model; sculptor (the artist) has
            -- no folder signal at all, so it's user/manifest-supplied only.
            -- release_name likewise overrides the scanned release.json value.
            designer       TEXT,
            sculptor       TEXT,
            release_name   TEXT,
            -- the facet between support and pose (weapon/mount/etc.)
            variant        TEXT
        );

        -- Group display-name overrides, keyed by the SCANNER's group name
        -- so they survive rescans (folder names are stable; the override
        -- rides on top). Renaming two groups to the same display name
        -- merges them — that's the manual "combine under one model" tool.
        CREATE TABLE IF NOT EXISTS group_renames (
            source_group TEXT PRIMARY KEY COLLATE NOCASE,
            display_name TEXT NOT NULL
        );

        -- Per-file pose/support assignment for libraries that dump every
        -- pose into one folder. Metadata only (keyed by path, like
        -- model_user_meta): the file never moves, but a dir carrying these
        -- rows fans out into one member per pose at read time. dir_path is
        -- denormalized from files so the read path can group without a join.
        CREATE TABLE IF NOT EXISTS file_variants (
            path           TEXT PRIMARY KEY,
            dir_path       TEXT NOT NULL,
            variant        TEXT,
            pose           TEXT,
            support_status TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_file_variants_dir ON file_variants(dir_path);

        -- Per-variant preview override. A dump folder fans out into several
        -- members that all share one dir_path, so model_user_meta.preview_path
        -- (keyed by dir_path) can't tell them apart: a render for one pose
        -- would overwrite every pose's picture. Keyed by the member's full
        -- variant_key (dir\u1f variant\u1f pose) instead, so each variant keeps
        -- its own shot. dir_path rides along for rescan-time pruning.
        -- Whole-folder models keep using model_user_meta.
        CREATE TABLE IF NOT EXISTS variant_previews (
            variant_key  TEXT PRIMARY KEY,
            dir_path     TEXT NOT NULL,
            preview_path TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_variant_previews_dir ON variant_previews(dir_path);

        -- The user's pick for a group card's main image: WHICH member
        -- represents the card, not a copied path — the member's current
        -- preview is resolved at read time, so re-renders follow along.
        -- Keyed by display name (case-insensitive) so it survives rescans.
        CREATE TABLE IF NOT EXISTS group_covers (
            group_name  TEXT PRIMARY KEY COLLATE NOCASE,
            dir_path    TEXT NOT NULL,
            variant_key TEXT
        );
        "#,
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to init catalog schema: {}", e)))?;

    // Column migrations are shape-checked, NOT version-gated: during dev
    // iteration a build can stamp user_version before an ALTER exists in
    // code, and a version gate then locks that ALTER out forever ("no such
    // column" with no way back). Asking the table what it actually has
    // makes the check idempotent and self-healing on every open.
    // Add any missing TEXT columns to a table. Racy-safe: several
    // connections open in parallel and can both see a column missing, so the
    // loser's "duplicate column" is the goal state, not a failure.
    let add_text_columns = |table: &str, columns: &[&str]| -> Result<(), AppError> {
        let existing: Vec<String> = conn
            .prepare(&format!("PRAGMA table_info({})", table))
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(1))
                    .and_then(|rows| rows.collect())
            })
            .map_err(|e| AppError::ConfigError(format!("Failed to inspect {}: {}", table, e)))?;
        for column in columns {
            if existing.iter().any(|c| c == column) {
                continue;
            }
            if let Err(e) = conn.execute(
                &format!("ALTER TABLE {} ADD COLUMN {} TEXT", table, column),
                [],
            ) {
                if !e.to_string().contains("duplicate column name") {
                    return Err(AppError::ConfigError(format!(
                        "Failed to migrate {} (add {}): {}",
                        table, column, e
                    )));
                }
            }
        }
        Ok(())
    };
    add_text_columns(
        "models",
        &[
            "pose",
            "scale",
            "support_status",
            "release_date",
            "group_name",
            "sculptor",
            "variant",
        ],
    )?;
    // designer already exists on models (from the release); these are the
    // per-model user overrides plus the artist, release-name and variant.
    add_text_columns(
        "model_user_meta",
        &["designer", "sculptor", "release_name", "variant"],
    )?;
    add_text_columns("file_variants", &["variant"])?;
    add_text_columns("files", &["file_identity"])?;
    // Outside the base batch: on a pre-existing db the column only exists
    // after the migration above, and indexing a missing column is an error
    // even under IF NOT EXISTS.
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_files_identity ON files(file_identity)
         WHERE file_identity IS NOT NULL",
        [],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to index file identities: {}", e)))?;

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

    // v5: the FTS index is derived, so switching it to the trigram tokenizer
    // is just a drop-and-rebuild. Existing dbs kept the old default-tokenizer
    // table via IF NOT EXISTS; replace it and repopulate from current models.
    if version < 5 {
        conn.execute("DROP TABLE IF EXISTS models_fts", [])
            .map_err(|e| AppError::ConfigError(format!("Failed to drop old FTS: {}", e)))?;
        conn.execute(
            "CREATE VIRTUAL TABLE models_fts USING fts5(
                 name, description, tags, dir_path, tokenize = 'trigram')",
            [],
        )
        .map_err(|e| AppError::ConfigError(format!("Failed to create trigram FTS: {}", e)))?;
        rebuild_fts(conn)
            .map_err(|e| AppError::ConfigError(format!("Failed to rebuild FTS: {}", e)))?;
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
    metadata_file_variants: &[FileVariantRow],
) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Catalog write failed: {}", e));
    let tx = conn.transaction().map_err(map_err)?;
    {
        // Preserve known content hashes (hashing is the expensive part of
        // duplicate detection) and file identities across the rebuild
        tx.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS old_hashes AS
                 SELECT path, size_bytes, modified_at, content_hash, file_identity
                 FROM files WHERE content_hash IS NOT NULL OR file_identity IS NOT NULL;",
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

        // Restore hashes and identities for files that didn't change
        tx.execute(
            "UPDATE files SET
                 (content_hash, file_identity) = (
                 SELECT oh.content_hash, oh.file_identity FROM old_hashes oh
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
                  release_date, group_name, sculptor, variant, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                  ?16, ?17, strftime('%s','now'))",
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
                    m.group_name,
                    m.sculptor,
                    m.variant
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
        // File-pose assignments are keyed by path; drop any whose file is
        // gone from disk so a curated split doesn't outlive its files.
        tx.execute(
            "DELETE FROM file_variants
             WHERE path NOT IN (SELECT path FROM files)",
            [],
        )
        .map_err(map_err)?;
        // Seed file-pose splits carried in model.json (the 3pk read side).
        // OR IGNORE: a user's own assignment (same path PK) always wins, and
        // metadata rows survive the rescan above just like user ones.
        {
            let mut import = tx
                .prepare(
                    "INSERT OR IGNORE INTO file_variants
                         (path, dir_path, variant, pose, support_status)
                     SELECT ?1, dir_path, ?2, ?3, ?4 FROM files WHERE path = ?1",
                )
                .map_err(map_err)?;
            for fv in metadata_file_variants {
                import
                    .execute(params![fv.path, fv.variant, fv.pose, fv.support_status])
                    .map_err(map_err)?;
            }
        }
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
/// Fold apostrophes and hyphens out of a SQL text expression so a query
/// typed without them ("trappers", "presupported") still matches the
/// indexed value ("Trapper's", "pre-supported"). Must mirror the query-side
/// stripping in `fts_query`. char(8217)/char(8216) are the curly quotes.
fn fts_norm(expr: &str) -> String {
    format!(
        "REPLACE(REPLACE(REPLACE(REPLACE({e}, '''', ''), '-', ''), char(8217), ''), char(8216), '')",
        e = expr
    )
}

/// The INSERT that (re)builds an FTS row. designer + sculptor ride in the
/// free-text `tags` column rather than their own FTS columns, keeping the
/// virtual table shape stable while making both searchable.
fn fts_insert_select() -> String {
    format!(
        "INSERT INTO models_fts (name, description, tags, dir_path)
         SELECT {name}, COALESCE(m.description, ''), {tags}, m.dir_path
         FROM models m
         LEFT JOIN model_user_meta u ON u.dir_path = m.dir_path
         LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name)",
        name = fts_norm("COALESCE(u.custom_name, m.name)"),
        tags = fts_norm(
            "COALESCE((SELECT group_concat(t.tag, ' ') FROM model_tags t
                       WHERE t.dir_path = m.dir_path), '')
                 || ' ' || COALESCE(r.display_name, '')
                 || ' ' || COALESCE(u.designer, m.designer, '')
                 || ' ' || COALESCE(u.sculptor, m.sculptor, '')
                 || ' ' || COALESCE(u.release_name, m.release_name, '')
                 || ' ' || COALESCE(u.variant, m.variant, '')"
        ),
    )
}

fn rebuild_fts(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM models_fts", [])?;
    conn.execute(&fts_insert_select(), [])?;
    Ok(())
}

/// Refresh the FTS row for one model after a tag or user-meta change.
fn refresh_fts_row(conn: &Connection, dir_path: &str) -> Result<(), rusqlite::Error> {
    conn.execute("DELETE FROM models_fts WHERE dir_path = ?1", [dir_path])?;
    conn.execute(
        &format!("{} WHERE m.dir_path = ?1", fts_insert_select()),
        [dir_path],
    )?;
    Ok(())
}

/// Build a trigram FTS query: each word becomes a quoted substring match,
/// ANDed. Punctuation is stripped to mirror the indexed normalization, and
/// sub-trigram (<3 char) words are dropped — trigram can't match them, so
/// keeping them would return nothing. An all-short query yields "" and the
/// caller skips the FTS filter entirely.
fn fts_query(text: &str) -> String {
    text.split_whitespace()
        .map(|word| {
            word.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
        })
        .filter(|word| word.chars().count() >= 3)
        .map(|word| format!("\"{}\"", word))
        .collect::<Vec<_>>()
        .join(" AND ")
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
        // May be empty if every word was sub-trigram (<3 chars); then we
        // skip the FTS filter rather than MATCH "" (which errors).
        let fts = fts_query(trimmed);
        if !fts.is_empty() {
            where_clauses.push(
                "m.dir_path IN (SELECT dir_path FROM models_fts WHERE models_fts MATCH ?)"
                    .to_string(),
            );
            bound.push(Box::new(fts));
        }
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
        "SELECT m.dir_path, COALESCE(u.custom_name, m.name), m.description,
                COALESCE(u.designer, m.designer),
                COALESCE(u.release_name, m.release_name),
                COALESCE(u.preview_path, m.preview_path),
                m.file_count, m.total_size_bytes,
                COALESCE((SELECT group_concat(t.tag, char(31)) FROM model_tags t
                          WHERE t.dir_path = m.dir_path), ''),
                COALESCE(u.pose, m.pose), COALESCE(u.scale, m.scale),
                COALESCE(u.support_status, m.support_status),
                COALESCE(u.release_date, m.release_date),
                u.custom_name, COALESCE(u.sculptor, m.sculptor),
                COALESCE(u.variant, m.variant),
                COALESCE(m.group_name, m.name)
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
        sculptor: row.get(14)?,
        variant: row.get(15)?,
        source_group: row.get(16)?,
        // Whole-folder member; expand_file_variants stamps a key on any
        // synthetic pose members it derives from this row.
        variant_key: None,
    })
}

/// Separator inside a variant_key. The unit separator can't occur in a path,
/// so a key never collides with a real directory. Format is
/// `dir\u{1f}variant\u{1f}pose`; empty variant AND pose = the residual pool.
const VARIANT_SEP: char = '\u{1f}';

/// Build a member's variant_key. Empty facet strings encode "no variant"/
/// "no pose"; both empty is the residual/unassigned member.
fn variant_key(dir_path: &str, variant: &str, pose: &str) -> String {
    format!("{dir_path}{VARIANT_SEP}{variant}{VARIANT_SEP}{pose}")
}

/// Recover (variant, pose) from a variant_key. dir_path is the authority for
/// which folder, so the leading segment is ignored; the last two fields are
/// the facets (either may be "" for unset).
fn parse_variant_key(key: &str) -> (&str, &str) {
    let mut fields = key.rsplit(VARIANT_SEP);
    let pose = fields.next().unwrap_or("");
    let variant = fields.next().unwrap_or("");
    (variant, pose)
}

/// path -> size for a dir's indexed files (model files only; images aren't
/// indexed). Used to recompute per-pose counts and sizes after a split.
fn file_sizes_for_dir(
    conn: &Connection,
    dir_path: &str,
) -> Result<std::collections::HashMap<String, i64>, AppError> {
    let map = |e: rusqlite::Error| AppError::ConfigError(format!("File size lookup failed: {}", e));
    let mut stmt = conn
        .prepare("SELECT path, size_bytes FROM files WHERE dir_path = ?1")
        .map_err(map)?;
    let rows = stmt
        .query_map([dir_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(map)?;
    rows.collect::<Result<_, _>>().map_err(map)
}

/// Fan a folder that carries file→pose assignments into one member per
/// assigned pose, plus a residual member for any still-unassigned model
/// files. Counts and sizes are recomputed per bucket from the files table.
/// Folders with no assignments pass through untouched, so nothing regresses
/// for the folder-per-model libraries. Ordered supported-before-unsupported
/// then by pose, matching the whole-folder member ordering.
fn expand_file_variants(
    conn: &Connection,
    entries: Vec<CatalogEntry>,
) -> Result<Vec<CatalogEntry>, AppError> {
    use std::collections::{BTreeMap, HashSet};
    let mut out = Vec::new();
    for entry in entries {
        let assigned: Vec<FileVariant> = get_file_variants(conn, &entry.dir_path)?
            .into_iter()
            .filter(|v| v.pose.as_deref().is_some_and(|p| !p.is_empty()))
            .collect();
        if assigned.is_empty() {
            out.push(entry);
            continue;
        }
        let sizes = file_sizes_for_dir(conn, &entry.dir_path)?;
        // Per-variant preview overrides for this folder, keyed by variant_key.
        // A member with its own render beats the folder-level preview it would
        // otherwise inherit from `entry` below.
        let previews = get_variant_previews(conn, &entry.dir_path)?;
        // (support, variant, pose) -> file paths; BTreeMap for a stable order
        let mut buckets: BTreeMap<(Option<String>, String, String), Vec<String>> = BTreeMap::new();
        let mut claimed: HashSet<String> = HashSet::new();
        for v in assigned {
            let variant = v.variant.unwrap_or_default();
            let pose = v.pose.unwrap_or_default();
            claimed.insert(v.path.clone());
            buckets
                .entry((v.support_status, variant, pose))
                .or_default()
                .push(v.path);
        }
        for ((support, variant, pose), paths) in buckets {
            let bytes: i64 = paths.iter().filter_map(|p| sizes.get(p)).sum();
            // label reads "mob sword 2" — base plus whichever facets are set
            let mut label = entry.name.clone();
            for facet in [&variant, &pose] {
                if !facet.is_empty() {
                    label.push(' ');
                    label.push_str(facet);
                }
            }
            let key = variant_key(&entry.dir_path, &variant, &pose);
            let preview_path = previews
                .get(&key)
                .cloned()
                .or_else(|| entry.preview_path.clone());
            out.push(CatalogEntry {
                name: label,
                variant: (!variant.is_empty()).then(|| variant.clone()),
                pose: (!pose.is_empty()).then(|| pose.clone()),
                support_status: support.or_else(|| entry.support_status.clone()),
                file_count: paths.len() as u32,
                total_size_bytes: bytes as f64,
                preview_path,
                variant_key: Some(key),
                ..entry.clone()
            });
        }
        // Whatever the user hasn't sorted yet stays visible as a residual
        // member so no file silently vanishes from the folder.
        let residual: Vec<&String> = sizes.keys().filter(|p| !claimed.contains(*p)).collect();
        if !residual.is_empty() {
            let bytes: i64 = residual.iter().filter_map(|p| sizes.get(*p)).sum();
            let key = variant_key(&entry.dir_path, "", "");
            let preview_path = previews
                .get(&key)
                .cloned()
                .or_else(|| entry.preview_path.clone());
            out.push(CatalogEntry {
                name: format!("{} (unassigned)", entry.name),
                variant: None,
                pose: None,
                file_count: residual.len() as u32,
                total_size_bytes: bytes as f64,
                preview_path,
                variant_key: Some(key),
                ..entry
            });
        }
    }
    Ok(out)
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
                MAX(COALESCE(u.designer, m.designer)),
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
    let mut groups = stmt
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

    // A user-picked cover beats the arbitrary MAX() representative
    for group in &mut groups {
        if let Some(preview) = cover_preview(conn, &group.group_name) {
            group.preview_path = Some(preview);
        }
    }

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
    // A curated dump folder becomes several pose members here; untouched
    // folders pass straight through.
    expand_file_variants(conn, entries)
}

/// Map every scanner-level source group currently SHOWN as `group_name`
/// to display as `new_name`. Returns how many mappings were written.
fn upsert_group_rename(
    conn: &Connection,
    group_name: &str,
    new_name: &str,
) -> Result<usize, rusqlite::Error> {
    conn.execute(
        "INSERT INTO group_renames (source_group, display_name)
         SELECT DISTINCT COALESCE(m.group_name, m.name), ?2
         FROM models m
         LEFT JOIN group_renames r
             ON r.source_group = COALESCE(m.group_name, m.name)
         WHERE lower(COALESCE(r.display_name, m.group_name, m.name)) = lower(?1)
         ON CONFLICT(source_group) DO UPDATE SET display_name = excluded.display_name",
        params![group_name, new_name],
    )
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
        let changed = upsert_group_rename(conn, group_name, new_name).map_err(map_err)?;
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

/// The scanner-level source groups currently shown under one display name —
/// more than one means the card is a combination (renamed-together groups),
/// which is what makes it splittable: clearing the renames (rename_group
/// with an empty name) restores exactly these names as separate cards.
pub fn group_sources(conn: &Connection, group_name: &str) -> Result<Vec<String>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Group source lookup failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT COALESCE(m.group_name, m.name) AS src
             FROM models m
             LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name)
             WHERE lower(COALESCE(r.display_name, m.group_name, m.name)) = lower(?1)
             ORDER BY src COLLATE NOCASE",
        )
        .map_err(map_err)?;
    let rows = stmt
        .query_map([group_name], |row| row.get(0))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?;
    Ok(rows)
}

/// Undo ONE source's membership in a combined card — the fix for "I checked
/// one card too many when combining". Deletes just that source's rename row,
/// so it reappears as its own card under its folder-derived name; the rest
/// of the combination is untouched. Errors when the source sits in the card
/// by its own folder name (nothing to detach — that's a folder rename/move).
pub fn detach_group_source(
    conn: &Connection,
    group_name: &str,
    source_group: &str,
) -> Result<(), AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Detach failed: {}", e));
    let removed = conn
        .execute(
            "DELETE FROM group_renames
             WHERE lower(source_group) = lower(?2) AND lower(display_name) = lower(?1)",
            params![group_name, source_group],
        )
        .map_err(map_err)?;
    if removed == 0 {
        return Err(AppError::InvalidInput(format!(
            "\"{}\" isn't combined into \"{}\" — it groups there under its own folder name, so rename or move the folder instead",
            source_group, group_name
        )));
    }
    rebuild_fts(conn).map_err(map_err)?;
    Ok(())
}

/// Remember which member fronts a group's card. Stored as the member's
/// identity (dir_path + optional variant_key), resolved to its CURRENT
/// preview at read time — a re-render updates the card automatically.
pub fn set_group_cover(
    conn: &Connection,
    group_name: &str,
    dir_path: &str,
    variant_key: Option<&str>,
) -> Result<(), AppError> {
    require_model(conn, dir_path)?;
    conn.execute(
        "INSERT INTO group_covers (group_name, dir_path, variant_key)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(group_name) DO UPDATE SET
             dir_path = excluded.dir_path,
             variant_key = excluded.variant_key",
        params![group_name, dir_path, variant_key],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to set card image: {}", e)))?;
    Ok(())
}

/// The chosen cover member's current preview, if a cover is set and its
/// member still has one.
fn cover_preview(conn: &Connection, group_name: &str) -> Option<String> {
    conn.query_row(
        "SELECT COALESCE(vp.preview_path, u.preview_path, m.preview_path)
         FROM group_covers gc
         LEFT JOIN variant_previews vp ON vp.variant_key = gc.variant_key
         LEFT JOIN model_user_meta u ON u.dir_path = gc.dir_path
         LEFT JOIN models m ON m.dir_path = gc.dir_path
         WHERE gc.group_name = ?1",
        [group_name],
        |row| row.get(0),
    )
    .ok()
    .flatten()
}

/// The explicit merge tool: map every listed group onto one display name.
/// This is rename_group's merge behavior made first-class — folder
/// inference only groups what a creator's structure happens to encode,
/// and every creator structures differently, so combining must never
/// DEPEND on inference. One transaction, one FTS rebuild.
pub fn combine_groups(
    conn: &mut Connection,
    group_names: &[String],
    target_name: &str,
) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Group combine failed: {}", e));
    let target_name = target_name.trim();
    if target_name.is_empty() {
        return Err(AppError::InvalidInput(
            "A combined model needs a name".to_string(),
        ));
    }
    let tx = conn.transaction().map_err(map_err)?;
    let mut changed = 0;
    for group_name in group_names {
        changed += upsert_group_rename(&tx, group_name, target_name).map_err(map_err)?;
    }
    if changed == 0 {
        return Err(AppError::NotFoundError(
            "None of the selected groups exist anymore".to_string(),
        ));
    }
    rebuild_fts(&tx).map_err(map_err)?;
    tx.commit().map_err(map_err)?;
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

/// The dir_paths shown under one card — the same display-name resolution as
/// group_members, for operations that apply to the whole logical model.
fn group_member_dirs(conn: &Connection, group_name: &str) -> Result<Vec<String>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Group member lookup failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT m.dir_path FROM models m
             LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name)
             WHERE lower(COALESCE(r.display_name, m.group_name, m.name)) = lower(?1)",
        )
        .map_err(map_err)?;
    stmt.query_map([group_name], |row| row.get(0))
        .and_then(|rows| rows.collect())
        .map_err(map_err)
}

/// Tag every member of a group. A tag describes the mini, not one build of
/// it — tagging the supported and unsupported variants separately was just
/// busywork that drifted out of sync.
pub fn add_group_tag(conn: &Connection, group_name: &str, tag: &str) -> Result<(), AppError> {
    let dirs = group_member_dirs(conn, group_name)?;
    if dirs.is_empty() {
        return Err(AppError::NotFoundError(format!(
            "No catalog group named '{}'",
            group_name
        )));
    }
    for dir in &dirs {
        add_tag(conn, dir, tag)?;
    }
    Ok(())
}

pub fn remove_group_tag(conn: &Connection, group_name: &str, tag: &str) -> Result<(), AppError> {
    for dir in group_member_dirs(conn, group_name)? {
        remove_tag(conn, &dir, tag)?;
    }
    Ok(())
}

/// The supported/unsupported (and format-variant) builds of the same sculpt:
/// model dirs in the same group whose paths are identical once
/// support-status segments are ignored. Exact structural match only — no
/// fuzzy pairing — so an edit can never propagate to the wrong model.
pub fn support_twins(conn: &Connection, dir_path: &str) -> Result<Vec<String>, AppError> {
    let map_err = |e: rusqlite::Error| AppError::ConfigError(format!("Twin lookup failed: {}", e));
    let support_neutral_key = |path: &str| -> String {
        Path::new(path)
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .filter(|seg| crate::catalog::scanner::support_from_segment(seg).is_none())
            .collect::<Vec<_>>()
            .join("\u{1f}")
            .to_lowercase()
    };
    let own_key = support_neutral_key(dir_path);
    let mut stmt = conn
        .prepare(
            "SELECT m2.dir_path FROM models m2
             WHERE lower(COALESCE(m2.group_name, m2.name)) =
                   (SELECT lower(COALESCE(group_name, name)) FROM models WHERE dir_path = ?1)
               AND m2.dir_path <> ?1",
        )
        .map_err(map_err)?;
    let candidates: Vec<String> = stmt
        .query_map([dir_path], |row| row.get(0))
        .and_then(|rows| rows.collect())
        .map_err(map_err)?;
    Ok(candidates
        .into_iter()
        .filter(|c| support_neutral_key(c) == own_key)
        .collect())
}

/// Partial user-meta upsert used for twin propagation: only Some fields are
/// written (COALESCE keeps the twin's own values for the rest), so a
/// file-split member sending null facets never clears its twin.
pub fn update_model_facets(
    conn: &Connection,
    dir_path: &str,
    variant: Option<&str>,
    pose: Option<&str>,
    scale: Option<&str>,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO model_user_meta (dir_path, variant, pose, scale)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(dir_path) DO UPDATE SET
             variant = COALESCE(excluded.variant, model_user_meta.variant),
             pose    = COALESCE(excluded.pose, model_user_meta.pose),
             scale   = COALESCE(excluded.scale, model_user_meta.scale)",
        params![dir_path, variant, pose, scale],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to propagate facets: {}", e)))?;
    Ok(())
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

/// Files for a member. `variant_key` (from a synthesized pose member)
/// narrows to just that pose's assigned files; `...{sep}` with an empty
/// pose returns the residual unassigned files; None returns every file in
/// the folder (the whole-folder member, and every non-split model).
pub fn model_files(
    conn: &Connection,
    dir_path: &str,
    variant_key: Option<&str>,
) -> Result<Vec<CatalogFile>, AppError> {
    let map = |e: rusqlite::Error| AppError::ConfigError(format!("File listing failed: {}", e));
    // The key's own dir prefix is ignored — dir_path is the authority — so a
    // stale key can never pull files from another folder.
    let facets = variant_key.map(parse_variant_key);
    let read = |row: &rusqlite::Row| {
        Ok(CatalogFile {
            path: row.get(0)?,
            file_name: row.get(1)?,
            extension: row.get(2)?,
            size_bytes: row.get::<_, i64>(3)? as f64,
        })
    };
    let select = "SELECT f.path, f.file_name, f.extension, f.size_bytes FROM files f WHERE ";
    let order = " ORDER BY f.file_name COLLATE NOCASE";
    let rows = match facets {
        // whole-folder member: every file
        None => {
            let sql = format!("{select}f.dir_path = ?1{order}");
            conn.prepare(&sql)
                .and_then(|mut s| s.query_map(params![dir_path], read)?.collect())
        }
        // residual pool: files with no (variant/pose) assignment
        Some(("", "")) => {
            let sql = format!(
                "{select}f.dir_path = ?1 AND f.path NOT IN
                     (SELECT path FROM file_variants WHERE dir_path = ?1
                      AND (COALESCE(variant,'') <> '' OR COALESCE(pose,'') <> '')){order}"
            );
            conn.prepare(&sql)
                .and_then(|mut s| s.query_map(params![dir_path], read)?.collect())
        }
        // a specific (variant, pose) bucket
        Some((variant, pose)) => {
            let sql = format!(
                "{select}f.path IN (SELECT path FROM file_variants
                     WHERE dir_path = ?1 AND COALESCE(variant,'') = ?2
                       AND COALESCE(pose,'') = ?3){order}"
            );
            conn.prepare(&sql).and_then(|mut s| {
                s.query_map(params![dir_path, variant, pose], read)?
                    .collect()
            })
        }
    }
    .map_err(map)?;
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
    // Hardlinked paths (same file_identity) occupy the disk once, however
    // many names they carry — subtract the extra names so the headline size
    // reports actual disk usage. Only duplicate-scan candidates carry an
    // identity, so this subquery stays small at any library size.
    let shared_savings: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(size_bytes * (n - 1)), 0) FROM (
                 SELECT MAX(size_bytes) AS size_bytes, COUNT(*) AS n FROM files
                 WHERE file_identity IS NOT NULL
                 GROUP BY file_identity HAVING COUNT(*) > 1
             )",
            [],
            |row| row.get(0),
        )
        .map_err(map_err)?;
    let total_size = total_size - shared_savings;
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

/// Batch-write physical-file identities in one transaction — a duplicate
/// scan refreshes every candidate, and per-row autocommits would turn
/// thousands of cheap stats into thousands of fsyncs.
pub fn store_identities(conn: &Connection, entries: &[(String, String)]) -> Result<(), AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Failed to store identities: {}", e));
    let tx = conn.unchecked_transaction().map_err(map_err)?;
    {
        let mut stmt = tx
            .prepare("UPDATE files SET file_identity = ?2 WHERE path = ?1")
            .map_err(map_err)?;
        for (path, identity) in entries {
            stmt.execute(params![path, identity]).map_err(map_err)?;
        }
    }
    tx.commit().map_err(map_err)?;
    Ok(())
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
            "SELECT content_hash, size_bytes, group_concat(path, char(31)),
                    COUNT(DISTINCT COALESCE(file_identity, path))
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
            Ok(DuplicateGroup {
                hash: row.get(0)?,
                size_bytes: row.get::<_, i64>(1)? as f64,
                paths: joined.split('\u{1f}').map(String::from).collect(),
                distinct_copies: row.get(3)?,
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
    designer: Option<String>,
    sculptor: Option<String>,
    release_name: Option<String>,
    variant: Option<String>,
) -> Result<(), AppError> {
    require_model(conn, dir_path)?;
    conn.execute(
        "INSERT INTO model_user_meta
             (dir_path, custom_name, pose, scale, support_status, release_date,
              designer, sculptor, release_name, variant)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(dir_path) DO UPDATE SET
             custom_name = excluded.custom_name,
             pose = excluded.pose,
             scale = excluded.scale,
             support_status = excluded.support_status,
             release_date = excluded.release_date,
             designer = excluded.designer,
             sculptor = excluded.sculptor,
             release_name = excluded.release_name,
             variant = excluded.variant",
        params![
            dir_path,
            custom_name,
            pose,
            scale,
            support_status,
            release_date,
            designer,
            sculptor,
            release_name,
            variant
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

/// Point a single fanned-out member (one pose/variant of a dump folder) at a
/// preview, keyed by its full variant_key so sibling poses in the same folder
/// keep their own pictures. dir_path (the owning folder) rides along so a
/// rescan can prune previews for folders that no longer exist.
pub fn set_variant_preview(
    conn: &Connection,
    dir_path: &str,
    variant_key: &str,
    preview_path: &str,
) -> Result<(), AppError> {
    require_model(conn, dir_path)?;
    conn.execute(
        "INSERT INTO variant_previews (variant_key, dir_path, preview_path)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(variant_key) DO UPDATE SET
             preview_path = excluded.preview_path,
             dir_path = excluded.dir_path",
        params![variant_key, dir_path, preview_path],
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to set variant preview: {}", e)))?;
    Ok(())
}

/// Route a preview to the right store: a fanned-out member (variant_key set)
/// gets a per-variant preview so poses in one folder don't clobber each other;
/// a whole-folder member falls back to model_user_meta.
pub fn set_preview(
    conn: &Connection,
    dir_path: &str,
    variant_key: Option<&str>,
    preview_path: &str,
) -> Result<(), AppError> {
    match variant_key {
        Some(key) => set_variant_preview(conn, dir_path, key, preview_path),
        None => set_model_preview(conn, dir_path, preview_path),
    }
}

/// variant_key -> preview_path for every per-variant preview under one folder.
/// Consulted by expand_file_variants to override the folder-level preview each
/// synthesized member would otherwise inherit.
fn get_variant_previews(
    conn: &Connection,
    dir_path: &str,
) -> Result<std::collections::HashMap<String, String>, AppError> {
    let map = |e: rusqlite::Error| {
        AppError::ConfigError(format!("Failed to read variant previews: {}", e))
    };
    let mut stmt = conn
        .prepare("SELECT variant_key, preview_path FROM variant_previews WHERE dir_path = ?1")
        .map_err(map)?;
    let rows = stmt
        .query_map([dir_path], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(map)?;
    rows.collect::<Result<_, _>>().map_err(map)
}

/// Assign a set of files to a pose bucket (and optional per-file support),
/// so a single dump folder can be split into pose members without touching
/// disk. dir_path is pulled from the files table, so unknown paths are
/// silently skipped rather than orphaning a row. A None pose clears the
/// pose while keeping the row — pass it through clear_file_variants to drop
/// the assignment entirely. Returns how many known files were assigned.
pub fn set_file_variants(
    conn: &mut Connection,
    paths: &[String],
    variant: Option<String>,
    pose: Option<String>,
    support_status: Option<String>,
) -> Result<u32, AppError> {
    let map = |e: rusqlite::Error| AppError::ConfigError(format!("Failed to assign files: {}", e));
    let tx = conn.transaction().map_err(map)?;
    let mut assigned = 0u32;
    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO file_variants (path, dir_path, variant, pose, support_status)
                 SELECT ?1, dir_path, ?2, ?3, ?4 FROM files WHERE path = ?1
                 ON CONFLICT(path) DO UPDATE SET
                     variant = excluded.variant,
                     pose = excluded.pose,
                     support_status = excluded.support_status",
            )
            .map_err(map)?;
        for path in paths {
            assigned += stmt
                .execute(params![path, variant, pose, support_status])
                .map_err(map)? as u32;
        }
    }
    tx.commit().map_err(map)?;
    Ok(assigned)
}

/// Drop pose assignments for the given files, reverting them to plain
/// members of their folder.
/// Returns how many assignments actually existed — files that were never
/// filed clear nothing, and the UI should say so instead of claiming success.
pub fn clear_file_variants(conn: &Connection, paths: &[String]) -> Result<u32, AppError> {
    let mut cleared = 0u32;
    for path in paths {
        cleared += conn
            .execute("DELETE FROM file_variants WHERE path = ?1", params![path])
            .map_err(|e| AppError::ConfigError(format!("Failed to clear assignment: {}", e)))?
            as u32;
    }
    Ok(cleared)
}

/// Every file-pose assignment under one model folder, for the split UI.
pub fn get_file_variants(conn: &Connection, dir_path: &str) -> Result<Vec<FileVariant>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT path, dir_path, variant, pose, support_status
             FROM file_variants WHERE dir_path = ?1 ORDER BY path",
        )
        .map_err(|e| AppError::ConfigError(format!("Failed to read assignments: {}", e)))?;
    let rows = stmt
        .query_map(params![dir_path], |row| {
            Ok(FileVariant {
                path: row.get(0)?,
                dir_path: row.get(1)?,
                variant: row.get(2)?,
                pose: row.get(3)?,
                support_status: row.get(4)?,
            })
        })
        .map_err(|e| AppError::ConfigError(format!("Failed to read assignments: {}", e)))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| AppError::ConfigError(format!("Failed to read assignments: {}", e)))
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

    fn file_row(path: &str, dir_path: &str, size_bytes: i64) -> FileRow {
        FileRow {
            path: path.into(),
            dir_path: dir_path.into(),
            file_name: path.rsplit('/').next().unwrap().into(),
            extension: path.rsplit('.').next().unwrap().into(),
            size_bytes,
            modified_at: 100,
        }
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
                variant: None,
                sculptor: None,
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
                variant: None,
                sculptor: None,
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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        add_tag(&conn, "/lib/newt", "painted").unwrap();
        // searchable immediately
        assert_eq!(search(&conn, "painted", &[], 10, 0).unwrap().total, 1);

        // rescan with metadata tags gone: user tag survives, metadata tag drops
        replace_catalog(&mut conn, &files, &models, &[], &[]).unwrap();
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
        replace_catalog(
            &mut conn,
            &only_bugbear_files,
            &only_bugbear_models,
            &[],
            &[],
        )
        .unwrap();
        let remaining: u32 = conn
            .query_row("SELECT COUNT(*) FROM model_tags", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn hashes_survive_rescan_when_file_unchanged() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
        store_hash(&conn, "/lib/newt/GiantNewt_v02.stl", "abc123").unwrap();

        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
        assert_eq!(
            known_hash(&conn, "/lib/newt/GiantNewt_v02.stl"),
            Some("abc123".to_string())
        );

        // changed mtime invalidates the stored hash
        let mut changed = files.clone();
        changed[0].modified_at = 999;
        replace_catalog(&mut conn, &changed, &models, &tags, &[]).unwrap();
        assert_eq!(known_hash(&conn, "/lib/newt/GiantNewt_v02.stl"), None);
    }

    #[test]
    fn stats_and_duplicate_candidates() {
        let mut conn = test_conn();
        let (mut files, models, tags) = sample_rows();
        // make the two files the same size -> duplicate candidates
        files[1].size_bytes = 2048;
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
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
        replace_catalog(&mut conn, &files, &models, &[], &[]).unwrap();
        assert_eq!(
            search(&conn, "", &["painted".to_string()], 10, 0)
                .unwrap()
                .total,
            1
        );
    }

    #[test]
    fn imported_file_poses_seed_but_never_clobber_a_user_split() {
        let path = "/lib/newt/GiantNewt_v02.stl";
        let seed = |variant: &str, pose: &str| {
            vec![FileVariantRow {
                path: path.into(),
                variant: Some(variant.into()),
                pose: Some(pose.into()),
                support_status: None,
            }]
        };
        let (files, models, tags) = sample_rows();

        // fresh catalog: the model.json split is imported
        let mut conn = test_conn();
        replace_catalog(&mut conn, &files, &models, &tags, &seed("sword", "1")).unwrap();
        let fv = get_file_variants(&conn, "/lib/newt").unwrap();
        assert_eq!(fv.len(), 1);
        assert_eq!(fv[0].variant.as_deref(), Some("sword"));
        assert_eq!(fv[0].pose.as_deref(), Some("1"));

        // but once the user has their own split, a rescan importing a
        // different one leaves theirs untouched (INSERT OR IGNORE on path)
        set_file_variants(&mut conn, &[path.into()], None, Some("Z".into()), None).unwrap();
        replace_catalog(&mut conn, &files, &models, &tags, &seed("bow", "9")).unwrap();
        let fv = get_file_variants(&conn, "/lib/newt").unwrap();
        assert_eq!(fv.len(), 1);
        assert_eq!(fv[0].pose.as_deref(), Some("Z"), "the user's split wins");
        assert!(fv[0].variant.is_none());
    }

    #[test]
    fn file_variants_round_trip_survive_rescan_and_prune() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        // assign the newt's file to pose B; an unknown path is silently
        // skipped (no file row to hang dir_path off of)
        let assigned = set_file_variants(
            &mut conn,
            &[
                "/lib/newt/GiantNewt_v02.stl".into(),
                "/lib/newt/does-not-exist.stl".into(),
            ],
            Some("sword".into()),
            Some("B".into()),
            Some("unsupported".into()),
        )
        .unwrap();
        assert_eq!(assigned, 1, "only the known file is assigned");

        let variants = get_file_variants(&conn, "/lib/newt").unwrap();
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].variant.as_deref(), Some("sword"));
        assert_eq!(variants[0].pose.as_deref(), Some("B"));
        assert_eq!(variants[0].support_status.as_deref(), Some("unsupported"));
        assert_eq!(
            variants[0].dir_path, "/lib/newt",
            "dir_path denormalized from files"
        );

        // reassigning updates in place rather than duplicating
        set_file_variants(
            &mut conn,
            &["/lib/newt/GiantNewt_v02.stl".into()],
            None,
            Some("C".into()),
            None,
        )
        .unwrap();
        let variants = get_file_variants(&conn, "/lib/newt").unwrap();
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].pose.as_deref(), Some("C"));
        assert!(variants[0].variant.is_none(), "variant cleared on reassign");

        // a rescan that still lists the file keeps the assignment
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
        assert_eq!(get_file_variants(&conn, "/lib/newt").unwrap().len(), 1);

        // but a rescan where the file is gone from disk prunes it
        let pruned_files = vec![files[1].clone()];
        let pruned_models = vec![models[1].clone()];
        replace_catalog(&mut conn, &pruned_files, &pruned_models, &[], &[]).unwrap();
        assert!(get_file_variants(&conn, "/lib/newt").unwrap().is_empty());

        // and clearing drops the assignment explicitly
        set_file_variants(
            &mut conn,
            &[files[1].path.clone()],
            None,
            Some("A".into()),
            None,
        )
        .unwrap();
        clear_file_variants(&conn, &[files[1].path.clone()]).unwrap();
        assert!(get_file_variants(&conn, "/lib/bugbear").unwrap().is_empty());
    }

    #[test]
    fn split_folder_fans_into_pose_members_with_scoped_files() {
        let mut conn = test_conn();
        // one dump folder holding three model files, no pose subfolders
        let files = vec![
            file_row("/dump/mob/a.stl", "/dump/mob", 100),
            file_row("/dump/mob/b.stl", "/dump/mob", 200),
            file_row("/dump/mob/c.stl", "/dump/mob", 400),
        ];
        let models = vec![ModelRow {
            dir_path: "/dump/mob".into(),
            name: "mob".into(),
            description: None,
            designer: None,
            release_name: None,
            preview_path: None,
            source: "heuristic".into(),
            uuid: None,
            file_count: 3,
            total_size_bytes: 700,
            pose: None,
            scale: None,
            support_status: None,
            release_date: None,
            variant: None,
            sculptor: None,
            group_name: Some("mob".into()),
        }];
        replace_catalog(&mut conn, &files, &models, &[], &[]).unwrap();

        // before any split: one whole-folder member, all files, no key
        let members = group_members(&conn, "mob").unwrap();
        assert_eq!(members.len(), 1);
        assert!(members[0].variant_key.is_none());
        assert_eq!(model_files(&conn, "/dump/mob", None).unwrap().len(), 3);

        // a.stl -> variant sword / pose 1; b.stl -> pose 2 (no variant);
        // c.stl left unassigned
        set_file_variants(
            &mut conn,
            &["/dump/mob/a.stl".into()],
            Some("sword".into()),
            Some("1".into()),
            None,
        )
        .unwrap();
        set_file_variants(
            &mut conn,
            &["/dump/mob/b.stl".into()],
            None,
            Some("2".into()),
            None,
        )
        .unwrap();

        let members = group_members(&conn, "mob").unwrap();
        // two facet members + one residual
        assert_eq!(members.len(), 3);
        let swordy = members
            .iter()
            .find(|m| m.variant.as_deref() == Some("sword"))
            .unwrap();
        assert_eq!(swordy.name, "mob sword 1", "label shows variant then pose");
        assert_eq!(swordy.pose.as_deref(), Some("1"));
        assert_eq!(swordy.file_count, 1);
        assert_eq!(swordy.total_size_bytes, 100.0);
        assert_eq!(
            swordy.variant_key.as_deref(),
            Some("/dump/mob\u{1f}sword\u{1f}1")
        );

        let pose2 = members
            .iter()
            .find(|m| m.pose.as_deref() == Some("2"))
            .unwrap();
        assert!(pose2.variant.is_none());
        assert_eq!(pose2.variant_key.as_deref(), Some("/dump/mob\u{1f}\u{1f}2"));

        let residual = members.iter().find(|m| m.pose.is_none()).unwrap();
        assert_eq!(residual.name, "mob (unassigned)");
        assert_eq!(residual.file_count, 1);
        assert_eq!(
            residual.variant_key.as_deref(),
            Some("/dump/mob\u{1f}\u{1f}")
        );

        // files are scoped per member, keyed on (variant, pose)
        let f1 = model_files(&conn, "/dump/mob", swordy.variant_key.as_deref()).unwrap();
        assert_eq!(f1.len(), 1);
        assert_eq!(f1[0].file_name, "a.stl");
        let fr = model_files(&conn, "/dump/mob", residual.variant_key.as_deref()).unwrap();
        assert_eq!(fr.len(), 1);
        assert_eq!(fr[0].file_name, "c.stl");

        // clearing every assignment collapses back to the whole-folder member
        clear_file_variants(&conn, &["/dump/mob/a.stl".into(), "/dump/mob/b.stl".into()]).unwrap();
        assert_eq!(group_members(&conn, "mob").unwrap().len(), 1);
    }

    #[test]
    fn per_pose_previews_do_not_clobber_each_other() {
        // The bug: rendering pose A then pose B in one dump folder made every
        // member show B, because the preview was keyed by the shared dir_path.
        let mut conn = test_conn();
        let files = vec![
            file_row("/dump/mob/a.stl", "/dump/mob", 100),
            file_row("/dump/mob/b.stl", "/dump/mob", 200),
        ];
        let models = vec![ModelRow {
            dir_path: "/dump/mob".into(),
            name: "mob".into(),
            description: None,
            designer: None,
            release_name: None,
            preview_path: None,
            source: "heuristic".into(),
            uuid: None,
            file_count: 2,
            total_size_bytes: 300,
            pose: None,
            scale: None,
            support_status: None,
            release_date: None,
            variant: None,
            sculptor: None,
            group_name: Some("mob".into()),
        }];
        replace_catalog(&mut conn, &files, &models, &[], &[]).unwrap();
        set_file_variants(
            &mut conn,
            &["/dump/mob/a.stl".into()],
            None,
            Some("A".into()),
            None,
        )
        .unwrap();
        set_file_variants(
            &mut conn,
            &["/dump/mob/b.stl".into()],
            None,
            Some("B".into()),
            None,
        )
        .unwrap();

        let key_a = variant_key("/dump/mob", "", "A");
        let key_b = variant_key("/dump/mob", "", "B");

        // render pose A, then pose B — the sequence that used to clobber
        set_preview(&conn, "/dump/mob", Some(&key_a), "/previews/a.png").unwrap();
        set_preview(&conn, "/dump/mob", Some(&key_b), "/previews/b.png").unwrap();

        let members = group_members(&conn, "mob").unwrap();
        let preview_of = |members: &[CatalogEntry], pose: &str| {
            members
                .iter()
                .find(|m| m.pose.as_deref() == Some(pose))
                .unwrap()
                .preview_path
                .clone()
        };
        assert_eq!(
            preview_of(&members, "A").as_deref(),
            Some("/previews/a.png")
        );
        assert_eq!(
            preview_of(&members, "B").as_deref(),
            Some("/previews/b.png"),
            "B did not clobber A",
        );

        // re-rendering A updates only A
        set_preview(&conn, "/dump/mob", Some(&key_a), "/previews/a2.png").unwrap();
        let members = group_members(&conn, "mob").unwrap();
        assert_eq!(
            preview_of(&members, "A").as_deref(),
            Some("/previews/a2.png")
        );
        assert_eq!(
            preview_of(&members, "B").as_deref(),
            Some("/previews/b.png")
        );

        // per-variant previews survive a rescan, like the other user metadata
        replace_catalog(&mut conn, &files, &models, &[], &[]).unwrap();
        set_file_variants(
            &mut conn,
            &["/dump/mob/a.stl".into()],
            None,
            Some("A".into()),
            None,
        )
        .unwrap();
        let members = group_members(&conn, "mob").unwrap();
        assert_eq!(
            preview_of(&members, "A").as_deref(),
            Some("/previews/a2.png")
        );
    }

    #[test]
    fn user_meta_edits_survive_rescan_and_reject_unknown_models() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        update_model_user_meta(
            &conn,
            "/lib/newt",
            Some("Newt, Giant (repose)".into()),
            Some("A".into()),
            Some("32mm".into()),
            Some("supported".into()),
            None,
            Some("Dragon Trapper's Lodge".into()),
            Some("A. Sculptor".into()),
            Some("Order of the Unicorn".into()),
            Some("mounted".into()),
        )
        .unwrap();
        set_model_preview(&conn, "/lib/newt", "/appdata/previews/abc.png").unwrap();

        // the whole point of model_user_meta: a full rescan keeps user edits
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
        let page = search(&conn, "repose", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1, "custom name is searchable after rescan");
        let entry = &page.entries[0];
        assert_eq!(entry.name, "Newt, Giant (repose)");
        assert_eq!(entry.custom_name.as_deref(), Some("Newt, Giant (repose)"));
        assert_eq!(entry.pose.as_deref(), Some("A"));
        assert_eq!(entry.scale.as_deref(), Some("32mm"));
        // designer overrides the release's, sculptor is user-only
        assert_eq!(entry.designer.as_deref(), Some("Dragon Trapper's Lodge"));
        assert_eq!(entry.sculptor.as_deref(), Some("A. Sculptor"));
        assert_eq!(entry.release_name.as_deref(), Some("Order of the Unicorn"));
        assert_eq!(entry.variant.as_deref(), Some("mounted"));
        assert_eq!(
            search(&conn, "mounted", &[], 10, 0).unwrap().total,
            1,
            "variant is searchable"
        );
        // fuzzy/trigram search: possessive apostrophe is folded out, so the
        // designer matches when typed as "trappers"; and a mid-word chunk of
        // sculptor matches by substring — neither worked with prefix-only FTS
        assert_eq!(search(&conn, "trappers", &[], 10, 0).unwrap().total, 1);
        assert_eq!(search(&conn, "ulpto", &[], 10, 0).unwrap().total, 1);
        // the release name is searchable too
        assert_eq!(search(&conn, "unicorn", &[], 10, 0).unwrap().total, 1);
        // a multi-field query still ANDs: designer word + the model name
        assert_eq!(
            search(&conn, "trappers repose", &[], 10, 0).unwrap().total,
            1
        );
        assert_eq!(
            entry.preview_path.as_deref(),
            Some("/appdata/previews/abc.png")
        );

        // clearing the override reverts to the scanner name and designer
        update_model_user_meta(
            &conn,
            "/lib/newt",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let page = search(&conn, "newt", &[], 10, 0).unwrap();
        assert_eq!(page.entries[0].name, "Giant Newt");
        assert_eq!(
            page.entries[0].designer.as_deref(),
            Some("DTL"),
            "reverts to release designer"
        );
        assert!(page.entries[0].sculptor.is_none());
        assert_eq!(
            page.entries[0].release_name.as_deref(),
            Some("Critterfolk"),
            "reverts to the scanned release name"
        );
        // ...but the preview set separately is untouched by a metadata save
        assert_eq!(
            page.entries[0].preview_path.as_deref(),
            Some("/appdata/previews/abc.png")
        );

        assert!(update_model_user_meta(
            &conn, "/nope", None, None, None, None, None, None, None, None, None
        )
        .is_err());
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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
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
            variant: None,
            sculptor: None,
            group_name: Some("galeb duhr".into()),
        };
        let models = vec![
            variant("unsupported", "B"),
            variant("unsupported", "A"),
            variant("supported", "A"),
            variant("supported", "B"),
        ];
        replace_catalog(&mut conn, &[], &models, &[], &[]).unwrap();

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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
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
    fn combine_groups_merges_selected_under_one_name() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        combine_groups(
            &mut conn,
            &["Giant Newt".to_string(), "Bugbear".to_string()],
            "Dungeon Denizens",
        )
        .unwrap();

        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.groups[0].group_name, "Dungeon Denizens");
        assert_eq!(group_members(&conn, "Dungeon Denizens").unwrap().len(), 2);
        // findable by the combined name
        assert_eq!(
            search_groups(&conn, "denizens", &[], 10, 0).unwrap().total,
            1
        );

        assert!(combine_groups(&mut conn, &["Dungeon Denizens".to_string()], "  ").is_err());
        assert!(combine_groups(&mut conn, &["ghost".to_string()], "x").is_err());
    }

    #[test]
    fn a_combined_group_reports_its_sources_and_splits_apart() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        // An untouched group is its own only source — nothing to split
        assert_eq!(group_sources(&conn, "Giant Newt").unwrap(), ["Giant Newt"]);

        combine_groups(
            &mut conn,
            &["Giant Newt".to_string(), "Bugbear".to_string()],
            "Dungeon Denizens",
        )
        .unwrap();

        // The combined card knows what it was made from (case-insensitive)
        assert_eq!(
            group_sources(&conn, "dungeon denizens").unwrap(),
            ["Bugbear", "Giant Newt"]
        );

        // Splitting = clearing the renames: the sources come back as cards
        rename_group(&conn, "Dungeon Denizens", "").unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        let names: Vec<_> = page.groups.iter().map(|g| g.group_name.clone()).collect();
        assert!(names.contains(&"Giant Newt".to_string()));
        assert!(names.contains(&"Bugbear".to_string()));
        assert!(!names.contains(&"Dungeon Denizens".to_string()));
    }

    #[test]
    fn detaching_one_source_leaves_the_rest_combined() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        combine_groups(
            &mut conn,
            &["Giant Newt".to_string(), "Bugbear".to_string()],
            "Dungeon Denizens",
        )
        .unwrap();

        // Pull one back out: it's its own card again, the other stays put
        detach_group_source(&conn, "Dungeon Denizens", "Bugbear").unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        let names: Vec<_> = page.groups.iter().map(|g| g.group_name.clone()).collect();
        assert!(names.contains(&"Bugbear".to_string()));
        assert!(names.contains(&"Dungeon Denizens".to_string()));
        assert_eq!(group_members(&conn, "Dungeon Denizens").unwrap().len(), 1);

        // Detaching something that isn't rename-combined is a clear error,
        // not a silent no-op
        assert!(detach_group_source(&conn, "Dungeon Denizens", "Bugbear").is_err());
    }

    #[test]
    fn a_user_picked_cover_fronts_the_group_card() {
        let mut conn = test_conn();
        let (files, mut models, tags) = sample_rows();
        for m in &mut models {
            m.group_name = Some("critters".into());
        }
        models[0].preview_path = Some("/previews/newt.png".into());
        models[1].preview_path = Some("/previews/bugbear.png".into());
        let picked_dir = models[0].dir_path.clone();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        set_group_cover(&conn, "critters", &picked_dir, None).unwrap();
        let page = search_groups(&conn, "", &[], 10, 0).unwrap();
        assert_eq!(
            page.groups[0].preview_path.as_deref(),
            Some("/previews/newt.png"),
            "the chosen member's preview wins over the arbitrary MAX()"
        );
    }

    #[test]
    fn support_twins_match_exact_structure_and_facets_propagate() {
        let mut conn = test_conn();
        let mk = |dir: &str, group: &str, support: &str| ModelRow {
            dir_path: dir.into(),
            name: group.into(),
            description: None,
            designer: None,
            release_name: None,
            preview_path: None,
            source: "heuristic".into(),
            uuid: None,
            file_count: 1,
            total_size_bytes: 10,
            variant: None,
            pose: None,
            scale: None,
            support_status: Some(support.into()),
            release_date: None,
            sculptor: None,
            group_name: Some(group.into()),
        };
        let models = vec![
            mk("/lib/knight/Supported/A", "knight", "supported"),
            mk("/lib/knight/Unsupported/A", "knight", "unsupported"),
            mk("/lib/knight/Unsupported/B", "knight", "unsupported"),
        ];
        replace_catalog(&mut conn, &[], &models, &[], &[]).unwrap();

        // A's builds pair up; B is the same model but a different pose dir
        let twins = support_twins(&conn, "/lib/knight/Supported/A").unwrap();
        assert_eq!(twins, ["/lib/knight/Unsupported/A"]);

        // Some values propagate, None leaves the twin's own value alone
        update_model_facets(&conn, "/lib/knight/Unsupported/A", None, Some("A"), None).unwrap();
        update_model_facets(
            &conn,
            "/lib/knight/Unsupported/A",
            Some("spear"),
            None,
            None,
        )
        .unwrap();
        let (variant, pose): (Option<String>, Option<String>) = conn
            .query_row(
                "SELECT variant, pose FROM model_user_meta WHERE dir_path = ?1",
                ["/lib/knight/Unsupported/A"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(variant.as_deref(), Some("spear"));
        assert_eq!(pose.as_deref(), Some("A"), "None must not clear the pose");

        // Group tags hit every member in one call
        add_group_tag(&conn, "knight", "Cavalry").unwrap();
        let tagged: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM model_tags WHERE tag = 'cavalry'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tagged, 3, "normalized tag lands on all three members");
        remove_group_tag(&conn, "knight", "cavalry").unwrap();
        let left: u32 = conn
            .query_row("SELECT COUNT(*) FROM model_tags", [], |row| row.get(0))
            .unwrap();
        assert_eq!(left, 0);
    }

    #[test]
    fn user_meta_follows_a_model_move() {
        let mut conn = test_conn();
        let (files, models, tags) = sample_rows();
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();
        update_model_user_meta(
            &conn,
            "/lib/newt",
            Some("Shiny Newt".into()),
            None,
            None,
            None,
            None,
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
        replace_catalog(&mut conn, &files, &models, &tags, &[]).unwrap();

        let releases = list_releases(&conn).unwrap();
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].release_name, "Critterfolk");
        assert_eq!(releases[0].designer.as_deref(), Some("DTL"));
        assert_eq!(releases[0].model_count, 1);
    }
}
