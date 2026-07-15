# Scatter — sprinkle debris (this spike: generated pebbles/rocks) onto a
# landscape STL and boolean it in. See docs/SCATTER.md — this file implements
# the pinned "landscape TRANSFORMER" pass: landscape.stl + ScatterParams ->
# scatter_landscape.py -> landscape-scattered.stl. It is NOT part of the cut
# job (docs/SCATTER.md "The architectural call"); base_cut.py runs unchanged
# against whatever STL is handed to it, decorated or not.
#
# Job JSON (path after `--job`), all lengths in mm, landscape units = mm,
# Z-up (same conventions as gen_landscape.py / base_cut.py):
# {
#   "landscape_path": "/path/to/landscape.stl",
#   "out_path": "/path/to/landscape-scattered.stl",
#   "params": {
#     "seed": 7,
#     "density_per_dm2": 25.0,        # pieces per 100x100mm (a "dm2" here)
#     "scale": [0.85, 1.15],          # random range AROUND each piece kind's
#                                      # canonical 28-32mm-scale size (see
#                                      # CANONICAL_MM below) — docs/SCATTER.md
#                                      # "Scale anchor: 28-32mm heroic"
#     "scale_factor": 1.0,            # whole-pass rescale for non-28mm work
#     "sink_mm": [0.0, 0.6],          # desired buried-depth range; the script
#                                      # enforces a FLOOR regardless (see
#                                      # "always buried" below) — this range
#                                      # only adds variety ABOVE that floor
#     "align_to_surface": true,       # tilt each piece to the local normal
#     "max_slope_deg": 55.0,          # reject candidates on steeper ground
#     "edge_margin_mm": 3.0,          # keep clear of the landscape's outer
#                                      # planar (x,y) bounding box
#     "pieces": [
#       {"piece": {"Generated": {"kind": "pebble"}}, "weight": 0.6},
#       {"piece": {"Generated": {"kind": "rock"}},   "weight": 0.4}
#     ]
#   }
# }
#
# `piece` is externally tagged, one key names the source — matches Rust's
# default serde derive (no #[serde(tag=...)]) for the PieceChoice.piece enum
# pinned in docs/SCATTER.md: {"Generated": {"kind": "pebble"|"rock"}} or
# {"Asset": {"id": "..."}}. Asset sources are S4 work (a curated/user-library
# mesh); this script recognizes the shape so the job format already matches
# the pinned interface, but FAILS GRACEFULLY on it — see validate_pieces().
#
# stdout protocol (parsed by basecutter::scatter, S2):
#   SCATTER_START
#   SCATTER_PROGRESS {"placed":i,"total":N}
#   SCATTER_DONE {"out":...,"placed":N,"manifold":bool,
#                 "non_manifold_edges":N,"total_edges":M}
#     (manifold/edge counts are measured on the EXPORTED file via a
#     re-import — see roundtrip_check — so they match what base_cut.py's
#     validate will see; mild non-manifold-ness is warning-grade data here,
#     the same lenient policy as base_cut.py's validate, and the job only
#     FAILS above MAX_NON_MANIFOLD_RATIO. The Rust DonePayload currently
#     parses out/placed/manifold and ignores the extra count fields.)
#   SCATTER_FAILED {"reason":...}
#   With --debug (after `--job <path>`), one extra line per placed piece:
#   SCATTER_PIECE {"index":i,"kind":...,"x_mm":...,"y_mm":...,"z_mm":...,
#                  "yaw_deg":...,"size_mm":...,"floor_mm":...,
#                  "embed_depth_mm":...,"aligned_deg":...}
#   (embed_depth_mm is the ACTUAL enforced sink; floor_mm is what the "always
#   buried" rule required — embed_depth_mm >= floor_mm always, the sink-floor
#   proof this line exists for.)
# Exit code: 0 on success; a caught failure prints SCATTER_FAILED then
# sys.exit(1); an uncaught exception propagates and Blender's own
# `--python-exit-code 1` (the render_mini.py/base_cut.py/gen_landscape.py
# convention) turns it into a non-zero exit regardless.
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
# then boolean-union it into the terrain (exact solver, one small piece at a
# time — cheap, per docs/SCATTER.md).
#
# Determinism: ONE seeded random.Random(seed) stream, drawn from in a FIXED
# order (grid cells visited row-major; every cell draws its two jitter
# numbers whether or not it's later accepted; every ACCEPTED candidate then
# draws its piece pick / yaw / size / sink / mesh-noise numbers in that same
# fixed order). Whether a candidate is accepted depends only on the
# landscape mesh + its fixed (x,y) position, never on the RNG, so for the
# same landscape STL + same seed + same params the accept/reject pattern —
# and therefore the whole sequence of numbers drawn — is bit-identical
# across runs. This mirrors gen_landscape.py's boulder layer (one seeded
# stream, a fixed-order per-instance loop), not its stones layer (a position
# hash) — the mechanism scatter needs is "for each of N discrete items",
# exactly the boulders' case, not "evaluate this continuous field at an
# arbitrary point".
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
#   - PEBBLE_MAX_SUBDIV=4 (1280 tris): pebbles get no fine octave and are
#     meant to read smooth/round, not textured, so a coarser cap costs
#     comparatively little visually (still a 16x tri increase over the
#     original 80-tri pebble) while removing the single biggest chunk of
#     the regression (most pebbles were landing on the same level-6 cap
#     as rocks, and there are more pebbles than rocks per the default 0.6/
#     0.4 weight split).
# Measured result at these caps: ~12.2s for the same 35-piece/120x80mm job
# — ~2.0x the original, inside the "if it regresses badly (>3x)" budget.
# See this commit's report for the full before/after/cap-sweep numbers.
ROCK_MAX_SUBDIV = 5
PEBBLE_MAX_SUBDIV = 4

MIN_SUBDIV = 2  # never below "vaguely round" even for a near-zero piece


def subdivision_for_size(size_mm, max_subdiv):
    """Pure function of size_mm (and the per-kind cap) -> Blender's
    icosphere `subdivisions` parameter — the smallest level whose modeled
    max facet edge (see ICOSPHERE_EDGE_K's comment) is <= TARGET_MAX_EDGE_MM,
    clamped to [MIN_SUBDIV, max_subdiv].

    Deterministic by construction: takes only size_mm (itself already
    drawn from the seeded rng stream earlier in build_generated_piece) and
    a compile-time constant — no rng access here, so it can never perturb
    the fixed-order draw sequence the module docstring's determinism proof
    depends on.
    """
    ratio = max(1.0, size_mm * ICOSPHERE_EDGE_K / TARGET_MAX_EDGE_MM)
    level = 1 + math.ceil(math.log2(ratio))
    return int(clamp(level, MIN_SUBDIV, max_subdiv))


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
PEBBLE_NOISE_AMOUNT = 0.30
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
# should never fully determine silhouette.
PEBBLE_SQUASH_Z = (0.55, 0.85)
ROCK_SQUASH_Z = (0.45, 0.78)
ASPECT_XY_RANGE = (0.80, 1.30)

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
# (new_object / apply_boolean / delete_object / bbox helpers / cleanup_and_check
#  are the same recipes — and the same rationale — as base_cut.py's; copied
#  rather than imported because these embedded scripts each run standalone
#  inside Blender's own Python, with no shared package between them.)


def new_object(name, bm):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def apply_boolean(obj, other_obj, operation):
    mod = obj.modifiers.new("bool", "BOOLEAN")
    mod.operation = operation
    mod.solver = "EXACT"
    mod.object = other_obj
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_apply(modifier=mod.name)


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


def cleanup_and_check(obj):
    """Merge stray verts, dissolve degenerate slivers, fix normals, return
    (non_manifold_edges, total_edges, dims_mm). base_cut.py's/
    gen_landscape.py's recipe at heart, run ONCE after every piece is
    unioned in (not per piece) — cheap and matches "cleanup" as the final
    step of the pipeline docs/SCATTER.md describes, not a per-union step —
    but strengthened in two ways for the densified pieces:

      - TRIANGULATE FIRST. The EXACT boolean leaves n-gons along every
        union seam, and the STL exporter triangulates those invisibly at
        write time — so without this step, cleanup checks a mesh that is
        NOT the mesh being exported. With dense pieces the seam n-gons are
        skinny enough that the export-time triangulation sheds degenerate/
        duplicate triangles; the importer then drops the duplicates,
        leaving T-junction holes — non-manifold edges that exist ONLY in
        the exported file (measured on the live-repro job: SCATTER_DONE
        said manifold:true while base_cut.py's validate on the same file
        counted 12 bad edges from 4 dropped duplicate triangles — the
        user-visible "landscape is not manifold" warning). Triangulating
        HERE puts those slivers in front of the dissolve pass below
        instead of hiding them until export, and measured 0 bad edges
        post-roundtrip on that same repro (a coarser dissolve threshold
        WITHOUT triangulation was tried first and made things worse,
        12 -> 42 — the n-gons, not the threshold, were the problem);
      - it iterates until stable (dissolving one sliver can degenerate a
        neighbor), bounded by CLEANUP_MAX_PASSES.

    Returns a COUNT of non-manifold edges (not the old all-or-nothing
    bool) so the caller can apply base_cut.py's lenient ratio gate instead
    of treating one bad edge out of 100k the same as a shredded mesh."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.triangulate(bm, faces=bm.faces)
    for _ in range(CLEANUP_MAX_PASSES):
        verts_before = len(bm.verts)
        faces_before = len(bm.faces)
        bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=MERGE_DIST_MM)
        bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=MERGE_DIST_MM)
        if len(bm.verts) == verts_before and len(bm.faces) == faces_before:
            break
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    total_edges = len(bm.edges)
    bad_edges = sum(1 for e in bm.edges if not e.is_manifold)
    min_x, min_y, min_z, max_x, max_y, max_z = bbox_minmax(bm.verts)
    dims = [round(max_x - min_x, 3), round(max_y - min_y, 3), round(max_z - min_z, 3)]
    bm.to_mesh(obj.data)
    bm.free()
    return bad_edges, total_edges, dims


def roundtrip_check(path):
    """Re-import the just-exported STL and count non-manifold edges THERE —
    the pre-export bmesh count is provably too optimistic: the STL
    importer's exact-duplicate-triangle drop can turn a sliver that was
    manifold in the bmesh into a T-junction in the file every downstream
    consumer (base_cut.py's validate, a slicer) actually reads. This is the
    same import base_cut.py will do, so the count SCATTER_DONE reports is
    the count the cut pipeline will see — no more "scatter said manifold,
    cut said not". Costs one STL import (~1s on a decorated plate),
    the honest price of reporting the truth."""
    imported = import_landscape(path)
    bm = bmesh.new()
    bm.from_mesh(imported.data)
    total_edges = len(bm.edges)
    bad_edges = sum(1 for e in bm.edges if not e.is_manifold)
    bm.free()
    delete_object(imported)
    return bad_edges, total_edges


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


def validate_pieces(pieces_json):
    """Parse the pinned PieceChoice shape and return [(kind, weight), ...]
    for Generated pieces only. An Asset entry is a recognized, well-formed
    part of the pinned interface (S4 will implement it) — so it must NOT
    look like a parse error. It fails with a clear, specific reason instead,
    exactly the case docs/SCATTER.md's task calls out: "parse the shape but
    fail gracefully"."""
    if not pieces_json:
        raise ValueError("params.pieces is empty — nothing to scatter")
    out = []
    for entry in pieces_json:
        piece = entry["piece"]
        weight = float(entry.get("weight", 1.0))
        if "Asset" in piece:
            raise ValueError("assets not supported yet (S4)")
        if "Generated" not in piece:
            raise ValueError(f"unknown piece source: {list(piece.keys())}")
        kind = piece["Generated"]["kind"]
        if kind not in CANONICAL_MM:
            raise ValueError(f"unknown generated piece kind: {kind}")
        if weight > 0.0:
            out.append((kind, weight))
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

def build_candidates(rng, min_x, min_y, max_x, max_y, density_per_dm2, edge_margin_mm):
    """Jittered-grid candidate (x, y) points in FIXED row-major order — see
    the module docstring's determinism note for why the order (and drawing
    both jitter numbers for every cell, accepted or not) matters. density is
    pieces per 100x100mm, so the average area per piece is 10000/density
    mm^2 and the grid step is that area's square root."""
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
    return candidates


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

def scatter(job, debug):
    landscape_path = job["landscape_path"]
    out_path = job["out_path"]
    params = job["params"]

    seed = int(params["seed"])
    density_per_dm2 = float(params["density_per_dm2"])
    scale_lo, scale_hi = params.get("scale", [0.85, 1.15])
    scale_factor = float(params.get("scale_factor", 1.0))
    sink_lo, sink_hi = params.get("sink_mm", [0.0, 0.6])
    align_to_surface = bool(params.get("align_to_surface", True))
    max_slope_deg = float(params.get("max_slope_deg", 55.0))
    edge_margin_mm = float(params.get("edge_margin_mm", 2.0))

    pieces = validate_pieces(params["pieces"])

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()

    landscape = import_landscape(landscape_path)
    src_bm = bmesh.new()
    src_bm.from_mesh(landscape.data)
    min_x, min_y, min_z, max_x, max_y, max_z = bbox_minmax(src_bm.verts)
    bvh = BVHTree.FromBMesh(src_bm)
    ray_z = max_z + RAY_MARGIN_MM
    ray_distance = (max_z - min_z) + 2.0 * RAY_MARGIN_MM

    rng = random.Random(seed)
    grid_candidates = build_candidates(
        rng, min_x, min_y, max_x, max_y, density_per_dm2, edge_margin_mm
    )

    accepted = []
    for x, y in grid_candidates:
        hit = raycast_accept(bvh, x, y, ray_z, ray_distance, max_slope_deg)
        if hit is not None:
            accepted.append(hit)
    src_bm.free()

    total = len(accepted)
    placed = 0
    for loc, normal in accepted:
        kind = pick_piece_kind(rng, pieces)
        yaw = rng.uniform(0.0, 2.0 * math.pi)
        bm, size_mm, bottom_local, height_local = build_generated_piece(
            kind, rng, (scale_lo, scale_hi), scale_factor
        )

        floor_mm = max(MIN_SINK_MM, SINK_FLOOR_FRACTION * height_local)
        raw_sink = rng.uniform(sink_lo, sink_hi)
        final_sink = max(floor_mm, raw_sink)

        world_origin = place_piece(bm, bottom_local, loc, normal, align_to_surface, final_sink, yaw)

        piece_obj = new_object(f"scatter_piece_{placed}", bm)
        apply_boolean(landscape, piece_obj, "UNION")
        delete_object(piece_obj)

        placed += 1
        if debug:
            tok(
                "SCATTER_PIECE",
                {
                    "index": placed - 1,
                    "kind": kind,
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

    cleanup_and_check(landscape)

    bpy.ops.object.select_all(action="DESELECT")
    landscape.select_set(True)
    bpy.context.view_layer.objects.active = landscape
    bpy.ops.wm.stl_export(filepath=out_path, export_selected_objects=True)

    # Validate what was actually WRITTEN (see roundtrip_check's docstring),
    # with base_cut.py's own lenient policy: a mild count is a warning in
    # the payload, only a catastrophic ratio (> MAX_NON_MANIFOLD_RATIO,
    # base_cut's exact threshold) fails the job — raising here lands in
    # main()'s except and becomes SCATTER_FAILED + exit 1. The exported
    # file is left on disk in that case; a failed job's output is never
    # consumed (basecutter::scatter only forwards the SCATTER_DONE path).
    bad_edges, total_edges = roundtrip_check(out_path)
    ratio = (bad_edges / total_edges) if total_edges else 1.0
    if ratio > MAX_NON_MANIFOLD_RATIO:
        raise RuntimeError(
            f"scattered landscape is catastrophically non-manifold "
            f"({bad_edges} of {total_edges} edges)"
        )

    return out_path, placed, bad_edges, total_edges


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    job_path = argv[argv.index("--job") + 1]
    debug = "--debug" in argv
    with open(job_path, encoding="utf-8") as f:
        job = json.load(f)

    tok("SCATTER_START")
    try:
        out, placed, bad_edges, total_edges = scatter(job, debug)
    except Exception as e:  # noqa: BLE001 — reported as a token, not a crash
        traceback.print_exc()
        tok("SCATTER_FAILED", {"reason": str(e)})
        sys.exit(1)

    # non_manifold_edges/total_edges are NEW payload fields: mild
    # non-manifold-ness (under MAX_NON_MANIFOLD_RATIO — above it scatter()
    # already raised) rides along as a warning-grade detail. serde's default
    # deserialize ignores unknown JSON fields, so basecutter::scatter's
    # existing DonePayload {out, placed, manifold} keeps parsing this line
    # unchanged — it just DISCARDS the counts until S2 grows fields for
    # them (see this change's report).
    tok(
        "SCATTER_DONE",
        {
            "out": out,
            "placed": placed,
            "manifold": bad_edges == 0,
            "non_manifold_edges": bad_edges,
            "total_edges": total_edges,
        },
    )


main()
