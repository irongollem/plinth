# Scatter — sprinkle debris (this spike: generated pebbles/rocks, plus
# curated Asset pieces) onto a landscape STL as a STACK of independent
# passes. See docs/SCATTER.md — this file implements the pinned "landscape
# TRANSFORMER" pass: landscape.stl + layers -> scatter_landscape.py ->
# landscape-scattered.stl. It is NOT part of the cut job (docs/SCATTER.md
# "The architectural call"); base_cut.py runs unchanged against whatever STL
# is handed to it, decorated or not.
#
# Job JSON (path after `--job`), all lengths in mm, landscape units = mm,
# Z-up (same conventions as gen_landscape.py / base_cut.py):
# {
#   "landscape_path": "/path/to/landscape.stl",
#   "out_path": "/path/to/landscape-scattered.stl",
#   "asset_paths": {"skull-hesperocyon": "/path/to/skull-hesperocyon.stl"},
#                                    # id -> absolute STL path for every
#                                    # {"Asset": {"id": ...}} piece referenced
#                                    # by ANY layer's pieces below
#                                    # (docs/SCATTER.md "Bundled assets" /
#                                    # basecutter::scatter's
#                                    # resolve_asset_paths — this map is the
#                                    # UNION across every layer's ids). Rust
#                                    # resolves bundled-vs-user-library and
#                                    # injects this map at job-write time —
#                                    # mirrors how base_cut.py's job JSON gets
#                                    # a "cut" footprint injected per
#                                    # placement; this script NEVER guesses a
#                                    # path from an id itself. Omitted/empty
#                                    # when no layer uses an Asset piece.
#   "layers": [                     # a STACK, not a single pass
#                                    # (docs/SCATTER.md "Layers") — each
#                                    # entry is a full param set, applied
#                                    # in order; ONE layer is the common
#                                    # case, but the list itself must be
#                                    # non-empty (Rust's start_scatter
#                                    # rejects an empty list before this
#                                    # script ever runs; this script checks
#                                    # again as defense in depth against a
#                                    # hand-edited job file — see scatter()).
#     {
#       "seed": 7,
#       "density_per_dm2": 25.0,        # pieces per 100x100mm (a "dm2" here)
#       "scale": [0.85, 1.15],          # random range AROUND each piece kind's
#                                        # canonical 28-32mm-scale size (see
#                                        # CANONICAL_MM below) — docs/SCATTER.md
#                                        # "Scale anchor: 28-32mm heroic"
#       "scale_factor": 1.0,            # whole-pass rescale for non-28mm work
#       "sink_mm": [0.0, 0.6],          # desired buried-depth range; the script
#                                        # enforces a FLOOR regardless (see
#                                        # "always buried" below) — this range
#                                        # only adds variety ABOVE that floor
#       "align_to_surface": true,       # tilt each piece to the local normal
#       "max_slope_deg": 55.0,          # reject candidates on steeper ground
#       "edge_margin_mm": 3.0,          # keep clear of the landscape's outer
#                                        # planar (x,y) bounding box
#       "clump": 0.0,                   # 0..1, default 0 — bias candidate
#                                        # placement toward seeded cluster
#                                        # centers instead of the even
#                                        # jittered grid; see build_candidates'
#                                        # own "clumping" comment block for the
#                                        # algorithm. 0 = today's even spread,
#                                        # EXACTLY (no warp step runs at all)
#       "pieces": [
#         {"piece": {"Generated": {"kind": "pebble"}}, "weight": 0.6},
#         {"piece": {"Generated": {"kind": "rock"}},   "weight": 0.4}
#         # also: {"Generated": {"kind": "twig"|"grass"|"mushroom"}} —
#         # see build_twig_piece/build_grass_piece/
#         # build_mushroom_piece
#       ]
#     }
#     # ... additional layers, same shape, own seed/density/pieces ...
#   ]
# }
#
# `piece` is externally tagged, one key names the source — matches Rust's
# default serde derive (no #[serde(tag=...)]) for the PieceChoice.piece enum
# pinned in docs/SCATTER.md: {"Generated": {"kind": "pebble"|"rock"}} or
# {"Asset": {"id": "..."}}. An Asset piece resolves via asset_paths (see
# above): the id must be a key in that map AND the path it names must exist
# on disk, checked in validate_pieces() for EVERY layer BEFORE any Blender
# work — an unknown id or a missing file is a clear SCATTER_FAILED, never a
# guess, regardless of which layer in the stack referenced it. Once
# resolved, an Asset piece is imported ONCE per unique id ACROSS THE WHOLE
# STACK (see AssetTemplateCache, shared by every layer) and instanced per
# placement with the SAME yaw/scale(scale range x scale_factor)/sink-floor
# treatment as a generated piece (see build_asset_piece) — the piece's own
# imported size at scale 1.0 IS its canonical size (every bundled asset is
# normalized to it at curation, docs/SCATTER.md "Scale anchor"), so there is
# no separate CANONICAL_MM lookup for asset pieces the way there is for
# pebble/rock.
#
# stdout protocol (parsed by basecutter::scatter):
#   SCATTER_START
#   SCATTER_PROGRESS {"placed":i,"total":N}
#     (placed/total span the WHOLE STACK, not one layer — docs/SCATTER.md
#     "Layers": "SCATTER progress spans all layers". total is known up
#     front: every layer's candidates are raycast-accepted before any piece
#     in ANY layer is placed — see scatter()'s two-pass structure.)
#   SCATTER_DONE {"out":...,"placed":N,"manifold":bool,
#                 "non_manifold_edges":N,"total_edges":M,"shells":S,
#                 "layers":L}
#     (manifold/edge counts are measured on the EXPORTED file via a
#     re-import — see roundtrip_check — so they match what base_cut.py's
#     validate will see; mild non-manifold-ness is warning-grade data here,
#     the same lenient policy as base_cut.py's validate, and the job only
#     FAILS above MAX_NON_MANIFOLD_RATIO. The Rust DonePayload currently
#     parses out/placed/manifold/shells/layers and ignores the extra count
#     fields.
#     "shells" (docs/SCATTER.md "Pieces are placed as LOOSE SHELLS"): pieces
#     are never boolean-unioned into the terrain, so the exported file is
#     terrain + one closed shell per placed piece ACROSS EVERY LAYER —
#     shells == 1 + placed by construction, but the number reported here is
#     RE-MEASURED on the same re-imported file the edge counts come from
#     (a flood-fill over face adjacency, see count_shells), not assumed —
#     the same "report what downstream will actually see" discipline
#     roundtrip_check already applies to manifoldness. Because nothing
#     unions anymore, non_manifold_edges is expected to be exactly 0 by
#     construction — see "Loose shells, not unions" below.
#     "layers" is NEW (docs/SCATTER.md "Layers"): the number of layers in
#     the stack that just ran, i.e. len(job["layers"]) — a plain count, not
#     re-measured, since there is nothing to re-derive it FROM downstream
#     the way shells/manifoldness are re-derived from the exported file.)
#   SCATTER_FAILED {"reason":...}
#   With --debug (after `--job <path>`), one extra line per placed piece,
#   in placement order (layer 0's pieces first, then layer 1's, etc.):
#   SCATTER_PIECE {"index":i,"layer":L,"kind":...,"x_mm":...,"y_mm":...,
#                  "z_mm":...,"yaw_deg":...,"size_mm":...,"floor_mm":...,
#                  "embed_depth_mm":...,"aligned_deg":...}
#   (embed_depth_mm is the ACTUAL enforced sink; floor_mm is what the "always
#   buried" rule required — embed_depth_mm >= floor_mm always, the sink-floor
#   proof this line exists for. "layer" is NEW: which entry of job["layers"]
#   (0-indexed) placed this piece — this is what the independence test reads
#   to isolate one layer's pieces regardless of how many other layers ran
#   alongside it. "index" stays a GLOBAL running count across the whole
#   stack, not reset per layer — since layers are processed in stack order,
#   layer 0's indices are always 0..N0-1 whether or not later layers exist,
#   which is itself a visible symptom of independence: adding layer 1 never
#   renumbers layer 0's pieces.)
# Exit code: 0 on success; a caught failure prints SCATTER_FAILED then
# sys.exit(1); an uncaught exception propagates and Blender's own
# `--python-exit-code 1` (the render_mini.py/base_cut.py/gen_landscape.py
# convention) turns it into a non-zero exit regardless.
#
# Layers — independent stacking (docs/SCATTER.md "Layers — build the debris
# up, peel it back"): every layer raycasts against the SAME BVH, built ONCE
# from the landscape mesh as originally imported (src_bm/bvh in scatter()) —
# placement never touches the terrain, so that BVH is never stale for a
# later layer. Each layer also gets its OWN random.Random(layer["seed"])
# stream, used for nothing but that layer's own candidate jitter and
# per-piece draws. The consequence: layer K's whole placement — which
# candidates are accepted, and every position/yaw/size/sink drawn for them —
# is a pure function of (terrain, layer K's own params), with no dependency
# on any other layer's params, presence, or position in the stack. Candidates
# for EVERY layer are computed in a first pass (before any piece in any
# layer is built) specifically so SCATTER_PROGRESS's total is known from the
# very first tick and so the two passes can never accidentally interleave
# rng draws between layers. This is verified directly against real Blender
# by scatter.rs's ignored integration test: run layer 0 alone, then layer 0
# plus a second layer, and diff layer 0's SCATTER_PIECE debug positions —
# they must match exactly.
#
# Placement algorithm (deterministic from seed — docs/SCATTER.md "Placement
# algorithm"): jittered-grid candidate points (poisson-flavoured, not pure
# random — pure random clumps ugly; same cell-jitter idea as
# gen_landscape.py's cobblestone Voronoi cells), raycast straight down onto
# the landscape via a BVH tree, reject by slope/edge-margin, then for each
# SURVIVING candidate (in the same fixed row-major grid order): pick a piece
# kind by weight, build a fresh noise-displaced icosphere for it (irregular
# outline/profile, never a sphere — the boulder lesson from
# gen_landscape.py's boulder layer, applied in 3D instead of on a
# heightfield), random yaw, random scale around the kind's canonical size,
# sink below the local surface (floor-enforced), optional align-to-normal,
# then keep it as its own LOOSE SHELL (see "Loose shells, not unions" below)
# instead of boolean-unioning it into the terrain.
#
# Loose shells, not unions (docs/SCATTER.md "Pieces are placed as LOOSE
# SHELLS"): earlier revisions of this script boolean-unioned each piece into
# the terrain as it was placed — that's exactly what commit 0654275's
# "manifold contradiction" fix was patching around (the exact solver's union
# seams grow n-gons that the STL exporter triangulates into T-junction
# slivers). Not unioning removes the whole defect class at its root instead
# of chasing it: every piece is built, cleaned, and exported as its own
# closed manifold shell (see cleanup_shell_bm), and the terrain gets the
# same per-shell cleanup once at the end — nothing ever touches another
# shell's geometry. The three wins docs/SCATTER.md names (each shell stays
# individually manifold, slicers/the cut pipeline handle overlapping shells
# natively, pieces stay IDENTIFIABLE at cut time for base_cut.py's
# `scatter_rim`) all fall out of this one change. It also makes placement
# cheap again: an EXACT boolean per piece was the one expensive op in the
# loop (see the historical tri-cost comments below); joining loose shells is
# pure mesh-data concatenation (bpy.ops.object.join), no solver involved.
#
# Determinism: EACH layer gets its OWN seeded random.Random(layer["seed"])
# stream (see parse_layer/scatter's per-layer rng), drawn from in a FIXED
# order within that layer (grid cells visited row-major; every cell draws
# its two jitter numbers whether or not it's later accepted; every ACCEPTED
# candidate then draws its piece pick / yaw / size / sink / mesh-noise
# numbers in that same fixed order). Whether a candidate is accepted depends
# only on the landscape mesh + its fixed (x,y) position, never on the RNG,
# so for the same landscape STL + same seed + same layer params the
# accept/reject pattern — and therefore the whole sequence of numbers drawn —
# is bit-identical across runs. Because layers are processed in a fixed
# order (stack order) and each owns an independent rng stream that nothing
# outside its own iteration ever touches, the SAME layers in the SAME order
# reproduce the SAME whole-stack export byte-for-byte, exactly generalizing
# the single-pass guarantee to N passes. This mirrors gen_landscape.py's
# boulder layer (one seeded stream, a fixed-order per-instance loop), not its
# stones layer (a position hash) — the mechanism scatter needs is "for each
# of N discrete items", exactly the boulders' case, not "evaluate this
# continuous field at an arbitrary point".
#
# Always buried: see docs/SCATTER.md's own section of that name. A piece
# resting tangent on the surface prints as a weak kiss-joint and is exactly
# the near-zero-overlap union the exact solver can turn into a non-manifold
# seam (the WELD_OVERLAP lesson base_cut.py's docstring names). So sink is
# never just sink_mm: floor_mm = max(0.4, 0.2 * piece_height_mm), and the
# final sink is max(floor_mm, a value drawn from sink_mm) — sink_mm only
# gets to ADD variety once it clears the floor, never to undercut it.

import json
import math
import os
import random
import sys
import traceback

import bmesh
import bpy
from mathutils import Matrix, Vector, noise
from mathutils.bvhtree import BVHTree

# ------------------------------------------------------------- constants

# Canonical sizes at the 28-32mm heroic anchor (docs/SCATTER.md "Scale
# anchor"): pebbles read right at 1-5mm, a "large rock" tops out ~12mm. Each
# kind gets ONE canonical scalar; ScatterParams.scale then multiplies AROUND
# it, and scale_factor rescales the whole pass. PIECE_SIZE_JITTER is a
# SEPARATE, always-on per-piece variety knob (real pebbles aren't all one
# size) — it is not user-facing and stacks with `scale`.
PEBBLE_CANONICAL_MM = 3.0
ROCK_CANONICAL_MM = 9.0
PIECE_SIZE_JITTER = (0.75, 1.35)

# "Always buried" floor (docs/SCATTER.md) — never below this regardless of
# what sink_mm asks for.
MIN_SINK_MM = 0.4
SINK_FLOOR_FRACTION = 0.20

# Jittered-grid candidates: fraction of a cell's half-width the center may
# drift, same convention as gen_landscape.py's _cell_center (0.7 of a half
# cell keeps stones off a graph-paper grid without letting two neighboring
# cells' jitter collide into a clump).
CELL_JITTER_FRACTION = 0.7

# Raycast headroom above the landscape's own max Z, and past its min Z, so
# the downward ray always starts above and reaches through the terrain
# regardless of local relief.
RAY_MARGIN_MM = 2.0

# Icosphere subdivision level is DERIVED PER PIECE from its final size_mm,
# not a fixed constant per kind — a fixed level (the old PEBBLE_SUBDIV=2 /
# ROCK_SUBDIV=1) bakes in a facet size that scales with the piece, so a
# 12mm rock and a 2mm pebble got the same 20/80-triangle icosahedron and
# both showed crisp polygonal facets at print scale (user report: "the
# generated rocks/pebbles are also too low poly" — unlike a render, where
# shade_smooth cosmetically hides facets, the exported STL keeps them raw
# and a resin printer's ~0.05mm layers reproduce them exactly).
#
# The model (measured empirically against Blender 5.1.2's own
# bmesh.ops.create_icosphere — see subdivision_for_size's docstring): a
# `subdivisions=N` icosphere of diameter `size_mm` has a maximum facet
# edge length of approximately
#     max_edge_mm = size_mm * ICOSPHERE_EDGE_K / 2**(N - 1)
# fit to <1% error for N in 3..8 against measured sizes 2/3/5/8/9/12mm (N=1
# is the bare icosahedron and the fit over-predicts its edge slightly,
# which only ever pushes the computed level UP, never under target — safe
# in the direction that matters).
ICOSPHERE_EDGE_K = 0.6614

# Target max facet edge at real (printed) scale — the loose (least-tris)
# end of the "fine enough a 0.05mm-layer resin printer can't crisply
# reproduce it" 0.08-0.12mm band, since bigger facets cost fewer tris and
# the cap below already gives up on hitting this exactly for large rocks.
TARGET_MAX_EDGE_MM = 0.12

# Tri cost of Blender's own `subdivisions` parameter (this is what
# create_icosphere actually builds — the "subdiv 1" base icosahedron has
# 20 tris regardless, then each level quadruples): levels 0-1=20, 2=80,
# 3=320, 4=1280, 5=5120, 6=20480. Uncapped, the size-driven formula asks
# for level 6 or more (up to 8, 327680 tris) for anything bigger than
# ~2.9mm — i.e. most rocks and a majority of pebbles in the documented
# 28-32mm-heroic canonical range (pebbles ~1.9-4.7mm, rocks ~5.7-14mm with
# the default scale/jitter) — since hitting the literal facet-size target
# at those sizes genuinely needs that many tris (see TARGET_MAX_EDGE_MM's
# comment). That's far past "cap sensibly", and measured expensive: a
# 35-piece / 120x80mm scatter (this repo's S1 job shape) went from ~6s
# wall-clock at the ORIGINAL fixed low-poly levels (rock=1/pebble=2, 20/80
# tris) to ~41s (~6.9x) with BOTH kinds uncapped at level 6 — the per-piece
# EXACT-solver boolean union (module docstring: "cheap, one small piece at
# a time") stops being cheap once every piece is 20480 tris. Landing under
# the 3x regression budget took capping BOTH kinds well below what the
# formula alone would ask for, and capping them UNEVENLY:
#   - ROCK_MAX_SUBDIV=5 (5120 tris): rocks carry the new fine noise octave
#     (see ROCK_FINE_NOISE_FRACTION), so they keep the higher of the two
#     caps — still a 256x tri increase over the original 20-tri rock, and
#     still well short of the formula's uncapped ask, but that's the
#     "give up precision, not just cheat on cost" compromise.
#   - PEBBLE_MAX_SUBDIV=2 (80 tris, the bare icosahedron's own next level):
#     lowered from an earlier 4/1280-tri cap. Turns out "meant to read
#     smooth/round" was itself the bug the task's forest-floor pass came
#     back to fix — a round, densely-subdivided pebble reads as a smooth
#     EGG once shade_smooth blurs its normals (user report verbatim), and
#     no amount of fine noise fixes a silhouette problem: shade_smooth only
#     erases NORMAL-level texture, never the actual polygonal OUTLINE a
#     coarse mesh keeps (see PEBBLE_NOISE_AMOUNT's own comment below for
#     why silhouette, not texture, is what survives smoothing). Fewer
#     facets at bigger, asymmetric noise amounts (see PEBBLE_NOISE_AMOUNT /
#     PEBBLE_SQUASH_Z just below) is what makes a "stone" instead of an
#     egg — and it costs FEWER tris, which only helps the carpet-density
#     budget (docs/SCATTER.md "hundreds of pieces").
# Measured result at these caps: ~12.2s for the same 35-piece/120x80mm job
# — ~2.0x the original, inside the "if it regresses badly (>3x)" budget.
# See this commit's report for the full before/after/cap-sweep numbers.
ROCK_MAX_SUBDIV = 5
# Pebble capped at subdiv 1 — the bare 20-triangle icosahedron. Lowered
# again (was 2, was 4 originally) as the FINAL step of the "smooth egg" fix
# the forest-floor task reported: at subdiv 2 (80 tris) a noise-displaced
# squashed pebble, once render_mini.py's shade_smooth interpolates its
# normals, still reads as a smooth ROUND BALL from top-down (verified in the
# forest acceptance render) — exactly the egg impression the task says must
# be ZERO. At subdiv 1 the facets are large enough that shade_smooth CAN'T
# fully round them away: a 20-tri icosahedron, squashed flat (PEBBLE_SQUASH_Z)
# and shoved lopsided by one dominant noise lobe (PEBBLE_NOISE_AMOUNT), reads
# as a small ANGULAR, FACETED chip of stone — the task's own "small, low,
# angular pebbles ... small + faceted" ask — and the raw STL keeps every
# facet crisp for the resin printer regardless of the render's shading. It's
# also the cheapest possible piece (20 tris), which only helps the
# carpet-density budget (docs/SCATTER.md "hundreds of pieces"). Rock stays
# high-subdiv/craggy (ROCK_MAX_SUBDIV) — it's the "big textured stone", not
# the little angular grit pebble is now.
PEBBLE_MAX_SUBDIV = 1

MIN_SUBDIV = 2  # never below "vaguely round" for a piece whose per-kind cap
                # allows it (rock) — but a per-kind cap BELOW this (pebble,
                # deliberately faceted at subdiv 1) wins, see
                # subdivision_for_size's effective-floor note.


def subdivision_for_size(size_mm, max_subdiv):
    """Pure function of size_mm (and the per-kind cap) -> Blender's
    icosphere `subdivisions` parameter — the smallest level whose modeled
    max facet edge (see ICOSPHERE_EDGE_K's comment) is <= TARGET_MAX_EDGE_MM,
    clamped to [min(MIN_SUBDIV, max_subdiv), max_subdiv]. The floor is
    min(MIN_SUBDIV, max_subdiv), NOT MIN_SUBDIV: a per-kind cap deliberately
    set BELOW MIN_SUBDIV (pebble's faceted subdiv 1 — see PEBBLE_MAX_SUBDIV)
    must actually be honored, whereas a plain clamp(level, MIN_SUBDIV,
    max_subdiv) with lo>hi would (wrongly) return MIN_SUBDIV and quietly
    undo the cap.

    Deterministic by construction: takes only size_mm (itself already
    drawn from the seeded rng stream earlier in build_generated_piece) and
    a compile-time constant — no rng access here, so it can never perturb
    the fixed-order draw sequence the module docstring's determinism proof
    depends on.
    """
    ratio = max(1.0, size_mm * ICOSPHERE_EDGE_K / TARGET_MAX_EDGE_MM)
    level = 1 + math.ceil(math.log2(ratio))
    effective_min = min(MIN_SUBDIV, max_subdiv)
    return int(clamp(level, effective_min, max_subdiv))


def facet_edge_mm(size_mm, subdiv):
    """The same model subdivision_for_size inverts, evaluated forward —
    used to scale the rock-only fine noise octave's wavelength to the
    ACTUAL facet size a piece ended up with (post-cap), so that octave's
    ripples stay resolvable by the mesh instead of aliasing against it
    (see ROCK_FINE_NOISE_* below)."""
    return size_mm * ICOSPHERE_EDGE_K / (2 ** (subdiv - 1))

# Noise displacement amplitude, as a fraction of the piece's own DIAMETER
# (size_mm). Rock rougher/craggier than pebble on purpose. Both sit well
# above what a barely-there wobble would give: render_mini.py's
# shade_smooth() runs on every imported model (base_cut plugs included —
# not something this script can or should opt out of), and smooth shading
# blurs away exactly the kind of fine, high-frequency ripple a naive "add
# some noise" first pass produces — the first two render iterations of this
# script both still read as plain spheres at print scale despite visible
# bumps in wireframe. What survives shade_smooth is large-scale SILHOUETTE
# asymmetry, which is why NOISE_FREQ below is tuned for one or two dominant
# lobes per piece (a lopsided potato), not many small ones — see
# build_generated_piece's docstring. Verified manifold post-cleanup at
# these values against the low-poly rock subdivision — see the S1
# verification render.
# Bumped from an earlier 0.30 (see PEBBLE_MAX_SUBDIV's comment above: the
# "smooth egg" fix is coarser facets PLUS a more asymmetric coarse lobe, not
# a smoother one) — now equal to rock's, so a low-subdivision pebble gets a
# genuinely lopsided, angular silhouette instead of reading as a shrunk
# smooth sphere.
PEBBLE_NOISE_AMOUNT = 0.42
ROCK_NOISE_AMOUNT = 0.42

# Rock-only THIRD octave: genuine surface grain on top of the two lobes
# above (which must stay — they're what reads as a silhouette at arm's
# length; this is deliberately NOT a replacement for them). Now that
# subdivision_for_size gives rocks enough vertices to carry it, a fine
# high-frequency ripple reads as craggy rock texture at print/macro scale
# instead of getting erased by render_mini.py's shade_smooth (the same
# erasure the module docstring's noise-tuning lesson already names — this
# octave is fine enough that shade_smooth WILL blur it in the render, same
# as any real fine surface texture does under smooth shading; the exported
# STL is what resin printing sees, and the STL keeps every facet raw).
# Pebbles do NOT get this octave — river pebbles are smooth by nature, and
# the task is exactly to make their silhouette a curve, not add texture.
#
# Amplitude: "a few %" of the piece's own diameter (deliberately small —
# this is texture, not shape). Wavelength: scaled off the piece's ACTUAL
# post-cap facet size (facet_edge_mm), not a fixed mm value, so the ripple
# never aliases against the mesh that carries it — FINE_NOISE_FACET_MULT
# facet-edges per wavelength keeps several triangles per ripple cycle
# regardless of how coarse or fine subdivision_for_size ended up landing a
# particular piece (rocks near the ROCK_MAX_SUBDIV cap have coarser facets
# than the size-driven formula would ideally want — see that constant's
# comment — so the safety margin matters most exactly there).
ROCK_FINE_NOISE_FRACTION = 0.035
FINE_NOISE_FACET_MULT = 4.0

# Vertical squash range per kind: a resting stone/pebble is flatter than a
# sphere, never a perfect ball (the "never spheres" requirement starts here,
# before noise is even applied). Rock squashes flatter on average (bigger
# stones settle more) but the ranges deliberately overlap — kind alone
# should never fully determine silhouette. Pebble's range lowered (was
# 0.55-0.85) alongside PEBBLE_MAX_SUBDIV/PEBBLE_NOISE_AMOUNT above — a
# flatter, lower-profile pebble reads as a small embedded stone lying in the
# litter rather than a ball sitting proud of it (task: "small, low, ANGULAR
# pebbles — not smooth eggs").
PEBBLE_SQUASH_Z = (0.35, 0.60)
ROCK_SQUASH_Z = (0.45, 0.78)
ASPECT_XY_RANGE = (0.80, 1.30)

# ------------------------------------------------------------- lies_flat
#
# Per-kind placement behavior (forest-floor fix): fallen debris rests
# roughly HORIZONTAL on the surface — free yaw, small random roll, never
# planted upright like a pole. A piece's final orientation is entirely
# decided by place_piece's `axis = normal.normalized() if align_to_surface
# else world Z` then rotating the piece's LOCAL +Z onto that axis — so
# "lies flat" is not a flag place_piece branches on, it falls out of WHICH
# of a piece's own axes is built to BE local +Z:
#   - A piece built with its long/salient axis along local Z (the round
#     pieces' "up", the ORIGINAL build_twig_piece's spine — "a twig anchors
#     into the terrain at its base and points up out of it", literally the
#     planted-pole bug) ends up standing, because align_to_surface always
#     tilts local Z toward the surface normal (near world-up on a mostly
#     flat plate).
#   - A piece built with its long axis IN THE LOCAL XY PLANE and only a
#     THIN axis (thickness/radius) along local Z lies flat instead: yaw
#     (rotated about local Z, BEFORE align — see place_piece) spins the
#     long axis freely around compass headings, and align then tilts the
#     thin Z axis onto the surface normal, laying the long axis flush
#     against the ground with whatever heading yaw picked. build_twig_piece
#     below follows this convention: its spine starts along local +X (not
#     +Z), so the thin Z axis is what align_to_surface tilts onto the
#     normal, laying the twig flat along the ground — with its
#     kinks giving the "small random roll" natural variety for free — no
#     extra rotation step needed, and bottom_local/height_local (measured
#     along Z as always) come out small (the tube's own radius) instead of
#     the whole twig length, so the sink math embeds it properly instead of
#     burying half its length.
#   - pebble/rock are round enough (icosphere + squash, no single dominant
#     axis) that this doesn't apply either way — align_to_surface already
#     settles them onto the surface like any resting stone.
#   - mushroom is the deliberate EXCEPTION: it explicitly wants to stand
#     upright, stem down (see build_mushroom_piece) — built with its stem
#     along local Z on purpose, so the default align-to-Z-normal behavior
#     is exactly right for it, unlike twig.
#   - The three bundled Poly Haven "hero" assets this task adds as low-
#     weight forest accents (forest-branch-scan/forest-stump-scan) were
#     scanned/normalized STANDING (Z-up per resources/scatter/manifest.json,
#     confirmed by measuring their own bounding boxes: the branch and stump
#     both carry a real vertical extent comparable to or larger than their
#     footprint) — this script can't rebuild THEIR geometry the way it can
#     build_twig_piece's, so LIES_FLAT_ASSET_IDS + topple_asset_bm below
#     apply the SAME "make local Z thin, put the long axis in-plane" idea
#     as a one-time 90-degree pre-rotation baked into the copied bmesh
#     before place_piece ever sees it — same destination (a thin axis ends
#     up as local Z), different mechanism (an explicit rotation instead of
#     a build-time axis choice), because the geometry itself is fixed data.
#     forest-log-scan is EXCLUDED from that set: it was already scanned
#     lying on its side (height_mm 1.454 vs a 15.89mm footprint — see the
#     manifest), so it already lies flat via the untouched default path,
#     exactly like leaf/pebble/rock need no extra step either.

# --------------------------------------------- twig / grass pieces
#
# Two swept/extruded solids, siblings of build_generated_piece but NOT
# icospheres — an icosphere can't read as a bent stick or an upright fin
# no matter how it's noise-displaced, so each of these gets
# its own from-scratch mesh recipe (see build_twig_piece/
# build_grass_piece below). Both share the hard requirement the task
# that added them calls out explicitly: a WATERTIGHT, MANIFOLD SOLID with
# real thickness — never a zero-thickness plane (the "billboard-plane"
# lesson: a flat blade-shaped card is not sliceable/printable no matter how
# thin the intended piece is meant to read). Every one of them is a closed
# shell built purely by ADDING vertices/faces (ring sweeps, fan caps) — no
# boolean operator touches any of them, so none of them can reintroduce the
# union-seam manifold defects "Loose shells, not unions" above already
# retired.
#
# Sizes below are the RANGE form of the "canonical 28-32mm-heroic size" the
# CANONICAL_MM table above uses a single scalar for — pebbles/rocks are
# round enough that one scalar diameter says everything, but "a twig is
# 8-15mm long" IS the intrinsic seed-varied shape variety (docs/SCATTER.md
# "Scale anchor"), analogous to build_generated_piece's own
# PIECE_SIZE_JITTER draw. `scale`/`scale_factor` still apply ON TOP of that
# intrinsic draw, exactly like every other piece kind: each build function
# draws its own dimensions from these ranges first (the piece's "natural"
# size), then multiplies the WHOLE built mesh by one final uniform scale
# (`rng.uniform(*scale_range) * scale_factor`) — a twig scaled to 0.5x is a
# smaller twig with every dimension shrunk together, not a twig with only
# its length or only its thickness changed.

# Twig: a bent, tapered stick swept along a short piecewise-straight spine.
TWIG_LENGTH_RANGE_MM = (8.0, 15.0)
TWIG_THICKNESS_RANGE_MM = (0.6, 1.2)  # diameter at the base (thickest end)
TWIG_TIP_THICKNESS_FRACTION = 0.45  # tapers to this fraction of base thickness at the tip
TWIG_RING_VERTS = 6  # hexagonal cross-section — enough to read as round in
                      # print at this scale without an icosphere-grade budget
TWIG_SEGMENTS = 5  # 6 rings total (base..tip) — enough resolution for 1-2 kinks
# "1-2 slight forks/kinks" (the task's own wording) is implemented as 1-2
# discrete BEND points along the spine, not literal Y-branching topology: a
# genuine branch would need either a boolean union — reintroducing the
# exact defect class "Loose shells, not unions" retired above — or a
# hand-built saddle patch far past what a twig this small needs in order to
# read correctly at arm's length/print scale. A bent, kinked stick with a
# tapered tip is what "twig" reads as at 28-32mm-heroic scale, and it keeps
# every twig a single, trivially-manifold swept tube (no boolean, ever).
TWIG_KINK_ANGLE_RANGE_DEG = (6.0, 22.0)
TWIG_ONE_KINK_CHANCE = 0.6  # vs. two kinks the rest of the time

# Grass: a thin upright blade/fin, WIDTH tapering to a near-point tip while
# THICKNESS stays constant (only width tapers — see build_grass_piece).
GRASS_HEIGHT_RANGE_MM = (8.0, 16.0)
GRASS_BASE_WIDTH_RANGE_MM = (0.8, 1.5)
GRASS_THICKNESS_MM = 0.5
GRASS_TIP_WIDTH_FRACTION = 0.08  # near-zero but never literally zero — a true
                                  # zero-width ring collapses two pairs of
                                  # verts onto each other, which the STL
                                  # float32 roundtrip can turn into an exact-
                                  # zero-area sliver (same lesson MERGE_DIST_MM
                                  # below already exists for)
GRASS_SEGMENTS = 5  # 6 rings — enough to carry one smooth lean curve
GRASS_LEAN_ANGLE_RANGE_DEG = (8.0, 32.0)  # total lean accumulated over the full height

# Mushroom: a surface-of-revolution toadstool — a narrow STEM ring-swept
# straight up local +Z (the one generated kind that DELIBERATELY stands
# upright, stem down — see the "lies_flat" comment block above), flaring at
# a defined RIM into a CAP wider than the stem, domed to an apex. Built as a
# single revolve (a list of (z, radius) profile rings around the Z axis,
# same "rings bridged by quads" mechanism as build_twig_piece/
# build_grass_piece) rather than two separate parts glued together, so it
# is one closed shell by construction — no boolean, no seam. The apex ring
# collapses to radius 0 (every ring vertex lands at the same point) — a
# deliberate coincident-vertex pinch; cleanup_shell_bm's
# remove_doubles/dissolve_degenerate pass (run on every piece, see that
# function's docstring) welds the resulting degenerate quads into a proper
# triangle fan, so no separate fan-cap code is needed at the tip either —
# the base ring (radius > 0, sitting at local Z=0, the buried/embedded end)
# gets an explicit flat n-gon cap the same way build_twig_piece's ring caps
# do.
MUSHROOM_HEIGHT_RANGE_MM = (5.0, 9.0)  # total stem+cap height (task: "~5-9mm tall")
MUSHROOM_STEM_HEIGHT_FRACTION_RANGE = (0.55, 0.72)  # stem's share of total height
MUSHROOM_STEM_RADIUS_RANGE_MM = (0.35, 0.55)
# Cap radius as a multiple of the stem's OWN radius — kept well above 1 so
# the cap always reads as "wider than the stem" (task's explicit ask) even
# at the low end of both ranges; the wide overlap with rocks/pebbles'
# ASPECT_XY_RANGE-style "never the same every time" philosophy is
# deliberate seed variety, not sloppy tuning.
MUSHROOM_CAP_RADIUS_FACTOR_RANGE = (2.2, 3.4)
MUSHROOM_CAP_UNDERSIDE_DIP_FRACTION = 0.16  # how far above the stem-top the rim
                                             # ring sits, as a fraction of cap
                                             # height — a small positive gap
                                             # here (not the same Z as the stem
                                             # top) is what gives the cap's
                                             # underside a defined, near-vertical
                                             # RIM WALL instead of a smooth cone,
                                             # the "defined rim/underside" the
                                             # task calls for
MUSHROOM_CAP_SHOULDER_FRACTION = 0.55  # where along the cap's height the
                                        # dome's "shoulder" ring sits, as a
                                        # fraction of cap height above the rim
MUSHROOM_CAP_SHOULDER_RADIUS_FACTOR = 0.78  # shoulder ring radius, as a
                                             # fraction of the rim radius —
                                             # <1 so the dome curves INWARD
                                             # toward the apex instead of
                                             # flaring further, reading as a
                                             # rounded toadstool cap from the
                                             # side
MUSHROOM_RING_VERTS = 9  # odd count avoids a perfectly mirrored front/back
                          # facet pair — reads round enough at this scale
                          # without an icosphere-grade budget (task: "LOW-tri")
MUSHROOM_CAP_OUTLINE_JITTER_FRACTION = 0.10  # per-vertex radius jitter on the
                                              # cap rim/shoulder rings only —
                                              # seed-varied lobe/rim irregularity,
                                              # same idea as LEAF_OUTLINE_JITTER_FRACTION

# Poly Haven "hero" asset ids that need the topple-to-horizontal pre-rotation
# (see the "lies_flat" comment block above and topple_asset_bm below) —
# scanned/normalized STANDING (Z-up per resources/scatter/manifest.json),
# unlike forest-log-scan which was already scanned lying on its side
# (height_mm 1.454 vs a 15.89mm footprint) and so needs no extra rotation.
LIES_FLAT_ASSET_IDS = frozenset({"forest-branch-scan", "forest-stump-scan"})
# Small random extra tilt applied on top of the fixed 90-degree topple, so a
# toppled branch/stump doesn't look identically flat every time (task:
# "small random roll ok").
TOPPLE_EXTRA_ROLL_RANGE_DEG = (-15.0, 15.0)

# Merge-by-distance / degenerate-face thresholds — same values as
# base_cut.py's/gen_landscape.py's cleanup_and_check, same reason (the STL
# float32 roundtrip can turn a near-zero sliver into an exact zero, dropping
# a pinhole in the shell).
MERGE_DIST_MM = 0.001

# Cleanup passes: dissolving one degenerate can degenerate a neighboring
# face, so the merge/dissolve pair runs until the mesh stops changing
# (bounded; in practice 1-2 passes suffice).
CLEANUP_MAX_PASSES = 4

# Same catastrophic threshold as base_cut.py's validate (and the same
# lenient-gate reasoning): a decorated plate with a few non-manifold edges
# still cuts fine — the exact solver copes and base_cut re-validates with
# this exact tolerance downstream — so mild non-manifold-ness is a WARNING
# carried in SCATTER_DONE's payload, never a failure. Only above this
# ratio is the plate genuinely broken enough to stop the pipeline.
MAX_NON_MANIFOLD_RATIO = 0.02


def tok(name, payload=None):
    line = name if payload is None else name + " " + json.dumps(payload)
    print(line, flush=True)


# ------------------------------------------------------------ mesh helpers
# (new_object / delete_object / bbox helpers / cleanup_shell_bm / count_shells
#  are the same recipes — and the same rationale — as base_cut.py's; copied
#  rather than imported because these embedded scripts each run standalone
#  inside Blender's own Python, with no shared package between them. There is
#  no apply_boolean here any more — see "Loose shells, not unions" above: the
#  per-piece EXACT union this script used to do is exactly the thing that
#  went away.)


def new_object(name, bm):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def delete_object(obj):
    """See base_cut.py's identical helper for why the mesh datablock must be
    dropped explicitly: a scatter run can create hundreds of transient piece
    objects (one per placed piece), and Blender never garbage-collects
    unlinked-but-still-referenced mesh datablocks mid-session."""
    mesh = obj.data
    bpy.data.objects.remove(obj, do_unlink=True)
    if mesh is not None and mesh.users == 0:
        bpy.data.meshes.remove(mesh)


def bbox_minmax(verts):
    """Streaming min/max over vertex coordinates — same one-pass reasoning as
    base_cut.py's/gen_landscape.py's bbox_dims (avoids materializing full
    coordinate lists on a large sculpt), but returning the actual bounds
    (not just spans) since scatter needs them for the candidate grid and the
    edge margin, not just a dimension report."""
    min_x = min_y = min_z = math.inf
    max_x = max_y = max_z = -math.inf
    for v in verts:
        co = v.co
        if co.x < min_x:
            min_x = co.x
        if co.x > max_x:
            max_x = co.x
        if co.y < min_y:
            min_y = co.y
        if co.y > max_y:
            max_y = co.y
        if co.z < min_z:
            min_z = co.z
        if co.z > max_z:
            max_z = co.z
    if min_x is math.inf:
        return (0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
    return (min_x, min_y, min_z, max_x, max_y, max_z)


def cleanup_shell_bm(bm):
    """Merge stray verts, dissolve degenerate slivers, fix normals — IN
    PLACE on a single shell's bmesh. base_cut.py's cleanup_and_check recipe
    at heart, but run PER SHELL (once per piece, once for the terrain)
    instead of once on a whole already-unioned mesh, because there is no
    "whole unioned mesh" any more (see "Loose shells, not unions" in the
    module docstring) — and, deliberately, this is also what keeps the
    final join safe: every remove_doubles call here only ever sees ONE
    shell's own vertices, so it is topologically impossible for it to weld
    two different shells together. A single global remove_doubles AFTER
    joining terrain+pieces was considered and rejected for exactly this
    reason — see join_shells' docstring for the verification that made
    that call.

      - TRIANGULATE FIRST. This mattered enormously when EXACT booleans
        were minting n-gons along union seams (commit 0654275's "manifold
        contradiction" fix) — the STL exporter triangulates those
        invisibly at write time, so a pre-triangulate check was seeing a
        DIFFERENT mesh than the one written to disk. Without any unions,
        icospheres and imported STL terrain are already all-triangle
        meshes, so this step is now a no-op in the common case — kept
        anyway because "no-op on already-clean input, correct on anything
        that isn't" is a free property that costs nothing to keep, and it
        is exactly what makes the roundtrip check now expect 0 defects
        instead of merely tolerating a few (see roundtrip_check).
      - it iterates until stable (dissolving one sliver can degenerate a
        neighbor), bounded by CLEANUP_MAX_PASSES.
    """
    bmesh.ops.triangulate(bm, faces=bm.faces)
    for _ in range(CLEANUP_MAX_PASSES):
        verts_before = len(bm.verts)
        faces_before = len(bm.faces)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=MERGE_DIST_MM)
        bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=MERGE_DIST_MM)
        if len(bm.verts) == verts_before and len(bm.faces) == faces_before:
            break
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)


def count_shells(bm):
    """Number of disconnected mesh islands ("loose shells") via a
    face-adjacency flood fill over shared edges — base_cut.py's identical
    helper (see its docstring), copied rather than imported for the same
    standalone-script reason as the rest of this section. Used here to
    RE-MEASURE (not assume) the shells count SCATTER_DONE reports: shells
    should equal 1 (terrain) + placed by construction once join_shells has
    run, but "should equal by construction" is exactly the kind of claim
    the roundtrip-honesty mechanism (commit 0654275) exists to verify
    against the actual exported file rather than trust blind."""
    unvisited = set(bm.faces)
    shells = 0
    while unvisited:
        shells += 1
        start = next(iter(unvisited))
        unvisited.discard(start)
        stack = [start]
        while stack:
            face = stack.pop()
            for edge in face.edges:
                for neighbor in edge.link_faces:
                    if neighbor in unvisited:
                        unvisited.discard(neighbor)
                        stack.append(neighbor)
    return shells


def join_shells(landscape, piece_objects):
    """Combine the terrain and every placed piece into `landscape`'s mesh
    data as separate LOOSE SHELLS (docs/SCATTER.md) — concatenation, not
    union. `bpy.ops.object.join()` appends each selected object's mesh data
    (vertices/faces) into the active object's mesh AS-IS: it does not run
    remove_doubles, does not require or create shared geometry between the
    joined objects, and never touches vertex coordinates (both the terrain,
    via import_landscape, and every piece, via place_piece's bm.transform
    bake, already carry their final world position IN their vertex data
    with an identity object matrix — join is therefore a pure data
    concatenation here, not a re-parent-and-hope operation).

    Why this can't bridge piece<->terrain (or piece<->piece) vertices: every
    shell already went through cleanup_shell_bm — its OWN remove_doubles
    pass, scoped to its OWN bmesh — before it ever reaches this function, so
    by the time join runs there is nothing left to merge; join itself calls
    no distance-based operator at all. This was verified directly against
    real Blender (S1 job shape, 35 pieces): shells re-measured via
    count_shells on the round-tripped export matched 1 + placed exactly,
    every run — see this change's report for the numbers.
    """
    bpy.ops.object.select_all(action="DESELECT")
    for obj in piece_objects:
        obj.select_set(True)
    landscape.select_set(True)
    bpy.context.view_layer.objects.active = landscape
    if piece_objects:
        bpy.ops.object.join()


def roundtrip_check(path):
    """Re-import the just-exported STL and count non-manifold edges AND
    shells THERE — the pre-export bmesh counts are provably too optimistic:
    the STL importer's exact-duplicate-triangle drop can turn a sliver that
    was manifold in the bmesh into a T-junction in the file every downstream
    consumer (base_cut.py's validate, a slicer) actually reads. This is the
    same import base_cut.py will do, so the counts SCATTER_DONE reports are
    what the cut pipeline will see — no more "scatter said manifold, cut
    said not". Costs one STL import (~1s on a decorated plate), the honest
    price of reporting the truth. `shells` is the new field (docs/SCATTER.md
    "Pieces are placed as LOOSE SHELLS") — measured the same honest way as
    the edge counts, not assumed from `1 + placed`."""
    imported = import_landscape(path)
    bm = bmesh.new()
    bm.from_mesh(imported.data)
    total_edges = len(bm.edges)
    bad_edges = sum(1 for e in bm.edges if not e.is_manifold)
    shells = count_shells(bm)
    bm.free()
    delete_object(imported)
    return bad_edges, total_edges, shells


def import_landscape(path):
    """Same floor as base_cut.py: the app's supported floor is Blender 4.2,
    where wm.stl_import/wm.stl_export both exist — no legacy operator
    fallback."""
    before = set(bpy.data.objects)
    bpy.ops.wm.stl_import(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    if len(new) != 1:
        raise RuntimeError(f"expected 1 imported object, got {len(new)}")
    return new[0]


def clamp(x, lo, hi):
    return lo if x < lo else hi if x > hi else x


# ------------------------------------------------------- piece validation

CANONICAL_MM = {"pebble": PEBBLE_CANONICAL_MM, "rock": ROCK_CANONICAL_MM}

# The full Generated-kind set (docs/SCATTER.md pins the enum in
# scatter.rs's GeneratedPieceKind): "pebble"/"rock" are noise-displaced
# icospheres built from a CANONICAL_MM scalar (build_generated_piece);
# "twig"/"grass"/"mushroom" are swept/extruded solids, each with its
# own size RANGE baked into its own build function (build_twig_piece/
# build_grass_piece/build_mushroom_piece) rather than a
# single CANONICAL_MM scalar — see those functions' docstrings for why a
# range reads better than one canonical size for organic debris.
# validate_pieces checks membership in this set (not CANONICAL_MM) so these
# non-icosphere kinds are accepted without needing a fake CANONICAL_MM
# entry.
GENERATED_KINDS = frozenset({"pebble", "rock", "twig", "grass", "mushroom"})


def validate_pieces(pieces_json, asset_paths):
    """Parse the pinned PieceChoice shape and return
    [((source, key), weight), ...] where `source` is "generated" (`key` is
    the CANONICAL_MM kind, "pebble"/"rock") or "asset" (`key` is the asset
    id) — a single uniform shape `pick_piece_kind` and the placement loop
    both consume without caring which source a given entry is.

    An Asset entry's id is resolved against `asset_paths` (the {id: path}
    map Rust injects into the job JSON — see this module's docstring) RIGHT
    HERE, before any Blender work: an unknown id (not a key in the map) or a
    resolved path that doesn't exist on disk both raise a clear ValueError,
    matching docs/SCATTER.md's Asset-source validation pin ("unknown id or
    missing file -> clear SCATTER_FAILED before any Blender work"). The
    script never guesses a path from an id itself — asset_paths is the only
    source of truth, exactly as Rust intends it.
    """
    if not pieces_json:
        raise ValueError("params.pieces is empty — nothing to scatter")
    out = []
    for entry in pieces_json:
        piece = entry["piece"]
        weight = float(entry.get("weight", 1.0))
        if "Asset" in piece:
            asset_id = piece["Asset"]["id"]
            path = asset_paths.get(asset_id)
            if path is None:
                raise ValueError(f"unknown scatter asset id: {asset_id!r} (not in asset_paths)")
            if not os.path.isfile(path):
                raise ValueError(f"scatter asset {asset_id!r} file not found: {path}")
            if weight > 0.0:
                out.append((("asset", asset_id), weight))
            continue
        if "Generated" not in piece:
            raise ValueError(f"unknown piece source: {list(piece.keys())}")
        kind = piece["Generated"]["kind"]
        if kind not in GENERATED_KINDS:
            raise ValueError(f"unknown generated piece kind: {kind}")
        if weight > 0.0:
            out.append((("generated", kind), weight))
    if not out:
        raise ValueError("every piece has weight <= 0 — nothing to scatter")
    return out


def pick_piece_kind(rng, pieces):
    total = sum(w for _, w in pieces)
    r = rng.uniform(0.0, total)
    acc = 0.0
    for kind, w in pieces:
        acc += w
        if r <= acc:
            return kind
    return pieces[-1][0]  # float-rounding fallback


# ------------------------------------------------------------ piece meshes

def build_generated_piece(kind, rng, scale_range, scale_factor):
    """A fresh noise-displaced icosphere for one piece — built from scratch
    per placement (not a stamped copy) so "per-piece variety from the seed"
    (docs/SCATTER.md) is real: every pebble/rock gets its own random size,
    squash, and noise offsets from the shared rng stream, in the same spirit
    as gen_landscape.py's boulder layer (one seeded stream, one fresh
    instance per loop iteration — see _boulders_layer's docstring for the
    ancestor of this idea, done there in 2D on a heightfield and here in 3D
    on a solid).

    Returns (bm, size_mm, bottom_local, height_local):
      - bm: the built bmesh, centered at local origin, NOT yet rotated/moved
        into world space (build_generated_piece never applies placement —
        see place_piece for why the local frame matters: piece_bottom_local
        must be measured in the UNROTATED frame, along local Z, for the sink
        math to hold after rotation).
      - size_mm: the piece's nominal (pre-squash) diameter, for reporting.
      - bottom_local: how far below local origin the piece's lowest vertex
        sits (a positive number) — the "always buried" floor and the sink
        placement math both need this.
      - height_local: full local Z extent (max - min) — the floor
        (max(0.4, 20% of piece height)) is 20% of THIS, not of size_mm,
        since squash/noise change the piece's actual vertical extent.

    Icosphere subdivision is now DERIVED from size_mm (subdivision_for_size),
    not a fixed level per kind — see that function's and ROCK_MAX_SUBDIV's
    docstrings/comments for the print-resolution target and the tri-cost
    cap. Rocks additionally get a third, high-frequency noise octave (see
    ROCK_FINE_NOISE_FRACTION) layered on top of the same two-lobe coarse
    displacement pebbles also get — pebbles stay smooth (river pebbles ARE
    smooth), just denser, so their silhouette reads as a curve rather than
    a polygon; rocks read as fine-grained stone rather than facets.
    """
    canonical = CANONICAL_MM[kind]
    size_jitter = rng.uniform(*PIECE_SIZE_JITTER)
    user_scale = rng.uniform(scale_range[0], scale_range[1])
    size_mm = max(0.05, canonical * size_jitter * scale_factor * user_scale)

    is_rock = kind != "pebble"
    max_subdiv = ROCK_MAX_SUBDIV if is_rock else PEBBLE_MAX_SUBDIV
    subdiv = subdivision_for_size(size_mm, max_subdiv)
    noise_fraction = PEBBLE_NOISE_AMOUNT if kind == "pebble" else ROCK_NOISE_AMOUNT
    squash_lo, squash_hi = PEBBLE_SQUASH_Z if kind == "pebble" else ROCK_SQUASH_Z

    squash_z = rng.uniform(squash_lo, squash_hi)
    aspect_x = rng.uniform(*ASPECT_XY_RANGE)
    aspect_y = rng.uniform(*ASPECT_XY_RANGE)

    # Two independent noise streams (coarse lobes + fine grain), each with
    # its own random 3D offset — same "seed-derived offset vector" mechanism
    # as gen_landscape.py's _seed_offset, drawn straight from the rng stream
    # here rather than hashed, since this is a fixed-order per-instance loop
    # (see the module docstring's determinism note).
    offset1 = Vector((rng.uniform(-1000.0, 1000.0) for _ in range(3)))
    offset2 = Vector((rng.uniform(-1000.0, 1000.0) for _ in range(3)))
    # Wavelength ~1.8-3x the piece's own size -> under one full noise cycle
    # spans the whole piece, so the coarse octave reads as ONE dominant
    # lopsided bulge (a silhouette shade_smooth can't erase) rather than a
    # ripple pattern (a texture shade_smooth blurs flat) — see
    # PEBBLE_NOISE_AMOUNT's comment for why that distinction matters here.
    freq1 = rng.uniform(0.33, 0.55) / max(size_mm, 0.5)
    freq2 = freq1 * rng.uniform(2.6, 3.6)
    noise_amount = noise_fraction * size_mm

    # Rock-only third octave (see ROCK_FINE_NOISE_FRACTION's comment): drawn
    # ONLY for rocks, so pebbles never spend an rng draw on it — that's fine
    # for determinism (see subdivision_for_size's docstring: the sequence
    # only needs to be fixed-order for a GIVEN kind sequence, not
    # kind-invariant in draw count) and keeps pebbles genuinely untouched by
    # this change beyond their denser icosphere.
    if is_rock:
        offset3 = Vector((rng.uniform(-1000.0, 1000.0) for _ in range(3)))
        fine_wavelength_mm = FINE_NOISE_FACET_MULT * facet_edge_mm(size_mm, subdiv)
        freq3 = 1.0 / max(fine_wavelength_mm, 1e-6)
        fine_amount = ROCK_FINE_NOISE_FRACTION * size_mm
    else:
        offset3 = None
        freq3 = 0.0
        fine_amount = 0.0

    # subdivisions=N builds 20 tris at N<=1, then quadruples per level
    # (20, 80, 320, 1280, 5120, 20480 for N=1..6) — see ROCK_MAX_SUBDIV's
    # comment for the per-piece tri-cost budget this level was chosen
    # against.
    bm = bmesh.new()
    bmesh.ops.create_icosphere(bm, subdivisions=subdiv, radius=size_mm / 2.0)
    bmesh.ops.scale(bm, vec=Vector((aspect_x, aspect_y, squash_z)), verts=bm.verts)

    for v in bm.verts:
        direction = v.co.normalized() if v.co.length > 1e-9 else Vector((0.0, 0.0, 1.0))
        p1 = Vector((v.co.x * freq1 + offset1.x, v.co.y * freq1 + offset1.y, v.co.z * freq1 + offset1.z))
        p2 = Vector((v.co.x * freq2 + offset2.x, v.co.y * freq2 + offset2.y, v.co.z * freq2 + offset2.z))
        n = noise.noise(p1) * 0.8 + noise.noise(p2) * 0.2
        displacement = n * noise_amount
        if is_rock:
            p3 = Vector((v.co.x * freq3 + offset3.x, v.co.y * freq3 + offset3.y, v.co.z * freq3 + offset3.z))
            displacement += noise.noise(p3) * fine_amount
        v.co += direction * displacement

    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)

    min_z = min((v.co.z for v in bm.verts), default=0.0)
    max_z = max((v.co.z for v in bm.verts), default=0.0)
    bottom_local = -min_z
    height_local = max(1e-6, max_z - min_z)
    return bm, size_mm, bottom_local, height_local


def _ring_frame(tangent):
    """Two unit vectors perpendicular to `tangent`, spanning its local
    cross-section plane — shared by build_twig_piece/build_grass_piece's
    ring-sweep construction. Picks whichever of world X/Y is LESS parallel
    to `tangent` as the reference axis to cross against, avoiding the
    degenerate near-zero cross product a single fixed reference would hit
    if `tangent` ever pointed close to it. Cheap and robust for the small
    bend angles twigs use (a full loop needing continuous parallel
    transport never happens at this piece scale)."""
    ref = Vector((1.0, 0.0, 0.0)) if abs(tangent.x) < 0.9 else Vector((0.0, 1.0, 0.0))
    u = tangent.cross(ref).normalized()
    v = tangent.cross(u).normalized()
    return u, v


def build_twig_piece(rng, scale_range, scale_factor):
    """A bent, tapered hexagonal-cross-section tube — see the "Twig"
    constants block above for the size ranges and the fork-as-kink design
    call, and the "lies_flat" comment block above THAT for why the spine now
    starts along local +X (a FALLEN twig lying on the ground), not local +Z
    (an earlier revision literally had it "anchor into the terrain at its
    base and point up out of it" — the planted-pole bug the forest-floor
    task reported). Built as TWIG_SEGMENTS+1 rings swept along a
    piecewise-straight spine that starts at local origin pointing local +X
    and bends by 1-2 small random kinks — _ring_frame(direction) picks kink
    axes PERPENDICULAR to the current spine direction (Z or -Y when the
    spine is still along X), so a kink can rotate the spine WITHIN the
    horizontal XY plane (a lying twig bending left/right as seen from above)
    or slightly OUT of it (a bit of vertical waviness, capped at
    TWIG_KINK_ANGLE_RANGE_DEG per kink so it stays reading as "flat with a
    small random roll", never enough to flip the twig upright). Each ring is
    a TWIG_RING_VERTS-gon bridged to its neighbor by quads; both end rings
    close with a flat n-gon cap — every ring is planar by construction (it's
    built from a fixed radius around a single center point), so
    cleanup_shell_bm's triangulate step can turn the hexagonal caps and the
    quad side walls into triangles with no manual fan triangulation needed
    here.

    Because the spine now lives in local X (not Z), bottom_local/
    height_local (still measured along Z, same as every build_* function)
    come out as roughly the tube's own RADIUS, not its length — place_piece
    then embeds the twig by that small radius, and align_to_surface's
    local-Z-to-normal tilt lays the whole spine flush against the terrain
    with whatever heading `yaw` picked, exactly the lie-flat mechanism the
    "lies_flat" comment block above describes — no separate
    "lay flat" rotation needed here, just building it right-side-around in
    the first place.

    Returns the same (bm, size_mm, bottom_local, height_local) shape
    build_generated_piece does, so the placement loop in scatter() treats
    every generated kind identically.
    """
    length_mm = rng.uniform(*TWIG_LENGTH_RANGE_MM)
    base_radius_mm = rng.uniform(*TWIG_THICKNESS_RANGE_MM) / 2.0
    tip_radius_mm = base_radius_mm * TWIG_TIP_THICKNESS_FRACTION
    user_scale = rng.uniform(scale_range[0], scale_range[1])
    final_scale = max(1e-6, user_scale * scale_factor)

    # All rng draws happen HERE, in one fixed-order block, before any
    # geometry is built — same "fixed-order per-instance draw" discipline
    # build_generated_piece follows (see the module docstring's determinism
    # section), so the spine-building loop below only ever READS these,
    # never draws from `rng` itself.
    num_kinks = 1 if rng.random() < TWIG_ONE_KINK_CHANCE else 2
    kink_ts = sorted(rng.uniform(0.3, 0.75) for _ in range(num_kinks))
    kink_angles_rad = [
        math.radians(rng.uniform(*TWIG_KINK_ANGLE_RANGE_DEG)) * rng.choice((-1.0, 1.0))
        for _ in range(num_kinks)
    ]
    kink_axis_picks = [rng.choice((0, 1)) for _ in range(num_kinks)]

    num_rings = TWIG_SEGMENTS + 1
    seg_len = length_mm / TWIG_SEGMENTS

    # Spine starts along local +X — see this function's docstring and the
    # "lies_flat" comment block above the twig/leaf/grass section header for
    # why: a twig built this way lies flat by construction (the same
    # convention build_leaf_piece already uses), instead of standing up
    # like the old +Z-spine version did.
    direction = Vector((1.0, 0.0, 0.0))
    position = Vector((0.0, 0.0, 0.0))
    ring_centers = [position.copy()]
    directions = [direction.copy()]
    kink_index = 0
    for seg in range(1, num_rings):
        t = seg / TWIG_SEGMENTS
        while kink_index < num_kinks and t >= kink_ts[kink_index]:
            axis_u, axis_v = _ring_frame(direction)
            axis = axis_u if kink_axis_picks[kink_index] == 0 else axis_v
            direction = (Matrix.Rotation(kink_angles_rad[kink_index], 3, axis) @ direction).normalized()
            kink_index += 1
        position = position + direction * seg_len
        ring_centers.append(position.copy())
        directions.append(direction.copy())

    bm = bmesh.new()
    rings = []
    for i, (center, dirn) in enumerate(zip(ring_centers, directions)):
        t = i / TWIG_SEGMENTS
        radius = base_radius_mm + (tip_radius_mm - base_radius_mm) * t
        u, v = _ring_frame(dirn)
        ring_verts = []
        for k in range(TWIG_RING_VERTS):
            a = 2.0 * math.pi * k / TWIG_RING_VERTS
            co = center + (math.cos(a) * u + math.sin(a) * v) * radius
            ring_verts.append(bm.verts.new(co))
        rings.append(ring_verts)

    for i in range(len(rings) - 1):
        a_ring, b_ring = rings[i], rings[i + 1]
        for k in range(TWIG_RING_VERTS):
            k2 = (k + 1) % TWIG_RING_VERTS
            bm.faces.new((a_ring[k], a_ring[k2], b_ring[k2], b_ring[k]))

    bm.faces.new(rings[0])
    bm.faces.new(tuple(reversed(rings[-1])))

    bmesh.ops.scale(bm, vec=Vector((final_scale,) * 3), verts=bm.verts)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)

    min_z = min((v.co.z for v in bm.verts), default=0.0)
    max_z = max((v.co.z for v in bm.verts), default=0.0)
    bottom_local = -min_z
    height_local = max(1e-6, max_z - min_z)
    size_mm = length_mm * final_scale
    return bm, size_mm, bottom_local, height_local


def build_grass_piece(rng, scale_range, scale_factor):
    """A thin upright blade — build_twig_piece's sibling with a flattened
    (diamond, 4-vertex) cross-section instead of a round one: WIDTH tapers
    from GRASS_BASE_WIDTH_RANGE_MM down to a near-point at the tip while
    THICKNESS (GRASS_THICKNESS_MM, the direction perpendicular to the lean
    plane) stays constant along the whole length — see the "Grass"
    constants block above. Bends with ONE continuous lean distributed
    evenly across every segment, confined to a single fixed plane (bend
    axis = local +X, so the blade always leans within the local Y-Z plane)
    — a real blade of grass has one dominant curve from wind/weight, not
    multiple kinks the way a woody twig does, hence its own build function
    rather than a shared one with build_twig_piece despite the similar
    ring-sweep skeleton. "Clumps come from placement, not one mesh" (the
    task's own wording) — see build_candidates' clumping warp for the tuft
    behavior; this function only ever builds ONE blade.

    Returns the same (bm, size_mm, bottom_local, height_local) shape as
    every other build_* function.
    """
    height_mm = rng.uniform(*GRASS_HEIGHT_RANGE_MM)
    base_width_mm = rng.uniform(*GRASS_BASE_WIDTH_RANGE_MM)
    tip_width_mm = base_width_mm * GRASS_TIP_WIDTH_FRACTION
    thickness_mm = GRASS_THICKNESS_MM
    user_scale = rng.uniform(scale_range[0], scale_range[1])
    final_scale = max(1e-6, user_scale * scale_factor)

    lean_total_rad = math.radians(rng.uniform(*GRASS_LEAN_ANGLE_RANGE_DEG)) * rng.choice((-1.0, 1.0))
    lean_axis = Vector((1.0, 0.0, 0.0))
    width_axis = Vector((1.0, 0.0, 0.0))  # perpendicular to the bend plane, constant

    num_rings = GRASS_SEGMENTS + 1
    seg_len = height_mm / GRASS_SEGMENTS
    seg_angle = lean_total_rad / GRASS_SEGMENTS

    direction = Vector((0.0, 0.0, 1.0))
    position = Vector((0.0, 0.0, 0.0))
    ring_centers = [position.copy()]
    directions = [direction.copy()]
    for _ in range(1, num_rings):
        direction = (Matrix.Rotation(seg_angle, 3, lean_axis) @ direction).normalized()
        position = position + direction * seg_len
        ring_centers.append(position.copy())
        directions.append(direction.copy())

    bm = bmesh.new()
    rings = []
    for i, (center, dirn) in enumerate(zip(ring_centers, directions)):
        t = i / GRASS_SEGMENTS
        half_w = (base_width_mm + (tip_width_mm - base_width_mm) * t) / 2.0
        half_th = thickness_mm / 2.0
        thickness_axis = dirn.cross(width_axis).normalized()
        corners = [
            center + width_axis * half_w,
            center + thickness_axis * half_th,
            center - width_axis * half_w,
            center - thickness_axis * half_th,
        ]
        rings.append([bm.verts.new(c) for c in corners])

    for i in range(len(rings) - 1):
        a_ring, b_ring = rings[i], rings[i + 1]
        for k in range(4):
            k2 = (k + 1) % 4
            bm.faces.new((a_ring[k], a_ring[k2], b_ring[k2], b_ring[k]))

    bm.faces.new(rings[0])
    bm.faces.new(tuple(reversed(rings[-1])))

    bmesh.ops.scale(bm, vec=Vector((final_scale,) * 3), verts=bm.verts)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)

    min_z = min((v.co.z for v in bm.verts), default=0.0)
    max_z = max((v.co.z for v in bm.verts), default=0.0)
    bottom_local = -min_z
    height_local = max(1e-6, max_z - min_z)
    size_mm = height_mm * final_scale
    return bm, size_mm, bottom_local, height_local


def build_mushroom_piece(rng, scale_range, scale_factor):
    """A toadstool: a narrow stem ring-swept straight up local +Z, flaring
    at a defined rim into a cap wider than the stem, domed to an apex — see
    the "Mushroom" constants block above for the size ranges and why this
    is a single surface-of-revolution rather than two glued parts. Unlike
    every other build_* function above, this one is deliberately built to
    STAND UPRIGHT (stem down) — see the "lies_flat" comment block earlier
    in this file — so, unlike build_twig_piece's fix, there is nothing to
    reorient here: local +Z staying "up" through align_to_surface is
    exactly the desired stem-down toadstool pose.

    The profile is 5 rings (base -> stem-top -> cap-rim -> cap-shoulder ->
    apex) revolved around local Z with MUSHROOM_RING_VERTS sides, built with
    the same "list of rings, bridge consecutive rings with quads" recipe
    build_twig_piece/build_grass_piece use. The cap-rim ring sits ABOVE the
    stem-top ring at (nearly) the same radius jump point but a different Z
    (MUSHROOM_CAP_UNDERSIDE_DIP_FRACTION) — that near-vertical side wall
    between them is what reads as the cap's own defined underside/rim edge
    from a side view, the "distinct thin STEM topped by a domed CAP wider
    than the stem with a defined rim/underside" the task calls for. The
    apex ring's radius is 0 — every one of its MUSHROOM_RING_VERTS vertices
    lands at the exact same point, a deliberate coincident-vertex pinch
    that cleanup_shell_bm's remove_doubles/dissolve_degenerate pass (run on
    every piece, see that function's docstring) collapses into a proper
    triangle fan — no separate top-cap code needed. The base ring (radius
    > 0, at local Z=0, the buried end) DOES get an explicit flat n-gon cap,
    the same as build_twig_piece's ring caps.

    Cap rings (rim + shoulder, not the stem) get a small per-vertex radius
    jitter (MUSHROOM_CAP_OUTLINE_JITTER_FRACTION) for seed-varied lobe/rim
    irregularity — a "noise.noise on a per-sample offset" idea, kept off
    the stem so it stays a clean
    read as a narrow stalk under the more organic cap.

    Returns the same (bm, size_mm, bottom_local, height_local) shape every
    other build_* function does.
    """
    total_height_mm = rng.uniform(*MUSHROOM_HEIGHT_RANGE_MM)
    stem_height_mm = total_height_mm * rng.uniform(*MUSHROOM_STEM_HEIGHT_FRACTION_RANGE)
    cap_height_mm = max(0.8, total_height_mm - stem_height_mm)
    stem_radius_mm = rng.uniform(*MUSHROOM_STEM_RADIUS_RANGE_MM)
    cap_radius_mm = stem_radius_mm * rng.uniform(*MUSHROOM_CAP_RADIUS_FACTOR_RANGE)
    user_scale = rng.uniform(scale_range[0], scale_range[1])
    final_scale = max(1e-6, user_scale * scale_factor)
    jitter_offset = Vector((rng.uniform(-1000.0, 1000.0) for _ in range(3)))

    rim_z = stem_height_mm + cap_height_mm * MUSHROOM_CAP_UNDERSIDE_DIP_FRACTION
    shoulder_z = stem_height_mm + cap_height_mm * MUSHROOM_CAP_SHOULDER_FRACTION
    apex_z = stem_height_mm + cap_height_mm

    # (z, radius, jitter_this_ring) — stem rings stay perfectly round
    # (jitter=False), cap rings (rim/shoulder) get the organic wobble; the
    # apex is pinned to exactly 0 so its ring collapses to one point (see
    # this function's docstring).
    profile = [
        (0.0, stem_radius_mm, False),
        (stem_height_mm, stem_radius_mm, False),
        (rim_z, cap_radius_mm, True),
        (shoulder_z, cap_radius_mm * MUSHROOM_CAP_SHOULDER_RADIUS_FACTOR, True),
        (apex_z, 0.0, False),
    ]

    n = MUSHROOM_RING_VERTS
    bm = bmesh.new()
    rings = []
    for z, r, jitter in profile:
        ring_verts = []
        for k in range(n):
            a = 2.0 * math.pi * k / n
            radius = r
            if jitter and r > 1e-9:
                p = Vector((math.cos(a) * r + jitter_offset.x, math.sin(a) * r + jitter_offset.y, z + jitter_offset.z))
                radius = max(0.0, r * (1.0 + MUSHROOM_CAP_OUTLINE_JITTER_FRACTION * noise.noise(p)))
            co = Vector((radius * math.cos(a), radius * math.sin(a), z))
            ring_verts.append(bm.verts.new(co))
        rings.append(ring_verts)

    bm.faces.new(rings[0])  # flat base cap (embed/buried end)
    for i in range(len(rings) - 1):
        a_ring, b_ring = rings[i], rings[i + 1]
        for k in range(n):
            k2 = (k + 1) % n
            bm.faces.new((a_ring[k], a_ring[k2], b_ring[k2], b_ring[k]))
    # No explicit apex cap: the last ring's coincident vertices already
    # close the tip via the degenerate quads above (see docstring).

    bmesh.ops.scale(bm, vec=Vector((final_scale,) * 3), verts=bm.verts)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)

    min_z = min((v.co.z for v in bm.verts), default=0.0)
    max_z = max((v.co.z for v in bm.verts), default=0.0)
    bottom_local = -min_z
    height_local = max(1e-6, max_z - min_z)
    size_mm = total_height_mm * final_scale
    return bm, size_mm, bottom_local, height_local


def topple_asset_bm(bm, rng):
    """Reorient an Asset piece scanned/normalized STANDING (see
    LIES_FLAT_ASSET_IDS's comment) into a FALLEN, lying-on-its-side pose —
    the Asset-source counterpart to build_twig_piece's build-time direction
    fix (see the "lies_flat" comment block earlier in this file): since this
    script can't rebuild the imported mesh's own geometry, it instead bakes
    a fixed 90-degree rotation about local X directly into the copied bmesh
    BEFORE place_piece ever sees it, turning whatever was the asset's own
    vertical (local Z, tall as authored) into a horizontal extent — after
    this call, place_piece's usual "tilt local Z to the surface normal"
    align step lands the piece's ORIGINAL vertical axis lying flush against
    the ground instead of standing up out of it. A second, small random
    rotation (TOPPLE_EXTRA_ROLL_RANGE_DEG) rides along on top so every
    toppled piece doesn't land at the exact same angle — the "small random
    roll ok" the task allows.

    Mutates `bm` in place and returns nothing; the caller (scatter()) must
    RE-MEASURE bottom_local/height_local from `bm` after this call — the
    ones build_asset_piece returned were measured in the PRE-topple pose and
    are stale afterwards.
    """
    roll_deg = rng.uniform(*TOPPLE_EXTRA_ROLL_RANGE_DEG)
    matrix = Matrix.Rotation(math.radians(90.0), 4, "X") @ Matrix.Rotation(math.radians(roll_deg), 4, "Y")
    bm.transform(matrix)


class AssetTemplateCache:
    """Imports each unique Asset id's STL ONCE (docs/SCATTER.md's Asset
    source pin: "Script imports the STL once per unique id (cache)") and
    hands out a fresh bmesh COPY per placement via `bm.from_mesh` — copying
    mesh data out of the cached object's `.data`, never touching the cached
    object itself, so N placements of the same id cost one import + N cheap
    data copies instead of N imports. The cached import objects are kept
    alive (never selected, never added to `piece_objects`) until
    `cleanup()` runs at the end of the job — they must never reach
    `join_shells`/the final export.
    """

    def __init__(self, asset_paths):
        self.asset_paths = asset_paths
        self._templates = {}

    def get(self, asset_id):
        obj = self._templates.get(asset_id)
        if obj is None:
            path = self.asset_paths[asset_id]  # presence already checked by validate_pieces
            obj = import_landscape(path)
            obj.name = f"__scatter_asset_template_{asset_id}"
            self._templates[asset_id] = obj
        return obj

    def cleanup(self):
        for obj in self._templates.values():
            delete_object(obj)
        self._templates.clear()


def build_asset_piece(asset_id, template_cache, rng, scale_range, scale_factor):
    """A fresh bmesh copy of a cached, imported Asset template — the
    Asset-source sibling of `build_generated_piece`, returning the exact
    same `(bm, size_mm, bottom_local, height_local)` shape so the placement
    loop and `place_piece` treat both sources identically.

    Every bundled asset is normalized ONCE at curation to its canonical
    28-32mm-scale size (docs/SCATTER.md "Scale anchor") — the imported
    mesh's OWN size at scale 1.0 already IS that canonical size, so unlike
    `build_generated_piece` there is no CANONICAL_MM lookup or
    PIECE_SIZE_JITTER here: `scale_range x scale_factor` is the only size
    knob, applied as a uniform scale about the template's own local origin
    (the copied bmesh's vertex coordinates are exactly the cached template's
    — bm.from_mesh does not re-center or re-orient anything).
    """
    template = template_cache.get(asset_id)
    bm = bmesh.new()
    bm.from_mesh(template.data)

    user_scale = rng.uniform(scale_range[0], scale_range[1])
    factor = max(1e-6, user_scale * scale_factor)
    bmesh.ops.scale(bm, vec=Vector((factor, factor, factor)), verts=bm.verts)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)

    min_z = min((v.co.z for v in bm.verts), default=0.0)
    max_z = max((v.co.z for v in bm.verts), default=0.0)
    bottom_local = -min_z
    height_local = max(1e-6, max_z - min_z)
    size_mm = height_local
    return bm, size_mm, bottom_local, height_local


def place_piece(bm, bottom_local, hit_loc, normal, align_to_surface, final_sink, yaw):
    """Bake world placement directly into the bmesh's vertex coordinates
    (the same "mesh data is ground truth, object stays identity" convention
    base_cut.py uses when re-homing a plug) — never via object.matrix_world.

    axis = the direction the piece's local +Z maps to: the surface normal
    when align_to_surface, world +Z otherwise (upright regardless of
    slope). Rotating by yaw FIRST (about local Z) and align SECOND means yaw
    varies the piece's look around its own vertical axis before that axis
    gets tilted to the slope — a natural "which way is this rock facing"
    order, and critically yaw never moves where local Z points, so
    bottom_local (measured before any rotation) still maps 1:1 onto `axis`
    afterwards.

    World origin placement derives from: the piece's lowest point must sit
    exactly `final_sink` mm below hit_loc along axis. lowest_point =
    origin - axis*bottom_local, so origin = hit_loc - axis*final_sink +
    axis*bottom_local = hit_loc + axis*(bottom_local - final_sink).
    """
    axis = normal.normalized() if align_to_surface else Vector((0.0, 0.0, 1.0))
    world_origin = hit_loc + axis * (bottom_local - final_sink)

    r_yaw = Matrix.Rotation(yaw, 4, "Z")
    if align_to_surface:
        quat = Vector((0.0, 0.0, 1.0)).rotation_difference(axis)
        r_align = quat.to_matrix().to_4x4()
    else:
        r_align = Matrix.Identity(4)

    transform = Matrix.Translation(world_origin) @ r_align @ r_yaw
    bm.transform(transform)
    return world_origin


# --------------------------------------------------------------- candidates

# ------------------------------------------------------------- clumping
#
# clump (0..1, ScatterParams.clump in scatter.rs): biases candidate
# placement toward CLUSTERS instead of the even jittered-grid spread above —
# what makes grass read as tufts and forest debris as drifts, instead of a
# uniform sprinkle. Algorithm: build the SAME jittered-grid candidates as
# clump=0 always has (nothing above this comment changes), then, only if
# clump > 0, WARP each candidate's (x, y) toward whichever of a small set of
# seeded cluster centers is nearest, by a fraction of the distance
# proportional to clump. A rectangle is convex, and both the original
# candidate and every cluster center already satisfy the edge-margin bounds
# by construction, so linear interpolation between them can never move a
# warped candidate outside the margin — no extra bounds check needed after
# the warp.
#
# Determinism / independence from the existing behavior: cluster centers are
# drawn from `random.Random(f"{seed}:scatter-clump:{num_clusters}")` — a
# stream INDEPENDENT of the `rng` passed in (the layer's own
# `random.Random(layer["seed"])`, already consumed above for the grid's cell
# jitter) — so clumping can never perturb the fixed-order draw sequence
# every other piece of this module's determinism proof depends on: the
# grid-candidate loop above runs byte-for-byte identically regardless of
# `clump`, and at clump<=0 this function returns that exact list, untouched
# — the pin `clump=0 == pre-clump behavior` is true by construction, not by
# coincidence.
CLUMP_CANDIDATES_PER_CLUSTER = 6.0  # average grid candidates gathered per tuft
CLUMP_MAX_PULL = 0.85  # cap so clump=1 still reads as a tight tuft, not every
                        # candidate collapsed onto one exact point per cluster


def _clump_cluster_centers(seed, min_x, min_y, max_x, max_y, edge_margin_mm, num_clusters):
    """Deterministic cluster centers for the clumping warp, from a stream
    independent of the layer's own candidate-jitter rng (see the clumping
    comment above for why that independence matters). Seeded by the STRING
    `f"{seed}:scatter-clump:{num_clusters}"` rather than reusing the layer
    stream — same "separate stream for a separate concern" idea as
    gen_landscape.py's `cluster_offset`/`_seed_offset`, applied here as an
    actual RNG object (discrete cluster POINTS) instead of a noise offset
    (a continuous field), since that's what this discrete-candidate case
    needs. A string, not a tuple: `random.Random()` only accepts
    None/int/float/str/bytes/bytearray as of Python 3.11 (tuples used to
    work via an implicit hash() fallback that no longer exists), and a
    formatted string is deterministic across runs/processes the way
    Python's built-in `hash()` on an arbitrary object is NOT (hash
    randomization) — `random.seed()`'s own str handling hashes via a fixed
    internal conversion, unaffected by PYTHONHASHSEED."""
    cluster_rng = random.Random(f"{seed}:scatter-clump:{num_clusters}")
    lo_x, hi_x = min_x + edge_margin_mm, max_x - edge_margin_mm
    lo_y, hi_y = min_y + edge_margin_mm, max_y - edge_margin_mm
    if hi_x <= lo_x or hi_y <= lo_y:
        # Degenerate plate (margin ate the whole area) — every candidate
        # will already have been rejected by the edge-margin check above,
        # so the exact center value here is moot; fall back to the bbox
        # center so callers still get a well-formed (non-empty) list.
        center = ((lo_x + hi_x) / 2.0, (lo_y + hi_y) / 2.0)
        return [center] * num_clusters
    return [(cluster_rng.uniform(lo_x, hi_x), cluster_rng.uniform(lo_y, hi_y)) for _ in range(num_clusters)]


def build_candidates(rng, min_x, min_y, max_x, max_y, density_per_dm2, edge_margin_mm, seed=None, clump=0.0):
    """Jittered-grid candidate (x, y) points in FIXED row-major order — see
    the module docstring's determinism note for why the order (and drawing
    both jitter numbers for every cell, accepted or not) matters. density is
    pieces per 100x100mm, so the average area per piece is 10000/density
    mm^2 and the grid step is that area's square root.

    `clump` (see the "clumping" comment block above this function) is
    applied as a position WARP after the grid is fully built — it never
    changes which/how many candidates exist, only where they sit. `seed` is
    only used to derive the independent cluster-center stream when
    `clump > 0`; pass the layer's own seed (NOT `rng`, which already carries
    the grid's own consumed state) — `None` is fine when `clump <= 0`, the
    common case, since the clumping branch is skipped entirely.
    """
    if density_per_dm2 <= 0:
        raise ValueError("density_per_dm2 must be > 0")
    step = math.sqrt(10000.0 / density_per_dm2)
    width = max(1e-6, max_x - min_x)
    depth = max(1e-6, max_y - min_y)
    nx = max(1, round(width / step))
    ny = max(1, round(depth / step))
    cell_w = width / nx
    cell_h = depth / ny

    candidates = []
    for iy in range(ny):
        for ix in range(nx):
            cx = min_x + (ix + 0.5) * cell_w
            cy = min_y + (iy + 0.5) * cell_h
            jx = (rng.random() - 0.5) * CELL_JITTER_FRACTION * cell_w
            jy = (rng.random() - 0.5) * CELL_JITTER_FRACTION * cell_h
            x, y = cx + jx, cy + jy
            if (
                x < min_x + edge_margin_mm
                or x > max_x - edge_margin_mm
                or y < min_y + edge_margin_mm
                or y > max_y - edge_margin_mm
            ):
                continue
            candidates.append((x, y))

    if clump <= 0.0 or not candidates:
        return candidates

    num_clusters = max(1, round(len(candidates) / CLUMP_CANDIDATES_PER_CLUSTER))
    centers = _clump_cluster_centers(seed, min_x, min_y, max_x, max_y, edge_margin_mm, num_clusters)
    pull = min(1.0, clump) * CLUMP_MAX_PULL

    warped = []
    for x, y in candidates:
        cx, cy = min(centers, key=lambda c: (c[0] - x) ** 2 + (c[1] - y) ** 2)
        warped.append((x + (cx - x) * pull, y + (cy - y) * pull))
    return warped


def raycast_accept(bvh, x, y, ray_z, ray_distance, max_slope_deg):
    """Raycast straight down at (x, y); return (loc, normal) if it hits and
    clears the slope gate, else None. Works on ANY mesh the BVH was built
    from (docs/SCATTER.md: scatter "never assumes a heightfield") — only the
    edge-margin check (against the planar bounding box, in build_candidates)
    assumes a roughly rectangular plate, which is exactly what
    gen_landscape.py's skirted heightfields are; an arbitrary designer blob
    would need a silhouette-distance margin instead, left for a later phase
    since S1's proving ground is a generated landscape."""
    origin = Vector((x, y, ray_z))
    direction = Vector((0.0, 0.0, -1.0))
    loc, normal, _index, _dist = bvh.ray_cast(origin, direction, ray_distance)
    if loc is None:
        return None
    slope_deg = math.degrees(math.acos(clamp(normal.z, -1.0, 1.0)))
    if slope_deg > max_slope_deg:
        return None
    return loc, normal


# --------------------------------------------------------------------- job

def parse_layer(layer_json, asset_paths):
    """Extract one layer's params from its job JSON entry, applying the same
    `params.get(key, default)` fallbacks the single-pass shape always used —
    see ScatterParams's own doc comment in scatter.rs for why each default
    here must match Rust's `#[serde(default = ...)]` exactly. Returns a plain
    dict the placement loop indexes by key, keeping `pieces` already resolved
    via `validate_pieces` (unknown id / missing file raised HERE, before any
    Blender work — same pin as the single-layer shape, now checked for every
    layer up front in `scatter()` before the landscape is even imported).
    """
    return {
        "seed": int(layer_json["seed"]),
        "density_per_dm2": float(layer_json["density_per_dm2"]),
        "scale": tuple(layer_json.get("scale", [0.85, 1.15])),
        "scale_factor": float(layer_json.get("scale_factor", 1.0)),
        "sink_mm": tuple(layer_json.get("sink_mm", [0.0, 0.6])),
        "align_to_surface": bool(layer_json.get("align_to_surface", True)),
        "max_slope_deg": float(layer_json.get("max_slope_deg", 55.0)),
        "edge_margin_mm": float(layer_json.get("edge_margin_mm", 2.0)),
        "clump": float(layer_json.get("clump", 0.0)),
        "pieces": validate_pieces(layer_json["pieces"], asset_paths),
    }


def scatter(job, debug):
    landscape_path = job["landscape_path"]
    out_path = job["out_path"]
    asset_paths = job.get("asset_paths", {})

    layers_json = job.get("layers")
    if not layers_json:
        # Rust's start_scatter already rejects an empty/missing stack before
        # this script ever runs (docs/SCATTER.md's ScatterJob pin: "must be
        # non-empty") — this is defense in depth against a hand-edited or
        # stale job file, same reasoning as validate_pieces' own id checks.
        raise ValueError("job.layers is empty — nothing to scatter")

    # Parse + validate EVERY layer (including resolving its Asset pieces
    # against asset_paths) before any Blender work — an unknown id in layer 2
    # must fail before layer 0 ever places a single piece, extending
    # validate_pieces' existing "before any Blender work" pin to the whole
    # stack instead of just one params blob.
    layers = [parse_layer(layer_json, asset_paths) for layer_json in layers_json]

    # Asset templates are shared across the WHOLE stack (docs/SCATTER.md:
    # "the script imports the STL once per unique id"): two layers pulling
    # the same skull must not double-import it.
    template_cache = AssetTemplateCache(asset_paths)

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()

    landscape = import_landscape(landscape_path)
    src_bm = bmesh.new()
    src_bm.from_mesh(landscape.data)
    min_x, min_y, min_z, max_x, max_y, max_z = bbox_minmax(src_bm.verts)
    bvh = BVHTree.FromBMesh(src_bm)
    ray_z = max_z + RAY_MARGIN_MM
    ray_distance = (max_z - min_z) + 2.0 * RAY_MARGIN_MM

    # Independence (docs/SCATTER.md "Layers", module docstring's own
    # "Layers — independent stacking" section): every layer raycasts against
    # THIS SAME bvh/src_bm — built ONCE from the landscape as originally
    # imported, never mutated by placement below — and draws from its OWN
    # random.Random(layer["seed"]) stream. Computing every layer's accepted
    # candidates in this FIRST pass, before any piece in ANY layer is built,
    # means (a) SCATTER_PROGRESS's total spans the whole stack from tick one,
    # and (b) there is no way for one layer's placement loop to accidentally
    # interleave rng draws with another's — each layer's rng object is only
    # ever touched inside its own iteration below and in its own slice of
    # the placement loop that follows.
    layer_accepted = []
    for layer in layers:
        layer_rng = random.Random(layer["seed"])
        grid_candidates = build_candidates(
            layer_rng, min_x, min_y, max_x, max_y,
            layer["density_per_dm2"], layer["edge_margin_mm"],
            seed=layer["seed"], clump=layer["clump"],
        )
        accepted = []
        for x, y in grid_candidates:
            hit = raycast_accept(bvh, x, y, ray_z, ray_distance, layer["max_slope_deg"])
            if hit is not None:
                accepted.append(hit)
        layer_accepted.append((layer_rng, accepted))
    src_bm.free()

    total = sum(len(accepted) for _, accepted in layer_accepted)
    placed = 0
    piece_objects = []
    for layer_index, (layer, (layer_rng, accepted)) in enumerate(zip(layers, layer_accepted)):
        pieces = layer["pieces"]
        scale_range = layer["scale"]
        scale_factor = layer["scale_factor"]
        sink_lo, sink_hi = layer["sink_mm"]
        align_to_surface = layer["align_to_surface"]

        for loc, normal in accepted:
            source, key = pick_piece_kind(layer_rng, pieces)
            yaw = layer_rng.uniform(0.0, 2.0 * math.pi)
            if source == "generated":
                if key in ("pebble", "rock"):
                    bm, size_mm, bottom_local, height_local = build_generated_piece(
                        key, layer_rng, scale_range, scale_factor
                    )
                elif key == "twig":
                    bm, size_mm, bottom_local, height_local = build_twig_piece(
                        layer_rng, scale_range, scale_factor
                    )
                elif key == "grass":
                    bm, size_mm, bottom_local, height_local = build_grass_piece(
                        layer_rng, scale_range, scale_factor
                    )
                else:  # "mushroom" — validate_pieces already restricted `key`
                    # to GENERATED_KINDS, so this is the last remaining option
                    bm, size_mm, bottom_local, height_local = build_mushroom_piece(
                        layer_rng, scale_range, scale_factor
                    )
                debug_kind = key
            else:  # "asset" — see validate_pieces/build_asset_piece
                bm, size_mm, bottom_local, height_local = build_asset_piece(
                    key, template_cache, layer_rng, scale_range, scale_factor
                )
                if key in LIES_FLAT_ASSET_IDS:
                    # See topple_asset_bm's docstring: bakes the fallen-on-
                    # its-side reorientation into `bm` directly, so
                    # bottom_local/height_local (measured in the PRE-topple
                    # pose above) must be re-measured before place_piece
                    # uses them for the sink-floor/embed math.
                    topple_asset_bm(bm, layer_rng)
                    min_z = min((v.co.z for v in bm.verts), default=0.0)
                    max_z = max((v.co.z for v in bm.verts), default=0.0)
                    bottom_local = -min_z
                    height_local = max(1e-6, max_z - min_z)
                debug_kind = f"asset:{key}"

            floor_mm = max(MIN_SINK_MM, SINK_FLOOR_FRACTION * height_local)
            raw_sink = layer_rng.uniform(sink_lo, sink_hi)
            final_sink = max(floor_mm, raw_sink)

            world_origin = place_piece(bm, bottom_local, loc, normal, align_to_surface, final_sink, yaw)

            # No union: clean this piece as its OWN shell (see
            # cleanup_shell_bm's docstring — this is what makes the later
            # join provably safe) and keep the object around for
            # join_shells instead of unioning it into the terrain and
            # discarding it.
            cleanup_shell_bm(bm)
            piece_obj = new_object(f"scatter_piece_{placed}", bm)
            piece_objects.append(piece_obj)

            placed += 1
            if debug:
                tok(
                    "SCATTER_PIECE",
                    {
                        "index": placed - 1,
                        "layer": layer_index,
                        "kind": debug_kind,
                        "x_mm": round(world_origin.x, 4),
                        "y_mm": round(world_origin.y, 4),
                        "z_mm": round(world_origin.z, 4),
                        "yaw_deg": round(math.degrees(yaw), 3),
                        "size_mm": round(size_mm, 4),
                        "floor_mm": round(floor_mm, 4),
                        "embed_depth_mm": round(final_sink, 4),
                        "aligned_deg": round(math.degrees(math.acos(clamp(normal.z, -1.0, 1.0))), 3),
                    },
                )
            tok("SCATTER_PROGRESS", {"placed": placed, "total": total})

    # Cached Asset template objects were never selected and never entered
    # piece_objects — join_shells/export below can't see them — but they
    # must still be swept before the job ends (see AssetTemplateCache's
    # docstring): a no-op when no Asset piece was ever placed by any layer.
    template_cache.cleanup()

    # Terrain gets the identical per-shell cleanup every piece already got
    # (not a bulk cleanup of a unioned whole — there is no such thing any
    # more), so every shell that reaches join_shells has already had its own
    # isolated remove_doubles pass.
    terrain_bm = bmesh.new()
    terrain_bm.from_mesh(landscape.data)
    cleanup_shell_bm(terrain_bm)
    terrain_bm.to_mesh(landscape.data)
    terrain_bm.free()

    join_shells(landscape, piece_objects)

    bpy.ops.object.select_all(action="DESELECT")
    landscape.select_set(True)
    bpy.context.view_layer.objects.active = landscape
    bpy.ops.wm.stl_export(filepath=out_path, export_selected_objects=True)

    # Validate what was actually WRITTEN (see roundtrip_check's docstring).
    # Loose shells means there is no union seam left to mint n-gons/slivers
    # (see the module docstring's "Loose shells, not unions"), so
    # non_manifold_edges is expected to be exactly 0 here by construction —
    # base_cut.py's lenient ratio gate is kept as the safety net regardless
    # (a designer-supplied landscape could still carry its own pre-existing
    # non-manifold-ness; scatter must not silently launder that away).
    # Raising here lands in main()'s except and becomes SCATTER_FAILED +
    # exit 1. The exported file is left on disk in that case; a failed
    # job's output is never consumed (basecutter::scatter only forwards the
    # SCATTER_DONE path).
    bad_edges, total_edges, shells = roundtrip_check(out_path)
    ratio = (bad_edges / total_edges) if total_edges else 1.0
    if ratio > MAX_NON_MANIFOLD_RATIO:
        raise RuntimeError(
            f"scattered landscape is catastrophically non-manifold "
            f"({bad_edges} of {total_edges} edges)"
        )

    return out_path, placed, bad_edges, total_edges, shells, len(layers)


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    job_path = argv[argv.index("--job") + 1]
    debug = "--debug" in argv
    with open(job_path, encoding="utf-8") as f:
        job = json.load(f)

    tok("SCATTER_START")
    try:
        out, placed, bad_edges, total_edges, shells, num_layers = scatter(job, debug)
    except Exception as e:  # noqa: BLE001 — reported as a token, not a crash
        traceback.print_exc()
        tok("SCATTER_FAILED", {"reason": str(e)})
        sys.exit(1)

    # non_manifold_edges/total_edges/shells/layers are all ADDITIVE payload
    # fields: mild non-manifold-ness (under MAX_NON_MANIFOLD_RATIO — above it
    # scatter() already raised) rides along as a warning-grade detail, shells
    # is the loose-shells honesty field (docs/SCATTER.md "Pieces are placed
    # as LOOSE SHELLS") — terrain + one shell per placed piece across the
    # whole stack, re-measured on the round-tripped file, not assumed — and
    # "layers" is the new stack-size field (docs/SCATTER.md "Layers"). serde's
    # default deserialize ignores unknown JSON fields, so basecutter::scatter's
    # DonePayload {out, placed, manifold, shells, layers} keeps parsing this
    # line whether or not it grows further fields later.
    tok(
        "SCATTER_DONE",
        {
            "out": out,
            "placed": placed,
            "manifold": bad_edges == 0,
            "non_manifold_edges": bad_edges,
            "total_edges": total_edges,
            "shells": shells,
            "layers": num_layers,
        },
    )


main()
