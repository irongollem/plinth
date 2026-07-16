"""Convert selected CC0 Poly Haven scan meshes into printable scatter STLs.

Source 1K .blend files are downloaded from the URLs recorded in the scatter
manifest and kept outside the repository. Run with Blender 5.1+:

  blender --background --python tools/curate_polyhaven_forest.py -- \
    /tmp/plinth-forest-source src-tauri/resources/scatter
"""

import math
import os
import sys

import bmesh
import bpy
from mathutils import Vector


SOURCE, OUT = [os.path.abspath(p) for p in sys.argv[sys.argv.index("--") + 1:][:2]]

ASSETS = [
    # source id, source object, output id, target XY footprint, rotation XYZ
    ("dead_quiver_branch_01", "dead_quiver_branch_01", "forest-branch-scan", 11.0, (math.pi / 2, 0, 0)),
    ("tree_stump_01", "tree_stump_01", "forest-stump-scan", 14.0, (0, 0, 0)),
    ("dead_tree_trunk", "dead_tree_trunk", "forest-log-scan", 16.0, (0, 0, 0)),
]

VOXEL_MM = 0.09
TRI_LIMIT = 15000


def reset():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)


def append_object(asset_id, object_name):
    path = os.path.join(SOURCE, f"{asset_id}_1k.blend")
    with bpy.data.libraries.load(path, link=False) as (src, dst):
        if object_name not in src.objects:
            raise RuntimeError(f"{object_name!r} missing from {path}")
        dst.objects = [object_name]
    obj = dst.objects[0]
    bpy.context.collection.objects.link(obj)
    return obj


def select(obj):
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj


def normalize(obj, target_footprint, rotation):
    select(obj)
    obj.rotation_euler = rotation
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=False)
    scale = target_footprint / max(obj.dimensions.x, obj.dimensions.y)
    obj.scale = (scale, scale, scale)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)

    corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
    lo = Vector((min(v.x for v in corners), min(v.y for v in corners), min(v.z for v in corners)))
    hi = Vector((max(v.x for v in corners), max(v.y for v in corners), max(v.z for v in corners)))
    obj.location += Vector((-(lo.x + hi.x) / 2, -(lo.y + hi.y) / 2, -lo.z))
    bpy.ops.object.transform_apply(location=True, rotation=False, scale=False)


def make_printable(obj):
    select(obj)
    remesh = obj.modifiers.new("watertight_scan", "REMESH")
    remesh.mode = "VOXEL"
    remesh.voxel_size = VOXEL_MM
    remesh.use_smooth_shade = True
    bpy.ops.object.modifier_apply(modifier=remesh.name)

    smooth = obj.modifiers.new("scan_cleanup", "SMOOTH")
    smooth.factor = 0.18
    smooth.iterations = 1
    bpy.ops.object.modifier_apply(modifier=smooth.name)

    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.quads_convert_to_tris(quad_method="BEAUTY", ngon_method="BEAUTY")
    bpy.ops.object.mode_set(mode="OBJECT")
    if len(obj.data.polygons) > TRI_LIMIT:
        dec = obj.modifiers.new("bundle_budget", "DECIMATE")
        dec.ratio = TRI_LIMIT / len(obj.data.polygons)
        bpy.ops.object.modifier_apply(modifier=dec.name)


def validate(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    nonmanifold = sum(not e.is_manifold for e in bm.edges)
    unseen = set(bm.verts)
    shells = 0
    while unseen:
        shells += 1
        stack = [unseen.pop()]
        while stack:
            v = stack.pop()
            for edge in v.link_edges:
                other = edge.other_vert(v)
                if other in unseen:
                    unseen.remove(other)
                    stack.append(other)
    tris = len(bm.faces)
    bm.free()
    if nonmanifold or shells != 1 or tris > TRI_LIMIT:
        raise RuntimeError(f"{obj.name}: nonmanifold={nonmanifold} shells={shells} tris={tris}")


def export(obj):
    select(obj)
    path = os.path.join(OUT, obj.name + ".stl")
    bpy.ops.wm.stl_export(filepath=path, export_selected_objects=True, ascii_format=False)
    print(f"CURATED {obj.name} tris={len(obj.data.polygons)} "
          f"dims={obj.dimensions.x:.3f}x{obj.dimensions.y:.3f}x{obj.dimensions.z:.3f} "
          f"bytes={os.path.getsize(path)} path={path}")


reset()
os.makedirs(OUT, exist_ok=True)
for source_id, object_name, output_id, footprint, rotation in ASSETS:
    obj = append_object(source_id, object_name)
    obj.name = output_id
    normalize(obj, footprint, rotation)
    make_printable(obj)
    validate(obj)
    export(obj)
