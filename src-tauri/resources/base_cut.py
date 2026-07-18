# Base Cutter — cut standard-base plugs out of a landscape sculpt and seat
# each on a parametric tapered plinth. See docs/BASECUTTER.md for the plan
# this implements; the geometry rules that look arbitrary are pinned there:
#
#   - Nominal base size is the BOTTOM face (bases tile at the table). The cut
#     footprint — the plug's/plinth's TOP face — is smaller: nominal - 2*inset,
#     inset = height * tan(taper). Rust is the single owner of that
#     derivation (basecutter::cutters::top_face_of); this script never
#     recomputes it. Each placement in the job JSON already carries the
#     derived footprint under "cut" (same tagged shape as "cutter") — the
#     script just consumes it.
#   - The plug sinks until the lowest point of its SCULPTED surface touches
#     the plinth top, then everything below is trimmed — carrier thickness
#     never becomes base height.
#   - Plinths are hollow shells with an optional magnet boss; the magnet
#     pocket opens at the bottom so the magnet glues in flush with the rim.
#   - A validation pass gates the whole job (see validate()'s docstring):
#     catastrophically broken landscapes are rejected before any cutting.
#
# Job JSON (path after `--job`), all lengths in mm, landscape units = mm:
# {
#   "landscape": "/path/to/landscape.stl",
#   "out_dir": "/dir",
#   "plinth": { "height_mm": 3.7, "taper_deg": 15.0, "hollow": true,
#               "wall_mm": 1.2, "top_mm": 1.2, "magnet_clearance_mm": 0.15 },
#   "placements": [ { "name": "round32",
#                     "cutter": { "kind": "circle", "diameter_mm": 32.0 },
#                     "cut": { "kind": "circle", "diameter_mm": 30.017 },
#                     "x_mm": 0.0, "y_mm": 0.0, "rotation_deg": 0.0,
#                     "magnet": { "diameter_mm": 5.0, "height_mm": 1.0,
#                                 "count": 1 } } ],
#   "topper_mm": null,
#   "scatter_rim": "keep",
#   "glb": false
#
# "cutter" is the NOMINAL (bottom-face) footprint; "cut" is Rust's already-
# derived top-face footprint (basecutter::job::write_job_file injects it) —
# the script must not re-derive it from taper/height itself.
#
# "glb" (job-level, not per placement; VTT GLB export design doc's "Base
# cut" section is the spec, "Global conventions" is binding) — see "glb
# mode" below. false/absent (serde(default) on the Rust side) = today's
# behavior EXACTLY: import the bare STL, cut, export STL per cut, nothing
# else touches color or materials.
#
# "scatter_rim" (job-level, not per placement): "keep" (default — an absent
# key behaves exactly like "keep", mirroring topper_mm's own
# serde(default)) or "slice". Only meaningful when `landscape` carries
# scatter shells (docs/SCATTER.md "Pieces are placed as LOOSE SHELLS":
# terrain + one closed shell per placed piece, never unioned) — a landscape
# with nothing scattered onto it (a plain generated bake, a designer
# sculpt) is a single shell and both values behave identically, since
# there is nothing to separate. "slice": every piece is fused into the
# terrain ONCE, job-wide, before any cut runs (fuse_pieces_into_terrain) —
# a piece straddling a rim gets sliced straight through by the cutter
# prism, like any other terrain detail. "keep": per cut, only the TERRAIN
# shell is intersected
# with the cutter prism (seat-and-trim proceeds exactly as if scatter never
# happened); separately, every piece whose CENTROID (XY, precomputed once
# per job) lies inside THIS placement's "cut" footprint is unioned in
# WHOLE — never sliced by the prism — after being carried through the same
# rigid transform the plug itself got (see incorporate_pieces), so it may
# overhang the rim like real hand-made scenic basing. A piece can be
# claimed by more than one placement if their footprints overlap — each
# cut is independent (see cut_one's docstring for the exact union
# ordering and why it was chosen).
#
# The magnet is per placement (chosen from the user's magnet inventory in
# the app; null = no pocket); the clearance is per job because hole fit is
# a printer/material property, not a magnet property. "count" >= 2 spaces
# that many boss/pocket pairs along the footprint's long axis (see
# _magnet_positions' docstring) instead of one centered pocket.
#
# "topper_mm" (job-level, not per placement): null = the normal seat-on-
# plinth flow. A number puts every cut in this job into BASE TOPPER mode —
# no plinth at all, the plug is flat-trimmed topper_mm below its lowest
# sculpted point and exported alone as a glue-on terrain slab for hard
# plastic bases. Clamped here to [MIN_TOPPER_MM, MAX_TOPPER_MM]; if the
# request fell outside that range the clamped value is echoed back in every
# CUT_DONE's "topper_mm_clamped" field. Magnets are ignored in this mode
# (nothing to pocket without a plinth) — a placement that carried one gets
# "magnet_ignored": true in its CUT_DONE instead of a silently dropped spec.
# Total height = topper_mm + relief (the trim plane falls out of the seat
# math below, not a separate computation).
#
# kinds: circle { diameter_mm } | ellipse { major_mm, minor_mm }
#      | rect { width_mm, depth_mm }   (sharp corners — unit blocks tile)
#
# glb mode ("glb": true) — colored GLB export for VTT use, alongside the
# STL that always gets cut regardless of this flag:
#   - The landscape is imported from its `.glb` twin (swap extension of
#     "landscape"), NOT the bare STL — a bare STL carries no color/material
#     data, so there is nothing to export. A missing twin is a hard error:
#     VALIDATION_FAILED before any cutting (see main()), same shape as an
#     ordinary failed validate() report, with a human-readable "warning".
#     After import, the color attribute is renamed to "Col" and the three
#     stlpack_* materials are normalized/recreated exactly like
#     scatter_landscape.py's import_landscape_colored/_ensure_landscape_
#     materials (see that script's docstring — this mirrors it). The
#     import is then re-welded (see _reweld_glb_import): the glTF format
#     is per-vertex, so a per-corner-colored mesh comes back from the
#     round-trip with (geometrically identical, but topologically) many
#     more vertices than gen_landscape.py originally built — every
#     topology-dependent step downstream (validate, separate_into_shells,
#     every boolean) needs that undone first.
#   - Plinth mode: before the plug<->plinth union, the plinth mesh's "Col"
#     corners are painted uniformly with the base color recovered from the
#     imported landscape's own bottom face (see _sample_base_color; falls
#     back to DEFAULT_BASE_HEX), and every plinth face points at the
#     landscape's own `stlpack_base` material datablock (reused, not
#     recreated, so the union/join never ends up with two same-named
#     materials as separate slots).
#   - Repair pass (glb mode only), after the final union/trim + scatter
#     incorporation + cleanup_and_check, right before export (see
#     repair_glb_colors): the EXACT boolean and the cut-wall/seam-ring
#     geometry it creates come back with ZERO-filled (0,0,0,0) corner
#     colors — not interpolated (empirically verified, see the design
#     doc). This pass (a) averages, per vertex, every corner with alpha >
#     0; (b) fills every zero-alpha corner from that vertex average, else
#     the average of its own face's painted corners, else the base color;
#     (c) forces every downward-facing (normal.z < -0.5) face to the base
#     color and the `stlpack_base` material slot (looked up by name on the
#     final object — never hardcoded, since a union/join's material-slot
#     order isn't guaranteed the same across modes). Topper mode (no
#     plinth) gets the identical pass; its cut walls just fill from
#     whatever terrain/base neighbors happen to be there.
#   - Export: the STL is written exactly as in non-glb mode. Immediately
#     after, the SAME object is scale-baked to true-size meters
#     (`obj.data.transform(Matrix.Scale(0.001, 4))` — mm -> m, done AFTER
#     the STL export so the STL never sees it) and exported as a `.glb`
#     twin next to the cut STL (design doc convention 6's exact export
#     call). CUT_DONE gains `"glb": <path>` — present only in glb mode.
#
# stdout protocol (parsed by basecutter/job.rs):
#   VALIDATING / VALIDATED {json} / VALIDATION_FAILED {json}
#   CUT_START {"index":i} / CUT_DONE {"index":i,"out":...,"dims_mm":[x,y,z],
#   "manifold":bool, ...additive fields below} / CUT_FAILED {"index":i,
#   "reason":...} / JOB_DONE {json}
#
# CUT_DONE's "out" is never a guessed "{out_dir}/{name}.stl" — it's whatever
# unique_out_path (below) actually wrote to, which gets a "-1", "-2", ...
# suffix the moment out_dir already holds a file of that name (a second job
# run into the same out_dir, or two placements that share a name in
# different jobs). Mirrors file::utils::unique_path's convention Rust-side
# (render/commands.rs and basecutter::commands::export_cuts both use it for
# THEIR outputs) so a re-run never silently clobbers an earlier base.
#
# CUT_DONE's additive fields (all optional, present only when relevant —
# see docs/BASECUTTER.md "Pinned interfaces"):
#   "fused": false + "shells": N  — normal mode only: the plug/plinth union
#     left N > 1 loose shells behind (the union tripwire below). The cut
#     still counts as a success; this only makes a silent non-fuse visible
#     instead of the accident that motivated this feature (CUT_DONE reported
#     success while the STL held two loose shells).
#   "topper_mm_clamped": t — topper mode only, present only when the
#     requested topper_mm was outside [MIN_TOPPER_MM, MAX_TOPPER_MM].
#   "magnet_ignored": true — topper mode only, present when the placement
#     carried a magnet spec that this mode can't pocket.
#   "glb": path — glb mode only (see "glb mode" above): the cut's GLB twin,
#     same stem as "out", meters scale.

import json
import math
import os
import sys
import traceback

import bmesh
import bpy
from mathutils import Matrix, Vector

# The exported base overlaps plug and plinth by this much so the union sees
# two overlapping solids, never two solids kissing on a shared plane (which
# the exact solver can turn into a non-manifold seam).
WELD_OVERLAP = 0.2

CIRCLE_SEGMENTS = 96

# Non-manifold-edge fraction (of total edges) above which validate() treats
# the landscape as catastrophically broken rather than merely noisy.
MAX_NON_MANIFOLD_RATIO = 0.02

# Any bounding-box dimension under this is "not a landscape" (a zero-
# thickness plane, an empty/degenerate import).
MIN_BBOX_DIM_MM = 0.1

# Sane upper bound on magnets-per-placement — past this it stops being a
# plausible mounting pattern for any base in the seed library.
MAX_MAGNET_COUNT = 4

# Usable range for BASE TOPPER mode's topper_mm (docs/BASECUTTER.md's
# BaseCutJob.topper_mm note: "t clamped ~1..3, default 1.5"). Requests
# outside this range are clamped, not rejected — main() echoes the clamped
# value back in every CUT_DONE via "topper_mm_clamped" when it changed the
# input. commands.rs's own guard is deliberately looser (it only rejects
# non-finite/absurd values); this script's clamp is the real usable range.
MIN_TOPPER_MM = 1.0
MAX_TOPPER_MM = 3.0

# Fallback plinth/plastic color when glb mode can't recover one from the
# imported landscape (missing "Col" data, no downward-facing face found —
# never observed against this script's own GLB twins, defense in depth
# only). Same value as MaterialPalette::default().base (generator.rs) and
# scatter_landscape.py's own DEFAULT_BASE_HEX — all three are independent
# copies of the one constant the design doc's palette table pins.
DEFAULT_BASE_HEX = "#232227"


def tok(name, payload=None):
    line = name if payload is None else name + " " + json.dumps(payload)
    print(line, flush=True)


# ---------------------------------------------------------------- footprints

def _ellipse_ring(a, b, segments=CIRCLE_SEGMENTS):
    """CCW ring of 2D Vectors for an axis-aligned ellipse with semi-axes
    (a, b), centered at origin. A circle is just the a == b case — see
    circle_ring below."""
    return [
        Vector((math.cos(t) * a, math.sin(t) * b))
        for t in (i * 2.0 * math.pi / segments for i in range(segments))
    ]


def circle_ring(diameter_mm, segments=CIRCLE_SEGMENTS):
    """CCW ring for a circle of the given diameter. Used directly by the
    magnet boss/pocket mounts instead of round-tripping through a synthetic
    {"kind": "circle"} cutter dict."""
    r = diameter_mm / 2.0
    return _ellipse_ring(r, r, segments)


def footprint_polygon(cutter):
    """Nominal or derived-cut outline (both use the same tagged shape), CCW,
    centered at origin. 2D Vectors."""
    kind = cutter["kind"]
    if kind == "circle":
        return circle_ring(cutter["diameter_mm"])
    if kind == "ellipse":
        return _ellipse_ring(cutter["major_mm"] / 2.0, cutter["minor_mm"] / 2.0)
    if kind == "rect":
        w, d = cutter["width_mm"] / 2.0, cutter["depth_mm"] / 2.0
        return [Vector((w, d)), Vector((-w, d)), Vector((-w, -d)), Vector((w, -d))]
    raise ValueError(f"unknown cutter kind: {kind}")


def long_axis(cutter):
    """(unit direction, length_mm) of the footprint's longest axis, used to
    space multiple magnets along big ovals/rects (docs/BASECUTTER.md's
    magnet-mount plan). Circles have no distinguished axis; +X is an
    arbitrary but harmless default since count>=2 on a round base is an
    unusual case, not one the seed library relies on."""
    kind = cutter["kind"]
    if kind == "circle":
        return Vector((1.0, 0.0)), cutter["diameter_mm"]
    if kind == "ellipse":
        return Vector((1.0, 0.0)), cutter["major_mm"]
    if kind == "rect":
        w, d = cutter["width_mm"], cutter["depth_mm"]
        return (Vector((1.0, 0.0)), w) if w >= d else (Vector((0.0, 1.0)), d)
    raise ValueError(f"unknown cutter kind: {kind}")


def offset_inward(poly, dist):
    """Uniform inward offset with mitered corners. Exact for circles and
    right-angle rects; for ellipses the miter is a per-vertex approximation
    of the true inner offset curve (good to well under a print line width
    at 96 segments). Used ONLY for the hollow plinth's cavity wall offset —
    the nominal->cut (top-face) derivation is Rust's job, not this
    function's (see footprint_polygon(placement["cut"]) in cut_one)."""
    n = len(poly)
    out = []
    for i in range(n):
        p_prev, p, p_next = poly[i - 1], poly[i], poly[(i + 1) % n]
        e1 = (p - p_prev).normalized()
        e2 = (p_next - p).normalized()
        n1 = Vector((-e1.y, e1.x))  # inward normal of a CCW polygon edge
        n2 = Vector((-e2.y, e2.x))
        m = n1 + n2
        if m.length < 1e-9:
            m = n1.copy()
        m.normalize()
        out.append(p + m * (dist / max(m.dot(n1), 0.2)))
    return out


# ------------------------------------------------------------ mesh building

def new_object(name, bm):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def loft_solid(name, rings):
    """Closed solid from stacked CCW polygon rings [(z, poly2d), ...],
    bottom to top. All rings must share a vertex count."""
    bm = bmesh.new()
    layers = [
        [bm.verts.new((p.x, p.y, z)) for p in poly] for z, poly in rings
    ]
    for a, b in zip(layers, layers[1:]):
        for i in range(len(a)):
            j = (i + 1) % len(a)
            bm.faces.new((a[i], a[j], b[j], b[i]))
    bm.faces.new(tuple(reversed(layers[0])))
    bm.faces.new(tuple(layers[-1]))
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return new_object(name, bm)


def big_box(name, z_lo, z_hi, half_extent=10000.0):
    e = half_extent
    poly = [Vector((e, e)), Vector((-e, e)), Vector((-e, -e)), Vector((e, -e))]
    return loft_solid(name, [(z_lo, poly), (z_hi, poly)])


def apply_boolean(obj, cutter_obj, operation):
    mod = obj.modifiers.new("bool", "BOOLEAN")
    mod.operation = operation
    mod.solver = "EXACT"
    mod.object = cutter_obj
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.modifier_apply(modifier=mod.name)


def delete_object(obj):
    """Removing the object alone leaves its mesh datablock orphaned — Blender
    never garbage-collects those mid-session. Each cut creates several
    transient bodies (prism, plug, trim, plinth body, cavity, boss(es),
    pocket(s)) — including full landscape-sized copies of the plug — so a
    ~50-cut batch that only unlinked objects would leak roughly 300 mesh
    datablocks over the run. Grab the mesh before removing the object, then
    drop the mesh too once nothing else references it."""
    mesh = obj.data
    bpy.data.objects.remove(obj, do_unlink=True)
    if mesh is not None and mesh.users == 0:
        bpy.data.meshes.remove(mesh)


def bbox_dims(verts):
    """Bounding-box dimensions via a single streaming min/max pass over
    `verts` — deliberately not `[v.co.x for v in verts]` three times over.
    On a multi-million-vert sculpt, materializing three full Python float
    lists (xs/ys/zs) is hundreds of MB of transient allocation just to take
    a max() of each; one pass with running min/max avoids it entirely."""
    min_x = min_y = min_z = math.inf
    max_x = max_y = max_z = -math.inf
    seen = False
    for v in verts:
        seen = True
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
    if not seen:
        return [0.0, 0.0, 0.0]
    return [max_x - min_x, max_y - min_y, max_z - min_z]


def count_shells(bm):
    """Number of disconnected mesh islands ("loose shells") via a
    face-adjacency flood fill over shared edges. Backs the plug-plinth union
    tripwire in cut_one: a union that silently failed to fuse leaves 2+
    shells behind even though the boolean op itself reported success and the
    result may still be technically manifold/printable — this is exactly the
    accident that motivated BASE TOPPER mode (CUT_DONE said success while
    the STL held a floating plug and a separate plinth)."""
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


def cleanup_and_check(obj):
    """Merge stray verts, fix normals; return (manifold, dims_mm, shells).

    dissolve_degenerate matters for the STL roundtrip: booleans can leave
    near-zero-area slivers that are manifold here but collapse to exactly
    zero area in float32 STL — the importer then drops them, leaving a
    pinhole in the printed shell.

    `shells` (see count_shells) is computed post-cleanup so it reflects what
    actually gets exported, not an intermediate state merge_doubles might
    still stitch back together."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=0.001)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=0.001)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    manifold = all(e.is_manifold for e in bm.edges)
    dims = bbox_dims(bm.verts)
    shells = count_shells(bm)
    bm.to_mesh(obj.data)
    bm.free()
    return manifold, [round(d, 3) for d in dims], shells


# ---------------------------------------------------------------- the plinth

def _magnet_positions(cutter, magnet):
    """x/y offsets (2D Vectors, mm) for `magnet["count"]` boss/pocket pairs.

    count == 1 (the common case): a single pocket centered at the origin,
    same as before magnet counts existed.

    count >= 2: `count` pairs spaced along the footprint's long axis
    (long_axis above), laid out symmetrically around the origin so the
    group is centered as a whole:

        spacing = long_dimension / (count + 1)
        position[i] = direction * (i - (count - 1) / 2) * spacing

    e.g. count=2 on a 120mm-long oval: spacing = 120/3 = 40mm, magnets at
    +/-20mm (40mm apart, each 20mm shy of the footprint's ends — comfortably
    inside the wall on any seed-library oval). count=4 gives four magnets at
    +/-0.5*spacing and +/-1.5*spacing. Clamped to MAX_MAGNET_COUNT."""
    count = max(1, min(int(magnet.get("count", 1)), MAX_MAGNET_COUNT))
    if count == 1:
        return [Vector((0.0, 0.0))]
    direction, long_dim = long_axis(cutter)
    spacing = long_dim / (count + 1)
    return [direction * ((i - (count - 1) / 2.0) * spacing) for i in range(count)]


def build_plinth(plinth, cutter, cut, magnet):
    """Hollow (or solid) tapered plinth at the origin: nominal footprint at
    z=0, the placement's already-derived `cut` footprint (a tagged cutter
    dict, same shape as `cutter`) at z=height. The nominal->cut shrink is
    Rust's derivation (basecutter::cutters::top_face_of) — this function
    just lofts between the two footprints it's handed, it does not compute
    the taper inset for the top face itself. `magnet` is the placement's
    chosen MagnetSpec dict (optionally with "count" >= 1 — see
    _magnet_positions), or None for no pocket(s)."""
    h = plinth["height_mm"]
    nominal = footprint_polygon(cutter)
    top = footprint_polygon(cut)
    body = loft_solid("plinth", [(0.0, nominal), (h, top)])

    clearance = plinth.get("magnet_clearance_mm", 0.15)
    positions = _magnet_positions(cutter, magnet) if magnet else []

    if plinth.get("hollow", True):
        wall = plinth["wall_mm"]
        top_plate = plinth["top_mm"]
        ceiling = h - top_plate
        # Cavity follows the taper at constant wall thickness; it pokes out
        # the bottom so the subtraction leaves an open rim, not a skin. This
        # offset_inward use is the cavity wall's own thickness — unrelated
        # to (and not a substitute for) the nominal->cut top-face derivation.
        cavity = loft_solid(
            "cavity",
            [
                (-1.0, offset_inward(nominal, wall)),
                (0.0, offset_inward(nominal, wall)),
                (ceiling, offset_inward(nominal, wall + ceiling * math.tan(math.radians(plinth["taper_deg"])))),
            ],
        )
        for j, offset in enumerate(positions):
            # Each boss survives the hollowing: a pillar from the ceiling to
            # the bottom plane that the pocket is later drilled into.
            r_boss = magnet["diameter_mm"] / 2.0 + clearance + wall
            boss = loft_solid(
                f"boss_{j}",
                [
                    (-1.0, [p + offset for p in circle_ring(r_boss * 2.0)]),
                    (ceiling, [p + offset for p in circle_ring(r_boss * 2.0)]),
                ],
            )
            apply_boolean(cavity, boss, "DIFFERENCE")
            delete_object(boss)
        apply_boolean(body, cavity, "DIFFERENCE")
        delete_object(cavity)

    for j, offset in enumerate(positions):
        r_pocket = magnet["diameter_mm"] / 2.0 + clearance
        pocket = loft_solid(
            f"pocket_{j}",
            [
                (-1.0, [p + offset for p in circle_ring(r_pocket * 2.0)]),
                (magnet["height_mm"], [p + offset for p in circle_ring(r_pocket * 2.0)]),
            ],
        )
        apply_boolean(body, pocket, "DIFFERENCE")
        delete_object(pocket)

    return body


# --------------------------------------------------- scatter_rim: shells

def _bbox_diagonal_mm(obj):
    """3D bounding-box diagonal length of one object's mesh, in its own
    local/world coordinates (separated shells carry an identity
    matrix_world — see piece_centroid_xy's docstring — so local == world
    here). The sizing signal separate_into_shells actually needs: see that
    function's docstring for why this replaced a vertex-count heuristic."""
    dx, dy, dz = bbox_dims(obj.data.vertices)
    return math.sqrt(dx * dx + dy * dy + dz * dz)


def separate_into_shells(landscape_obj):
    """Split `landscape_obj` into one object per loose (disconnected) mesh
    island, via Blender's own LOOSE separate — docs/SCATTER.md "Pieces are
    placed as LOOSE SHELLS": scatter_landscape.py no longer unions pieces
    into the terrain, so the file this script imports is terrain + one
    closed shell per placed piece, and this is the inverse operation that
    pulls them back apart. Returns (terrain_obj, piece_objs): the shell
    with the LARGEST BOUNDING-BOX DIAGONAL is the terrain, everything else
    is a piece.

    This was a vertex-count heuristic in an earlier revision of this
    function — "a landscape sculpt is thousands to millions of verts, a
    scatter piece is at most a few thousand" — and that reasoning is
    exactly backwards for a coarse/low-poly landscape: verified against
    real Blender, a 100x100mm test plate with only ~2000 base vertices
    scattered with rocks near scatter_landscape.py's ROCK_MAX_SUBDIV cap
    (5120 tris, ~2562 verts each) produced SEVERAL rock shells that
    out-verted the terrain outright, so vertex count picked a 9x9x5.6mm
    rock as "terrain" — the cutter prism then intersected empty space for
    every dead-center placement (reported as "cut is empty", the exact
    bug class this function's diagnostics exist to catch — see cut_one's
    `shell_diagnostics` parameter). Bounding-box diagonal has no such
    failure mode: it's a PHYSICAL-SCALE signal, not a mesh-density one.
    docs/SCATTER.md's own scale anchor guarantees the margin is huge in
    either direction that matters — "Pieces are placed as LOOSE SHELLS"
    describes the landscape as "a fair bit larger than any single base"
    (which itself runs up to 160mm+ in the seed library), while scatter
    pieces are anchored at 28-32mm-heroic scale (a "large rock tops out
    around 12mm" even before jitter/scale_factor) — there is no design-
    intended landscape+scatter combination where a piece's bbox diagonal
    approaches the terrain's.

    A landscape with nothing scattered onto it — a plain generated bake, a
    designer sculpt straight off disk — is the normal single-shell case:
    it comes back with `piece_objs == []`, one shell, nothing to claim.
    `terrain_obj` may or may not be the SAME Python object as the
    `landscape_obj` passed in — see the caller warning below.

    CALLER MUST use the RETURNED `terrain_obj`, never the `landscape_obj`
    it passed in, for anything downstream (the cutter-prism intersect,
    z-range, etc.). `bpy.ops.mesh.separate(type="LOOSE")` mutates
    `landscape_obj`'s OWN mesh data in place to hold WHICHEVER shell
    Blender's operator happens to leave in it — not necessarily the
    terrain — and creates new objects for the rest. Reusing the original
    `landscape_obj` reference after calling this function is exactly the
    bug this docstring exists to prevent. main() reassigns its `landscape`
    variable to this function's return value for exactly this reason, and
    cut_one's own empty-plug error is enriched with the shell count /
    terrain dims computed here so a future regression of this class fails
    loudly instead of looking like a placement-coordinate bug."""
    before = set(bpy.data.objects)
    bpy.ops.object.select_all(action="DESELECT")
    landscape_obj.select_set(True)
    bpy.context.view_layer.objects.active = landscape_obj
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.separate(type="LOOSE")
    bpy.ops.object.mode_set(mode="OBJECT")

    new_objs = [o for o in bpy.data.objects if o not in before]
    all_objs = [landscape_obj] + new_objs
    # Primary: bbox diagonal (the physically meaningful signal — see the
    # docstring above). Secondary/tertiary: vertex count then object name,
    # purely to keep the sort fully deterministic in the astronomically
    # unlikely event two shells tie on bbox diagonal exactly.
    all_objs.sort(key=lambda o: (_bbox_diagonal_mm(o), len(o.data.vertices), o.name), reverse=True)
    terrain_obj = all_objs[0]
    piece_objs = all_objs[1:]
    terrain_obj.name = "terrain"
    for i, obj in enumerate(piece_objs):
        obj.name = f"scatter_piece_{i}"
    return terrain_obj, piece_objs


def piece_centroid_xy(obj):
    """Mean vertex position (X, Y only — Z doesn't matter for the
    point-in-footprint test) of one separated piece, in WORLD space.
    `obj.matrix_world` should be identity after separate_into_shells (the
    piece's world position is already baked into its vertex coordinates —
    see scatter_landscape.py's place_piece), but this reads through
    matrix_world anyway rather than assuming identity, since that's the one
    API call that's correct regardless. Computed ONCE per piece at job
    start (docs/BASECUTTER.md's scatter_rim note: "compute each piece's
    centroid + bbox once"), then reused unchanged for every placement's own
    point_in_polygon test — the piece doesn't move until a placement claims
    it and transforms its own COPY (see incorporate_pieces)."""
    mw = obj.matrix_world
    verts = obj.data.vertices
    n = len(verts)
    if n == 0:
        return Vector((0.0, 0.0))
    sum_x = sum_y = 0.0
    for v in verts:
        co = mw @ v.co
        sum_x += co.x
        sum_y += co.y
    return Vector((sum_x / n, sum_y / n))


def point_in_polygon(pt, poly):
    """Even-odd (crossing-number) point-in-polygon test, 2D, ray cast along
    +X. `poly` is a closed ring of world-space Vector((x, y)) points — the
    same shape footprint_polygon/the `placed` ring in cut_one already use,
    so this is the natural "is this piece's centroid under this cut" test
    scatter_rim="keep" needs (docs/BASECUTTER.md: pieces whose CENTROID
    lies inside the placement's CUT footprint are claimed by that cut).
    Works for any simple polygon regardless of cutter kind (circle/ellipse/
    rect all go through footprint_polygon already) and honors whatever
    rotation/translation the caller baked into `poly` (cut_one passes the
    already placement-rotated-and-translated `placed` ring, never the
    origin-centered `cut_poly`)."""
    x, y = pt.x, pt.y
    inside = False
    n = len(poly)
    for i in range(n):
        x1, y1 = poly[i].x, poly[i].y
        x2, y2 = poly[(i + 1) % n].x, poly[(i + 1) % n].y
        if (y1 > y) != (y2 > y):
            x_at_y = x1 + (y - y1) * (x2 - x1) / (y2 - y1)
            if x < x_at_y:
                inside = not inside
    return inside


def fuse_pieces_into_terrain(terrain_obj, piece_objs):
    """scatter_rim="slice": weld every scattered piece into the terrain
    ONCE, before any cut runs, removing the piece/terrain shell boundary
    before cut_one ever sees it — a piece straddling a rim then gets
    sliced straight through by the cutter prism, exactly like any other
    terrain detail. This runs job-wide (not per cut), since "slice" doesn't
    distinguish between cuts: a fused piece is just terrain, for every
    placement in this job.

    The pieces are first JOINED into a single mesh (plain data
    concatenation, no solver) and then unioned into the terrain in ONE
    EXACT-solver pass. The obvious alternative — a union per piece in a loop
    — is quadratic: every union rebuilds the BVH over the whole GROWING
    terrain, so a dense carpet (hundreds of leaves/twigs) turns into
    hundreds of ever-costlier solver calls and the cut appears to hang at
    0%. One join + one union is a single BVH build over the full piece set
    and yields the same welded solid (EXACT handles the joined mesh's own
    piece-piece overlaps in the same pass). Mutates `terrain_obj` in place
    and consumes (deletes) every piece object; the caller's `piece_objs`
    list is empty of anything usable afterward."""
    if not piece_objs:
        return
    bpy.ops.object.select_all(action="DESELECT")
    for piece_obj in piece_objs:
        piece_obj.select_set(True)
    bpy.context.view_layer.objects.active = piece_objs[0]
    bpy.ops.object.join()  # all pieces collapse into piece_objs[0], no solver
    merged = piece_objs[0]
    apply_boolean(terrain_obj, merged, "UNION")
    delete_object(merged)


def incorporate_pieces(target, claimed, rehome, seat_shift):
    """Add each CLAIMED scatter piece (scatter_rim="keep", see cut_one) to
    `target` as a LOOSE closed shell — a plain data JOIN, never a boolean
    union. Each piece first gets the exact same rigid transform the plug
    itself already got: `rehome` (undo this placement's rotation, recenter
    the cut at the origin) then a pure Z shift of `seat_shift` (h - seat in
    normal mode, topper_mm - seat in topper mode — see cut_one) — same
    two-call sequence, same values, so a piece's embedding depth relative to
    the terrain (scatter's own "always buried" floor, docs/SCATTER.md)
    survives exactly as it was at scatter time.

    Why JOIN, not UNION: a boolean union WHOLE was the original design, but
    the EXACT solver SHATTERS on carpet-density scatter — dozens of thin,
    curled, mutually-overlapping leaf/twig shells make it produce non-
    manifold slivers and, in the worst case, fragment the base body itself
    (a 25mm base cut from a forest floor came back as 70+ non-manifold
    shells; terrain-only cut clean). Loose overlapping closed shells are
    what the scatter architecture already ships and what slicers union
    natively at print time (docs/SCATTER.md "Pieces are placed as LOOSE
    SHELLS... slicers and the cut pipeline handle overlapping shells"), and
    the piece still overhangs the rim like scenic basing — it's just welded
    by the slicer, not by a fragile pre-print boolean. The base BODY's own
    plug/plinth fusion is still a real boolean union, gated separately in
    cut_one BEFORE these loose pieces are added.

    A fresh COPY per placement, never the original piece object: the same
    piece can be claimed by more than one placement (docs/BASECUTTER.md —
    "two overlapping cutters can both contain a centroid... each cut is
    independent"), so the separated piece object must survive untouched,
    at its original position, for the next placement's own claim test and
    its own independent copy."""
    if not claimed:
        return
    copies = []
    for j, (piece_obj, _centroid) in enumerate(claimed):
        piece_copy = piece_obj.copy()
        piece_copy.data = piece_obj.data.copy()
        piece_copy.name = f"{target.name}_piece_{j}"
        bpy.context.collection.objects.link(piece_copy)
        piece_copy.data.transform(rehome)
        piece_copy.data.transform(Matrix.Translation((0.0, 0.0, seat_shift)))
        copies.append(piece_copy)
    bpy.ops.object.select_all(action="DESELECT")
    target.select_set(True)
    for piece_copy in copies:
        piece_copy.select_set(True)
    bpy.context.view_layer.objects.active = target
    bpy.ops.object.join()  # loose shells concatenated into target, no solver


# ------------------------------------------------------------------ the cut

def unique_out_path(out_dir, name):
    """First non-existing `{out_dir}/{name}.stl`, else `{name}-1.stl`,
    `{name}-2.stl`, ... — the never-clobber convention Rust already applies
    to ITS OWN outputs (file::utils::unique_path, shared by
    render/commands.rs and basecutter::commands::export_cuts). Placement
    names come from a per-JOB counter on the frontend (BaseCutter.vue's
    nextNames — restarts at -1 for a fresh placements list, since names are
    scoped to one job/session), so two SEPARATE jobs cutting into the same
    out_dir can easily mint the same "round32.stl" — that used to silently
    overwrite the earlier base. Checked here, at write time, rather than
    only at the frontend's validate_placements (which only ever sees ONE
    job's placements and can't know what an earlier job already wrote to
    disk)."""
    candidate = os.path.join(out_dir, f"{name}.stl")
    if not os.path.exists(candidate):
        return candidate
    n = 1
    while True:
        candidate = os.path.join(out_dir, f"{name}-{n}.stl")
        if not os.path.exists(candidate):
            return candidate
        n += 1


def seat_height(plug_obj):
    """Lowest z of the sculpted surface: min over vertices of upward-facing
    faces. The plug's walls are vertical and its carrier bottom faces down,
    so upward faces are terrain by construction."""
    lowest = None
    mesh = plug_obj.data
    for poly in mesh.polygons:
        if poly.normal.z > 0.5:
            for vi in poly.vertices:
                z = mesh.vertices[vi].co.z
                if lowest is None or z < lowest:
                    lowest = z
    return lowest


def cut_one(
    landscape_obj,
    placement,
    plinth,
    out_dir,
    index,
    topper_mm=None,
    pieces=None,
    shell_diagnostics=None,
    glb=False,
    base_hex=None,
    base_mats=None,
    base_mat_index=None,
):
    """Cut one placement.

    `glb`/`base_hex`/`base_mats`/`base_mat_index` are glb-mode-only (see
    the module docstring's "glb mode" section); `glb=False` (the default)
    means none of that code runs — this function's normal-mode behavior is
    otherwise byte-for-byte what it was before glb mode existed. `base_hex`
    is the palette base color main() recovered once per job
    (_sample_base_color); `base_mats`/`base_mat_index` are the imported
    landscape's full material list and `stlpack_base`'s position within it
    — see _paint_plinth_base's docstring for why the plinth needs the WHOLE
    list, not just the one slot it uses, before it ever touches the union.

    `topper_mm` is None for the normal seat-on-plinth flow (unchanged). A
    float (already clamped to [MIN_TOPPER_MM, MAX_TOPPER_MM] by main())
    puts this cut in BASE TOPPER mode instead: build the cutter prism and
    boolean-intersect exactly as normal, but rather than seating the plug on
    a generated plinth, flat-trim it topper_mm below its lowest sculpted
    point and export it alone — no plinth is built, no union happens. The
    cut footprint stays the TOP face in both modes (it's the face that glues
    onto the hard plastic base in topper mode) — see the module docstring's
    "topper_mm" note.

    `pieces` is scatter_rim="keep"'s claim pool: `None` (scatter_rim="slice",
    or a landscape with no scatter shells at all — see main()) means there
    is nothing to claim, and this function then does exactly the
    seat+trim+plinth-union flow with nothing else touching plug/base.
    Otherwise a list of `(piece_obj, centroid_xy)` — see
    separate_into_shells/piece_centroid_xy — precomputed ONCE per job, not
    per placement. `landscape_obj` here is ALWAYS the already-verified
    terrain shell (main() reassigns its `landscape` variable to
    separate_into_shells' return value before any cut runs — see that
    function's docstring for why using anything else is a bug class of its
    own, not just a style preference).

    `shell_diagnostics` is `(shell_count, terrain_dims_mm)` or `None`
    (a single-shell landscape — nothing scattered onto it — never
    separates, so there's nothing extra to report) — folded into the
    empty-plug error message so
    a future regression in shell selection fails with facts ("terrain shell
    is 40x40x2mm, 3 shells detected") instead of the misleading "placement
    outside the landscape?" a mis-picked tiny piece shell would otherwise
    produce for a placement that is demonstrably centered on the plate.

    Returns (out_path, dims_mm, manifold, extra) where `extra` is a dict of
    additive CUT_DONE payload fields (magnet_ignored / fused+shells / glb —
    the last only in glb mode), empty when there's nothing to report for
    this cut."""
    pieces = pieces or []
    cutter = placement["cutter"]
    h = plinth["height_mm"]
    # The cut footprint is Rust's derivation (placement["cut"]), not
    # recomputed here — see the module docstring and build_plinth's.
    cut_poly = footprint_polygon(placement["cut"])

    # Cutter prism at the placement, spanning past the landscape's z-range.
    rot = Matrix.Rotation(math.radians(placement["rotation_deg"]), 2)
    placed = [rot @ p + Vector((placement["x_mm"], placement["y_mm"])) for p in cut_poly]
    z_lo = min(v.co.z for v in landscape_obj.data.vertices) - 1.0
    z_hi = max(v.co.z for v in landscape_obj.data.vertices) + 1.0
    prism = loft_solid("prism", [(z_lo, placed), (z_hi, placed)])

    # Plug = landscape ∩ prism, on a copy so the landscape survives the run.
    plug = landscape_obj.copy()
    plug.data = landscape_obj.data.copy()
    plug.name = f"plug_{index}"
    bpy.context.collection.objects.link(plug)
    apply_boolean(plug, prism, "INTERSECT")
    delete_object(prism)
    if len(plug.data.vertices) == 0:
        reason = "cut is empty — placement outside the landscape?"
        if shell_diagnostics is not None:
            shell_count, terrain_dims = shell_diagnostics
            reason += (
                f" ({shell_count} shell(s) detected, terrain shell dims "
                f"{terrain_dims[0]:.1f}x{terrain_dims[1]:.1f}x{terrain_dims[2]:.1f}mm — "
                f"if the placement looks correct, this may be a shell "
                f"mis-selection, not a coordinate problem)"
            )
        raise RuntimeError(reason)

    # Re-home the plug: placement point to origin, rotation undone, so the
    # exported base is axis-aligned like any standalone model.
    unrot = Matrix.Rotation(-math.radians(placement["rotation_deg"]), 4, "Z")
    rehome = unrot @ Matrix.Translation((-placement["x_mm"], -placement["y_mm"], 0.0))
    plug.data.transform(rehome)

    seat = seat_height(plug)
    if seat is None:
        raise RuntimeError("no upward-facing surface inside the cut")

    # scatter_rim="keep": which pieces THIS cut claims, by CENTROID (XY)
    # against the world-space cut footprint `placed` (see point_in_polygon).
    # Centroids themselves are precomputed once per job (docstring above);
    # only the inside/outside test is per placement, since it depends on
    # this placement's own footprint. A piece can be claimed by more than
    # one placement — see incorporate_pieces' docstring.
    claimed = [(obj, centroid) for obj, centroid in pieces if point_in_polygon(centroid, placed)]

    extra = {}
    if topper_mm is not None:
        # Topper mode: sink so the lowest sculpted point sits topper_mm
        # above a flat bottom at z=0, then trim everything below that plane.
        # That flat trim plane IS the final bottom face — unlike the normal
        # flow there's no plinth union afterwards, so no WELD_OVERLAP is
        # needed. Total height falls out of this for free: max_z -
        # (seat - topper_mm) = topper_mm + (max_z - seat) = topper_mm +
        # relief. Claimed pieces are unioned in WHOLE here, BEFORE the flat
        # trim (docs/BASECUTTER.md: "Topper mode composes: keep/slice apply
        # the same way, then the flat trim") — the trim is the one rule
        # that defines this mode's entire bottom face, so it must run LAST,
        # after every solid (terrain plug + any claimed pieces) that could
        # possibly dip below it is already part of the assembly. A piece
        # that pokes below the trim plane gets trimmed exactly like terrain
        # would — consistent, not a special case.
        seat_shift = topper_mm - seat
        plug.data.transform(Matrix.Translation((0.0, 0.0, seat_shift)))
        incorporate_pieces(plug, claimed, rehome, seat_shift)
        if claimed:
            extra["scatter_pieces"] = len(claimed)
        trim = big_box("trim", -1000.0, 0.0)
        apply_boolean(plug, trim, "DIFFERENCE")
        delete_object(trim)
        base = plug
        if placement.get("magnet"):
            # There's no plinth to pocket a magnet into — surface that
            # instead of silently dropping the placement's magnet spec.
            extra["magnet_ignored"] = True
        manifold, dims, shells = cleanup_and_check(base)
    else:
        # Seat: lowest sculpted point onto the plinth top, trim what's
        # beneath (WELD_OVERLAP deep into the top plate, so union gets a
        # real overlap). With `claimed` empty (no scatter, or no piece
        # under this footprint) this is exactly the seat+trim+plinth-union
        # flow with nothing else touching plug/base — see cut_one's
        # docstring.
        seat_shift = h - seat
        plug.data.transform(Matrix.Translation((0.0, 0.0, seat_shift)))
        trim = big_box("trim", -1000.0, h - WELD_OVERLAP)
        apply_boolean(plug, trim, "DIFFERENCE")
        delete_object(trim)

        base = build_plinth(plinth, cutter, placement["cut"], placement.get("magnet"))
        if glb:
            # Paint the FULLY BUILT plinth (cavity/bosses/pockets already
            # drilled) before it ever touches the plug — see
            # _paint_plinth_base's docstring for why it needs plug's WHOLE
            # material table (not just the one slot it uses) to avoid a
            # boolean-merge ambiguity that empirically drops materials.
            _paint_plinth_base(base, base_hex, base_mats, base_mat_index)
        apply_boolean(base, plug, "UNION")
        delete_object(plug)

        # Check the base BODY (plug + plinth, the WELD_OVERLAP seam) FIRST,
        # before any scatter is added. This is the real fusion gate — a plug
        # that silently failed to weld to its plinth leaves >1 shell behind
        # even though the boolean and manifold check both "succeeded" (the
        # accident that motivated topper mode; see the module docstring). By
        # gating here, a keep-mode cut's core seam is validated
        # boolean-call-for-boolean-call IDENTICALLY to a scatter-free cut,
        # and — crucially — the deliberately-LOOSE scatter shells added next
        # can't false-trip this tripwire, nor can cleanup_and_check's
        # remove_doubles weld overlapping leaves into non-manifold junctions
        # (it runs here, on the body alone, not over the loose pieces).
        manifold, dims, shells = cleanup_and_check(base)
        if shells > 1:
            extra["fused"] = False
            extra["shells"] = shells

        # Claimed pieces join in as LOOSE shells AFTER the body is fused and
        # checked (see incorporate_pieces for why loose, not unioned). They
        # overhang the rim like scenic basing and the slicer welds them at
        # print time. `dims` is re-measured over the whole assembly so the
        # reported footprint includes an overhanging piece.
        if claimed:
            incorporate_pieces(base, claimed, rehome, seat_shift)
            extra["scatter_pieces"] = len(claimed)
            verts = base.data.vertices
            dims = [
                round(max(v.co[i] for v in verts) - min(v.co[i] for v in verts), 3)
                for i in range(3)
            ]

    if glb:
        # After the final union/trim, scatter incorporation, and
        # cleanup_and_check above (both branches), before export — see
        # repair_glb_colors' docstring. Runs in both plinth and topper
        # mode; topper mode's "base" is just the plug, no plinth to have
        # pre-painted.
        repair_glb_colors(base, base_hex)

    name = placement.get("name") or f"base_{index}"
    out = unique_out_path(out_dir, name)
    bpy.ops.object.select_all(action="DESELECT")
    base.select_set(True)
    bpy.context.view_layer.objects.active = base
    bpy.ops.wm.stl_export(filepath=out, export_selected_objects=True)

    if glb:
        # Scale-bake true-size meters AFTER the STL export (design doc
        # convention 5 — the STL must never see this transform) then export
        # the GLB twin with the same selection state the STL export just
        # used (convention 6).
        base.data.transform(Matrix.Scale(0.001, 4))
        glb_out = os.path.splitext(out)[0] + ".glb"
        bpy.ops.export_scene.gltf(
            filepath=glb_out,
            export_format="GLB",
            use_selection=True,
            export_vertex_color="ACTIVE",
            export_active_vertex_color_when_no_material=True,
        )
        extra["glb"] = glb_out
    delete_object(base)
    return out, dims, manifold, extra


# ---------------------------------------------------------------------- job

def import_landscape(path):
    """The app's supported floor is Blender 4.2, where wm.stl_import (and
    wm.stl_export) both exist — no fallback to the legacy import_mesh.stl
    operator, which would falsely suggest pre-4.1 support while export (no
    legacy equivalent shipped) would crash regardless."""
    before = set(bpy.data.objects)
    bpy.ops.wm.stl_import(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    if len(new) != 1:
        raise RuntimeError(f"expected 1 imported object, got {len(new)}")
    return new[0]


# ------------------------------------------------------------- glb mode
# (color helpers, GLB import, plinth painting, and the post-boolean color
# repair pass — see the module docstring's "glb mode" section. The color
# helpers below are byte-for-byte the same recipe gen_landscape.py's and
# scatter_landscape.py's own identically-named copies use — VTT GLB export
# design doc's "Global conventions" #2/#3 are binding on every script that
# touches "Col"/stlpack_* materials, and each embedded script runs inside
# Blender's own Python with no shared package to import a common copy from
# — see scatter_landscape.py's own docstring for the same standalone-script
# reasoning.)


def _srgb_to_linear(c):
    """sRGB [0,1] -> linear [0,1] — Blender shader node `default_value`
    inputs want linear (design doc convention 3). `.color_srgb` (used for
    the "Col" attribute itself) converts on its own and must NOT be
    pre-linearized."""
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def _hex_to_rgb01(hex_str):
    """"#rrggbb" -> (r, g, b) each in [0, 1], sRGB (no conversion)."""
    h = hex_str.lstrip("#")
    return tuple(int(h[i : i + 2], 16) / 255.0 for i in (0, 2, 4))


def _hex_to_linear_rgba(hex_str):
    r, g, b = _hex_to_rgb01(hex_str)
    return (_srgb_to_linear(r), _srgb_to_linear(g), _srgb_to_linear(b), 1.0)


def _rgb01_to_hex(r, g, b):
    """Inverse of _hex_to_rgb01, clamped — used by _sample_base_color to
    turn a sampled "Col" corner back into the "#rrggbb" string the rest of
    this module's color helpers (and DEFAULT_BASE_HEX) traffic in."""
    def byte(c):
        return max(0, min(255, round(c * 255)))

    return "#{:02x}{:02x}{:02x}".format(byte(r), byte(g), byte(b))


def _make_terrain_material():
    """stlpack_terrain: Base Color driven by the "Col" corner attribute via
    a Color Attribute node (design doc convention 2). Defense in depth only
    — see _ensure_landscape_materials' docstring — never observed missing
    against this script's own GLB twins."""
    mat = bpy.data.materials.new("stlpack_terrain")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Roughness"].default_value = 0.85
    bsdf.inputs["Metallic"].default_value = 0.0
    col_node = mat.node_tree.nodes.new("ShaderNodeVertexColor")
    col_node.layer_name = "Col"
    mat.node_tree.links.new(col_node.outputs["Color"], bsdf.inputs["Base Color"])
    return mat


def _make_base_material(base_hex):
    """stlpack_base: uniform "plastic" — no vertex-color node. Defense in
    depth only, same as _make_terrain_material above."""
    mat = bpy.data.materials.new("stlpack_base")
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = _hex_to_linear_rgba(base_hex)
    bsdf.inputs["Roughness"].default_value = 0.45
    bsdf.inputs["Metallic"].default_value = 0.0
    return mat


def _material_index(mesh, name):
    """Slot index of the material named `name` on `mesh`, or None. Used
    instead of a hardcoded index (e.g. "stlpack_base is always slot 1")
    because a boolean union's/join's merged material-slot order is an
    implementation detail this script must not assume — see the module
    docstring's "glb mode" note on why the repair pass looks this up by
    name on the FINAL object instead."""
    for i, mat in enumerate(mesh.materials):
        if mat is not None and mat.name == name:
            return i
    return None


def import_landscape_glb(glb_path):
    """Import a landscape's `.glb` twin (glb mode has no bare-STL fallback
    — see the module docstring: a bare STL carries no color/material data,
    so there is nothing to export, and main() turns a missing twin into a
    VALIDATION_FAILED token before this is ever called). Mirrors scatter_
    landscape.py's import_landscape_colored, minus the had_glb_twin return
    (glb mode here is all-or-nothing, never a fallback path) — filters to
    MESH type for the same defense-in-depth reason that function's
    docstring gives (the glTF importer isn't guaranteed to produce only a
    mesh for every possible file, even though this script's own twins
    always do)."""
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=glb_path)
    new = [o for o in bpy.data.objects if o not in before and o.type == "MESH"]
    if len(new) != 1:
        raise RuntimeError(f"expected 1 imported mesh object from {glb_path}, got {len(new)}")
    return new[0]


def _reweld_glb_import(obj):
    """Re-weld vertices the glTF export/import round-trip split apart by
    per-corner attributes, restoring the topology gen_landscape.py's own
    mesh had before it was ever exported to GLB.

    glTF is a per-VERTEX format: every attribute (position, normal, "Col")
    must be uniform across a vertex, so Blender's exporter emits one glTF
    vertex per unique (position, normal, color) combination, and the
    importer reconstructs exactly that — one Blender vertex per unique
    corner instead of one per shared position. Since every top-surface
    vertex in this pipeline carries its own small per-vertex color jitter
    (gen_landscape.py's _paint_landscape: "organic mottling, deterministic"
    — see also the ±6% per-piece jitter scatter_landscape.py applies),
    virtually every corner ends up attribute-unique, so a freshly
    GLB-imported landscape comes back geometrically identical but almost
    entirely vertex-split: two faces that share an edge on the original
    heightfield now reference two DIFFERENT (though co-located) vertex
    objects, so bmesh — and this script's own validate() — sees that edge
    as a boundary, not a shared one. Empirically verified: a clean 120x80mm
    lava-flow bake came back from a straight GLB round-trip reporting
    essentially every edge as non-manifold, tripping VALIDATION_FAILED on
    landscapes that gen_landscape.py's own STL-export path validates
    clean.

    remove_doubles at the same tolerance cleanup_and_check already uses
    (0.001mm) merges these coincident verts back into single topological
    vertices by POSITION — CORNER-domain "Col" data is untouched (it's
    keyed by loop, not vertex; see the design doc's own empirical note
    that "Col" SURVIVES remove_doubles on original geometry), so every
    face keeps exactly the color it had. Only vertex IDENTITY is restored,
    which is exactly what every downstream topology-dependent step
    (validate, separate_into_shells, every boolean) needs. Mirrors
    cleanup_and_check's own bmesh round-trip shape, minus the
    dissolve_degenerate/recalc_normals/manifold-report parts that function
    needs and this one doesn't — this runs on a mesh nothing has cut yet."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=0.001)
    bm.to_mesh(obj.data)
    bm.free()


def _ensure_landscape_materials(obj):
    """Guarantee `obj` ends up with "Col" as its active color attribute
    name and canonical (un-suffixed) `stlpack_*` material names, mirroring
    scatter_landscape.py's _ensure_landscape_materials — see that
    function's docstring for the full rationale (the GLB importer comes
    back with the color attribute possibly named "Color" and materials
    suffixed like "stlpack_terrain.001", design doc convention 7).
    base_cut.py's glb mode has no STL-fallback branch (unlike scatter's),
    so this only ever runs against a real GLB import; recreating a
    canonical material that's missing entirely is defense in depth only,
    same as scatter's copy — never observed against this script's own GLB
    twins, which always carry stlpack_terrain + stlpack_base (glow is
    lava-only and NOT reconstructed here, same reasoning as scatter: this
    script has no palette to rebuild its color/strength from)."""
    mesh = obj.data

    if "Col" not in mesh.color_attributes:
        active = mesh.color_attributes.active_color
        if active is not None:
            active.name = "Col"
    for mat in mesh.materials:
        if mat is None:
            continue
        for canonical in ("stlpack_terrain", "stlpack_base", "stlpack_glow"):
            if mat.name == canonical or mat.name.startswith(canonical + "."):
                mat.name = canonical
                break

    col = mesh.color_attributes.get("Col")
    if col is not None:
        mesh.color_attributes.active_color = col

    names = {m.name for m in mesh.materials if m is not None}
    if "stlpack_terrain" not in names:
        mesh.materials.append(_make_terrain_material())
    if "stlpack_base" not in names:
        mesh.materials.append(_make_base_material(DEFAULT_BASE_HEX))


def _sample_base_color(obj):
    """Recover the palette "base" color baked into the imported landscape's
    skirt/bottom (gen_landscape.py's _paint_landscape colors every bottom/
    skirt corner uniformly with palette["base"], material_index = 1 — see
    that function's docstring). Samples one corner of a face already
    assigned the stlpack_base MATERIAL slot and converts it back to a
    "#rrggbb" string. Never raises — a landscape whose .glb twin somehow
    lost its base slot or its "Col" data falls back to DEFAULT_BASE_HEX
    rather than failing the whole cut over a cosmetic detail.

    The material slot is the one identification that's SEMANTIC, not
    geometric — both geometric candidates fail on a scattered landscape:
    "any z-down face" finds a mushroom cap's underside first (empirically
    painted a whole plinth mushroom-brown), and "the global-min-z face"
    finds a grass tuft, because sunk pieces genuinely poke below the
    z=0 bottom plane (empirically min z was -0.1mm on a scattered bake).
    Pieces never carry the base slot — gen_landscape.py assigns it to
    skirt/bottom faces only, and scatter paints every piece face
    stlpack_terrain."""
    mesh = obj.data
    col = mesh.color_attributes.get("Col")
    if col is None:
        return DEFAULT_BASE_HEX
    base_idx = _material_index(mesh, "stlpack_base")
    if base_idx is None:
        return DEFAULT_BASE_HEX
    for poly in mesh.polygons:
        if poly.material_index == base_idx:
            r, g, b, _a = col.data[poly.loop_indices[0]].color_srgb
            return _rgb01_to_hex(r, g, b)
    return DEFAULT_BASE_HEX


def _paint_plinth_base(plinth_obj, base_hex, base_mats, base_mat_index):
    """Paint the plinth's "Col" corners uniformly with `base_hex` and point
    every face at the `stlpack_base` slot — called on the FULLY BUILT
    plinth (cavity + bosses + pockets already drilled by build_plinth),
    BEFORE the plug<->plinth union (see cut_one).

    `base_mats` is the imported landscape's FULL material list (main()
    reads it once via `list(landscape.data.materials)`) and `base_mat_index`
    is `stlpack_base`'s position within it. The plinth's OWN material list
    is set to this exact same list, in the exact same order, rather than
    just the one slot it actually uses — empirically verified against real
    Blender: the boolean modifier's material-list merge does NOT reliably
    extend the TARGET object's own slot table to cover material indices the
    OPERAND references that the target never had (a plinth pre-seeded with
    only `[stlpack_base]` came back from the union with the plug's terrain/
    glow faces landing on a slot Blender's glTF exporter reported as
    "DefaultMaterial" — i.e. no material at all — even though the datablock
    identity was shared, per the identity-dedup reasoning below). Giving
    the plinth the SAME table `plug` already carries (plug.data.materials
    is the identical list, unmutated, since plug = landscape_obj.copy()) at
    the SAME base_mat_index means both operands agree on every slot before
    the union ever runs — no merge/extend ambiguity for the solver to get
    wrong. This is also why identity (not just name) matters: reusing the
    landscape's own material datablocks, not fresh lookalikes, is what lets
    the repair pass find "the" `stlpack_base` slot afterward with the union
    not somehow ending up with duplicate same-named slots."""
    mesh = plinth_obj.data
    for mat in base_mats:
        mesh.materials.append(mat)
    col = mesh.color_attributes.new(name="Col", type="BYTE_COLOR", domain="CORNER")
    mesh.color_attributes.active_color = col
    r, g, b = _hex_to_rgb01(base_hex)
    for li in range(len(mesh.loops)):
        col.data[li].color_srgb = (r, g, b, 1.0)
    for poly in mesh.polygons:
        poly.material_index = base_mat_index


def repair_glb_colors(obj, base_hex):
    """Fill every zero-alpha "Col" corner the EXACT boolean/dissolve left
    behind on newly created geometry (cut walls, the top seam ring, drilled
    magnet pockets — empirically verified: original geometry keeps its
    colors, new geometry comes back (0,0,0,0), see the module docstring's
    "glb mode" section and the design doc's top-of-file empirical notes).
    Runs once, at the very end of cut_one (after the final union/trim,
    scatter incorporation, and cleanup_and_check), on whatever `obj` is in
    either mode — plinth mode's plug+plinth union, or topper mode's plug
    alone (no plinth, so cut walls fill from whatever terrain/base
    neighbors happen to be there instead).

    Three passes over the mesh, in order (design doc's exact recipe):
      1. Per-vertex average of every corner with alpha > 0, one streaming
         pass over all loops (same reason bbox_dims avoids materializing
         full coordinate lists — a multi-million-loop landscape shouldn't
         cost hundreds of MB of transient Python objects for this).
      2. Every zero-alpha corner takes its vertex's pass-1 average if one
         exists; else the average of its OWN FACE's other painted corners
         (computed fresh per face, from the pass-1 state — never from a
         corner this same pass already repaired, since corner (loop)
         indices are per-face and never shared across faces even when they
         reference the same vertex); else `base_hex` as a last resort.
      3. Every face whose normal points down (normal.z < -0.5) is forced to
         `base_hex` and the `stlpack_base` material slot — looked up by
         name on `obj` itself (see _material_index's docstring for why
         nothing here assumes a fixed slot number)."""
    mesh = obj.data
    col = mesh.color_attributes.get("Col")
    if col is None:
        return  # defense in depth only — glb mode always paints "Col" in
        # somewhere upstream (import or _paint_plinth_base)

    r_base, g_base, b_base = _hex_to_rgb01(base_hex)

    # Pass 1: per-vertex average of painted corners, one streaming pass.
    vert_sum = {}
    vert_n = {}
    for li, loop in enumerate(mesh.loops):
        c = col.data[li].color_srgb
        if c[3] > 0.0:
            vi = loop.vertex_index
            s = vert_sum.get(vi)
            if s is None:
                vert_sum[vi] = [c[0], c[1], c[2]]
                vert_n[vi] = 1
            else:
                s[0] += c[0]
                s[1] += c[1]
                s[2] += c[2]
                vert_n[vi] += 1
    vert_avg = {vi: (s[0] / vert_n[vi], s[1] / vert_n[vi], s[2] / vert_n[vi]) for vi, s in vert_sum.items()}

    # Pass 2: fill zero-alpha corners (vertex average -> face-mate average
    # -> base color).
    for poly in mesh.polygons:
        loop_idxs = poly.loop_indices
        painted = [col.data[li].color_srgb for li in loop_idxs if col.data[li].color_srgb[3] > 0.0]
        if painted:
            n = len(painted)
            face_avg = (
                sum(c[0] for c in painted) / n,
                sum(c[1] for c in painted) / n,
                sum(c[2] for c in painted) / n,
            )
        else:
            face_avg = None
        for li in loop_idxs:
            c = col.data[li].color_srgb
            if c[3] > 0.0:
                continue
            vi = mesh.loops[li].vertex_index
            if vi in vert_avg:
                r, g, b = vert_avg[vi]
            elif face_avg is not None:
                r, g, b = face_avg
            else:
                r, g, b = r_base, g_base, b_base
            col.data[li].color_srgb = (r, g, b, 1.0)

    # Pass 3: bottom faces are always base color + stlpack_base, regardless
    # of what pass 2 just interpolated for them.
    base_idx = _material_index(mesh, "stlpack_base")
    if base_idx is not None:
        for poly in mesh.polygons:
            if poly.normal.z < -0.5:
                poly.material_index = base_idx
                for li in poly.loop_indices:
                    col.data[li].color_srgb = (r_base, g_base, b_base, 1.0)


def validate(obj):
    """Lenient-by-design gate (docs/BASECUTTER.md "an up-front validation
    pass... gates the whole job"): FAIL only when the landscape is
    catastrophically broken —

      - zero faces (empty/failed import),
      - any bounding-box dimension under MIN_BBOX_DIM_MM (a paper-thin
        plane, a degenerate mesh — not real terrain), or
      - non-manifold edges over MAX_NON_MANIFOLD_RATIO of all edges.

    The exact boolean solver tolerates plenty of ordinary mesh noise, so a
    small amount of non-manifold-ness alone still PASSES, just with a
    warning in the report — only a landscape that's unusable outright stops
    the job before any cutting happens (via VALIDATION_FAILED, see main())."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    total_edges = len(bm.edges)
    bad_edges = sum(1 for e in bm.edges if not e.is_manifold)
    dims = bbox_dims(bm.verts)
    face_count = len(bm.faces)
    vert_count = len(bm.verts)
    bm.free()

    report = {
        "non_manifold_edges": bad_edges,
        "dims_mm": [round(d, 2) for d in dims],
        "verts": vert_count,
    }

    non_manifold_ratio = (bad_edges / total_edges) if total_edges else 1.0
    catastrophic = (
        face_count == 0
        or any(d < MIN_BBOX_DIM_MM for d in dims)
        or non_manifold_ratio > MAX_NON_MANIFOLD_RATIO
    )
    if catastrophic:
        return False, report
    if bad_edges:
        report["warning"] = "landscape is not manifold"
    return True, report


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    job_path = argv[argv.index("--job") + 1]
    with open(job_path, encoding="utf-8") as f:
        job = json.load(f)

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()

    os.makedirs(job["out_dir"], exist_ok=True)

    # BASE TOPPER mode (job-level, applies to every placement in this job —
    # see the module docstring's "topper_mm" note). Clamped here, not just
    # bounds-checked in Rust: commands.rs only guards non-finite/absurd
    # requests, this script owns the real usable range.
    raw_topper_mm = job.get("topper_mm")
    topper_mm = None
    topper_mm_clamped = None
    if raw_topper_mm is not None:
        topper_mm = min(max(float(raw_topper_mm), MIN_TOPPER_MM), MAX_TOPPER_MM)
        if abs(topper_mm - raw_topper_mm) > 1e-9:
            topper_mm_clamped = topper_mm

    # scatter_rim (docs/BASECUTTER.md's BaseCutJob.scatter_rim note):
    # "keep" (default, mirrors topper_mm's own serde(default) pattern — see
    # job.rs) or "slice". Only two values reach this script — Rust's
    # ScatterRim enum is the validation gate — so an unrecognized string
    # here (a hand-edited job file) is treated as "keep", the safer of the
    # two (never silently fuses pieces the caller didn't ask to fuse).
    scatter_rim = job.get("scatter_rim", "keep")

    # glb mode (job-level — see the module docstring's "glb mode" section).
    # false/absent = today's behavior exactly, byte-for-byte: import the
    # bare STL, cut, export STL, nothing else touches color/materials.
    glb = bool(job.get("glb", False))

    tok("VALIDATING")
    if glb:
        # A bare STL carries no color/material data — glb mode has no
        # STL-fallback branch (unlike scatter_landscape.py's own colored
        # import), so a missing twin is a hard, up-front failure, shaped
        # exactly like an ordinary failed validate() report (job.rs/
        # commands.rs read the "warning" string regardless of which path
        # produced VALIDATION_FAILED).
        glb_landscape_path = os.path.splitext(job["landscape"])[0] + ".glb"
        if not os.path.isfile(glb_landscape_path):
            tok(
                "VALIDATION_FAILED",
                {
                    "non_manifold_edges": 0,
                    "dims_mm": [0.0, 0.0, 0.0],
                    "verts": 0,
                    "warning": (
                        "glb mode requires a GLB twin next to the landscape "
                        f"— not found: {glb_landscape_path}"
                    ),
                },
            )
            return
        landscape = import_landscape_glb(glb_landscape_path)
        _ensure_landscape_materials(landscape)
        _reweld_glb_import(landscape)
    else:
        landscape = import_landscape(job["landscape"])
    ok, report = validate(landscape)
    if not ok:
        # The gate is real (not a dead protocol arm): a catastrophically
        # broken landscape stops here, before any Blender time is spent
        # cutting from it. job.rs treats VALIDATION_FAILED as fatal and
        # kills the run — see basecutter::job::spawn_and_parse. Validation
        # runs on the STILL-COMBINED import (before separate_into_shells
        # below): closed shells contribute 0 non-manifold edges regardless
        # of how many there are, so a multi-shell scattered landscape is
        # validated exactly like a single-shell one — no separation needed
        # for this check to be correct (verified against real Blender: a
        # 26-shell scattered plate validates with non_manifold_edges: 0,
        # same as an unscattered one).
        tok("VALIDATION_FAILED", report)
        return
    tok("VALIDATED", report)

    # Recovered once per job, not per cut: the palette "base" color baked
    # into the landscape's own skirt/bottom, and the landscape's FULL
    # material list + stlpack_base's index within it — every plinth this
    # job builds gets the identical list (see _paint_plinth_base's
    # docstring for why the plinth needs the whole table, not just the one
    # slot it uses, before it ever touches the union).
    base_hex = _sample_base_color(landscape) if glb else None
    base_mats = list(landscape.data.materials) if glb else None
    base_mat_index = _material_index(landscape.data, "stlpack_base") if glb else None

    # Separate the imported landscape into its loose shells (docs/
    # SCATTER.md "Pieces are placed as LOOSE SHELLS") and reassign
    # `landscape` to the VERIFIED terrain shell — see
    # separate_into_shells' docstring for why reusing the pre-separation
    # reference here would be a bug, not a style choice. A landscape with
    # nothing scattered onto it (a plain generated bake, a designer
    # sculpt) is the normal single-shell case: it comes back with
    # `piece_objs == []` and `landscape` unchanged in all but name, so
    # everything below this point is a no-op for it — both scatter_rim
    # modes are then identical, per the pinned interface.
    landscape, piece_objs = separate_into_shells(landscape)
    shell_diagnostics = (
        1 + len(piece_objs),
        bbox_dims(landscape.data.vertices),
    )

    pieces_for_cut = None
    if scatter_rim == "slice":
        # Fuse every piece into the terrain ONCE, job-wide, before any cut
        # runs (see fuse_pieces_into_terrain's docstring). After this,
        # `landscape` is a single-shell mesh and cut_one needs no
        # awareness of scatter at all: pieces_for_cut stays None.
        fuse_pieces_into_terrain(landscape, piece_objs)
    elif piece_objs:
        # scatter_rim="keep": precompute every piece's centroid ONCE
        # (docs/BASECUTTER.md — "compute each piece's centroid + bbox
        # once"); the per-placement inside/outside test happens inside
        # cut_one, since it depends on that placement's own cut footprint.
        pieces_for_cut = [(obj, piece_centroid_xy(obj)) for obj in piece_objs]

    done = 0
    for i, placement in enumerate(job["placements"]):
        tok("CUT_START", {"index": i})
        try:
            out, dims, manifold, extra = cut_one(
                landscape,
                placement,
                job["plinth"],
                job["out_dir"],
                i,
                topper_mm,
                pieces_for_cut,
                shell_diagnostics,
                glb=glb,
                base_hex=base_hex,
                base_mats=base_mats,
                base_mat_index=base_mat_index,
            )
            payload = {"index": i, "out": out, "dims_mm": dims, "manifold": manifold}
            if topper_mm_clamped is not None:
                payload["topper_mm_clamped"] = topper_mm_clamped
            payload.update(extra)
            tok("CUT_DONE", payload)
            done += 1
        except Exception as e:  # one bad placement must not kill the batch
            traceback.print_exc()
            tok("CUT_FAILED", {"index": i, "reason": str(e)})
    tok("JOB_DONE", {"total": len(job["placements"]), "ok": done})


main()
