# Scatter — phase 7 of the Base Cutter (plan)

Sprinkle debris — bones, skulls, rocks, mushrooms, plants — onto a
landscape before cutting. See docs/BASECUTTER.md "Further horizon" for the
original sketch and the three-source policy (generated / bundled / user
library) with its license rules; this doc pins the design.

## The architectural call: scatter is a LANDSCAPE TRANSFORMER

Scatter is NOT part of the cut job. It is its own headless-Blender pass:

    landscape.stl + ScatterParams -> scatter_landscape.py -> landscape-scattered.stl

- Works on ANY landscape (generated or designer sculpt), because it never
  assumes a heightfield — placement is raycast-from-above onto whatever
  mesh is there.
- The placement viewport shows the decorated STL — you place cutters over
  the ACTUAL scatter, no surprises after the cut.
- The pinned cut-job interfaces don't change at all. A piece straddling a
  cutter boundary simply gets sliced by the intersect — a half bone at the
  rim is exactly what a hand-made scenic base looks like.
- Re-scatter never compounds: the UI keeps the UNDECORATED source path and
  always scatters from it (new seed = new debris, not debris-on-debris).

Pieces are boolean-UNIONED into the terrain (exact solver, per piece —
small meshes, cheap) so the decorated STL passes the same validation gate
as any landscape.

## Pinned interfaces

```text
// commands (basecutter/scatter.rs, riding render::engine::run_blender_lines)
get_scatter_assets() -> Vec<ScatterAsset>          // bundled set (embedded)
scan_scatter_library(dir: String) -> Vec<ScatterAsset>  // user folder; validation-gated
start_scatter(job: ScatterJob) -> Result<String /* job_id */>
cancel_scatter(job_id) -> Result<()>

ScatterAsset  = { id, label, source: "bundled" | "user", path,
                  footprint_mm, height_mm }         // dims measured at scan
PieceChoice   = { piece: Generated { kind: "pebble" | "rock" }
                        | Asset { id }, weight: f64 }
ScatterParams = { seed: u32,
                  density_per_dm2: f64,             // pieces per 100x100mm
                  scale: (f64, f64),                // random range AROUND the
                                                    // piece's canonical 28-32mm-
                                                    // scale size (see below)
                  scale_factor: f64,                // whole-pass rescale for
                                                    // non-28mm work (default 1)
                  sink_mm: (f64, f64),              // buried depth range; the
                                                    // script enforces a floor —
                                                    // see "always buried" below
                  align_to_surface: bool,           // tilt with the slope
                  max_slope_deg: f64,               // skip cliff walls
                  edge_margin_mm: f64,
                  pieces: Vec<PieceChoice> }
ScatterJob    = { landscape_path, out_path, params }
// events: ScatterStatus = Started | Progress { placed, total }
//   | Finished { out } | Failed | Cancelled — user cancel is Cancelled.
// script: resources/scatter_landscape.py, TOKEN {json} stdout lines,
//   --python-exit-code 1, job JSON after `--` — the render/base_cut/
//   gen_landscape conventions verbatim.
```

Placement algorithm (deterministic from seed): jittered-grid candidate
points (poisson-flavoured, not pure random — pure random clumps ugly),
raycast down, reject by slope/edge-margin, per-piece random pick by
weight, random yaw + scale + sink, optional align-to-normal. Generated
pebbles/rocks are noise-displaced icospheres built in-script (the boulder
lessons apply: irregular outline, varied profile — never spheres).

**Always buried**: a piece resting tangent on the surface prints as a
weak kiss-joint and snaps off the first time it's handled — and a barely
touching union is exactly the non-manifold seam the exact solver chokes
on (the plug-into-plate WELD_OVERLAP lesson). So the script enforces a
sink FLOOR regardless of what sink_mm asks for: every piece is embedded
at least max(0.4 mm, 20% of its own height) below the local surface,
measured along the sink direction. The sink_mm range randomizes ABOVE
that floor for variety (a skull half-swallowed by the mud vs one just
peeking out), never below it.

## Scale anchor: 28–32 mm heroic

Most (not all) models cut with this tool are 28–32 mm tabletop scale, so
scatter defaults are ANCHORED there and expressed in real millimetres on
the base: a humanoid skull reads right at ~4–6 mm, loose bones 3–10 mm,
mushrooms 3–8 mm, pebbles 1–5 mm, a "large rock" tops out around 12 mm.
Every bundled asset is normalized ONCE at curation to its canonical
28–32 mm-scale size (recorded in ScatterAsset.footprint_mm/height_mm);
`ScatterParams.scale` then multiplies around that canonical size, and a
global `scale_factor` (default 1.0) lets the exceptions — 15 mm gaming,
54 mm display plinths — scale the whole pass without retuning per piece.
The user-library scan applies the same lens: it warns (not blocks) when a
piece's footprint suggests it's a mini, not debris, at the current scale
factor.

## Bundled assets

Curation flows from docs/SCATTER-ASSETS.md (the license-vetted scout
list): CC0 preferred, CC-BY with credits-panel attribution acceptable —
the user picks the final set. Each admitted asset is remeshed/decimated to
a budget (≤ ~15k tris), verified manifold IN Blender, embedded via the
resources/ + materialize pattern, and listed in an in-app credits panel +
CREDITS file when attribution is owed. Total bundle budget: a few MB, not
tens. Until curation lands, the feature ships usefully with generated
pieces + the user library.

## UI (BaseCutter view)

A folded "SCATTER" section between the generator and the landscape picker
(it decorates whatever landscape is active): preset mixes as chips
(Boneyard / Rocky debris / Overgrown), density + seed + reroll up front,
the rest behind the fold (RenderAdvanced pattern). Piece mix editor:
checkbox + weight per available piece (generated kinds first, then
bundled, then user library once a folder is set — one list, source
badges). Run → progress → the decorated STL becomes the active landscape
(reload token), with "re-scatter" and "remove scatter" both operating from
the kept undecorated source path. All gating follows the house rule:
disabled with tooltip, never click-then-toast.

## Execution phases (clean-slate Sonnet agents, coordinator-managed)

- **S1 — script spike**: `scatter_landscape.py` alone; generated pebbles
  onto a generated landscape, hand-run against local Blender. _Done
  when_: a decorated STL bakes deterministically from a seed, passes the
  validation gate, and cuts cleanly end-to-end with `base_cut.py`.
- **S2 — job + commands**: basecutter/scatter.rs (embed script, job JSON,
  parse loop, cancel, events), registration, bindings, unit tests + an
  ignored real-Blender integration test. _Done when_: harness-started
  scatter emits the full event sequence and cancel kills the child.
- **S3 — UI**: the section above, incl. undecorated-source bookkeeping
  and viewport reload. Must wait for the groups/undo work to land
  (BaseCutter.vue is single-writer).
- **S4 — assets + user library**: curation from the scout list (user
  approves picks), manifold vetting, embedding, credits panel,
  scan_scatter_library with the validation gate and mm-scale sanity
  check ("a scatter piece is not a mini").

S1 can start immediately (new file, no collisions). S2 after S1 proves
the protocol. S3 after the current BaseCutter.vue work commits. S4 is
independent of S2/S3 once the scout list is approved.
