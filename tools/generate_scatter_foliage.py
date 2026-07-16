"""Generate Plinth's original, CC0 foliage scatter pack in Blender 5.1+.

Run:
  blender --background --python tools/generate_scatter_foliage.py -- \
    src-tauri/resources/scatter

The result is one editable .blend source plus print-ready binary STLs.  Units
are millimetres.  Every STL is a single connected, manifold shell, centred in
XY, floored at Z=0, and kept below the bundle's 15k triangle limit.
"""

import math
import os
import sys

import bmesh
import bpy
from mathutils import Vector


OUT = os.path.abspath(sys.argv[sys.argv.index("--") + 1]) if "--" in sys.argv else os.getcwd()
VOXEL_MM = 0.105
TRI_LIMIT = 14500


def reset():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)


def uv_sphere(name, location, scale, segments=20, rings=12):
    bpy.ops.mesh.primitive_uv_sphere_add(segments=segments, ring_count=rings, location=location)
    obj = bpy.context.object
    obj.name = name
    obj.scale = scale
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
    return obj


def cylinder_between(name, a, b, radius, vertices=14):
    a, b = Vector(a), Vector(b)
    d = b - a
    bpy.ops.mesh.primitive_cylinder_add(vertices=vertices, radius=radius, depth=d.length, location=(a + b) / 2)
    obj = bpy.context.object
    obj.name = name
    obj.rotation_mode = "QUATERNION"
    obj.rotation_quaternion = Vector((0, 0, 1)).rotation_difference(d.normalized())
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)
    return obj


def leaf(name, center, length, width, thickness, angle=0.0, tilt=0.0, lobes=0):
    """Closed lens-shaped leaf with an integral midrib; optionally lobed."""
    cx, cy, cz = center
    parts = []
    samples = 5 if lobes else 3
    for i in range(samples):
        t = (i + 1) / (samples + 1)
        x = (t - 0.5) * length
        envelope = math.sin(math.pi * t)
        if lobes:
            envelope *= 0.72 + 0.28 * math.cos((t - 0.5) * math.pi * 6)
        r = max(thickness * 1.5, width * envelope * 0.48)
        z = thickness * (0.65 + 0.35 * math.sin(math.pi * t))
        parts.append(uv_sphere(name + "_blade", (x, 0, 0), (length / (samples * 1.75), r, z), 16, 8))
    parts.append(cylinder_between(name + "_rib", (-length * 0.53, 0, 0), (length * 0.48, 0, thickness * 0.15), thickness * 0.62, 10))
    for obj in parts:
        obj.rotation_euler[0] = tilt
        obj.rotation_euler[2] = angle
        obj.location += Vector((cx, cy, cz))
        bpy.ops.object.select_all(action="DESELECT")
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        bpy.ops.object.transform_apply(location=False, rotation=True, scale=False)
    return parts


def join_remesh(name, parts, voxel=VOXEL_MM):
    bpy.ops.object.select_all(action="DESELECT")
    for p in parts:
        p.select_set(True)
    bpy.context.view_layer.objects.active = parts[0]
    bpy.ops.object.join()
    obj = bpy.context.object
    obj.name = name
    remesh = obj.modifiers.new("print_fuse", "REMESH")
    remesh.mode = "VOXEL"
    remesh.voxel_size = voxel
    remesh.use_smooth_shade = True
    bpy.ops.object.modifier_apply(modifier=remesh.name)
    smooth = obj.modifiers.new("surface_smooth", "SMOOTH")
    smooth.factor = 0.32
    smooth.iterations = 2
    bpy.ops.object.modifier_apply(modifier=smooth.name)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    floor_center(obj)
    triangulate_and_limit(obj)
    validate(obj)
    return obj


def floor_center(obj):
    corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
    lo = Vector((min(v.x for v in corners), min(v.y for v in corners), min(v.z for v in corners)))
    hi = Vector((max(v.x for v in corners), max(v.y for v in corners), max(v.z for v in corners)))
    obj.location += Vector((-(lo.x + hi.x) / 2, -(lo.y + hi.y) / 2, -lo.z))
    bpy.context.view_layer.update()
    bpy.ops.object.transform_apply(location=True, rotation=False, scale=False)


def triangulate_and_limit(obj):
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    bpy.ops.mesh.select_all(action="SELECT")
    bpy.ops.mesh.quads_convert_to_tris(quad_method="BEAUTY", ngon_method="BEAUTY")
    bpy.ops.object.mode_set(mode="OBJECT")
    if len(obj.data.polygons) > TRI_LIMIT:
        dec = obj.modifiers.new("triangle_budget", "DECIMATE")
        dec.ratio = TRI_LIMIT / len(obj.data.polygons)
        bpy.ops.object.modifier_apply(modifier=dec.name)


def validate(obj):
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    nonmanifold = [e for e in bm.edges if not e.is_manifold]
    shells = 0
    unseen = set(bm.verts)
    while unseen:
        shells += 1
        stack = [unseen.pop()]
        while stack:
            vert = stack.pop()
            for e in vert.link_edges:
                other = e.other_vert(vert)
                if other in unseen:
                    unseen.remove(other)
                    stack.append(other)
    tris = len(bm.faces)
    bm.free()
    if nonmanifold or shells != 1 or tris > 15000:
        raise RuntimeError(f"{obj.name}: manifold={not nonmanifold}, shells={shells}, tris={tris}")


def blade_mesh(name, length, width, thickness, segments=20, lobes=0, serration=0.0):
    """A continuous, closed, cambered leaf blade running from x=0 to x=length."""
    outline = []
    for side in (1, -1):
        indices = range(segments + 1) if side == 1 else range(segments, -1, -1)
        for i in indices:
            t = i / segments
            envelope = math.sin(math.pi * t) ** 0.72
            envelope = max(0.065, envelope)
            if lobes:
                envelope *= 0.76 + 0.24 * math.cos((t - 0.5) * math.pi * lobes * 2)
            if serration and 0 < i < segments:
                envelope *= 1.0 + serration * (1 if i % 2 else -0.35)
            y = side * width * 0.5 * envelope
            z = thickness * (0.55 * math.sin(math.pi * t) + 0.18 * math.sin(t * math.pi * 2))
            outline.append((length * t, y, z))
    n = len(outline)
    verts = [(x, y, z + thickness * 0.5) for x, y, z in outline]
    verts += [(x, y, z - thickness * 0.5) for x, y, z in outline]
    faces = [tuple(range(n)), tuple(range(2 * n - 1, n - 1, -1))]
    for i in range(n):
        j = (i + 1) % n
        faces.append((i, j, n + j, n + i))
    mesh = bpy.data.meshes.new(name + "_mesh")
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def orient(obj, origin, yaw=0.0, pitch=0.0, roll=0.0):
    local_location = obj.location.copy()
    obj.rotation_euler = (roll, pitch, yaw)
    obj.location = Vector(origin) + obj.rotation_euler.to_matrix() @ local_location
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=False)
    return obj


def detailed_leaf(name, origin, length, width, thickness, yaw=0.0, pitch=0.0,
                  lobes=0, serration=0.0, veins=3):
    parts = [blade_mesh(name + "_blade", length, width, thickness, 22, lobes, serration)]
    parts.append(cylinder_between(name + "_midrib", (length * 0.08, 0, thickness * 0.55),
                                  (length * 0.86, 0, thickness * 0.78), thickness * 0.34, 10))
    for i in range(1, veins + 1):
        t = i / (veins + 1)
        half = width * 0.5 * math.sin(math.pi * t) * 0.62
        for side in (-1, 1):
            parts.append(cylinder_between(
                name + "_vein", (length * t, 0, thickness * 0.38),
                (length * t, side * half, thickness * 0.30), thickness * 0.32, 8))
    for p in parts:
        orient(p, origin, yaw, pitch)
    return parts


def tapered_tube(name, points, radii, sides=14):
    verts = []
    for i, p in enumerate(points):
        p = Vector(p)
        tangent = Vector(points[min(i + 1, len(points) - 1)]) - Vector(points[max(0, i - 1)])
        tangent.normalize()
        side = tangent.cross(Vector((0, 0, 1)))
        if side.length < 0.01:
            side = tangent.cross(Vector((0, 1, 0)))
        side.normalize()
        up = tangent.cross(side).normalized()
        for j in range(sides):
            a = j * math.tau / sides
            bark = 1.0 + 0.08 * math.sin(j * 3.0 + i * 1.7)
            verts.append(tuple(p + (side * math.cos(a) + up * math.sin(a)) * radii[i] * bark))
    faces = []
    for i in range(len(points) - 1):
        for j in range(sides):
            k = (j + 1) % sides
            faces.append((i * sides + j, i * sides + k, (i + 1) * sides + k, (i + 1) * sides + j))
    faces.append(tuple(range(sides - 1, -1, -1)))
    end = (len(points) - 1) * sides
    faces.append(tuple(end + j for j in range(sides)))
    mesh = bpy.data.meshes.new(name + "_mesh")
    mesh.from_pydata(verts, [], faces); mesh.update()
    obj = bpy.data.objects.new(name, mesh); bpy.context.collection.objects.link(obj)
    return obj


def twig(name, branched=False):
    parts = []
    points = [(-5.5, 0, 0.55), (-2.8, 0.35, 0.7), (0, -0.15, 0.55), (2.8, 0.3, 0.7), (5.5, 0, 0.48)]
    for i in range(len(points) - 1):
        parts.append(cylinder_between(name, points[i], points[i + 1], 0.42 - i * 0.035, 12))
    for p in points[1:-1]:
        parts.append(uv_sphere(name, p, (0.52, 0.48, 0.44), 14, 8))
    if branched:
        parts += [
            cylinder_between(name, (-1.0, 0, 0.62), (-2.1, 2.8, 1.05), 0.30, 12),
            cylinder_between(name, (2.1, 0.2, 0.62), (3.2, -2.2, 1.15), 0.27, 12),
            cylinder_between(name, (-2.1, 2.8, 1.05), (-2.5, 3.7, 1.35), 0.19, 10),
        ]
    return join_remesh(name, parts, 0.09)


def make_assets():
    assets = []
    assets.append(join_remesh("leaf-oval", detailed_leaf("leaf-oval", (0, 0, 0.28), 7.2, 3.25, 0.24, serration=0.10), 0.065))
    assets.append(join_remesh("leaf-lobed", detailed_leaf("leaf-lobed", (0, 0, 0.30), 7.8, 4.5, 0.25, lobes=4, serration=0.06, veins=4), 0.065))
    parts = []
    parts += detailed_leaf("leaf-cluster", (-2.1, -0.5, 0.34), 6.0, 2.6, 0.23, yaw=0.28, pitch=-0.04, serration=0.08, veins=0)
    parts += detailed_leaf("leaf-cluster", (-1.1, 0.35, 0.42), 5.6, 2.35, 0.22, yaw=-0.52, pitch=0.05, serration=0.08, veins=0)
    parts += detailed_leaf("leaf-cluster", (-0.4, -0.15, 0.50), 5.1, 2.15, 0.21, yaw=1.72, pitch=-0.06, serration=0.08, veins=0)
    assets.append(join_remesh("leaf-cluster", parts, 0.07))

    parts = [uv_sphere("grass-tuft", (0, 0, 0.28), (1.15, 1.15, 0.38), 18, 10)]
    for i in range(11):
        a = i * math.tau / 11
        length = 4.5 + (i % 4) * 0.55
        blade = blade_mesh("grass-tuft_blade", length, 0.58, 0.25, 14, 0, 0)
        root = (0.10 * math.cos(a), 0.10 * math.sin(a), 0.24)
        orient(blade, root, a, -math.radians(58 + (i % 3) * 7), math.radians(90))
        parts.append(blade)
        parts.append(uv_sphere("grass-tuft_root", root, (0.48, 0.48, 0.42), 12, 8))
    assets.append(join_remesh("grass-tuft", parts, 0.06))

    parts = [uv_sphere("broadleaf-plant", (0, 0, 0.32), (0.9, 0.9, 0.38), 18, 10)]
    for i in range(7):
        a = i * math.tau / 7
        parts += detailed_leaf("broadleaf-plant", (0, 0, 0.45), 4.2 + (i % 2) * 0.45, 1.75, 0.22,
                               yaw=a, pitch=-0.34 + (i % 3) * 0.08, serration=0.07, veins=0)
    assets.append(join_remesh("broadleaf-plant", parts, 0.075))

    parts = [tapered_tube("fern_stem", [(0,0,0.15),(0,0,2.5),(0,0,5.8)], [0.28,0.22,0.13], 12)]
    for i in range(7):
        z = 0.85 + i * 0.68
        span = 3.1 - i * 0.27
        for side in (-1, 1):
            parts += detailed_leaf("fern", (0, 0, z), span, 0.78, 0.17,
                                   yaw=0 if side > 0 else math.pi, pitch=-0.13, serration=0.05, veins=0)
    assets.append(join_remesh("fern", parts, 0.07))

    main = [(-5.5,0,0.52),(-3.3,0.25,0.72),(-1.2,-0.22,0.58),(1.1,0.18,0.70),(3.4,-0.16,0.56),(5.5,0.12,0.45)]
    assets.append(join_remesh("twig-straight", [tapered_tube("twig", main, [0.47,0.44,0.40,0.36,0.31,0.22])], 0.065))
    parts = [tapered_tube("twig", main, [0.47,0.44,0.40,0.36,0.31,0.22])]
    parts.append(tapered_tube("twig_branch", [(-1.4,-0.18,0.60),(-2.15,1.55,0.82),(-2.65,3.0,1.02)], [0.32,0.24,0.13], 12))
    parts.append(tapered_tube("twig_branch", [(2.2,-0.05,0.60),(3.0,-1.35,0.78),(3.45,-2.4,0.92)], [0.27,0.20,0.12], 12))
    assets.append(join_remesh("twig-forked", parts, 0.065))
    return assets


def export(assets):
    os.makedirs(OUT, exist_ok=True)
    for obj in assets:
        bpy.ops.object.select_all(action="DESELECT")
        obj.select_set(True)
        bpy.context.view_layer.objects.active = obj
        path = os.path.join(OUT, obj.name + ".stl")
        bpy.ops.wm.stl_export(filepath=path, export_selected_objects=True, ascii_format=False)
        dims = obj.dimensions
        print(f"ASSET {obj.name} tris={len(obj.data.polygons)} size={dims.x:.3f}x{dims.y:.3f}x{dims.z:.3f} path={path}")
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(OUT, "foliage-twigs-source.blend"), compress=True)


reset()
export(make_assets())
