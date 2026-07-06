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
    base_round_mm: Option<u32>,
    base_square_mm: Option<u32>,
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
                COALESCE(u.sculptor, m.sculptor),
                COALESCE(u.base_round_mm, m.base_round_mm),
                COALESCE(u.base_square_mm, m.base_square_mm)
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
            base_round_mm: row.get(12)?,
            base_square_mm: row.get(13)?,
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

/// lowercase variant -> the CONVENTIONAL spelling (layout::title_case),
/// so case-variant names ("Sword"/"sword"/"SWORD") resolve to ONE leaf
/// everywhere the layout is built — the tool decides casing, not
/// whichever member got typed first.
fn canonical_variants(members: &[&MemberRow]) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    for m in members {
        if let Some(v) = m.variant.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            map.entry(v.to_lowercase())
                .or_insert_with(|| layout::title_case(v));
        }
    }
    map
}

struct FileRowLite {
    path: String,
    file_name: String,
}

/// Rebuild `computed` segment by segment from `root`, adopting the exact
/// casing of any directory that already exists along the way. Metadata
/// carries display case (an old sidecar said "AELVES - THE FARWOOD"; the
/// disk says "Aelves - The Farwood") — fighting the difference produces
/// ghost renames that never converge on case-insensitive volumes, and
/// would fork a SECOND tree on case-sensitive ones (the NAS). Existing
/// dirs win; metadata case only ever names dirs that don't exist yet.
fn adopt_disk_casing(root: &Path, computed: &Path) -> PathBuf {
    let Ok(rel) = computed.strip_prefix(root) else {
        return computed.to_path_buf();
    };
    let mut out = root.to_path_buf();
    for comp in rel.components() {
        let want = comp.as_os_str().to_string_lossy().into_owned();
        let existing = std::fs::read_dir(&out).ok().and_then(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name())
                .find(|n| n.to_string_lossy().eq_ignore_ascii_case(&want))
        });
        match existing {
            Some(name) => out.push(name),
            None => out.push(comp),
        }
    }
    out
}

fn is_image_file(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| super::IMAGE_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// OS litter that should neither be planned as a move nor keep a dir alive.
fn is_litter(name: &str) -> bool {
    name.starts_with('.') || name == "Thumbs.db"
}

/// Regular files directly inside `dir` ON DISK, litter skipped, sorted.
/// The plan's merge paths ask the DISK, not the files table, on purpose:
/// the scanner only indexes model files, so images/readmes/licences have
/// no rows at all — an index-driven merge would silently leave them
/// behind (and the vanished-production-thumbnail bug was exactly this).
fn disk_files(dir: &Path) -> Vec<FileRowLite> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut out: Vec<FileRowLite> = entries
        .flatten()
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if is_litter(&name) {
                return None;
            }
            Some(FileRowLite {
                path: e.path().to_string_lossy().into_owned(),
                file_name: name,
            })
        })
        .collect();
    out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
    out
}

/// Image files anywhere under `dir` on disk (recursive, `exclude` subtrees
/// skipped), as dir-relative paths — shallowest first so the model-root
/// render beats one buried in an "extras" folder as the preview.
fn disk_images_under(dir: &Path, exclude: &[&str]) -> Vec<String> {
    let mut found: Vec<(usize, String)> = Vec::new();
    let mut stack: Vec<(PathBuf, usize)> = vec![(dir.to_path_buf(), 0)];
    while let Some((current, depth)) = stack.pop() {
        if depth > 6 {
            continue; // a runaway tree is not a preview hunt
        }
        let Ok(entries) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                let path_str = path.to_string_lossy();
                if !exclude.iter().any(|x| is_under(&path_str, x)) {
                    stack.push((path, depth + 1));
                }
            } else if !is_litter(&name) && is_image_file(&name) {
                if let Ok(rel) = path.strip_prefix(dir) {
                    found.push((depth, rel.to_string_lossy().into_owned()));
                }
            }
        }
    }
    found.sort();
    found.into_iter().map(|(_, rel)| rel).collect()
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

/// Plan one file's landing spot in a merge bucket.
///
/// File names are the DESIGNER'S — they stay untouched unless an actual
/// clash forces a choice (pose is metadata: file_variants + model.json,
/// not a mandatory name mutation). Clash policy, in order:
/// 1. byte-identical to the claimant -> reviewable "drop" (the copy is
///    redundant once one lands — the repeated-bases case)
/// 2. pose suffix, when the member has a pose and it frees the name
///    (identically-named files from pose dirs A/B/C)
/// 3. numbered name — never skip, never lose a file
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
    let mut target_name = file.file_name.clone();
    if let Some(kept_original) = used_names.get(&target_name.to_lowercase()) {
        if same_content(Path::new(kept_original), Path::new(&file.path)) {
            ops.push(NormalizeOp {
                from: current,
                to: format!("{}{}{}", leaf, MAIN_SEPARATOR, target_name),
                kind: "drop".into(),
                pose: None,
            });
            return;
        }
        let suffixed = layout::pose_suffixed_name(&file.file_name, pose.unwrap_or(""));
        target_name = if suffixed != file.file_name
            && !used_names.contains_key(&suffixed.to_lowercase())
        {
            suffixed
        } else {
            numbered_name(&file.file_name, used_names)
        };
        notes.push(format!(
            "{} exists twice with different contents — one becomes {}",
            file.file_name, target_name
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
        // nothing moves, but the pose still lands as file-level metadata
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
        let model_dir = adopt_disk_casing(
            root,
            &layout::model_dir(
                root,
                &designer,
                release.as_deref(),
                date.as_deref(),
                &display,
            ),
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
                // a model dir that ALREADY exists (earlier cleanup, second
                // batch of the same release) can't be rename-created again —
                // per-member mode merges into it instead. Case-only fixes of
                // the same dir are still a legal rename.
                && (!Path::new(&model_dir_str).exists()
                    || b.eq_ignore_ascii_case(&model_dir_str))
                && all_dirs
                    .iter()
                    .filter(|d| is_under(d, b))
                    .all(|d| group_dirs.contains(d)) // no foreign models beneath
        });

        // kept past the match: phase-2 must map leaf paths BACK through
        // the phase-1 move to ask the disk what will exist after it lands
        let base_move: Option<String> = wholesale.map(str::to_string);

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

        // ---- stranded images in EXCLUSIVE ancestor dirs. A source model
        // dir like "Little Knight's Command Group/" often holds its
        // thumbnails BESIDE the build folders; the build folders merge
        // away as members and the images stay stranded in a husk dir
        // forever. An ancestor qualifies while every model dir beneath it
        // belongs to this group — the walk stops the moment a foreign
        // model shares the dir, so release-level images that belong to
        // everybody are never claimed. IMAGES ONLY, deliberately: an
        // exclusive month folder can also hold backup archives and other
        // freight that has no business inside a model dir.
        {
            // above the wholesale base (it carries its own insides), or
            // above each member in per-member mode
            let walk_from: Vec<&str> = match base_move.as_deref() {
                Some(b) => vec![b],
                None => dirs.to_vec(),
            };
            let mut extra_dirs: Vec<String> = Vec::new();
            for d in walk_from {
                let mut current = Path::new(d).parent();
                while let Some(dir) = current {
                    let dir_str = dir.to_string_lossy().into_owned();
                    if dir_str == root_str
                        || !is_under(&dir_str, &root_str)
                        || is_under(&dir_str, &model_dir_str)
                        || group_dirs.contains(dir_str.as_str())
                    {
                        break;
                    }
                    let exclusive = all_dirs
                        .iter()
                        .filter(|x| is_under(x, &dir_str))
                        .all(|x| group_dirs.contains(*x));
                    if !exclusive {
                        break;
                    }
                    if !extra_dirs.contains(&dir_str) {
                        extra_dirs.push(dir_str.clone());
                    }
                    current = dir.parent();
                }
            }
            if !extra_dirs.is_empty() {
                // images land at the model ROOT, colliding against
                // whatever is already there
                let mut used_names: HashMap<String, String> = HashMap::new();
                for f in disk_files(&model_dir) {
                    used_names.insert(f.file_name.to_lowercase(), f.path.clone());
                }
                for dir in &extra_dirs {
                    for f in disk_files(Path::new(dir)) {
                        if !is_image_file(&f.file_name) {
                            continue;
                        }
                        place_file(
                            f.path.clone(),
                            &f,
                            None,
                            &model_dir_str,
                            &mut used_names,
                            &mut ops,
                            &mut notes,
                        );
                    }
                    if !old_dirs.contains(dir) {
                        old_dirs.push(dir.clone());
                    }
                }
            }
        }

        // ---- phase 2: reshape into Supported/Unsupported[/variant] leaves
        // "Sword" and "sword" are the same variant: unify case-variant
        // spellings onto the first one seen, or they'd bucket into two
        // case-variant leaves that are the SAME dir on macOS/Windows and
        // a forked pair on the case-sensitive NAS
        let variant_case = canonical_variants(members);
        let desired: Vec<String> = members
            .iter()
            .map(|m| {
                let variant = m
                    .variant
                    .as_deref()
                    .and_then(|v| variant_case.get(&v.to_lowercase()).map(String::as_str));
                layout::member_dir(&model_dir, m.support.as_deref(), variant)
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
            // A leaf that already exists — or WILL exist the moment the
            // wholesale move lands (its pre-image inside the old base is on
            // disk: Dark Wardens/Supported traveled along with B->M) — is
            // normal, not an error. Nothing may dir-rename onto it;
            // everything merges INTO it per-file, colliding against
            // whatever it already holds.
            let occupant = idxs
                .iter()
                .copied()
                .find(|&i| cur[i].eq_ignore_ascii_case(leaf));
            // the leaf's location BEFORE phase 1, when a wholesale move is
            // planned — that's where the disk can be asked at plan time
            let pre_image = base_move.as_deref().and_then(|b| {
                leaf.strip_prefix(model_dir_str.as_str())
                    .map(|suffix| format!("{}{}", b, suffix))
            });
            let leaf_now = Path::new(leaf).exists();
            let leaf_on_disk = leaf_now
                || pre_image
                    .as_deref()
                    .is_some_and(|p| Path::new(p).exists());
            let leaf_exists_foreign = leaf_on_disk && occupant.is_none();

            // target name (lowercased) -> ORIGINAL path of the file that
            // claimed it, so clashes can be settled by comparing contents.
            // Pre-seeded with the leaf's existing disk files (read from the
            // pre-image when the leaf only exists after phase 1) so merges
            // dedup/disambiguate against them too.
            let mut used_names: HashMap<String, String> = HashMap::new();
            if leaf_exists_foreign {
                let seed_source = if leaf_now {
                    Some(leaf.to_string())
                } else {
                    pre_image.clone()
                };
                if let Some(source) = seed_source {
                    for f in disk_files(Path::new(&source)) {
                        if f.file_name != "model.json" && f.file_name != "release.json" {
                            used_names.insert(f.file_name.to_lowercase(), f.path.clone());
                        }
                    }
                }
            }

            let merging = idxs.len() > 1 || leaf_exists_foreign;
            let mut anchored = occupant.is_some() || leaf_exists_foreign;

            // the member already AT the leaf claims its file names first —
            // renaming an in-place file away because a merged file got to
            // the registry earlier would be exactly backwards
            let mut order: Vec<usize> = idxs.clone();
            if let Some(o) = occupant {
                order.retain(|&i| i != o);
                order.insert(0, o);
            }

            for &i in &order {
                let member = members[i];
                let from_dir = &cur[i];
                let is_occupant = Some(i) == occupant;
                let is_nested_parent = cur
                    .iter()
                    .enumerate()
                    .any(|(j, other)| j != i && is_under(other, from_dir) && other != from_dir);
                // a dir rename is legal only onto a spot that's genuinely
                // free on disk (case-only fixes of the same dir excepted).
                // Both nesting directions are fatal: a leaf inside the
                // member can't receive it, and a member inside its own
                // leaf (Supported/Clean Bases -> Supported) would rename a
                // child onto its own parent.
                let can_rename = !anchored
                    && !is_nested_parent
                    && !is_under(leaf, from_dir)
                    && !is_under(from_dir, leaf)
                    && (!leaf_on_disk || from_dir.eq_ignore_ascii_case(leaf));

                if is_occupant || can_rename {
                    if from_dir != *leaf {
                        ops.push(NormalizeOp {
                            from: from_dir.clone(),
                            to: (*leaf).to_string(),
                            kind: "dir".into(),
                            pose: None,
                        });
                    }
                    anchored = true;
                    // in a merge, the anchor's files still register their
                    // names (and pose metadata) so incomers collide with
                    // them correctly
                    if merging {
                        for f in disk_files(Path::new(&member.dir)) {
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
                for f in disk_files(Path::new(&member.dir)) {
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
        clean_names: out
            .iter()
            .filter(|g| g.clean)
            .map(|g| g.group_name.clone())
            .collect(),
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
    // targets of failed folder moves: every later op addressing paths the
    // rename would have created is doomed — skip them quietly instead of
    // burying the ONE real error under a wall of "Source not found"
    let mut failed_dir_targets: Vec<String> = Vec::new();
    let mut suppressed = 0u32;

    for op in ops {
        if failed_dir_targets
            .iter()
            .any(|t| is_under(&op.from, t) || is_under(&op.to, t))
        {
            suppressed += 1;
            continue;
        }
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
            match super::db::remove_files(conn, std::slice::from_ref(&op.from)) {
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
            if op.kind == "dir" {
                failed_dir_targets.push(op.to.clone());
            }
            continue;
        }
        // A case-only rename ("unsupported" -> "Unsupported") reports the
        // destination as existing on case-insensitive filesystems (macOS,
        // Windows) even though it's the SAME entry — rename handles it fine
        let case_only = op.from.eq_ignore_ascii_case(&op.to) && op.from != op.to;
        if to.exists() && !case_only {
            errors.push(format!("Destination already exists: {}", op.to));
            if op.kind == "dir" {
                failed_dir_targets.push(op.to.clone());
            }
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
            if op.kind == "dir" {
                failed_dir_targets.push(op.to.clone());
            }
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
    if suppressed > 0 {
        errors.push(format!(
            "{} follow-up move{} skipped because their folder move failed above",
            suppressed,
            if suppressed == 1 { "" } else { "s" }
        ));
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
        // adopt existing casing here too, or the image walk misses the
        // real dir on case-sensitive volumes
        let model_dir = adopt_disk_casing(
            root,
            &layout::model_dir(
                root,
                &designer,
                release.as_deref(),
                date.as_deref(),
                &display,
            ),
        );
        let model_dir_str = model_dir.to_string_lossy().into_owned();

        // Group members by the LEAF the plan sent their files to — NOT by
        // their plan-time dir. Per-file merges empty the old pose dirs and
        // the sweep removes them; writing metadata there throws it away
        // with the dir. That is exactly how Dark Wardens' Supported side
        // lost its identity: sidecars went into dying pose folders while
        // the variant folders holding every file got nothing, and the next
        // scan shattered them into heuristic per-variant cards.
        let member_refs: Vec<&MemberRow> = members.iter().collect();
        let variant_case = canonical_variants(&member_refs);
        let mut leaves: BTreeMap<String, Vec<&MemberRow>> = BTreeMap::new();
        for member in &members {
            // same case unification as plan — the sidecar must land in the
            // ONE leaf both spellings map to
            let variant = member
                .variant
                .as_deref()
                .and_then(|v| variant_case.get(&v.to_lowercase()).map(String::as_str));
            let computed = layout::member_dir(&model_dir, member.support.as_deref(), variant);
            let leaf = if computed.is_dir() {
                computed.to_string_lossy().into_owned()
            } else if Path::new(&member.dir).is_dir() {
                // apply skipped/failed this member — describe it where it is
                member.dir.clone()
            } else {
                warnings.push(format!(
                    "{}: neither the target folder nor the source exists",
                    member.dir
                ));
                continue;
            };
            leaves.entry(leaf).or_default().push(member);
        }

        // Images anywhere under the model dir ON DISK — the root itself,
        // or a sibling folder like "Images"/"renders" some designers ship
        // beside Supported/Unsupported — are candidate fallback previews.
        // The disk, not the files table: the scanner never indexes images,
        // so an index lookup wrote empty images lists and the sidecar
        // (being authoritative) then ERASED previews that heuristics used
        // to find. Excludes anything inside a leaf: that image is the
        // leaf's OWN preview (own_images below).
        let leaf_dirs: Vec<&str> = leaves.keys().map(String::as_str).collect();
        let root_images: Vec<String> = disk_images_under(&model_dir, &leaf_dirs);

        for (leaf, leaf_members) in &leaves {
            if let Err(e) =
                write_leaf_json(conn, leaf, leaf_members, &model_dir_str, &root_images)
            {
                warnings.push(format!("{}: {}", leaf, e));
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

/// Write one LEAF's model.json from the state of every member whose files
/// landed there. Facets resolve first-non-null across those members (they
/// share support/variant by construction; poses differ and live at file
/// level). File-level poses beat a dir-level pose: when file_poses exist
/// the dir pose is omitted (and its user override cleared) so the two
/// mechanisms can't fight after a rescan.
fn write_leaf_json(
    conn: &Connection,
    leaf: &str,
    leaf_members: &[&MemberRow],
    model_dir: &str,
    root_images: &[String],
) -> Result<(), AppError> {
    let leaf_path = Path::new(leaf);
    let map_err =
        |e: rusqlite::Error| AppError::ConfigError(format!("model.json query failed: {}", e));

    // tags may still ride on old member dirs (per-file merges don't re-key
    // dir-scoped rows) — union them with the leaf's own
    let mut tag_stmt = conn
        .prepare("SELECT tag FROM model_tags WHERE dir_path = ?1 ORDER BY tag")
        .map_err(map_err)?;
    let mut tags: Vec<String> = Vec::new();
    let mut tag_dirs: Vec<&str> = vec![leaf];
    tag_dirs.extend(leaf_members.iter().map(|m| m.dir.as_str()));
    for dir in tag_dirs {
        let rows: Vec<String> = tag_stmt
            .query_map([dir], |row| row.get(0))
            .and_then(|rows| rows.collect())
            .map_err(map_err)?;
        for tag in rows {
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
    }
    tags.sort();

    // apply re-keys file_variants to the leaf as files land, so the leaf
    // query sees every pose that survived the merge
    let mut fp_stmt = conn
        .prepare(
            "SELECT path, variant, pose, support_status FROM file_variants
             WHERE dir_path = ?1 AND COALESCE(pose, '') != '' ORDER BY path",
        )
        .map_err(map_err)?;
    let file_poses: Vec<serde_json::Value> = fp_stmt
        .query_map([leaf], |row| {
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

    // image references: own images first, else a root/sibling one found by
    // finalize — reached via "../" x (this leaf's depth below model_dir),
    // since root_images entries are already model_dir-relative subpaths
    let own_images: Vec<String> = disk_images_under(leaf_path, &[]);
    let images: Vec<String> = if !own_images.is_empty() {
        own_images
    } else if is_under(leaf, model_dir) && leaf != model_dir {
        let depth = leaf[model_dir.len()..].matches(MAIN_SEPARATOR).count();
        root_images
            .iter()
            .map(|img| format!("{}{}", "../".repeat(depth), img))
            .collect()
    } else {
        vec![]
    };

    let refs: Vec<&MemberRow> = leaf_members.to_vec();
    let dir_pose = if file_poses.is_empty() && leaf_members.len() == 1 {
        leaf_members[0].pose.clone()
    } else {
        None
    };
    if !file_poses.is_empty() {
        // poses live on files now — a lingering dir-level user pose would
        // resurrect through COALESCE on the next read
        for dir in std::iter::once(leaf).chain(leaf_members.iter().map(|m| m.dir.as_str())) {
            conn.execute(
                "UPDATE model_user_meta SET pose = NULL WHERE dir_path = ?1",
                [dir],
            )
            .map_err(map_err)?;
        }
    }

    let json = serde_json::json!({
        "id": first_some(&refs, |r| r.uuid.as_ref()),
        "name": leaf_members[0].gname,
        "description": first_some(&refs, |r| r.description.as_ref()),
        "tags": tags,
        "images": images,
        "variant": first_some(&refs, |r| r.variant.as_ref())
            .map(|v| layout::title_case(&v)),
        "pose": dir_pose,
        "scale": first_some(&refs, |r| r.scale.as_ref()),
        "support_status": first_some(&refs, |r| r.support.as_ref()),
        "release_date": first_some(&refs, |r| r.date.as_ref()),
        "designer": first_some(&refs, |r| r.designer.as_ref()),
        "sculptor": first_some(&refs, |r| r.sculptor.as_ref()),
        "release_name": first_some(&refs, |r| r.release.as_ref()),
        "base_round_mm": refs.iter().find_map(|r| r.base_round_mm),
        "base_square_mm": refs.iter().find_map(|r| r.base_square_mm),
        "file_poses": file_poses,
    });
    let pretty = serde_json::to_string_pretty(&json)
        .map_err(|e| AppError::ConfigError(format!("model.json encode failed: {}", e)))?;
    std::fs::write(leaf_path.join("model.json"), pretty)
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
            base_round_mm: None,
            base_square_mm: None,
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
        // NOTE: no FileRow for render.png — the real scanner indexes model
        // files only; images exist on disk but never in the files table
        let files = vec![
            file_row(&sup.join("bog.lys"), &sup),
            file_row(&unsup.join("bog.stl"), &unsup),
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
        assert_eq!(count, 2);

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
        // identical names, DIFFERENT sculpts — the true pose-dir shape
        fs::create_dir_all(old.join("A")).unwrap();
        fs::create_dir_all(old.join("B")).unwrap();
        fs::write(old.join("A/galeb duhr.stl"), b"pose a sculpt").unwrap();
        fs::write(old.join("B/galeb duhr.stl"), b"pose b sculpt").unwrap();

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
        // both poses side by side in ONE build folder. The first keeps the
        // designer's original name — renames happen ONLY to resolve the
        // clash, and the pose suffix is the preferred disambiguator
        assert!(target.join("Supported/galeb duhr.stl").is_file());
        assert!(target.join("Supported/galeb duhr B.stl").is_file());
        assert!(!target.join("Supported/galeb duhr A.stl").exists());

        // pose survived as file-level metadata on BOTH files
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
        assert!(
            poses
                .iter()
                .any(|(p, pose)| p.ends_with("galeb duhr.stl") && pose == "A")
        );
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

    #[test]
    fn merges_into_an_already_existing_build_folder() {
        // The Centaurs incident: Supported/ already existed at the target
        // (holding earlier-merged files) while pose A still lived in a
        // nested A/ subdir inside it. The old planner elected A for a dir
        // rename ONTO its own parent — "Destination already exists" plus a
        // wall of dependent "Source not found" errors. An occupied leaf is
        // normal (second batch of a release, partial earlier run): merge
        // INTO it per-file, colliding against what's already there.
        let root = std::env::temp_dir().join(format!("plinth_norm_occ_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let leaf = root.join("Bestiarum/2026-07 Dread Swamp/Centaurs/Supported");
        let nested = leaf.join("A");
        fs::create_dir_all(&nested).unwrap();
        fs::write(leaf.join("centaur_B.lys"), b"pose b").unwrap();
        fs::write(nested.join("centaur_A.lys"), b"pose a").unwrap();
        fs::write(nested.join("shared_base.stl"), b"same base").unwrap();
        fs::write(leaf.join("shared_base.stl"), b"same base").unwrap();
        fs::write(nested.join("model.json"), b"{\"name\":\"Centaurs\"}").unwrap();

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        // only the nested pose dir is indexed as a member — exactly the
        // mid-cleanup state the incident's rescan produced
        let mut row_a = model_row(&nested, "Centaurs", "Centaurs");
        row_a.support_status = Some("supported".into());
        row_a.pose = Some("A".into());
        let files = vec![
            file_row(&nested.join("centaur_A.lys"), &nested),
            file_row(&nested.join("shared_base.stl"), &nested),
        ];
        db::replace_catalog(&mut conn, &files, &[row_a], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        let group = &plan.groups[0];
        // NOTHING may dir-rename onto the occupied leaf
        assert!(
            group.ops.iter().all(|op| op.kind != "dir"),
            "occupied leaf must merge per-file: {:?}",
            group.ops
        );

        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        // unique file moved UP with its designer name intact...
        assert!(leaf.join("centaur_A.lys").is_file());
        assert!(!leaf.join("centaur_A A.lys").exists());
        // ...the identical base collapsed instead of erroring...
        assert!(leaf.join("shared_base.stl").is_file());
        assert!(!nested.join("shared_base.stl").exists());
        // ...and what was already in the leaf is untouched
        assert!(leaf.join("centaur_B.lys").is_file());

        let warnings = finalize(
            &conn,
            &root,
            &["Centaurs".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);
        // the emptied pose dir (stale sidecar and all) is gone
        assert!(!nested.exists());
        // ...and the SURVIVING leaf carries the metadata, not the dead dir
        let meta: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(leaf.join("model.json")).unwrap()).unwrap();
        assert_eq!(meta["name"], "Centaurs");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn preview_in_a_sibling_images_folder_is_not_lost() {
        // Some designers ship a promo thumbnail beside Supported/Unsupported
        // in its own "Images" folder rather than directly at the model
        // root. Before the fix, finalize's root-image lookup was an EXACT
        // dir match, so it never saw a nested sibling folder — the
        // authoritative model.json ended up with no images at all, and the
        // scanner (which trusts model.json completely once it exists) lost
        // the preview on the very next rescan.
        let root = std::env::temp_dir().join(format!("plinth_norm_img_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let old = root.join("Collector of Names");
        let sup = old.join("Supported");
        let unsup = old.join("Unsupported");
        let images = old.join("Images");
        touch(&sup.join("names.lys"));
        touch(&unsup.join("names.stl"));
        touch(&images.join("Product Thumbnail - Collector of Names.jpg"));

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut sup_row = model_row(&sup, "Collector of Names supported", "Collector of Names");
        sup_row.support_status = Some("supported".into());
        let mut unsup_row = model_row(&unsup, "Collector of Names", "Collector of Names");
        unsup_row.support_status = Some("unsupported".into());
        // like the real scanner: the jpg exists on disk only, never as a
        // files-table row — the lookup must not depend on the index
        let files = vec![
            file_row(&sup.join("names.lys"), &sup),
            file_row(&unsup.join("names.stl"), &unsup),
        ];
        db::replace_catalog(&mut conn, &files, &[sup_row, unsup_row], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        let group = &plan.groups[0];
        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);

        let target = root.join("Bestiarum/2026-07 Dread Swamp/Collector of Names");
        // the Images folder rides along with the wholesale move, untouched
        assert!(target
            .join("Images/Product Thumbnail - Collector of Names.jpg")
            .is_file());

        let warnings = finalize(
            &conn,
            &root,
            &["Collector of Names".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);

        for leaf in ["Supported", "Unsupported"] {
            let meta: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(target.join(leaf).join("model.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(
                meta["images"][0],
                "../Images/Product Thumbnail - Collector of Names.jpg",
                "{} lost its preview reference",
                leaf
            );
        }

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn metadata_casing_defers_to_existing_dirs() {
        // An old sidecar said release "AELVES - THE FARWOOD"; the folder on
        // disk says "2026-05 Aelves - The Farwood". Deriving the target from
        // metadata case-sensitively made the group permanently 'dirty' with
        // ghost moves into a path that IS the same dir on macOS — and would
        // fork a second tree on the case-sensitive NAS. Existing dirs win;
        // metadata case only names dirs that don't exist yet.
        let root = std::env::temp_dir().join(format!("plinth_norm_case_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let sup = root.join("Dragon Trappers/2026-05 Aelves - The Farwood/Centaurs/Supported");
        fs::create_dir_all(&sup).unwrap();
        fs::write(sup.join("centaur.lys"), b"x").unwrap();

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut row = model_row(&sup, "Centaurs", "Centaurs");
        row.designer = Some("Dragon Trappers".into());
        row.release_name = Some("AELVES - THE FARWOOD".into());
        row.release_date = Some("2026-05".into());
        row.support_status = Some("supported".into());
        let files = vec![file_row(&sup.join("centaur.lys"), &sup)];
        db::replace_catalog(&mut conn, &files, &[row], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        assert_eq!(
            plan.groups.len(),
            0,
            "ghost moves planned: {:?}",
            plan.groups
        );
        assert_eq!(plan.clean_groups, 1);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn wholesale_move_merges_into_dirs_that_travel_along() {
        // The Dark Wardens incident. Wholesale-moving the base carries
        // Supported/ (and the variant dirs) along — so those leaves EXIST
        // the moment phase 1 lands, even though Path::exists() said no at
        // plan time. The old plan then dir-renamed Supported/Clean Bases
        // onto its own parent: 'Destination already exists' + 39 skipped.
        // Leaf existence must be asked at the PRE-IMAGE path, and a member
        // nested under its own leaf must always merge per-file (upward).
        let root = std::env::temp_dir().join(format!("plinth_norm_pre_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let old = root.join("trapper_tier/Dark Wardens");
        let bases = old.join("Supported/Clean Bases");
        let pose_a = old.join("Supported/Great Swords/Pose A");
        let pose_b = old.join("Supported/Great Swords/Pose B");
        // the unsupported tree is what makes the MODEL dir the common
        // ancestor — so Supported/ itself is in the traveling set
        let unsup = old.join("Unsupported");
        for d in [&bases, &pose_a, &pose_b, &unsup] {
            fs::create_dir_all(d).unwrap();
        }
        fs::write(bases.join("base_25mm.stl"), b"base").unwrap();
        fs::write(pose_a.join("warden.lys"), b"pose a").unwrap();
        fs::write(pose_b.join("warden.lys"), b"pose b").unwrap();
        fs::write(unsup.join("warden.stl"), b"raw").unwrap();

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut base_row = model_row(&bases, "Dark Wardens bases", "Dark Wardens");
        base_row.support_status = Some("supported".into());
        let mut row_a = model_row(&pose_a, "Dark Wardens A", "Dark Wardens");
        row_a.support_status = Some("supported".into());
        row_a.variant = Some("Great Swords".into());
        row_a.pose = Some("A".into());
        let mut row_b = model_row(&pose_b, "Dark Wardens B", "Dark Wardens");
        row_b.support_status = Some("supported".into());
        row_b.variant = Some("Great Swords".into());
        row_b.pose = Some("B".into());
        let mut unsup_row = model_row(&unsup, "Dark Wardens", "Dark Wardens");
        unsup_row.support_status = Some("unsupported".into());
        let files = vec![
            file_row(&bases.join("base_25mm.stl"), &bases),
            file_row(&pose_a.join("warden.lys"), &pose_a),
            file_row(&pose_b.join("warden.lys"), &pose_b),
            file_row(&unsup.join("warden.stl"), &unsup),
        ];
        db::replace_catalog(
            &mut conn,
            &files,
            &[base_row, row_a, row_b, unsup_row],
            &[],
            &[],
        )
        .unwrap();

        let plan = plan(&conn, &root, None, None).unwrap();
        let group = &plan.groups[0];
        // exactly ONE dir op: the wholesale base move. Supported/ and
        // Great Swords/ travel along — nothing may rename onto them
        let dir_ops: Vec<_> = group.ops.iter().filter(|op| op.kind == "dir").collect();
        assert_eq!(dir_ops.len(), 1, "ops: {:?}", group.ops);

        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        let target = root.join("Bestiarum/2026-07 Dread Swamp/Dark Wardens");
        // bases merged UP into the build folder, poses into the variant
        // folder with the clash pose-suffixed
        assert!(target.join("Supported/base_25mm.stl").is_file());
        assert!(target.join("Supported/Great Swords/warden.lys").is_file());
        assert!(target.join("Supported/Great Swords/warden B.lys").is_file());

        let warnings = finalize(
            &conn,
            &root,
            &["Dark Wardens".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);
        assert!(!target.join("Supported/Clean Bases").exists());
        assert!(!target.join("Supported/Great Swords/Pose A").exists());

        // THE placement regression: sidecars must land in the LEAVES
        // holding the files, not in the swept pose dirs — or the next scan
        // shatters the model into heuristic per-variant cards
        for leaf in ["Supported", "Supported/Great Swords", "Unsupported"] {
            let meta: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(target.join(leaf).join("model.json")).unwrap_or_else(|_| {
                    panic!("{} must carry a sidecar", leaf)
                }),
            )
            .unwrap();
            assert_eq!(meta["name"], "Dark Wardens", "{} sidecar name", leaf);
        }
        let gs_meta: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join("Supported/Great Swords/model.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(gs_meta["variant"], "Great Swords");
        assert_eq!(gs_meta["file_poses"].as_array().unwrap().len(), 2);

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn stranded_thumbnails_follow_the_model() {
        // Little Knights: the command group's thumbnails sat BESIDE its
        // build folders in its own source dir. The build folders merged
        // away as members and the jpgs stayed stranded in a husk directory
        // the sweep couldn't remove. Images in EXCLUSIVE ancestor dirs
        // (only this group's models beneath) must ride along to the model
        // root; the foreign sibling keeps release-level files unclaimed.
        let root = std::env::temp_dir().join(format!("plinth_norm_husk_{}", std::process::id()));
        fs::remove_dir_all(&root).ok();
        let rel = root.join("pt1");
        let foreign = rel.join("Peryton");
        let src = rel.join("Little Knight's Command Group");
        let sup = src.join("Supported");
        fs::create_dir_all(&foreign).unwrap();
        fs::create_dir_all(&sup).unwrap();
        fs::write(foreign.join("peryton.stl"), b"x").unwrap();
        fs::write(sup.join("cmd.stl"), b"x").unwrap();
        fs::write(src.join("thumb.jpg"), b"jpg").unwrap();
        // release-level image shared by every model — must NOT be claimed
        fs::write(rel.join("group shot.jpg"), b"jpg").unwrap();

        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        db::test_init(&conn);
        let mut knight = model_row(&sup, "Little Knights", "Little Knights");
        knight.support_status = Some("supported".into());
        let peryton = model_row(&foreign, "Peryton", "Peryton");
        let files = vec![
            file_row(&sup.join("cmd.stl"), &sup),
            file_row(&foreign.join("peryton.stl"), &foreign),
        ];
        db::replace_catalog(&mut conn, &files, &[knight, peryton], &[], &[]).unwrap();

        let plan = plan(&conn, &root, None, Some("Little Knights")).unwrap();
        let group = &plan.groups[0];
        let target = root.join("Bestiarum/2026-07 Dread Swamp/Little Knights");
        assert!(
            group
                .ops
                .iter()
                .any(|op| op.kind == "file" && op.to.ends_with("thumb.jpg")),
            "husk image must be planned: {:?}",
            group.ops
        );

        let outcome = apply_ops(&mut conn, &group.ops).unwrap();
        assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
        assert!(target.join("thumb.jpg").is_file());
        assert!(target.join("Supported/cmd.stl").is_file());
        // the shared release image stayed where it was
        assert!(rel.join("group shot.jpg").is_file());

        let warnings = finalize(
            &conn,
            &root,
            &["Little Knights".to_string()],
            &group.old_dirs,
        )
        .unwrap();
        assert!(warnings.is_empty(), "{:?}", warnings);
        // the husk is gone, and the sidecar references the rescued render
        assert!(!src.exists());
        let meta: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(target.join("Supported/model.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(meta["images"][0], "../thumb.jpg");
        // the foreign model was untouched
        assert!(foreign.join("peryton.stl").is_file());

        fs::remove_dir_all(&root).ok();
    }
}
