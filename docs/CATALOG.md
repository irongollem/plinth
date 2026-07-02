# STL-Pack Cataloger — Feature Spec & Architecture

STL-Pack grows from a release bundler into a disk-scale 3D model manager.
The cataloger indexes terabytes of STLs, makes them searchable in
milliseconds, and becomes the hub the other features (render, bundle,
unbundle) hang off.

## Feature list

### v1 (this iteration)
- **Index the entire disk** — pick a catalog root, scan it in the
  background with progress/cancel (same job pattern as compression and
  rendering). Model files recognized: stl, obj, 3mf, lys, chitubox,
  blend, gcode. Full rescan is idempotent and preserves user-added data.
- **Read our metadata** — `model.json` / `release.json` sidecars written
  by the bundler are imported: names, descriptions, tags, designer,
  release, image previews. Folders without metadata are cataloged
  heuristically (directory = model, dir name = model name, first image =
  preview).
- **Preview** — every model card shows its best image (metadata image or
  first image in the folder); the detail pane can open the live 3D
  viewport (the Render tab's StlViewport) on any STL, no Blender needed.
- **Tag system** — tags from metadata are imported automatically;
  user tags can be added/removed in the UI and survive rescans (keyed by
  folder, not by scan-generated ids). Tag filter chips in search.
- **Fast search** — SQLite FTS5 over name/description/tags/path with
  prefix matching ("bug" finds Bugbear), combined with tag filters,
  paginated grid results.
- **Print (v1)** — a Print button on a model reveals its folder in
  Finder/Explorer so you can drag files into your slicer.
- **Disk stats** — total models/files/bytes, breakdown per extension,
  last scan time.
- **Duplicate detection** — same-size prefilter, then staged content
  hashing (first 128 KiB, then full BLAKE3 only on candidates), grouped
  report with paths and wasted bytes. Hashes are stored so repeat runs
  are cheap.

### v2 / roadmap (deliberately not in this iteration)
- **Send to slicer** — detect installed slicers (Lychee, Chitubox,
  PrusaSlicer, Cura) and open the file directly (`open -a` /
  registered handler). Falls back to v1 reveal.
- **Active duplicate prevention** — hash on import (Add STL flow) and
  warn "this file already exists in release X" before copying.
- **Incremental scanning & watch mode** — mtime/size short-circuit for
  unchanged files; filesystem watcher keeps the index live.
- **Multi-root catalogs** — several disks/folders in one index.
- **Batch promo rendering** — one Blender launch renders thumbnails for
  every un-previewed model in the catalog (manifest mode from the
  render handover); stored rotation per model.
- **Collections & favorites** — arbitrary groupings ("Painted", "Next
  print"), print history.
- **Unbundle integration** — .3dpak archives listed in the catalog and
  extractable in place.
- **Health checks** — corrupt/zero-byte STL detection, orphaned
  metadata, empty dirs, cleanup suggestions.
- **Design pass** — the v1 UI is functional daisyUI; a dedicated design
  iteration (Claude design tooling) once the workflows settle.

## Architecture

### Storage: SQLite (rusqlite, bundled) at `app_data_dir/catalog.db`
One file, zero admin, WAL mode so scans (writes) and searches (reads)
run concurrently, FTS5 built in. Scale sanity: millions of file rows is
comfortable SQLite territory.

Schema (v1):
```sql
files(path PK, dir_path, file_name, extension, size_bytes, modified_at,
      content_hash NULL, indexed_at)
models(dir_path PK, name, description, designer, release_name,
       preview_path, source 'metadata'|'heuristic', uuid, indexed_at)
model_tags(dir_path, tag, source 'metadata'|'user', PK(dir_path, tag))
models_fts(name, description, tags, dir_path)  -- FTS5, rebuilt per scan
meta(key PK, value)                            -- schema_version, last_scan
```
Keying models/tags by `dir_path` (not autoincrement ids) is what lets a
full rescan wipe and rebuild `files`/`models` while `source='user'` tag
rows survive untouched.

### Scanner: background job, same shape as compression/render
`start_catalog_scan` returns a job id immediately; a `scan-status` event
stream (Started/Progress/Completed/Failed/Cancelled) drives the UI;
cancellation via AtomicBool. The walk collects rows in memory, then a
single transaction replaces the catalog (delete-then-insert is simpler
and faster than row-wise upserts at v1 scale; incremental is roadmap).
`model.json` files are parsed during the walk; images resolve to
absolute paths at import time.

### Search: FTS5 with a LIKE fallback path
Query is tokenized, each token quoted with a trailing `*` for prefix
match. Tag filters intersect via `model_tags`. Results are pages of
`CatalogEntry { dir_path, name, description, designer, release_name,
preview_path, tags, file_count, total_size_kb }`.

### Duplicates: staged hashing
1. `GROUP BY size_bytes HAVING count > 1` (free, from the index)
2. hash first 128 KiB of candidates (BLAKE3)
3. full-file hash only where partials collide
Hashes persist into `files.content_hash`, so the expensive step
amortizes. Runs as a background job with progress (hashing terabytes of
same-size candidates can take minutes).

### Frontend: Catalog tab
- Toolbar: root picker (persisted in settings as `catalog_root`),
  Scan/Cancel with progress, search box, tag chips.
- Paginated card grid (preview, name, designer, tags).
- Detail drawer: preview, 3D view (StlViewport), file list with sizes,
  tag editor, Print (reveal in file manager), Render shortcut.
- Stats bar + Duplicates panel.

### What deliberately stays out of the Rust layer
Preview *images* come from disk paths served over the asset protocol;
the 3D preview reuses the existing StlViewport; promo rendering stays in
the Render tab. The catalog only stores paths.
