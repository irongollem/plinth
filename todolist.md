# Todo List

## Doing

## To Do

### Render engine (Blender promo renders)

- [ ] Bundle or download-on-first-run a portable Blender (GPL allows shipping it as a separate CLI process; include license + source link). Detector already checks exe-relative `Resources/blender/`.
- [ ] Batch render mode: manifest of many minis in ONE Blender launch (see raw/HANDOVER.md, biggest speed win for terabyte-scale cataloging)
- [ ] Store chosen rotation in model metadata (`model.json`) so catalog re-renders don't need repositioning
- [ ] Scale reference figure / true-scale rendering (handover roadmap; current look normalizes size)
- [ ] Sidecar JSON per render (dimensions mm, parts, rotation) for future catalog search
- [ ] Promo overlay compositing: the Render studio branding panel (logo/text, position/font/size) previews live over the viewport but isn't baked into the output PNG yet — needs a font-rendering approach + Rust-side compositing design
- [ ] render_mini.py uses `use_nodes` (deprecated, removal in Blender 6.0) — needs a tweak when 6.x lands
- [ ] Parse STLs in a Web Worker: mergeVertices on million-triangle minis still freezes the main thread during load (the overlay paints now, but a worker would keep the UI fully responsive)

- [ ] ARCHITECTURE: Currently we are only storing the paths in the json, but in doing so also reduce the info available in the UI. The data in the UI should be complete. From creation dont throw away the data too soon and when revisiting compose the UI info from the json PLUS the underlying jsons.
- [ ] Add checkboxes to release fields to store the field data permanently like settings (so creators dont have to type in their own name every time for example)
- [ ] use TAR+Zstd for local compression and cataloging
- [ ] default releasedate current?
- [ ] recover/continue mode (quick so testing becomes less tedious!)

### Duplicate handling — share, don't delete (hardlink dedup)

Duplicates across variants (e.g. one base STL repeated in 5 weapon variants) should be stored once but stay present in every variant. Mechanism: hardlinks (one inode/file-ID, a real directory entry per variant) so print, render, packing, and Finder all keep working with no resolution layer. UI never says hardlink/inode/hash — the user sees "Merge — free X MB" and "shared by N variants".

- [ ] Phase 1 — inode-aware catalog: add `device`+`inode` columns to `files` (cross-platform identity via `same-file` crate); `duplicate_groups()` treats same-inode groups as already shared (reclaimable 0, mixed groups = size × (distinct inodes − 1)); `stats()` sums distinct inodes
- [ ] Phase 2 — "Merge" action in duplicates UI beside Reclaim: verify keeper hash, hardlink keeper → temp name in dup's dir, atomic rename over dup; refresh inode in catalog. Per-volume capability probe (create a test hardlink at scan time — ground truth for NAS/SMB/exFAT); volumes without support get delete-only + plain-language hint. Optional one-click "Optimize library" that merges all groups
- [ ] Phase 3 — print-to-slicer: `printAction` setting `open-in-slicer` (opener plugin `openPath` per file → OS-default slicer) vs `reveal-folder` (current). Mind the plugin-opener version pin
- [ ] Phase 4 — 3pk checksum dedup: component archives store each unique blake3 blob once; manifest lists all names against the same checksum (fold into v1 spec + docs/3PK.md while there are no external readers); import materializes first name, hardlinks the rest where the destination volume supports it, else copies
- [ ] Phase 5 (only if real users are stuck on link-less volumes) — virtual sharing tier: catalog-pointer fallback, viable once print resolves paths in-app; Finder browsing is the only remaining gap
- [ ] Cleanup-tooling contract: same-volume `rename()` preserves links; deleting a variant folder only drops one name; cross-volume moves split shared inodes — cleaner must re-merge on the destination side

### Modular Package Strategy Implementation

- [ ] Create a modular compression system that packages each group separately
- [ ] Create update detection system that compares local files with metadata checksums
- [ ] Add selective download functionality to only retrieve changed/new components
- [ ] Design reconstruction tool UI for end users to assemble downloaded components
- [ ] Implement preview generation for .3dpak files (thumbnail/icon)
- [ ] Create documentation for creators explaining the modular release strategy
- [ ] Add bandwidth estimation and progress indicators for partial downloads
- [ ] Implement integrity verification for downloaded components
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
