//! The on-disk normalizer: makes the DISK match the curated catalog by
//! moving folders into the canonical layout (see layout.rs) and writing an
//! authoritative model.json into every leaf it touches.
//!
//! Shape of the operation — three explicit stages, so the user sees and
//! approves every move before anything happens:
//!
//! 1. `plan` — read-only. Computes the full move list per model group.
//! 2. `apply_ops` — executes approved moves; every rename immediately
//!    re-keys the catalog index so user curation (tags, overrides, pose
//!    assignments) never orphans.
//! 3. `finalize` — writes model.json per leaf dir (this is what makes a
//!    rescan re-derive the identical catalog with ZERO folder heuristics),
//!    deletes stale sidecars, sweeps empty dirs.
//!
//! Moves are plain fs::rename — on the same volume that's a metadata op
//! that preserves hardlinks (the dedup merge invariant on the NAS). A
//! cross-volume rename fails loudly and is reported, never silently
//! degraded to copy+delete.

use super::{
    dups, layout, BatchOutcome, NormalizeGroupPlan, NormalizeOp, NormalizePlan, NormalizeSkip,
};
use crate::error::AppError;
use rusqlite::Connection;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf, MAIN_SEPARATOR};

/// One catalog member with every facet resolved user-override-first —
/// the same COALESCE(u.x, m.x) rule the rest of the catalog reads by.
struct MemberRow {
    dir: String,
    gname: String,
    designer: Option<String>,
    release: Option<String>,
    date: Option<String>,
    support: Option<String>,
    variant: Option<String>,
    pose: Option<String>,
    description: Option<String>,
    uuid: Option<String>,
    scale: Option<String>,
    sculptor: Option<String>,
}

fn member_rows(conn: &Connection, group: Option<&str>) -> Result<Vec<MemberRow>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Normalize query failed: {}", e));
    let base = "SELECT m.dir_path,
                COALESCE(r.display_name, m.group_name, m.name),
                COALESCE(u.designer, m.designer),
                COALESCE(u.release_name, m.release_name),
                COALESCE(u.release_date, m.release_date),
                COALESCE(u.support_status, m.support_status),
                COALESCE(u.variant, m.variant),
                COALESCE(u.pose, m.pose),
                m.description, m.uuid,
                COALESCE(u.scale, m.scale),
                COALESCE(u.sculptor, m.sculptor)
         FROM models m
         LEFT JOIN model_user_meta u ON u.dir_path = m.dir_path
         LEFT JOIN group_renames r ON r.source_group = COALESCE(m.group_name, m.name)";
    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<MemberRow> {
        Ok(MemberRow {
            dir: row.get(0)?,
            gname: row.get(1)?,
            designer: row.get(2)?,
            release: row.get(3)?,
            date: row.get(4)?,
            support: row.get(5)?,
            variant: row.get(6)?,
            pose: row.get(7)?,
            description: row.get(8)?,
            uuid: row.get(9)?,
            scale: row.get(10)?,
            sculptor: row.get(11)?,
        })
    }
    let rows = match group {
        Some(g) => {
            let sql = format!(
                "{} WHERE lower(COALESCE(r.display_name, m.group_name, m.name)) = lower(?1)
                 ORDER BY m.dir_path",
                base
            );
            let mut stmt = conn.prepare(&sql).map_err(map_err)?;
            let rows = stmt
                .query_map([g], map_row)
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?;
            rows
        }
        None => {
            let sql = format!("{} ORDER BY m.dir_path", base);
            let mut stmt = conn.prepare(&sql).map_err(map_err)?;
            let rows = stmt
                .query_map([], map_row)
                .map_err(map_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_err)?;
            rows
        }
    };
    Ok(rows)
}

/// `child` is `base` itself or lies beneath it (path-segment aware —
/// "/lib/ab" is NOT under "/lib/a").
fn is_under(child: &str, base: &str) -> bool {
    child == base
        || (child.len() > base.len()
            && child.starts_with(base)
            && child[base.len()..].starts_with(MAIN_SEPARATOR))
}

/// Deepest directory containing every member dir.
fn common_ancestor(dirs: &[&str]) -> Option<PathBuf> {
    let mut iter = dirs.iter();
    let mut acc: Vec<std::path::Component> = Path::new(iter.next()?).components().collect();
    for dir in iter {
        let other: Vec<_> = Path::new(dir).components().collect();
        let shared = acc
            .iter()
            .zip(other.iter())
            .take_while(|(a, b)| a == b)
            .count();
        acc.truncate(shared);
    }
    if acc.is_empty() {
        None
    } else {
        Some(acc.iter().collect())
    }
}

fn first_some(rows: &[&MemberRow], get: fn(&MemberRow) -> Option<&String>) -> Option<String> {
    rows.iter().find_map(|r| get(r).cloned())
}

struct FileRowLite {
    path: String,
    file_name: String,
}

fn files_in_dir(conn: &Connection, dir: &str) -> Result<Vec<FileRowLite>, AppError> {
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("Normalize file query failed: {}", e));
    let mut stmt = conn
        .prepare("SELECT path, file_name FROM files WHERE dir_path = ?1 ORDER BY file_name")
        .map_err(map_err)?;
    let rows = stmt
        .query_map([dir], |row| {
            Ok(FileRowLite {
                path: row.get(0)?,
                file_name: row.get(1)?,
            })
        })
        .map_err(map_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(map_err)?;
    Ok(rows)
}

/// The same bytes in two places? Hardlinked names are trivially identical
/// (one inode); otherwise size then a full BLAKE3 compare settles it. Only
/// ever called for name CLASHES, so the hashing cost stays negligible.
fn same_content(a: &Path, b: &Path) -> bool {
    if let (Some(ia), Some(ib)) = (dups::file_identity(a), dups::file_identity(b)) {
        if ia == ib {
            return true;
        }
    }
    let (Ok(ma), Ok(mb)) = (a.metadata(), b.metadata()) else {
        return false;
    };
    ma.len() == mb.len()
        && matches!(
            (dups::hash_file(a, None), dups::hash_file(b, None)),
            (Ok(ha), Ok(hb)) if ha == hb
        )
}

/// "oval.stl" -> "oval 2.stl", "oval 3.stl"… first free number wins.
fn numbered_name(name: &str, taken: &HashMap<String, String>) -> String {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    for n in 2.. {
        let candidate = match ext {
            Some(ext) => format!("{} {}.{}", stem, n, ext),
            None => format!("{} {}", stem, n),
        };
        if !taken.contains_key(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!("ran out of integers before file names")
}

/// Plan one file's landing spot in a merge bucket. Clash policy: a file
/// that is byte-identical to the one already claiming the name becomes a
/// reviewable "drop" (the copy is redundant once one lands — the unicorn-
/// bases case, where every part folder repeats the same base STLs); files
/// that merely SHARE a name get a numbered one instead of being skipped.
#[allow(clippy::too_many_arguments)]
fn place_file(
    current: String,
    file: &FileRowLite,
    pose: Option<&str>,
    leaf: &str,
    used_names: &mut HashMap<String, String>,
    ops: &mut Vec<NormalizeOp>,
    notes: &mut Vec<String>,
) {
    let desired = layout::pose_suffixed_name(&file.file_name, pose.unwrap_or(""));
    let mut target_name = desired.clone();
    if let Some(kept_original) = used_names.get(&desired.to_lowercase()) {
        if same_content(Path::new(kept_original), Path::new(&file.path)) {
            ops.push(NormalizeOp {
                from: current,
                to: format!("{}{}{}", leaf, MAIN_SEPARATOR, desired),
                kind: "drop".into(),
                pose: None,
            });
            return;
        }
        target_name = numbered_name(&desired, used_names);
        notes.push(format!(
            "{} exists twice with different contents — one becomes {}",
            desired, target_name
        ));
    }
    used_names.insert(target_name.to_lowercase(), file.path.clone());
    let to = format!("{}{}{}", leaf, MAIN_SEPARATOR, target_name);
    if current != to {
        ops.push(NormalizeOp {
            from: current,
            to,
            kind: "file".into(),
            pose: pose.map(String::from),
        });
    } else if pose.is_some() {
        // name already carries the pose; still record it as metadata
        ops.push(NormalizeOp {
            from: current,
            to,
            kind: "pose".into(),
            pose: pose.map(String::from),
        });
    }
}

pub fn plan(
    conn: &Connection,
    root: &Path,
    designer_filter: Option<&str>,
    group_filter: Option<&str>,
) -> Result<NormalizePlan, AppError> {
    let root_str = root.to_string_lossy().into_owned();
    let rows = member_rows(conn, None)?;
    let all_dirs: HashSet<&str> = rows.iter().map(|r| r.dir.as_str()).collect();

    // group by display name, case-insensitive, preserving first spelling
    let mut groups: BTreeMap<String, Vec<&MemberRow>> = BTreeMap::new();
    for row in &rows {
        groups.entry(row.gname.to_lowercase()).or_default().push(row);
    }

    let mut out: Vec<NormalizeGroupPlan> = Vec::new();
    let mut skipped: Vec<NormalizeSkip> = Vec::new();

    for members in groups.values() {
        let display = members[0].gname.clone();
        // scope to one model when the drawer's per-model cleanup asked for it
        if let Some(filter) = group_filter {
            if !display.eq_ignore_ascii_case(filter.trim()) {
                continue;
            }
        }
        let designer = first_some(members, |r| r.designer.as_ref());
        if let Some(filter) = designer_filter {
            let matches = designer
                .as_deref()
                .is_some_and(|d| d.eq_ignore_ascii_case(filter.trim()));
            if !matches {
                continue;
            }
        }
        let Some(designer) = designer else {
            skipped.push(NormalizeSkip {
                group_name: display,
                reason: "no designer set — give the model a designer first".into(),
            });
            continue;
        };
        let release = first_some(members, |r| r.release.as_ref());
        let date = first_some(members, |r| r.date.as_ref());
        let model_dir = layout::model_dir(
            root,
            &designer,
            release.as_deref(),
            date.as_deref(),
            &display,
        );
        let model_dir_str = model_dir.to_string_lossy().into_owned();

        let group_dirs: HashSet<&str> = members.iter().map(|r| r.dir.as_str()).collect();
        let dirs: Vec<&str> = members.iter().map(|r| r.dir.as_str()).collect();

        // members must live inside the catalog root to be movable at all
        if let Some(outside) = dirs.iter().find(|d| !is_under(d, &root_str)) {
            skipped.push(NormalizeSkip {
                group_name: display,
                reason: format!("{} is outside the catalog root", outside),
            });
            continue;
        }

        let mut ops: Vec<NormalizeOp> = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        let mut old_dirs: Vec<String> = Vec::new();

        // ---- phase 1: relocate the group's base wholesale when possible.
        // One rename moves everything (renders, readmes, nested folders)
        // and preserves hardlinks; per-member moves are the fallback when
        // the base is shared with other models (e.g. a release folder).
        let base = common_ancestor(&dirs).map(|b| b.to_string_lossy().into_owned());
        let wholesale = base.as_deref().filter(|b| {
            *b != root_str
                && is_under(b, &root_str)
                && !is_under(b, &model_dir_str)      // already at/inside the target
                && !is_under(&model_dir_str, b)      // can't rename a dir into itself
                && all_dirs
                    .iter()
                    .filter(|d| is_under(d, b))
                    .all(|d| group_dirs.contains(d)) // no foreign models beneath
        });

        // where each member dir sits AFTER phase 1
        let cur: Vec<String> = match wholesale {
            Some(b) => {
                ops.push(NormalizeOp {
                    from: b.to_string(),
                    to: model_dir_str.clone(),
                    kind: "dir".into(),
                    pose: None,
                });
                old_dirs.push(b.to_string());
                dirs.iter()
                    .map(|d| format!("{}{}", model_dir_str, &d[b.len()..]))
                    .collect()
            }
            None => {
                // per-member mode: a member entangled with foreign model
                // dirs can't be moved safely — skip the whole group
                if let Some(tangled) = dirs.iter().find(|d| {
                    all_dirs
                        .iter()
                        .any(|x| *x != **d && is_under(x, d) && !group_dirs.contains(x))
                }) {
                    skipped.push(NormalizeSkip {
                        group_name: display,
                        reason: format!("other models' folders are nested inside {}", tangled),
                    });
                    continue;
                }
                old_dirs.extend(dirs.iter().map(|d| d.to_string()));
                dirs.iter().map(|d| d.to_string()).collect()
            }
        };

        // ---- phase 2: reshape into Supported/Unsupported[/variant] leaves
        let desired: Vec<String> = members
            .iter()
            .map(|m| {
                layout::member_dir(&model_dir, m.support.as_deref(), m.variant.as_deref())
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();

        // bucket member indexes by their desired leaf
        let mut buckets: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, d) in desired.iter().enumerate() {
            buckets.entry(d.as_str()).or_default().push(i);
        }

        for (leaf, idxs) in &buckets {
            let merging = idxs.len() > 1;
            // pick the dir-rename anchor: a member already at the leaf, or
            // the first one that can legally rename there. A member whose
            // dir contains another member (nested) or contains the leaf
            // itself must move per-file instead.
            let mut anchored = false;
            // target name (lowercased) -> ORIGINAL path of the file that
            // claimed it, so clashes can be settled by comparing contents
            let mut used_names: HashMap<String, String> = HashMap::new();
            for &i in idxs.iter() {
                let member = members[i];
                let from_dir = &cur[i];
                let is_nested_parent = cur
                    .iter()
                    .enumerate()
                    .any(|(j, other)| j != i && is_under(other, from_dir) && other != from_dir);
                let can_rename = !anchored
                    && !is_nested_parent
                    && !is_under(leaf, from_dir)
                    && !cur.iter().enumerate().any(|(j, other)| {
                        j != i && other == *leaf && desired[j] != **leaf
                    });

                if from_dir == *leaf || can_rename {
                    if from_dir != *leaf {
                        ops.push(NormalizeOp {
                            from: from_dir.clone(),
                            to: (*leaf).to_string(),
                            kind: "dir".into(),
                            pose: None,
                        });
                    }
                    anchored = true;
                    // when poses merge into one build folder, even the
                    // anchor's files gain their pose suffix so the whole
                    // set stays distinguishable side by side
                    if merging {
                        for f in files_in_dir(conn, &member.dir)? {
                            if f.file_name == "model.json" || f.file_name == "release.json" {
                                continue;
                            }
                            // after the anchor rename the file sits in the leaf
                            let current = format!("{}{}{}", leaf, MAIN_SEPARATOR, f.file_name);
                            place_file(
                                current,
                                &f,
                                member.pose.as_deref(),
                                leaf,
                                &mut used_names,
                                &mut ops,
                                &mut notes,
                            );
                        }
                    }
                    continue;
                }

                // per-file move (merge into the leaf, pose baked into names)
                for f in files_in_dir(conn, &member.dir)? {
                    if f.file_name == "model.json" || f.file_name == "release.json" {
                        continue; // stale sidecars die with their dir
                    }
                    // translate the indexed path through the phase-1 move
                    let current = format!("{}{}", cur[i], &f.path[member.dir.len()..]);
                    place_file(
                        current,
                        &f,
                        member.pose.as_deref(),
                        leaf,
                        &mut used_names,
                        &mut ops,
                        &mut notes,
                    );
                }
                // nested folders under a per-file-merged member stay put
                let nested_prefix = format!("{}{}", member.dir, MAIN_SEPARATOR);
                if all_dirs.iter().any(|d| d.starts_with(&nested_prefix)) {
                    notes.push(format!(
                        "nested folders under {} were left in place",
                        member.dir
                    ));
                }
                if !old_dirs.contains(&cur[i]) {
                    old_dirs.push(cur[i].clone());
                }
            }
        }

        let clean = ops.is_empty();
        out.push(NormalizeGroupPlan {
            group_name: display,
            designer,
            target_dir: model_dir_str,
            ops,
            old_dirs,
            notes,
            clean,
        });
    }

    let total_ops = out.iter().map(|g| g.ops.len() as u32).sum();
    Ok(NormalizePlan {
        clean_groups: out.iter().filter(|g| g.clean).count() as u32,
        groups: out.into_iter().filter(|g| !g.clean).collect(),
        skipped,
        total_ops,
    })
}

/// Execute approved moves in order. Disk and index move together per op:
/// a rename that succeeds on disk but fails to re-key is reported, and a
/// rescan repairs the rows (user tables survive re-keyed or orphaned, never
/// silently wrong).
pub fn apply_ops(conn: &mut Connection, ops: &[NormalizeOp]) -> Result<BatchOutcome, AppError> {
    let mut succeeded = 0u32;
    let mut errors: Vec<String> = Vec::new();

    for op in ops {
        // "pose" ops only record metadata — no filesystem side
        if op.kind == "pose" {
            if let Err(e) = record_pose(conn, &op.to, op.pose.as_deref()) {
                errors.push(format!("Failed to record pose for {}: {}", op.to, e));
            } else {
                succeeded += 1;
            }
            continue;
        }
        // "drop": op.from is a redundant copy of op.to. The plan proved
        // them identical — verify AGAIN before deleting (same paranoia as
        // the dup merge: anything can change between plan and apply).
        if op.kind == "drop" {
            let from = Path::new(&op.from);
            let to = Path::new(&op.to);
            if !to.is_file() {
                errors.push(format!(
                    "Kept copy missing, duplicate left in place: {}",
                    op.from
                ));
                continue;
            }
            if !from.is_file() || !same_content(from, to) {
                errors.push(format!(
                    "No longer identical to the kept copy, left in place: {}",
                    op.from
                ));
                continue;
            }
            if let Err(e) = std::fs::remove_file(from) {
                errors.push(format!("Failed to remove duplicate {}: {}", op.from, e));
                continue;
            }
            match super::db::remove_files(conn, &[op.from.clone()]) {
                Ok(()) => succeeded += 1,
                Err(e) => errors.push(format!(
                    "Removed duplicate {} but failed to update the catalog (rescan to fix): {}",
                    op.from, e
                )),
            }
            continue;
        }
        let from = Path::new(&op.from);
        let to = Path::new(&op.to);
        if !from.exists() {
            errors.push(format!("Source not found: {}", op.from));
            continue;
        }
        // A case-only rename ("unsupported" -> "Unsupported") reports the
        // destination as existing on case-insensitive filesystems (macOS,
        // Windows) even though it's the SAME entry — rename handles it fine
        let case_only = op.from.eq_ignore_ascii_case(&op.to) && op.from != op.to;
        if to.exists() && !case_only {
            errors.push(format!("Destination already exists: {}", op.to));
            continue;
        }
        if let Some(parent) = to.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                errors.push(format!("Failed to create {}: {}", parent.display(), e));
                continue;
            }
        }
        if let Err(e) = std::fs::rename(from, to) {
            // EXDEV lands here too: cross-volume moves are refused loudly,
            // because a copy+delete would silently split shared hardlinks
            errors.push(format!("Failed to move {} to {}: {}", op.from, op.to, e));
            continue;
        }
        let index_result = if op.kind == "dir" {
            super::db::move_tree_index(conn, &op.from, &op.to)
        } else {
            super::db::move_file_index(conn, &op.from, &op.to).and_then(|()| {
                record_pose(conn, &op.to, op.pose.as_deref()).map_err(|e| {
                    AppError::ConfigError(format!("pose record failed: {}", e))
                })
            })
        };
        match index_result {
            Ok(()) => succeeded += 1,
            Err(e) => errors.push(format!(
                "Moved {} on disk but failed to update the catalog (rescan to fix): {}",
                op.to, e
            )),
        }
    }
    Ok(BatchOutcome { succeeded, errors })
}

/// Remember a file's pose as metadata at its (new) path.
fn record_pose(conn: &Connection, path: &str, pose: Option<&str>) -> Result<(), rusqlite::Error> {
    let Some(pose) = pose.filter(|p| !p.trim().is_empty()) else {
        return Ok(());
    };
    let dir = Path::new(path)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    conn.execute(
        "INSERT INTO file_variants (path, dir_path, pose)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(path) DO UPDATE SET dir_path = excluded.dir_path,
                                         pose = excluded.pose",
        rusqlite::params![path, dir, pose],
    )?;
    Ok(())
}

/// Post-move bookkeeping for the given groups: write the authoritative
/// model.json per leaf, drop sidecars that traveled along stale, clear
/// now-redundant user-meta poses, sweep emptied source dirs, and rebuild
/// the search index. Returns human-readable warnings.
pub fn finalize(
    conn: &Connection,
    root: &Path,
    group_names: &[String],
    old_dirs: &[String],
) -> Result<Vec<String>, AppError> {
    let mut warnings: Vec<String> = Vec::new();

    for group in group_names {
        let members = member_rows(conn, Some(group))?;
        if members.is_empty() {
            warnings.push(format!("{}: no members found after the move", group));
            continue;
        }
        let refs: Vec<&MemberRow> = members.iter().collect();
        let designer = first_some(&refs, |r| r.designer.as_ref()).unwrap_or_default();
        let release = first_some(&refs, |r| r.release.as_ref());
        let date = first_some(&refs, |r| r.date.as_ref());
        let display = members[0].gname.clone();
        let model_dir = layout::model_dir(
            root,
            &designer,
            release.as_deref(),
            date.as_deref(),
            &display,
        );
        let model_dir_str = model_dir.to_string_lossy().into_owned();

        // images at the model root are referenced relatively from each leaf
        let root_images: Vec<String> = files_in_dir(conn, &model_dir_str)?
            .into_iter()
            .filter(|f| {
                Path::new(&f.file_name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| {
                        super::IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str())
                    })
            })
            .map(|f| f.file_name)
            .collect();

        for member in &members {
            if let Err(e) = write_member_json(conn, member, &model_dir_str, &root_images) {
                warnings.push(format!("{}: {}", member.dir, e));
            }
        }
        // a release.json that traveled inside the moved base would claim
        // the whole model subtree with stale values on the next scan
        let stale_release = model_dir.join("release.json");
        if stale_release.is_file() {
            std::fs::remove_file(&stale_release).ok();
        }
    }

    // sweep: source dirs that only hold our own sidecars/OS litter go away,
    // then empty parents up to (never including) the root
    for dir in old_dirs {
        sweep_upward(Path::new(dir), root);
    }

    super::db::rebuild_search_index(conn)?;
    Ok(warnings)
}

/// Write one leaf's model.json from catalog state. File-level poses beat a
/// dir-level pose: when file_poses exist the dir pose is omitted (and its
/// user override cleared) so the two mechanisms can't fight after a rescan.
fn write_member_json(
    conn: &Connection,
    member: &MemberRow,
    model_dir: &str,
    root_images: &[String],
) -> Result<(), AppError> {
    let leaf = Path::new(&member.dir);
    if !leaf.is_dir() {
        return Err(AppError::NotFoundError(format!(
            "leaf dir missing: {}",
            member.dir
        )));
    }

    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("model.json query failed: {}", e));
    let mut tag_stmt = conn
        .prepare("SELECT tag FROM model_tags WHERE dir_path = ?1 ORDER BY tag")
        .map_err(map_err)?;
    let tags: Vec<String> = tag_stmt
        .query_map([&member.dir], |row| row.get(0))
        .and_then(|rows| rows.collect())
        .map_err(map_err)?;

    let mut fp_stmt = conn
        .prepare(
            "SELECT path, variant, pose, support_status FROM file_variants
             WHERE dir_path = ?1 AND COALESCE(pose, '') != '' ORDER BY path",
        )
        .map_err(map_err)?;
    let file_poses: Vec<serde_json::Value> = fp_stmt
        .query_map([&member.dir], |row| {
            let path: String = row.get(0)?;
            let variant: Option<String> = row.get(1)?;
            let pose: Option<String> = row.get(2)?;
            let support: Option<String> = row.get(3)?;
            Ok((path, variant, pose, support))
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(map_err)?
        .into_iter()
        .map(|(path, variant, pose, support)| {
            let name = path
                .rsplit(MAIN_SEPARATOR)
                .next()
                .unwrap_or(&path)
                .to_string();
            serde_json::json!({
                "name": name,
                "variant": variant,
                "pose": pose,
                "support_status": support,
            })
        })
        .collect();

    // image references: own images first, else the model-root ones a level up
    let own_images: Vec<String> = files_in_dir(conn, &member.dir)?
        .into_iter()
        .filter(|f| {
            Path::new(&f.file_name)
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| super::IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        })
        .map(|f| f.file_name)
        .collect();
    let images: Vec<String> = if !own_images.is_empty() {
        own_images
    } else if is_under(&member.dir, model_dir) && member.dir != model_dir {
        let depth = member.dir[model_dir.len()..]
            .matches(MAIN_SEPARATOR)
            .count();
        root_images
            .iter()
            .map(|img| format!("{}{}", "../".repeat(depth), img))
            .collect()
    } else {
        vec![]
    };

    let dir_pose = if file_poses.is_empty() {
        member.pose.clone()
    } else {
        // poses live on files now — a lingering dir-level user pose would
        // resurrect through COALESCE on the next read
        conn.execute(
            "UPDATE model_user_meta SET pose = NULL WHERE dir_path = ?1",
            [&member.dir],
        )
        .map_err(map_err)?;
        None
    };

    let json = serde_json::json!({
        "id": member.uuid,
        "name": member.gname,
        "description": member.description,
        "tags": tags,
        "images": images,
        "variant": member.variant,
        "pose": dir_pose,
        "scale": member.scale,
        "support_status": member.support,
        "release_date": member.date,
        "designer": member.designer,
        "sculptor": member.sculptor,
        "release_name": member.release,
        "file_poses": file_poses,
    });
    let pretty = serde_json::to_string_pretty(&json)
        .map_err(|e| AppError::ConfigError(format!("model.json encode failed: {}", e)))?;
    std::fs::write(leaf.join("model.json"), pretty)
        .map_err(|e| AppError::IoError(format!("model.json write failed: {}", e)))?;
    Ok(())
}

/// Remove `dir` if it holds nothing but our sidecars / OS litter, then walk
/// toward the root removing newly-empty parents. Stops at the first dir
/// with real content — user files are never deleted.
fn sweep_upward(dir: &Path, root: &Path) {
    let mut current = Some(dir.to_path_buf());
    while let Some(d) = current {
        if d == root || !d.starts_with(root) {
            break;
        }
        // a dir that was itself moved away no longer exists — its parents
        // may still be empty shells worth sweeping
        if !d.is_dir() {
            current = d.parent().map(Path::to_path_buf);
            continue;
        }
        let removable = ["model.json", "release.json", ".DS_Store", "Thumbs.db"];
        let Ok(entries) = std::fs::read_dir(&d) else {
            break;
        };
        let mut leftovers: Vec<PathBuf> = Vec::new();
        let mut only_litter = true;
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if entry.path().is_file() && removable.contains(&name.as_str()) {
                leftovers.push(entry.path());
            } else {
                only_litter = false;
            }
        }
        if !only_litter {
            break;
        }
        for f in leftovers {
            std::fs::remove_file(f).ok();
        }
        if std::fs::remove_dir(&d).is_err() {
            break;
        }
        current = d.parent().map(Path::to_path_buf);
    }
}

#[cfg(test)]
mod tests {
    use super::super::db;
    use super::*;
    use crate::catalog::{FileRow, ModelRow};
    use std::fs;

    fn file_row(path: &std::path::Path, dir: &std::path::Path) -> FileRow {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        FileRow {
            path: path.to_string_lossy().into_owned(),
            dir_path: dir.to_string_lossy().into_owned(),
            file_name: name,
            extension: ext,
            size_bytes: 4,
            modified_at: 0,
        }
    }

    fn model_row(dir: &std::path::Path, name: &str, group: &str) -> ModelRow {
        ModelRow {
            dir_path: dir.to_string_lossy().into_owned(),
            name: name.into(),
            description: None,
            designer: Some("Bestiarum".into()),
            release_name: Some("Dread Swamp".into()),
            preview_path: None,
            source: "heuristic".into(),
            uuid: None,
            file_count: 1,
            total_size_bytes: 4,
            pose: None,
            scale: None,
            support_status: None,
            release_date: Some("7/2026".into()),
            variant: None,
            sculptor: None,
            group_name: Some(group.into()),
        }
    }

    fn touch(path: &std::path::Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"x").unwrap();
    }

    #[test]
    fn wholesale_move_reshapes_and_round_trips() {
        let root = std::env::temp_dir().join(format!("plinth_norm_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let old = root.join("bestiarum-07-2026/Dread Swamp/Bog Hag");
        let sup = old.join("supported stl");
        let unsup = old.join("unsupported");
        touch(&sup.join("bog.lys"));
        touch(&unsup.join("bog.stl"));
        touch(&old.join("render.png"));

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut sup_row = model_row(&sup, "bog hag supported", "Bog Hag");
        sup_row.support_status = Some("supported".into());
        let mut unsup_row = model_row(&unsup, "bog hag", "Bog Hag");
        unsup_row.support_status = Some("unsupported".into());
        let files = vec![
            file_row(&sup.join("bog.lys"), &sup),
            file_row(&unsup.join("bog.stl"), &unsup),
            file_row(&old.join("render.png"), &old),
        ];
        db::replace_catalog(&mut conn, &files, &[sup_row, unsup_row], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        assert_eq!(plan.groups.len(), 1);
        let group = &plan.groups[0];
        let target = root.join("Bestiarum/2026-07 Dread Swamp/Bog Hag");
        assert_eq!(group.target_dir, target.to_string_lossy());
        // one wholesale move + two build-folder renames
        assert_eq!(group.ops.len(), 3, "ops: {:?}", group.ops);
        assert_eq!(group.ops[0].kind, "dir");

        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(target.join("Supported/bog.lys").is_file());
        assert!(target.join("Unsupported/bog.stl").is_file());
        assert!(target.join("render.png").is_file());
        assert!(!old.exists());

        // the index moved with the disk
        let count: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE path LIKE ?1 || '%'",
                [target.to_string_lossy()],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 3);

        let warnings = finalize(
            &conn,
            &root,
            &["Bog Hag".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);
        // authoritative sidecars in every leaf, with the root render linked
        let meta: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join("Supported/model.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["name"], "Bog Hag");
        assert_eq!(meta["designer"], "Bestiarum");
        assert_eq!(meta["support_status"], "supported");
        assert_eq!(meta["images"][0], "../render.png");
        // the emptied source chain is gone
        assert!(!root.join("bestiarum-07-2026").exists());

        // idempotence: a second plan finds nothing to do
        let again = super::plan(&conn, &root, None, None).unwrap();
        assert_eq!(again.groups.len(), 0, "{:?}", again.groups);
        assert_eq!(again.clean_groups, 1);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn pose_dirs_merge_with_filename_suffixes() {
        let root = std::env::temp_dir().join(format!("plinth_norm_pose_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let old = root.join("galeb duhr/supported");
        touch(&old.join("A/galeb duhr.stl"));
        touch(&old.join("B/galeb duhr.stl"));

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut row_a = model_row(&old.join("A"), "galeb duhr A", "galeb duhr");
        row_a.support_status = Some("supported".into());
        row_a.pose = Some("A".into());
        let mut row_b = model_row(&old.join("B"), "galeb duhr B", "galeb duhr");
        row_b.support_status = Some("supported".into());
        row_b.pose = Some("B".into());
        let files = vec![
            file_row(&old.join("A/galeb duhr.stl"), &old.join("A")),
            file_row(&old.join("B/galeb duhr.stl"), &old.join("B")),
        ];
        db::replace_catalog(&mut conn, &files, &[row_a, row_b], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        let group = &plan.groups[0];
        let target = root.join("Bestiarum/2026-07 Dread Swamp/galeb duhr");

        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        // both poses side by side in ONE build folder, names disambiguated
        assert!(target.join("Supported/galeb duhr A.stl").is_file());
        assert!(target.join("Supported/galeb duhr B.stl").is_file());

        // pose survived as file-level metadata
        let poses: Vec<(String, String)> = {
            let mut stmt = conn
                .prepare("SELECT path, pose FROM file_variants ORDER BY path")
                .unwrap();
            let rows = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            rows
        };
        assert_eq!(poses.len(), 2);
        assert!(poses.iter().any(|(p, pose)| p.ends_with("A.stl") && pose == "A"));
        assert!(poses.iter().any(|(p, pose)| p.ends_with("B.stl") && pose == "B"));

        let warnings = finalize(
            &conn,
            &root,
            &["galeb duhr".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);
        let meta: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join("Supported/model.json")).unwrap(),
        )
        .unwrap();
        // file poses beat a dir pose — the dir level stays null
        assert!(meta["pose"].is_null());
        assert_eq!(meta["file_poses"].as_array().unwrap().len(), 2);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn identical_files_collapse_and_different_ones_get_numbered() {
        // the unicorn-bases shape: every part folder repeats the same base
        // STLs under an identically-named dir, no poses to disambiguate
        let root = std::env::temp_dir().join(format!("plinth_norm_dup_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let pt1 = root.join("unicorn/pt1/Bases");
        let pt2 = root.join("unicorn/pt2/Bases");
        fs::create_dir_all(&pt1).unwrap();
        fs::create_dir_all(&pt2).unwrap();
        fs::write(pt1.join("oval.stl"), b"same bytes").unwrap();
        fs::write(pt2.join("oval.stl"), b"same bytes").unwrap();
        fs::write(pt1.join("square.stl"), b"contents a").unwrap();
        fs::write(pt2.join("square.stl"), b"contents b").unwrap();

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut row_1 = model_row(&pt1, "Bases", "Unicorn Bases");
        row_1.support_status = Some("supported".into());
        let mut row_2 = model_row(&pt2, "Bases", "Unicorn Bases");
        row_2.support_status = Some("supported".into());
        let files = vec![
            file_row(&pt1.join("oval.stl"), &pt1),
            file_row(&pt1.join("square.stl"), &pt1),
            file_row(&pt2.join("oval.stl"), &pt2),
            file_row(&pt2.join("square.stl"), &pt2),
        ];
        db::replace_catalog(&mut conn, &files, &[row_1, row_2], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        let group = &plan.groups[0];
        assert!(
            group.ops.iter().any(|op| op.kind == "drop"),
            "identical copy should plan as a drop: {:?}",
            group.ops
        );
        assert!(
            group.notes.iter().any(|n| n.contains("square 2.stl")),
            "differing contents should get a numbered name: {:?}",
            group.notes
        );

        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let target = root.join("Bestiarum/2026-07 Dread Swamp/Unicorn Bases/Supported");
        // ONE oval survives; both squares survive under distinct names
        assert!(target.join("oval.stl").is_file());
        assert!(!target.join("oval 2.stl").exists());
        assert!(target.join("square.stl").is_file());
        assert!(target.join("square 2.stl").is_file());
        // the dropped copy left the index too
        let oval_rows: u32 = conn
            .query_row(
                "SELECT COUNT(*) FROM files WHERE file_name = 'oval.stl'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(oval_rows, 1);

        let warnings = finalize(
            &conn,
            &root,
            &["Unicorn Bases".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);
        // the emptied part folders are gone
        assert!(!root.join("unicorn").exists());

        fs::remove_dir_all(&root).ok();
    }
}
