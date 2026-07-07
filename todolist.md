# Todo List

## Doing

## To Do

### Render engine (Blender promo renders)

- [ ] Bundle or download-on-first-run a portable Blender (GPL allows shipping it as a separate CLI process; include license + source link). Detector already checks exe-relative `Resources/blender/`.
- [ ] Batch render mode: manifest of many minis in ONE Blender launch (see raw/HANDOVER.md, biggest speed win for terabyte-scale cataloging)
- [ ] Store chosen rotation in model metadata (`model.json`) so catalog re-renders don't need repositioning
- [ ] Scale reference figure / true-scale rendering (handover roadmap; current look normalizes size)
- [ ] Measured geometry into model.json (NOT a separate sidecar): dimensions mm, part count, chosen rotation — computed at render/parse time, stored in the same model.json the 3pk manifest and scanner already round-trip, so it becomes searchable catalog metadata for free
- [x] Promo overlay compositing: branding (logo image + title/credit text) now bakes into the output PNG after Blender finishes — the webview composites on a canvas (same font engine as the preview = guaranteed WYSIWYG; no bundled fonts, no Rust glyph layout), Rust writes it back atomically with PNG-magic + exists guards. Ink color auto-picks dark/light by sampling the pixels under the text.
- [ ] render_mini.py uses `use_nodes` (deprecated, removal in Blender 6.0) — needs a tweak when 6.x lands
- [ ] Parse STLs in a Web Worker: mergeVertices on million-triangle minis still freezes the main thread during load (the overlay paints now, but a worker would keep the UI fully responsive)

- [ ] ARCHITECTURE: Currently we are only storing the paths in the json, but in doing so also reduce the info available in the UI. The data in the UI should be complete. From creation dont throw away the data too soon and when revisiting compose the UI info from the json PLUS the underlying jsons.
- [x] Add checkboxes to release fields to store the field data permanently like settings (so creators dont have to type in their own name every time for example) — "Remember for future releases" on Designer; stored in settings as `release_field_defaults` (a map, so more fields are one-liners)
- [x] ~~use TAR+Zstd for local compression and cataloging~~ superseded by ZIP+Zstd (2026-07-07): the pack archives keep the ZIP container (existing reader/writer, per-entry random access for selective extraction) with Zstd-compressed entries — same compression win, no new format
- [x] default releasedate current? — MonthYear form field now defaults to the current month
- [x] recover/continue mode (quick so testing becomes less tedious!) — draft (staged models + step + release) snapshots to localStorage from the store; unsaved details form mirrors separately; both restore on launch with a toast
- [ ] Combine safety: group_renames match scanner group names globally — combining a generically-named group ("Spear") can capture same-named groups from other releases. Scope renames (e.g. per release subtree) or warn when a source name is ambiguous
- [x] On-disk normalizer/cleaner v1 (2026-07-05): "Clean up…" in the catalog toolbar plans read-only, shows every move for review, then applies in chunks with progress. Canonical layout: `designer / YYYY-MM release / model / Supported|Unsupported [/variant]`, poses by filename only (merged pose dirs get the pose baked into names + file-level pose metadata). Wholesale base-dir renames when safe (hardlink-preserving; extras/renders travel), authoritative model.json written into every leaf (rescan re-derives the catalog with zero heuristics), index re-keyed live, emptied dirs swept, cross-volume moves refused loudly. Respects the designer facet so the NAS can be cleaned one designer at a time.
- [ ] Normalizer follow-ups: release builder should emit the same canonical layout (drift prevention half of the plan); per-file VARIANT assignments in dump folders currently stay in one dir (variant lives in metadata — materializing variant subdirs is a later pass); needs a real-NAS shakedown before bulk use
- [ ] Easter-egg minihoard integration: if minihoard is installed alongside Plinth, a minihoard menu quietly appears in Plinth (detect the binary at startup). Deliberately NOT advertised in Plinth's readme/website — only minihoard's docs mention the Plinth integration
- [ ] Compressed-at-rest option: after cleanup, keep each model bundled on disk (pack/unpack on demand) to save space while staying workable — the finalize_release compression path is the seed tech
  - [x] Phase 1 — explicit pack/unpack (2026-07-07): per-member-folder `model.plinthpack` (ZIP+Zstd, blake3-verified before originals are deleted) + `pack.json` sidecar the scanner reads instead of the archive; index stays complete (drawer/file lists/sizes work packed); Pack/Unpack in the drawer, card badge, savings in stats; packed models refuse normalize/move/dup-merge with clear hints; dup DETECTION works on packed models via sidecar checksums; bulk jobs are sequential + resumable (re-run skips finished folders)
  - [x] Phase 2 — transparent use (2026-07-07): PRINT/3D/RENDER work on packed models — `ensure_model_files` extracts only the needed entries (elided twins read from their in-zip donor), an in-memory registry + size/mtime guard makes cleanup safe (slicer-saved supports are never deleted), print modal gains the "clean up extracted files after" checkbox (15s grace so the slicer can read first), 3D cleans up on viewer close, render leaves files for the exit sweep (`RunEvent::Exit`)
  - [x] Phase 2.5 — bulk packing (2026-07-07): "Pack…" in the toolbar (designer facet → whole designer, else whole catalog) and in the card-selection bar; `get_pack_candidates` lists what a run would touch, confirm shows the count, page-level progress + cancel; per-folder atomicity makes any interrupted run resumable by clicking Pack… again
  - [x] Phase 3 — polish + review hardening (2026-07-07): sidecar self-heal from the archive (scanner rebuilds a lost/corrupt pack.json; vetoed while loose files exist — that's pack_model's crash-repair), dup panel greys packed copies (per-path 📦 + safe keeper/reclaimable math), pack level clamped. An adversarial /code-review then caught and fixed 5 data-safety holes: bare-vs-prefixed hash mismatch (packed files never grouped with loose twins), orphan-archive discard that could destroy a lost-sidecar archive (central-dir coverage check now decides), missing scan↔pack job exclusion (prefixed job ids), unpack truncating user-edited loose files (moved aside as "(edited)"), mark_packed flipping kept files — plus temp-name races, traversal-hardened entry paths, hash-fallback delete guards, and a batch of frontend races (extract-vs-pack toast/refresh, toggle3d selection race, render-held path protection, packed reveal fallbacks)
  - [ ] Real-NAS shakedown before bulk use: pack one small designer over SMB → cancel mid-run → re-run resumes; kill the app mid-pack → restart converges; verify hardlink probe + timestamps behave on the Synology volume
- [ ] PARKED — Remaining catalog filter facets: year (from release date), release name, and category (new concept, still TBD) — designer facet + grouping shipped 2026-07-05; revisit after the normalizer
- [x] Print modal: PRINT now opens a file picker (pre-sliced scenes pre-checked over raw geometry) and sends only the ticked files to the slicer; reveal-folder setting keeps its direct flow (reveal takes no file list)
- [x] docs/CATALOG.md refreshed: now documents shipped reality (schema v5, groups/facets, hardlink dedup, 3pk import, webview overlay bake) + the full model.json interchange contract incl. file_poses; roadmap section replaced by a pointer to todolist.md

### Duplicate handling — share, don't delete (hardlink dedup)

Duplicates across variants (e.g. one base STL repeated in 5 weapon variants) should be stored once but stay present in every variant. Mechanism: hardlinks (one inode/file-ID, a real directory entry per variant) so print, render, packing, and Finder all keep working with no resolution layer. UI never says hardlink/inode/hash — the user sees "Merge — free X MB" and "shared by N variants".

- [x] Phase 1 — inode-aware catalog: `files.file_identity` (opaque "device:inode"/volume:index string via `file-id` crate, refreshed each dup scan); `duplicate_groups()` reports `distinct_copies` so same-inode groups read as shared (reclaimable 0); `stats()` subtracts hardlink savings
- [x] Phase 2 — "merge — free X" beside "delete copies": full-hash verify both sides (refuses files that diverged since the scan), hardlink keeper → hidden temp in dup's dir, atomic rename over dup, identities refreshed in place. Per-volume probe (`supports_file_links` makes a real test link — ground truth for NAS/SMB/exFAT) gates the buttons; link-less volumes get delete-only + plain-language hint. "merge all" batches every group
- [x] Phase 3 — print-to-slicer: `print_action` setting, default `open-in-slicer` (`openPath` per file → OS-default slicer; pre-sliced .lys/.chitu beat raw stl/obj/3mf when present) vs `reveal-folder` (the old flow, for multi-slicer users); toggle in Settings
- [x] Phase 4 — 3pk checksum dedup: `compress_files` stores each unique blake3 blob once (size-prefiltered, one read for unique-size files) and returns per-entry checksums; manifest lists every name against its checksum with `component.dedup` marking elision (spec'd in docs/3PK.md); `manifest::extract_component_archive` rematerializes elided names (hardlink where supported, else copy) — wired into pack, awaiting the reconstruction UI on the import side
- [ ] Phase 5 (only if real users are stuck on link-less volumes) — virtual sharing tier: catalog-pointer fallback, viable once print resolves paths in-app; Finder browsing is the only remaining gap
- [ ] Cleanup-tooling contract: same-volume `rename()` preserves links; deleting a variant folder only drops one name; cross-volume moves split shared inodes — cleaner must re-merge on the destination side

### 3pk writer (spec + reader done; see docs/3PK.md)

The format (`manifest::Manifest` structs + BLAKE3) and the scanner-side reader
(rich `model.json` → catalog, incl. per-file poses) are built. The writer is
the remaining half and depends on the release-builder flow settling first.

- [x] Enrich what `add_model` writes to `model.json`: `StlModel` carries the full curation (variant/pose/scale/support/designer/sculptor/release_name/release_date + `file_poses`), `addToDraftRelease` stages it from the catalog (incl. per-file assignments filtered to the member's own files), `add_model` passes it through untouched, and `pack_manifest` maps it onto `ManifestModel`/`ManifestFile` (with release-level fallbacks for designer/date/name). `ManifestModel` gained the additive `variant` field.
- [x] Container `manifest.json` in `release.3pk`: one component per group with a BLAKE3 archive checksum + per-file checksums, built at pack time (`file/pack_manifest.rs`, sequenced components → manifest → 3pk in `compression_jobs`) with the Phase-4 checksum-dedup folded in. Emits null for the fields `model.json` doesn't carry yet — filled by the enrichment bullet above.
- [x] Wire the finalize flow to emit the manifest and verify round-trip: finalize packs manifest.json into release.3pk; `import_release` verifies checksums, extracts with dedup rematerialized, and the scanner restores curation from the sidecars. Tested end-to-end (pack → import → tree + curation intact; tampered archive refused).

### Modular Package Strategy Implementation

- [ ] Create a modular compression system that packages each group separately
- [ ] Create update detection system that compares local files with metadata checksums
- [ ] Add selective download functionality to only retrieve changed/new components
- [x] Reconstruction v1: opening/dropping a release.3pk imports it — confirm dialog → library dir (catalog root default) → checksum-verified extraction → auto-scan restores curation. (Selective per-component UI still open below.)
- [ ] Implement preview generation for .3dpak files (thumbnail/icon)
- [ ] Create documentation for creators explaining the modular release strategy
- [ ] Add bandwidth estimation and progress indicators for partial downloads
- [x] Implement integrity verification for downloaded components (import_release refuses any component whose archive bytes don't match the manifest's blake3)
- [ ] Create a manifest generator that builds the .3dpak file from component ZIPs

## Done

- [x] Implement file registry to associate `.3dpak` files with the application
- [x] Add events to specta setup for typesafety on the frontend
- [x] Remove 7z binaries as these trigger another flow through mac Gatekeeper
- [x] use zip crate for distribution (MVP)
- [x] process the new progress emits
- [x] tags should be lowercase always and using \_ for spaces
- [x] Replace finalize call release dir. Now uses the one written in the JSON which isnt correct (check that too)
- [x] add model list to release for fixing data or just overviewing
- [x] remove tar options and only allow chunking and local total release compression for 7z
- [x] Have the group field auto-suggest from groups already in the release
- [x] Update the models field on the metadata json when adding a file
- [x] FIX Dir not created
- [x] Replace fileinput with tauri dialogs
- [x] BUG: fileSelect shouldnt overwrite but add
- [x] Stop enter from instantly posting model
- [x] Inside the release should come the models, they shouldnt be in a "models" subdirectory first
- [x] Storage images and files releated to create release as well
- [x] Add STL-Pack logo instead of tauri logo to the taskbar
- [x] Clear filelist after save model is complete
- [x] BUG: Saving model triggers: _"Failed to save model: Error: Release directory name is missing"_
- [x] Let users edit premade models when selecting them in the release tab
- [x] Fix the finalize action, now throws a "Failed to finalize release: [object Object]
- [x] BUG: make sure tab navigation works and respects disabled tabs
- [x] Make sure the release remains in the release tab
- [x] Add uuid to models (and releases) to find them back once they move or to "symlink" them in case a model is part of multiple releases
- [x] Rebrand to Plinth; implement the "Plinth App" design overhaul (persistent sidebar nav, 4-step release stepper folding in Release/Add STL/Finalize, catalog list+grid, dark/light theme, de-purpled palette)
