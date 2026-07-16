"""Decimate + rescale the CC BY-SA Printables leaf set into scatter STLs.

  blender --background --python curate_leaves.py -- <src_root> <out_dir>

Source leaves are ~90mm print models (flat side down, detail up). Two
transforms make them scatter debris: XY down to a small footprint, and Z
floored to a printable thickness INDEPENDENTLY (a uniform downscale would
thin the 1.2-2.3mm blade to ~0.1mm foil). No voxel remesh — the sources are
already clean manifold shells and a remesh would eat the thin blade.
"""
import bmesh
import bpy
import glob
import os
import sys
from mathutils import Vector

SRC, OUT = [os.path.abspath(p) for p in sys.argv[sys.argv.index("--") + 1:][:2]]

# filename stem -> (output id, label)
LEAVES = {
    "leaf1 - maple B": ("leaf-maple", "Maple leaf"),
    "leaf2 - apple": ("leaf-apple", "Apple leaf"),
    "leaf3 - cherry": ("leaf-cherry", "Cherry leaf"),
    "leaf4 - oak": ("leaf-oak", "Oak leaf"),
    "leaf5 - hazel": ("leaf-hazel", "Hazel leaf"),
}

FOOTPRINT_MM = 5.0   # longest XY dimension after normalize (tunable canonical)
# The modeling-for-print thickness tradeoff: a real leaf is ~0.2mm, but at a
# 5mm footprint that prints as unreliable foil AND is swallowed whole by the
# scatter's 0.4mm stitch-sink (it clips into the base and vanishes). 1.2mm is
# deliberately "a bit faker" — chunky for a leaf — so it prints robustly and
# sits proud of the sink as visible relief. The sink still stitches it into
# the base; the thickness just stops the stitch from consuming the whole leaf.
THICKNESS_MM = 1.2
TRI_BUDGET = 1500    # a flat leaf needs far less than the 15k scan budget
# Dead leaves cup ("taco" along the midrib) and lift at the tips rather than
# lying dead flat. Baked as a per-vertex Z lift: edges fold up quadratically
# across the width, ends lift quadratically along the length. Gentle enough
# that the center still sinks into the base and the lifted edges stay near
# the surface as printable relief, not thin proud flanges. Random yaw at
# placement spins the cup direction, so a species' single mesh still reads
# varied across a carpet.
CURL_EDGE_MM = 0.55  # max lateral (across-width) edge lift
CURL_TIP_MM = 0.30   # max longitudinal (along-length) end lift


def reset():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)


def select(obj):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj


def keep_largest_shell(obj):
    """Oak ships as 2 shells (blade + a detached fragment); keep the body."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    unseen = set(bm.verts)
    islands = []
    while unseen:
        island = {unseen.pop()}
        stack = list(island)
        while stack:
            v = stack.pop()
            for e in v.link_edges:
                o = e.other_vert(v)
                if o in unseen:
                    unseen.remove(o)
                    island.add(o)
                    stack.append(o)
        islands.append(island)
    if len(islands) > 1:
        def extent(isl):
            xs = [v.co.x for v in isl]; ys = [v.co.y for v in isl]; zs = [v.co.z for v in isl]
            return (max(xs) - min(xs)) ** 2 + (max(ys) - min(ys)) ** 2 + (max(zs) - min(zs)) ** 2
        keep = max(islands, key=extent)
        bmesh.ops.delete(bm, geom=[v for v in bm.verts if v not in keep], context="VERTS")
    bm.to_mesh(obj.data)
    bm.free()


def normalize(obj):
    select(obj)
    # XY footprint scale (uniform in-plane), Z set to the thickness floor.
    xy = FOOTPRINT_MM / max(obj.dimensions.x, obj.dimensions.y)
    z = THICKNESS_MM / obj.dimensions.z
    obj.scale = (xy, xy, z)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    # Center XY, sit the flat side on Z=0 (detail up).
    corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
    lo = Vector((min(v.x for v in corners), min(v.y for v in corners), min(v.z for v in corners)))
    hi = Vector((max(v.x for v in corners), max(v.y for v in corners), max(v.z for v in corners)))
    obj.location += Vector((-(lo.x + hi.x) / 2, -(lo.y + hi.y) / 2, -lo.z))
    bpy.ops.object.transform_apply(location=True, rotation=False, scale=False)


def curl(obj):
    """Cup the blade (taco fold across width) and lift the ends, so the leaf
    breaks the flat-paper read. Length axis = the longer XY span; width axis
    = the shorter. Both lifts are quadratic from the centerline, added to Z."""
    d = obj.dimensions
    length_is_x = d.x >= d.y
    half_len = (d.x if length_is_x else d.y) / 2 or 1e-6
    half_wid = (d.y if length_is_x else d.x) / 2 or 1e-6
    me = obj.data
    for v in me.vertices:
        along = v.co.x if length_is_x else v.co.y   # centered at 0 already
        across = v.co.y if length_is_x else v.co.x
        lift = (CURL_EDGE_MM * (across / half_wid) ** 2
                + CURL_TIP_MM * (along / half_len) ** 2)
        v.co.z += lift


def decimate(obj):
    select(obj)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.quads_convert_to_tris(quad_method="BEAUTY", ngon_method="BEAUTY")
    bpy.ops.object.mode_set(mode="OBJECT")
    if len(obj.data.polygons) > TRI_BUDGET:
        dec = obj.modifiers.new("budget", "DECIMATE")
        dec.ratio = TRI_BUDGET / len(obj.data.polygons)
        bpy.ops.object.modifier_apply(modifier=dec.name)


def validate(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    nonman = sum(not e.is_manifold for e in bm.edges)
    unseen = set(bm.verts); shells = 0
    while unseen:
        shells += 1; stack = [unseen.pop()]
        while stack:
            v = stack.pop()
            for e in v.link_edges:
                o = e.other_vert(v)
                if o in unseen: unseen.remove(o); stack.append(o)
    tris = len(bm.faces)
    bm.free()
    if nonman or shells != 1 or tris > TRI_BUDGET:
        raise RuntimeError(f"{obj.name}: nonman={nonman} shells={shells} tris={tris}")


def export(obj):
    select(obj)
    path = os.path.join(OUT, obj.name + ".stl")
    bpy.ops.wm.stl_export(filepath=path, export_selected_objects=True, ascii_format=False)
    d = obj.dimensions
    print(f"CURATED {obj.name} tris={len(obj.data.polygons)} "
          f"dims={d.x:.3f}x{d.y:.3f}x{d.z:.3f} bytes={os.path.getsize(path)}")


reset()
os.makedirs(OUT, exist_ok=True)
for stem, (out_id, _label) in LEAVES.items():
    src = os.path.join(SRC, stem + ".stl")
    reset()
    bpy.ops.wm.stl_import(filepath=src)
    obj = bpy.context.selected_objects[0]
    obj.name = out_id
    keep_largest_shell(obj)
    normalize(obj)
    curl(obj)
    decimate(obj)
    validate(obj)
    export(obj)
