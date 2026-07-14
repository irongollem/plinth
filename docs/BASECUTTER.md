# Base Cutter — Implementation Plan

Scenic bases, but industrialised. A designer sculpts one continuous patch
of landscape — a rectangle or blob of terrain a fair bit larger than any
single base. Plinth supplies the cutters: the user drags standard base
footprints over the landscape, the tool cuts each plug out, seats it on a
standard tapered wargaming plinth, and exports a watertight, printable STL. One
sculpt yields dozens of unique bases; the designer never models a base rim
again.

The name fits. This is the feature the app was named after, apparently.

Base Cutter is a **new tool in the sidebar, directly below Render**. It
shares Render's spine — headless Blender, embedded script, stdout
protocol, typed progress events — but is its own tab, own module, own
script.

## Why this is cheap for us

The hard 90% — robust booleans on messy meshes, manifold repair, STL
export — is exactly what Blender does natively, and Plinth already ships
the full headless pipeline: detection + managed 5.1.2 install
(`render/provision.rs`), process spawning (`render/engine.rs::new_command`),
embedded scripts materialised at runtime (the `include_str!` pattern), and
batch jobs that stream machine-readable stdout tokens into typed events
(`render/batch.rs`). The cutter is **a second embedded Python script plus
a placement UI**. No CSG code in Rust, ever.

The placement UI is also cheaper than it first looks: `StlViewport.vue`
already proves out three.js + the STL-decode worker
(`stlGeometry.worker.ts`). The cutter view reuses that machinery with a
top-down orthographic camera — no Blender render needed just to *see* the
landscape.

## Cutters are data, not code

Every cutter is `kind + dimensions`. Three kinds cover the entire range
of standard base shapes:

| kind      | params        |
| --------- | ------------- |
| `circle`  | diameter      |
| `ellipse` | major × minor |
| `rect`    | w × d         |

The standard library ships as a table — a Rust const in the new
`basecutter` module, served to the frontend by a `get_cutter_library`
command so specta keeps the types honest. Because cutters are data,
**custom shapes are a later feature, not a later rewrite**: a user-defined
size is just a new row, and an arbitrary-outline cutter (traced polygon,
maybe SVG import) is one new `kind` handled in the script — the whole
pipeline downstream of "build the cutter prism" is shape-agnostic. Nothing
in v1 may assume the kind enum is closed.

### Seed library (the common wargaming sizes)

- **Rounds**: 25, 28.5, 32, 40, 50, 60, 100 mm — plus the larger
  rounds (80, 90, 130, 160 mm) behind the same mechanism.
- **Ovals**: 60×35, 75×42, 90×52, 105×70, 120×92, 170×105 mm.
- **Squares / rectangles** (rank-and-flank systems): 20, 25, 30, 40, 50 mm
  squares; 25×50 cavalry; 50×100 chariot. Corners are **sharp**, never
  rounded — these bases rank up edge-to-edge into unit blocks.

> Verify the exact table against off-the-shelf bases before it
> freezes — the lists above are seed data, not gospel.

## The plinth

The de-facto industry standard: **3.7 mm tall, tapered, and nominal size
is the BOTTOM face**. Caliper-measured off a real 32 mm round (±0.2 mm):
32 mm at the table, 30 mm on top, 3.7 mm tall, 1.2 mm wall — so the inset
is 1.0 mm and the taper ~15°.
A real base is widest at the table and slopes inward going up: a "25 mm
square" measures 25 mm where it touches the table, and its top face is
smaller by twice the taper inset. Ranked square bases touch — and tile
flush — at their bottom edges, which is exactly how real unit blocks
behave.

Consequences, all deliberate:

- **The cut footprint is the top face, not the nominal.** The script
  derives it: `cut = nominal − 2 × inset`, where
  `inset = height × tan(taper)`. A 25 mm square cutter therefore cuts a
  plug slightly under 25 mm. One function owns this derivation
  (`top_face_of(nominal, plinth)`) so the script, the viewport overlay,
  and a future render-tool consumer can never disagree.
- The plug wall meets the plinth's top rim exactly: clean
  vertical-to-taper seam, no ledge.
- Walls lean inward going up — no FDM overhang.
- The placement overlay in the viewport draws the **nominal** outline
  (where the base will stand); the derived cut line can be shown as an
  inner stroke.

Profile is parametric (`height: 3.7 mm`, `taper: 15°`), defaults taken
from that measurement.

### Hollow, with magnet mounts

Plinths are **hollow by default** — an open-bottom shell (tapered wall +
top plate), which is what real bases are, saves material, and prints
without support (the "ceiling" is the top plate, bridged at the wall).
Solid stays available as a flag.

Magnet mounts are designed in from the start because hollowing dictates
their shape: a cylindrical boss hanging from the top plate's underside,
reaching the bottom plane, with a downward-opening pocket. The magnet
glues in flush with the bottom rim — zero gap to the steel tray, maximum
pull. Pocket = magnet diameter + fit clearance; bigger bases suggest
bigger neodymium magnets. Suggested pairing lives in the cutter table
(data again, user-overridable): small bases one 5×1 mm, mids 6×2 mm,
large 10×2 mm, big ovals two pockets. Seed values — verify against the
magnets people actually buy before freezing.

## The cut pipeline (one Blender run per job, N cuts per run)

Blender cold-start costs seconds, so one run processes the whole
placement set — same reasoning as batch render. The script reads a JSON
job file (landscape path, plinth params, list of placements) rather than
a per-cut CLI, and per cut:

1. **Duplicate** the imported landscape (import once per run).
2. **Build the cutter**: the footprint extruded through the landscape's
   full height at the placement's x/y/rotation.
3. **Boolean intersect** (exact solver) → the terrain plug. The plug
   still carries the landscape's flat underside ("carrier" thickness),
   which must not become base height.
4. **Seat** the plug: find the lowest point of its *top* (sculpted)
   surface — raycast a grid down inside the footprint — and sink the
   plug so that point sits exactly on the plinth's top plane. Trim
   everything below that plane (boolean with a half-space box), so the
   finished base is plinth height + terrain relief only. Side walls then show the
   terrain's height profile around the rim, like a hand-made scenic base.
5. **Generate the plinth** (top face = derived cut footprint); **union**.
   The trim leaves the plug reaching ~0.2 mm *into* the top plate so the
   union sees two overlapping solids — two solids merely kissing on a
   shared plane can hand the exact solver a non-manifold seam.
6. **Cleanup**: merge-by-distance, recalc normals, manifold check.
7. **Export STL**, print a machine-readable stdout token, next placement.

Progress protocol clones batch render: `TOKEN {json}` lines on stdout
(see the pinned interfaces below), parsed by the Rust side into events; a stdout tail ring buffer for post-mortems, child
spawned with `kill_on_drop`, cancel by job id.

An up-front **validation pass** in the same run gates the whole job:
manifold (or voxel-remeshable), roughly flat bottom, minimum thickness,
sane scale (mm, Z-up). The validation rules double as the published
"cuttable landscape" spec for designers (→ CREATORS.md, phase 5).

## Backend plan

New module `src-tauri/src/basecutter/`, mirroring `render/`:

- `cutters.rs` — `CutterKind` (`Circle` / `Ellipse` / `Rect`, open for
  extension), `Cutter`, `PlinthParams` (height: 3.7, taper: 15), `Placement`
  (cutter + x/y mm + rotation deg), the seed library const, and
  `get_cutter_library`.
- `job.rs` — job JSON serialisation, spawn via `render::engine`
  detection + `new_command`, the stdout parse loop, cancellation. Reuses
  the render module's Blender plumbing rather than duplicating it; if
  that needs `engine.rs` items to go `pub(crate)`, that's the correct
  change, not a copy.
- `commands.rs` — `get_cutter_library`, `start_base_cut`,
  `cancel_base_cut`.
- `resources/base_cut.py` — the embedded script, materialised through
  the same always-overwrite pattern as `render_mini.py` (same stale-copy
  trap, same fix).

New event `BaseCutStatus` in `models/events.rs` (started / validating /
cut progress / per-cut done / failed / job done — shaped like
`BatchRenderStatus`). Register commands + event in
`lib.rs::create_specta_builder`; `cargo test` regenerates `bindings.ts`.

## Frontend plan

- **Tab**: add `"basecutter"` to the `Tab` type in `releasesStore.ts`,
  the switch in `App.vue`, and a Sidebar entry **below Render**.
- **`views/BaseCutter.vue`**: landscape picker (file dialog first;
  catalog integration later), cutter palette from `get_cutter_library`,
  plinth options, placement list, output folder, "Cut N bases" + progress.
- **`components/LandscapeViewport.vue`**: three.js, top-down orthographic
  camera, landscape loaded through the existing `stlGeometry.worker.ts`.
  Placements are overlay outlines (three.js line loops at the terrain's
  max-Z — correct in a top-down ortho view): drag to move (raycast onto
  the mesh), rotate, duplicate, delete. Overlap between placements is 2D
  shape math → warning badge, not a hard block (overlapping cuts are
  geometrically valid — each cut is independent — just usually
  unintended).
- **`composables/useBaseCut.ts`** — mirrors `useBatchRender` /
  `useRenderStatus`: subscribe to `BaseCutStatus`, expose job state.

Shaded top-down geometry gives height cues for free; a 3D preview of a
finished base is polish, not foundation.

## Synergy: standard bases in the Render tool

A rendered mini on empty ground gives no sense of size; the same mini on
a 32 mm round is instantly legible to any hobbyist — the standard base is
the scale reference the whole hobby already shares. Once the parametric
plinth generator exists, the render script can offer "place on a standard
base" (auto-suggested from the model's footprint, user-overridable)
essentially for free.

Design consequence for phase 1, not a feature of it: write the plinth
generator in `base_cut.py` as a **self-contained function** (footprint +
plinth params in, mesh out, no job-global state) so `render_mini.py` can
lift it verbatim later. The cutter library command is already
tool-agnostic, so the Render UI can reuse it for the size picker.

## Risks

- **Designer mesh quality** is the real one. Exact-solver booleans are
  tolerant but not magic; voxel remesh is the fallback, the validation
  pass is the gate. Test landscapes get generated *with Blender itself* —
  imported junk meshes fake unrelated symptoms (see the inverted-normals
  incident).
- **Performance**: booleans on multi-million-triangle sculpts take
  seconds-to-minutes per cut. Acceptable as a batch job with per-cut
  progress; decimation stays available as a later option.
- **Licensing**: cutting for personal printing sits squarely inside
  personal-use licences; *sharing* a cut STL does not. Exports stay
  local/catalog-bound; no share path for cut output.

## Pinned interfaces

Implementation must not improvise these; change them here first.

```text
// commands (specta; bindings regenerate via `cargo test`)
get_cutter_library() -> Vec<Cutter>
start_base_cut(job: BaseCutJob) -> Result<String /* job_id */>
cancel_base_cut(job_id: String) -> Result<()>

// core types (basecutter/cutters.rs)
CutterKind = Circle { diameter_mm } | Ellipse { major_mm, minor_mm }
           | Rect { width_mm, depth_mm }          // open — more kinds later
Cutter     = { id, label, kind, magnet: Option<MagnetSpec> }  // dims are NOMINAL (bottom face)
MagnetSpec = { diameter_mm, height_mm, count }    // suggested pairing, user-overridable
PlinthParams = { height_mm: 3.7, taper_deg: 15.0,  // from a measured real base
                 hollow: true, wall_mm, top_mm,
                 magnet: Option<MagnetSpec> }     // None = no pocket
Placement  = { cutter: CutterKind, x_mm, y_mm, rotation_deg }
BaseCutJob = { landscape_path, placements: Vec<Placement>,
               plinth: PlinthParams, out_dir }

// script stdout protocol (one `TOKEN {json}` per line, parsed by job.rs —
// same shape as render_mini.py's BATCH_* tokens; base_cut.py is the
// source of truth for the payloads)
VALIDATING | VALIDATED {…} | VALIDATION_FAILED {…}
CUT_START {"index":i} | CUT_DONE {"index":i,"out":…,"dims_mm":[…],"manifold":…}
CUT_FAILED {"index":i,"reason":…} | JOB_DONE {"total":N,"ok":n}
```

The script receives the job as a JSON file (path after `--` in the
Blender CLI, same convention as `render_mini.py`), not as flags per cut.

## Phases (commit-sized, in order)

1. **Script spike** — hand-run `base_cut.py` against the local 5.1.2:
   Blender-generated test landscape + one placement → `base.stl`;
   confirm the measured profile (3.7 mm, 15°). *Done when*: a printed
   base has a clean plug/plinth seam, correct nominal footprint at the
   table, and total height = 3.7 mm + relief (seat logic proven).
2. **Cutter library** — `basecutter/cutters.rs`: types, seed table,
   `top_face_of`, `get_cutter_library`. *Done when*: unit tests pin the
   table and the nominal→cut derivation.
3. **Job pipeline** — embed the script, `job.rs` + commands +
   `BaseCutStatus`, registration in `lib.rs`. *Done when*: a job started
   from a test harness (no UI) emits the full event sequence, produces N
   STLs, and cancel kills the child mid-run.
4. **Tab + viewport** — sidebar entry, `BaseCutter.vue`,
   `LandscapeViewport.vue` with single-placement drag/rotate. *Done
   when*: one cut runs end-to-end from the UI with live progress.
5. **Multi-cut + polish** — placement list, duplicate/overlap warnings,
   plinth options UI, validation surfacing, export-into-catalog.
6. **Later**: placement generators — one click for a 5×2 regiment of one
   cutter, or "N random bases of size X" auto-scattered without overlap
   (pure frontend: the job already takes a placement list, and every
   regiment member gets unique terrain because it's cut from a different
   spot); custom cutter shapes (new `kind`s + editor), standard
   bases as scale reference in the Render tool (lift the plinth function
   into `render_mini.py`), magnet-hole recess, hollow plinth, rim
   texture, designer-facing landscape spec in CREATORS.md.

## Open questions

- ~~Exact taper angle~~ answered: 15° from the measured 32 mm round
  (32→30 mm over 3.7 mm). Spot-check one square base — rank-and-flank
  rims may use a different bevel.
- Where cut output lands by default: loose folder first; catalog
  integration once the normalizer settles?
- Does the seam want a tiny plug inset (0.1–0.2 mm) for slicer
  friendliness, or is exact-match fine? Answer with the phase-1 print.
