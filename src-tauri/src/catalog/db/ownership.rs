use crate::error::AppError;
use rusqlite::Connection;

/// Install the ownership lookup index for a catalog connection. Kept out of
/// schema.rs so this feature can compose cleanly with newer schema migrations;
/// `db::open` calls it once per production connection.
pub(super) fn ensure_content_hash_index(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_files_content_hash ON files(content_hash)\n         WHERE content_hash IS NOT NULL;",
    )
    .map_err(|e| AppError::ConfigError(format!("Failed to create ownership index: {}", e)))?;
    Ok(())
}

/// Every place bare-hex blake3 `hash` is currently indexed in the library.
/// Loose rows come first because they can be copied directly; packed rows
/// require an ephemeral extract. Returning all candidates matters because
/// the catalog is an index, not an authority: one row can be stale while a
/// second row for the same bytes is still perfectly usable.
pub fn find_owners(
    conn: &Connection,
    hash: &str,
) -> Result<Vec<(String, Option<String>)>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Ownership lookup failed: {}", e));
    let mut stmt = conn
        .prepare(
            "SELECT path, archive_path FROM files WHERE content_hash = ?1\n             ORDER BY archive_path IS NOT NULL, path COLLATE NOCASE",
        )
        .map_err(map_err)?;
    stmt.query_map([hash], |row| Ok((row.get(0)?, row.get(1)?)))
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)
}

/// Cheap "do we own these bytes" primitive. Materialization must use
/// `find_owners` so it can recover when the first indexed candidate is stale.
pub fn find_owner(
    conn: &Connection,
    hash: &str,
) -> Result<Option<(String, Option<String>)>, AppError> {
    Ok(find_owners(conn, hash)?.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::db::test_util::{file_row, test_conn};
    use crate::catalog::db::replace_catalog;

    #[test]
    fn owner_lookup_prefers_loose_and_keeps_fallback_candidates() {
        let mut conn = test_conn();
        assert!(find_owner(&conn, "deadbeef").unwrap().is_none());
        assert!(find_owners(&conn, "deadbeef").unwrap().is_empty());

        let mut packed = file_row("/lib/newt/body.stl", "/lib/newt", 2048);
        packed.archive_path = Some("/lib/newt/model.plinthpack".into());
        packed.content_hash = Some("deadbeef".into());
        replace_catalog(&mut conn, "/lib/newt", &[packed], &[], &[], &[], &[]).unwrap();

        let mut loose = file_row("/lib/freebies/renamed_body.stl", "/lib/freebies", 2048);
        loose.content_hash = Some("deadbeef".into());
        replace_catalog(&mut conn, "/lib/freebies", &[loose], &[], &[], &[], &[]).unwrap();

        let owners = find_owners(&conn, "deadbeef").unwrap();
        assert_eq!(owners.len(), 2);
        assert_eq!(owners[0].0, "/lib/freebies/renamed_body.stl");
        assert!(owners[0].1.is_none());
        assert_eq!(owners[1].0, "/lib/newt/body.stl");
        assert_eq!(owners[1].1.as_deref(), Some("/lib/newt/model.plinthpack"));
    }
}
