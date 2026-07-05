# Plinth Cataloger — Architecture

Plinth grew from a release bundler into a disk-scale 3D model manager.
The cataloger indexes terabytes of STLs, makes them searchable in
milliseconds, and is the hub the other features (render, bundle,
import) hang off.

This documents what is **built**. Open work lives in `todolist.md` —
the single source of truth for the roadmap; nothing is planned here.

## What the catalog does today

- **Index the entire disk** — pick a catalog root, scan in the
  background with progress/cancel (same job pattern as compression and
  rendering). Model files recognized: stl, obj, 3mf, lys, chitubox,
  blend, gcode. A full rescan is idempotent and preserves user-added
  data (tags, metadata overrides, renames, pose assignments).
- **Logical models (groups)** — variant dirs sharing a scanner group
  (supported/unsupported builds, poses A/B/C) collapse into ONE card
  with aggregate counts. Cards can be renamed, combined, and split;
  renames are stored against scanner-level source groups so they
  survive rescans.
- **Poses as metadata, not folders** — a "dump everything in one
  folder" model can be split by assigning files to variant/pose
  buckets (`file_variants`); the folder then fans out into one member
  per pose at query time. Files never move on disk for this.
- **Search + facets** — FTS5 (prefix matching) over
  name/description/tags/designer/sculptor/release/path, tag chips, an
  exact designer facet, and designer › release grouping (releases A–Z
  or newest-first, parsed from the M/YYYY release date in SQL).
- **Metadata round-trip** — `model.json` / `release.json` sidecars are
  read on scan and written on pack, so curation travels with a release
  from one user's catalog to the next (see the contract below and
  docs/3PK.md).
- **Print** — the PRINT button opens a file picker (pre-sliced
  .lys/.chitu scenes pre-selected over raw geometry) and hands the
  ticked files to the OS-associated slicer; a `reveal-folder` setting
  keeps the old reveal-in-Finder flow for multi-slicer users.
- **Duplicates: share, don't delete** — same-size prefilter, staged
  BLAKE3 hashing, then "merge — free X MB" hardlinks duplicates so
  every variant keeps a working file (inode-aware: already-shared
  groups report 0 reclaimable). Link support is probed per volume with
  a real test link — never assumed (NAS/SMB/exFAT). Delete remains for
  link-less volumes.
- **3pk import** — opening/dropping a release.3pk verifies checksums,
  extracts with dedup rematerialized, and auto-scans so the release's
  curation lands in the catalog.
- **Render integration** — any member's STLs open in the Render studio;
  the finished promo (with branding baked in) becomes that member's
  preview, per pose (`variant_previews`) or per card (`group_covers`).

## The `model.json` contract (interchange format)

One `model.json` per model dir. Everything except `name` is optional —
a minimal sidecar still parses, unknown fields are ignored. This is the
read side of metadata portability: whatever a release was packed with
is restored on scan (source of truth: `ModelJson` in
`catalog/scanner.rs`, written from `StlModel` on pack).

```jsonc
{
  "id": "uuid", // stable identity across moves
  "name": "Bog Hag",
  "description": "…",
  "tags": ["swamp", "hag"],
  "images": ["preview.png"], // relative to the model dir
  "variant": "sword", // build variant of the same sculpt
  "pose": "A",
  "scale": "32mm",
  "support_status": "supported", // or "unsupported"
  "release_date": "7/2026", // M/YYYY, as the release builder writes it
  "designer": "Bestiarum", // studio/brand
  "sculptor": "…", // individual artist (user/manifest only)
  "release_name": "Dread Swamp",
  "file_poses": [
    // per-file split of a dump folder;
    {
      // restored into file_variants on scan
      "name": "hag_arm_L.stl", // basename, matched within the dir's subtree
      "variant": "sword",
      "pose": "A",
      "support_status": "supported",
    },
  ],
}
```

A `release.json` above the model dirs contributes release-level
fallbacks (name, designer, date) to every model beneath it.

## Storage: SQLite (rusqlite, bundled) at `app_data_dir/catalog.db`

One file, zero admin, WAL mode so scans (writes) and searches (reads)
run concurrently, FTS5 built in. Millions of file rows is comfortable
SQLite territory.

Schema (v5):

```sql
files(path PK, dir_path, file_name, extension, size_bytes, modified_at,
      content_hash NULL,      -- staged BLAKE3, persisted so re-runs are cheap
      file_identity NULL,     -- "device:inode" — hardlink awareness
      indexed_at)
models(dir_path PK, name, description, designer, release_name,
       preview_path, source 'metadata'|'heuristic', uuid, file_count,
       total_size_bytes, pose, scale, support_status, release_date,
       variant, sculptor, group_name, indexed_at)
model_user_meta(dir_path PK, custom_name, pose, scale, support_status,
       release_date, designer, sculptor, release_name, variant)
model_tags(dir_path, tag, source 'metadata'|'user', PK(dir_path, tag))
group_renames(source_group PK, display_name)  -- rename/combine, rescan-safe
file_variants(path PK, dir_path, variant, pose, support_status)
variant_previews(variant_key PK, dir_path, preview_path)  -- per-pose render
group_covers(group_name PK, dir_path, variant_key)  -- user-picked card image
models_fts(name, description, tags, designer, sculptor, release, dir_path)
meta(key PK, value)  -- schema_version, last_scan
```

The dividing line that makes rescans safe: `files`/`models` are wiped
and rebuilt by every scan; everything the **user** created
(`model_user_meta`, user tags, `group_renames`, `file_variants`,
`variant_previews`, `group_covers`) is keyed by path/dir/group-name and
survives untouched. Reads resolve user overrides over scanner values
(`COALESCE(u.x, m.x)`) everywhere.

Base CREATEs run on every open (only versioned migrations are gated) —
a version stamp must never be able to hide a missing table or column;
that failure mode locked users out of the catalog once.

## Scanner: background job, same shape as compression/render

`start_catalog_scan` returns a job id immediately; a `scan-status`
event stream (Started/Progress/Completed/Failed/Cancelled) drives the
UI; cancellation via AtomicBool. The walk collects rows in memory, then
one transaction replaces the catalog. During the walk: `model.json` /
`release.json` are parsed (metadata wins over heuristics), support
status is inferred from presupported-only formats (.lys/.chitu) or
file names, poses/variants from folder structure, and the designer is
matched against the user-editable lexicon in settings
(`known_designers`).

## Search: FTS5 + facets, grouped

Query tokens are quoted with a trailing `*` for prefix match. Tag
filters intersect via `model_tags`; the designer facet matches exactly
(case-insensitive). `search_catalog_groups` returns one row per
logical model with a caller-chosen sort (`name`, `designer`,
`designer_date`) — sorting stays in SQL so grouping holds across
pagination; the frontend only draws section headers where the designer
or release changes between consecutive rows. `group_members` fans a
group out into its variant/pose members for the detail drawer.

## Frontend: Catalog tab

- Toolbar: search, designer facet, group/sort mode, list/grid toggle,
  root picker + Scan/Cancel with progress.
- Card grid or list, both selectable for batch move/combine.
- Detail drawer (resizable): preview or inline 3D (opt-in per member —
  never latched, big parses freeze the main thread), support/variant/
  pose navigation tiers, file list with pose-assignment tooling, tag
  editor, metadata editor (edits propagate to support twins), PRINT /
  3D / RENDER actions.
- Stats bar + duplicates panel (merge/delete, link-support gated).

## What deliberately stays out of the Rust layer

Preview images come from disk paths served over the asset protocol;
the 3D preview reuses StlViewport; promo rendering and branding
compositing stay in the Render tab (the webview composites overlays —
same font engine as the preview). The catalog stores paths, never file
bodies.
