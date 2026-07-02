# STL Mini Renderer — Handover

A headless Blender renderer that turns any printable STL into a standardized
"resin promo" image: warm resin material with subsurface scattering, a soft warm
key light on pure black with a deepened shadow side, rendered in Cycles with
denoising. Built to be called per-mini from your Rust `stl-pack` tool as it grows
into a cataloging tool.

## Files in this folder

- `render_mini.py` — the renderer. Run it with headless Blender. This is the deliverable.
- `render_mini_example.rs` — reference Rust code that shells out to Blender + the script.
- `HANDOVER.md` — this file.

## What "the look" is (already locked in)

- **Material:** warm resin, base color `(0.80, 0.54, 0.35)`, roughness `0.52`,
  subsurface scattering (weight `0.35`, reddish radius `(0.7, 0.35, 0.2)`, scale `0.12`).
  The SSS is what makes it read as a physical resin print instead of a digital model.
- **Lighting:** 3 area lights, all warm. Strong soft key, a _very low_ fill (that's
  what gives the deep shadow side / contrast), a gentle rim. Pure black world.
- **Camera:** 60mm lens, slight high angle, auto-framed to fit the model.
- **Render:** Cycles, 96 samples, denoise, GPU if available (auto-detects
  OptiX/CUDA/HIP/Metal/oneAPI, falls back to CPU). View transform = Standard.
- Output: 1600x1600 PNG, saved next to the first STL by default.

All of these live in the `LOOK` dict at the top of `render_mini.py` — tweak there.

## Prerequisites

- **Blender 4.x or 5.x.** No UI is ever shown (`-b` = background/headless). Blender
  is _portable_ — you do not need to "install" it; a downloaded Blender folder with
  the executable is enough. The script auto-handles the STL importer API difference
  between Blender 4.x (`wm.stl_import`) and older versions.
- **GPU optional.** It renders on CPU if no GPU is found, just slower.

## Quick start

From a terminal on your dev machine (with `blender` on PATH, or use its full path):

```
blender -b -P render_mini.py -- "PATH\TO\Mini.stl"
```

Multi-part mini (body + base + weapon are joined into one object):

```
blender -b -P render_mini.py -- "PATH\Body.stl" "PATH\Base.stl"
```

The PNG is written next to the first STL (`Mini.stl` -> `Mini.png`).

## Options

| Flag              | Default           | Meaning                                                               |
| ----------------- | ----------------- | --------------------------------------------------------------------- |
| positional paths  | —                 | One or more STL files that make up the mini (joined)                  |
| `--out PATH`      | `<first stl>.png` | Output PNG path                                                       |
| `--rotate x,y,z`  | `90,0,0`          | Degrees. Stands up based sculpts. See note below.                     |
| `--color r,g,b`   | `0.80,0.54,0.35`  | Resin base color                                                      |
| `--azimuth deg`   | `-15`             | Camera angle around the model                                         |
| `--elev f`        | `0.22`            | Camera height factor                                                  |
| `--zoom f`        | `1.15`            | Framing (larger = more padding)                                       |
| `--res N`         | `1600`            | Square resolution in px                                               |
| `--samples N`     | `96`              | Cycles samples (higher = cleaner, slower)                             |
| `--contact-sheet` | off               | Render a grid of candidate rotations instead of one image (see below) |
| `--sheet-cols N`  | `3`               | Columns in the contact sheet                                          |
| `--sheet-res N`   | `420`             | Per-tile resolution in the contact sheet                              |

Exit code is `0` on success, non-zero on failure — easy to check from Rust.

## Orientation note (the one gotcha)

STLs don't store a canonical "up". This library's **based creatures** (a single
figure on a round base, e.g. the Giant Newt, the Bugbear) stand up correctly with
the default `--rotate 90,0,0`. **Floating / diorama-fragment pieces** (e.g. a fairy
that's one part of a big multi-model diorama) have no natural "up" and may need a
different rotation, found by trial. For a catalog, defaulting to `90,0,0` and
allowing a per-model override in your metadata is the pragmatic approach.

Verified working example (this exact call produced a promo-quality render):

```
blender -b -P render_mini.py -- ^
  "Z:\...\giant newt\Unsupported\GiantNewt_v02.stl" ^
  "Z:\...\giant newt\Unsupported\GiantNewt_Base_v02.stl"
```

## In-tool orientation picking — Stage 1 (shipped): contact sheet

The orientation gotcha above is handled without any 3D UI work by letting the user
pick from a grid. Run:

```
blender -b -P render_mini.py -- Mini.stl --contact-sheet --out sheet.png
```

This renders the model at 9 candidate rotations into one grid image and prints a
machine-readable line to stdout:

```
CONTACT_SHEET {"cols":3,"rows":3,"tile":420,"rotations":[[0,0,0],[90,0,0],[-90,0,0],[180,0,0],[0,90,0],[0,-90,0],[90,0,90],[90,0,-90],[0,0,90]]}
```

Tiles are laid out left-to-right, top-to-bottom in exactly the order of `rotations`.
So the tool flow is:

1. Call the script with `--contact-sheet`, capture stdout, parse the `CONTACT_SHEET` JSON.
2. Show the grid image; the user clicks the tile where the mini stands up correctly.
3. Map clicked cell index -> `rotations[index]`, store it in the mini's catalog metadata.
4. For the final render, pass it as `--rotate x,y,z`.

Verified: for the Giant Newt, cell index 1 = `[90,0,0]` is the correct upright.
The candidate list lives in `CANDIDATE_ROTATIONS` at the top of `render_mini.py` if
you want to add or reorder options (keep the order stable so stored indices stay valid).

Cost: one Blender launch renders the whole sheet (9 low-res tiles), a few seconds.

## In-tool orientation picking — Stage 2 (next): live 3D viewer

The contact sheet is pick-from-9. The better long-term UX is free rotation inside the
tool, with Blender only invoked once for the final beauty render:

- **Load the STL in Rust.** Use the `stl_io` crate (or `nom_stl`) to read vertices/normals.
- **Display it.** Use a Rust GPU crate — `three-d` (easiest, has built-in orbit controls
  and a clay/matcap shader), or `bevy`, or `wgpu` directly. A simple matcap or Lambert
  clay material is enough; you don't need Cycles here — this view is only for choosing
  orientation, so it should be instant and Blender-free.
- **Let the user tumble.** Mouse-drag orbits a trackball. Provide a "this side up" action:
  the user rotates until the mini stands correctly, then confirms.
- **Capture the rotation.** Read back the object's orientation as XYZ euler degrees
  (convert the trackball quaternion to euler). That triple is exactly what `render_mini.py`
  takes as `--rotate`.
- **Persist + render.** Store the euler in the mini's catalog metadata; pass it to the
  final `blender -b -P render_mini.py -- ... --rotate x,y,z` beauty render.

Key point: the interactive step is pure Rust and real-time; Blender is only launched
once, at the end, for the final image. Both stages feed the same `--rotate` field, so
you can ship Stage 1 now and drop Stage 2 in later without changing the render pipeline.

## Rust integration

See `render_mini_example.rs`. The pattern is just `std::process::Command`:
spawn `blender -b -P render_mini.py -- <parts...> <flags...>`, check the exit status.
No Blender-Rust bindings or FFI needed. Set the `BLENDER_BIN` env var to point at a
bundled Blender, or rely on `blender` being on PATH.

## Deployment options (how end users get Blender)

1. **Bundle portable Blender (recommended).** Ship Blender's portable build (~300 MB)
   inside/next to your app and point `BLENDER_BIN` at its executable. No install, no
   admin, no UI, deterministic version. Optionally download it on first run so your
   installer stays small. Keeps full Cycles quality (the resin look depends on Cycles).
2. **`pip install bpy`.** Blender-as-a-Python-module, headless, no separate exe. Drive
   it from Python (from Rust you'd embed Python via pyo3). "Fully embedded" but still a
   ~1 GB native dependency and more brittle to build than option 1.
3. **Pure-Rust renderer (no Blender).** Not recommended — Cycles/SSS/denoise aren't
   available as a standalone Rust crate; you'd rewrite a path tracer and lose this look.

## Performance / batching

Each `blender -b` call costs ~2-4s of startup before it renders. For a whole library,
don't spawn one process per mini. Two good options:

- Render many minis in **one** Blender process (loop inside the script over a manifest).
  A `--batch manifest.json` mode is a natural next addition.
- Or keep per-process calls but run several in parallel (bounded by GPU/CPU).

## Roadmap for the cataloging tool

- **Orientation picking** — Stage 1 (contact sheet) is shipped in `render_mini.py`
  (`--contact-sheet`). Stage 2 (live 3D orbit viewer in the Rust app) is specced in the
  two "In-tool orientation picking" sections above; both feed the same `--rotate` field.
- **Batch mode** — feed a JSON/CSV manifest (paths, per-model rotation, color) and
  render the whole library in one Blender launch. Biggest speed win.
- **Scale reference figure ("the same dude in the corner").** Design decision required:
  the current script normalizes every model to the same on-screen size, which _discards_
  real scale. For a true scale reference you keep the model's real dimensions (assume STL
  units = mm) and composite a fixed-size reference (a standard 28/32mm human silhouette,
  or a ruler/base footprint) at true relative scale. Then a 160mm diorama and a 25mm mini
  look correctly different next to the same reference. This is a deliberate change to the
  framing/normalize step — worth doing for a catalog.
- **Thumbnail + full-res pair** — render a small catalog thumbnail and a full-res detail
  shot in the same pass.
- **Consistent naming/metadata** — emit a sidecar JSON per render (dimensions in mm,
  part list, rotation used) to power catalog search/filtering.

## Contact / regenerating

The whole recipe is data at the top of `render_mini.py` (`LOOK` dict) plus the small
functions below it. Everything is adjustable there without touching the render logic.
