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
#   "scatter_rim": "keep"
#
# "cutter" is the NOMINAL (bottom-face) footprint; "cut" is Rust's already-
# derived top-face footprint (basecutter::job::write_job_file injects it) —
# the script must not re-derive it from taper/height itself.
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
# stdout protocol (parsed by basecutter/job.rs):
#   VALIDATING / VALIDATED {json} / VALIDATION_FAILED {json}
#   CUT_START {"index":i} / CUT_DONE {"index":i,"out":...,"dims_mm":[x,y,z],
#   "manifold":bool, ...additive fields below} / CUT_FAILED {"index":i,
#   "reason":...} / JOB_DONE {json}
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
    terrain detail. Each union is a plain EXACT-solver boolean, done here
    job-wide, once per piece, rather than per cut, since "slice" doesn't
    distinguish between cuts: a fused piece is just terrain, for every
    placement in this job. Mutates `terrain_obj` in place and consumes
    (deletes) every piece object; the caller's `piece_objs` list is empty
    of anything usable afterward."""
    for piece_obj in piece_objs:
        apply_boolean(terrain_obj, piece_obj, "UNION")
        delete_object(piece_obj)


def incorporate_pieces(target, claimed, rehome, seat_shift):
    """Union each CLAIMED scatter piece (scatter_rim="keep", see cut_one)
    WHOLE into `target`, after applying the exact same rigid transform the
    plug itself already got: `rehome` (undo this placement's rotation,
    recenter the cut at the origin) then a pure Z shift of `seat_shift`
    (h - seat in normal mode, topper_mm - seat in topper mode — see
    cut_one) — same two-call sequence, same values, so a piece's embedding
    depth relative to the terrain (scatter's own "always buried" floor,
    docs/SCATTER.md) survives the transform exactly as it was at scatter
    time. The piece is NEVER intersected with the cutter prism: "union it
    in WHOLE" (docs/BASECUTTER.md) is what lets a piece overhang the rim
    like real hand-made scenic basing instead of being sliced at the
    boundary — slicing IS scatter_rim="slice"'s job, done once at job start
    by fuse_pieces_into_terrain, not here.

    A fresh COPY per placement, never the original piece object: the same
    piece can be claimed by more than one placement (docs/BASECUTTER.md —
    "two overlapping cutters can both contain a centroid... each cut is
    independent"), so the separated piece object must survive untouched,
    at its original position, for the next placement's own claim test and
    its own independent copy."""
    for j, (piece_obj, _centroid) in enumerate(claimed):
        piece_copy = piece_obj.copy()
        piece_copy.data = piece_obj.data.copy()
        piece_copy.name = f"{target.name}_piece_{j}"
        bpy.context.collection.objects.link(piece_copy)
        piece_copy.data.transform(rehome)
        piece_copy.data.transform(Matrix.Translation((0.0, 0.0, seat_shift)))
        apply_boolean(target, piece_copy, "UNION")
        delete_object(piece_copy)


# ------------------------------------------------------------------ the cut

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
):
    """Cut one placement.

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
    additive CUT_DONE payload fields (magnet_ignored / fused+shells),
    empty when there's nothing to report for this cut."""
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
        trim = big_box("trim", -1000.0, 0.0)
        apply_boolean(plug, trim, "DIFFERENCE")
        delete_object(trim)
        base = plug
        if placement.get("magnet"):
            # There's no plinth to pocket a magnet into — surface that
            # instead of silently dropping the placement's magnet spec.
            extra["magnet_ignored"] = True
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
        apply_boolean(base, plug, "UNION")
        delete_object(plug)
        # Claimed pieces union in WHOLE AFTER the plinth union, not before
        # (docs/BASECUTTER.md gives the choice explicitly) — deliberately,
        # so the plug<->plinth WELD_OVERLAP seam (the exact union this
        # tripwire was built to police, see cleanup_and_check/the fused
        # tripwire below) runs on the SAME two operands, in the SAME order,
        # as a job with no scatter pieces at all: a keep-mode cut whose
        # footprint claims zero pieces is therefore not just similar to but
        # IDENTICAL, boolean-call-for-boolean-call, to a cut with no piece
        # anywhere near it. When a piece IS claimed, it becomes a third,
        # separate union against an ALREADY-fused plinth+plug — so the final shell count
        # below still validates the whole assembly (a piece that fails to
        # weld is just as visible as a plug that fails to weld), but the
        # core seam's own correctness is never entangled with, or put at
        # risk by, whatever geometry a scattered rock happens to bring in.
        incorporate_pieces(base, claimed, rehome, seat_shift)

    manifold, dims, shells = cleanup_and_check(base)
    if topper_mm is None and shells > 1:
        # The union tripwire: a plug that silently failed to fuse with its
        # plinth leaves >1 loose shell behind even though the boolean op and
        # the manifold check both reported success — the exact accident
        # that motivated topper mode existing (see the module's top
        # docstring). The cut still counts as a success (the mesh may still
        # be printable as loose parts); this only makes the silent case
        # visible.
        extra["fused"] = False
        extra["shells"] = shells

    name = placement.get("name") or f"base_{index}"
    out = os.path.join(out_dir, f"{name}.stl")
    bpy.ops.object.select_all(action="DESELECT")
    base.select_set(True)
    bpy.context.view_layer.objects.active = base
    bpy.ops.wm.stl_export(filepath=out, export_selected_objects=True)
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

    tok("VALIDATING")
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
