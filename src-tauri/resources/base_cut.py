# Base Cutter — cut standard-base plugs out of a landscape sculpt and seat
# each on a parametric tapered plinth. See docs/BASECUTTER.md for the plan
# this implements; the geometry rules that look arbitrary are pinned there:
#
#   - Nominal base size is the BOTTOM face (bases tile at the table), so the
#     cut footprint is the smaller derived top face: nominal - 2*inset,
#     inset = height * tan(taper).
#   - The plug sinks until the lowest point of its SCULPTED surface touches
#     the plinth top, then everything below is trimmed — carrier thickness
#     never becomes base height.
#   - Plinths are hollow shells with an optional magnet boss; the magnet
#     pocket opens at the bottom so the magnet glues in flush with the rim.
#
# Job JSON (path after `--job`), all lengths in mm, landscape units = mm:
# {
#   "landscape": "/path/to/landscape.stl",
#   "out_dir": "/dir",
#   "plinth": { "height_mm": 3.7, "taper_deg": 15.0, "hollow": true,
#               "wall_mm": 1.2, "top_mm": 1.2,
#               "magnet": { "diameter_mm": 5.0, "height_mm": 1.0,
#                           "clearance_mm": 0.15 } | null },
#   "placements": [ { "name": "round32",
#                     "cutter": { "kind": "circle", "diameter_mm": 32.0 },
#                     "x_mm": 0.0, "y_mm": 0.0, "rotation_deg": 0.0,
#                     "magnet": true } ]
# }
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


def tok(name, payload=None):
    line = name if payload is None else name + " " + json.dumps(payload)
    print(line, flush=True)


# ---------------------------------------------------------------- footprints

def footprint_polygon(cutter):
    """Nominal (bottom-face) outline, CCW, centered at origin. 2D Vectors."""
    kind = cutter["kind"]
    if kind == "circle":
        r = cutter["diameter_mm"] / 2.0
        return [
            Vector((math.cos(t) * r, math.sin(t) * r))
            for t in (i * 2.0 * math.pi / CIRCLE_SEGMENTS for i in range(CIRCLE_SEGMENTS))
        ]
    if kind == "ellipse":
        a, b = cutter["major_mm"] / 2.0, cutter["minor_mm"] / 2.0
        return [
            Vector((math.cos(t) * a, math.sin(t) * b))
            for t in (i * 2.0 * math.pi / CIRCLE_SEGMENTS for i in range(CIRCLE_SEGMENTS))
        ]
    if kind == "rect":
        w, d = cutter["width_mm"] / 2.0, cutter["depth_mm"] / 2.0
        return [Vector((w, d)), Vector((-w, d)), Vector((-w, -d)), Vector((w, -d))]
    raise ValueError(f"unknown cutter kind: {kind}")


def offset_inward(poly, dist):
    """Uniform inward offset with mitered corners. Exact for circles and
    right-angle rects; for ellipses the miter is a per-vertex approximation
    of the true inner offset curve (good to well under a print line width
    at 96 segments)."""
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
    bpy.data.objects.remove(obj, do_unlink=True)


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
    xs = [v.co.x for v in bm.verts]
    ys = [v.co.y for v in bm.verts]
    zs = [v.co.z for v in bm.verts]
    dims = [max(xs) - min(xs), max(ys) - min(ys), max(zs) - min(zs)]
    bm.to_mesh(obj.data)
    bm.free()
    return manifold, [round(d, 3) for d in dims]


# ---------------------------------------------------------------- the plinth

def build_plinth(plinth, cutter, with_magnet):
    """Hollow (or solid) tapered plinth at the origin: nominal footprint at
    z=0, derived top face at z=height. Returns the finished object."""
    h = plinth["height_mm"]
    inset = h * math.tan(math.radians(plinth["taper_deg"]))
    nominal = footprint_polygon(cutter)
    top = offset_inward(nominal, inset)
    body = loft_solid("plinth", [(0.0, nominal), (h, top)])

    magnet = plinth.get("magnet") if with_magnet else None

    if plinth.get("hollow", True):
        wall = plinth["wall_mm"]
        top_plate = plinth["top_mm"]
        ceiling = h - top_plate
        # Cavity follows the taper at constant wall thickness; it pokes out
        # the bottom so the subtraction leaves an open rim, not a skin.
        cavity = loft_solid(
            "cavity",
            [
                (-1.0, offset_inward(nominal, wall)),
                (0.0, offset_inward(nominal, wall)),
                (ceiling, offset_inward(nominal, wall + ceiling * math.tan(math.radians(plinth["taper_deg"])))),
            ],
        )
        if magnet:
            # The boss survives the hollowing: a pillar from the ceiling to
            # the bottom plane that the pocket is later drilled into.
            r_boss = magnet["diameter_mm"] / 2.0 + magnet["clearance_mm"] + plinth["wall_mm"]
            boss = loft_solid(
                "boss",
                [
                    (-1.0, footprint_polygon({"kind": "circle", "diameter_mm": r_boss * 2.0})),
                    (ceiling, footprint_polygon({"kind": "circle", "diameter_mm": r_boss * 2.0})),
                ],
            )
            apply_boolean(cavity, boss, "DIFFERENCE")
            delete_object(boss)
        apply_boolean(body, cavity, "DIFFERENCE")
        delete_object(cavity)

    if magnet:
        r_pocket = magnet["diameter_mm"] / 2.0 + magnet["clearance_mm"]
        pocket = loft_solid(
            "pocket",
            [
                (-1.0, footprint_polygon({"kind": "circle", "diameter_mm": r_pocket * 2.0})),
                (magnet["height_mm"], footprint_polygon({"kind": "circle", "diameter_mm": r_pocket * 2.0})),
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
    inset = h * math.tan(math.radians(plinth["taper_deg"]))
    cut_poly = offset_inward(footprint_polygon(cutter), inset)

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

    base = build_plinth(plinth, cutter, placement.get("magnet", False))
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
    before = set(bpy.data.objects)
    if hasattr(bpy.ops.wm, "stl_import"):
        bpy.ops.wm.stl_import(filepath=path)
    else:
        bpy.ops.import_mesh.stl(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    if len(new) != 1:
        raise RuntimeError(f"expected 1 imported object, got {len(new)}")
    return new[0]


def validate(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bad = sum(1 for e in bm.edges if not e.is_manifold)
    zs = [v.co.z for v in bm.verts]
    xs = [v.co.x for v in bm.verts]
    ys = [v.co.y for v in bm.verts]
    report = {
        "non_manifold_edges": bad,
        "dims_mm": [round(max(xs) - min(xs), 2), round(max(ys) - min(ys), 2), round(max(zs) - min(zs), 2)],
        "verts": len(bm.verts),
    }
    bm.free()
    return bad == 0, report


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
        # Spike policy: report loudly but keep cutting — the exact solver
        # often copes. The app-side gate can harden this later.
        tok("VALIDATED", {**report, "warning": "landscape is not manifold"})
    else:
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
