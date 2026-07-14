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
#                                 "count": 1 } } ]
#
# "cutter" is the NOMINAL (bottom-face) footprint; "cut" is Rust's already-
# derived top-face footprint (basecutter::job::write_job_file injects it) —
# the script must not re-derive it from taper/height itself.
#
# The magnet is per placement (chosen from the user's magnet inventory in
# the app; null = no pocket); the clearance is per job because hole fit is
# a printer/material property, not a magnet property. "count" >= 2 spaces
# that many boss/pocket pairs along the footprint's long axis (see
# _magnet_positions' docstring) instead of one centered pocket.
#
# kinds: circle { diameter_mm } | ellipse { major_mm, minor_mm }
#      | rect { width_mm, depth_mm }   (sharp corners — unit blocks tile)
#
# stdout protocol (parsed by basecutter/job.rs):
#   VALIDATING / VALIDATED {json} / VALIDATION_FAILED {json}
#   CUT_START {"index":i} / CUT_DONE {"index":i,"out":...,"dims_mm":[x,y,z],
#   "manifold":bool} / CUT_FAILED {"index":i,"reason":...} / JOB_DONE {json}

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


def cleanup_and_check(obj):
    """Merge stray verts, fix normals; return (manifold, dims_mm).

    dissolve_degenerate matters for the STL roundtrip: booleans can leave
    near-zero-area slivers that are manifold here but collapse to exactly
    zero area in float32 STL — the importer then drops them, leaving a
    pinhole in the printed shell."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=0.001)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=0.001)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    manifold = all(e.is_manifold for e in bm.edges)
    dims = bbox_dims(bm.verts)
    bm.to_mesh(obj.data)
    bm.free()
    return manifold, [round(d, 3) for d in dims]


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


def cut_one(landscape_obj, placement, plinth, out_dir, index):
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
        raise RuntimeError("cut is empty — placement outside the landscape?")

    # Re-home the plug: placement point to origin, rotation undone, so the
    # exported base is axis-aligned like any standalone model.
    unrot = Matrix.Rotation(-math.radians(placement["rotation_deg"]), 4, "Z")
    plug.data.transform(
        unrot @ Matrix.Translation((-placement["x_mm"], -placement["y_mm"], 0.0))
    )

    # Seat: lowest sculpted point onto the plinth top, trim what's beneath
    # (WELD_OVERLAP deep into the top plate, so union gets a real overlap).
    seat = seat_height(plug)
    if seat is None:
        raise RuntimeError("no upward-facing surface inside the cut")
    plug.data.transform(Matrix.Translation((0.0, 0.0, h - seat)))
    trim = big_box("trim", -1000.0, h - WELD_OVERLAP)
    apply_boolean(plug, trim, "DIFFERENCE")
    delete_object(trim)

    base = build_plinth(plinth, cutter, placement["cut"], placement.get("magnet"))
    apply_boolean(base, plug, "UNION")
    delete_object(plug)

    manifold, dims = cleanup_and_check(base)
    name = placement.get("name") or f"base_{index}"
    out = os.path.join(out_dir, f"{name}.stl")
    bpy.ops.object.select_all(action="DESELECT")
    base.select_set(True)
    bpy.context.view_layer.objects.active = base
    bpy.ops.wm.stl_export(filepath=out, export_selected_objects=True)
    delete_object(base)
    return out, dims, manifold


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

    tok("VALIDATING")
    landscape = import_landscape(job["landscape"])
    ok, report = validate(landscape)
    if not ok:
        # The gate is real (not a dead protocol arm): a catastrophically
        # broken landscape stops here, before any Blender time is spent
        # cutting from it. job.rs treats VALIDATION_FAILED as fatal and
        # kills the run — see basecutter::job::spawn_and_parse.
        tok("VALIDATION_FAILED", report)
        return
    tok("VALIDATED", report)

    done = 0
    for i, placement in enumerate(job["placements"]):
        tok("CUT_START", {"index": i})
        try:
            out, dims, manifold = cut_one(landscape, placement, job["plinth"], job["out_dir"], i)
            tok("CUT_DONE", {"index": i, "out": out, "dims_mm": dims, "manifold": manifold})
            done += 1
        except Exception as e:  # one bad placement must not kill the batch
            traceback.print_exc()
            tok("CUT_FAILED", {"index": i, "reason": str(e)})
    tok("JOB_DONE", {"total": len(job["placements"]), "ok": done})


main()
