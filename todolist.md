# Todo List

## Doing

- [ ] Design `.3pk` file format specification (JSON structure with version, checksums, components)

## To Do

### Render engine (Blender promo renders)

- [ ] Bundle or download-on-first-run a portable Blender (GPL allows shipping it as a separate CLI process; include license + source link). Detector already checks exe-relative `Resources/blender/`.
- [ ] Batch render mode: manifest of many minis in ONE Blender launch (see raw/HANDOVER.md, biggest speed win for terabyte-scale cataloging)
- [ ] Store chosen rotation in model metadata (`model.json`) so catalog re-renders don't need repositioning
- [ ] Scale reference figure / true-scale rendering (handover roadmap; current look normalizes size)
- [ ] Sidecar JSON per render (dimensions mm, parts, rotation) for future catalog search
- [ ] Promo overlay: title text + designer logo + base-size badge composited on renders (save with and without), matching the formal DTL card layout
- [ ] Full in-app bundle flow: catalog → new release → render per model (image auto-returns to Add STL — that leg ships already) → finalize
- [ ] render_mini.py uses `use_nodes` (deprecated, removal in Blender 6.0) — needs a tweak when 6.x lands
- [ ] Parse STLs in a Web Worker: mergeVertices on million-triangle minis still freezes the main thread during load (the overlay paints now, but a worker would keep the UI fully responsive)

- [ ] ARCHITECTURE: Currently we are only storing the paths in the json, but in doing so also reduce the info available in the UI. The data in the UI should be complete. From creation dont throw away the data too soon and when revisiting compose the UI info from the json PLUS the underlying jsons.
- [ ] Add checkboxes to release fields to store the field data permanently like settings (so creators dont have to type in their own name every time for example)
- [ ] use TAR+Zstd for local compression and cataloging
- [ ] default releasedate current?
- [ ] recover/continue mode (quick so testing becomes less tedious!)

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
