"""
render_mini.py  —  Headless "resin promo" renderer for printable STL minis.

Bakes the locked look: warm resin material with subsurface scattering, a soft
warm key on a pure-black backdrop with a deepened shadow side, Cycles + denoise,
auto scale / center / floor / frame. Saves a PNG next to the first STL by default.

USAGE (headless, no UI):
    blender -b -P render_mini.py -- MODEL.stl
    blender -b -P render_mini.py -- BODY.stl BASE.stl            # multi-part mini
    blender -b -P render_mini.py -- MODEL.stl --out shot.png
    blender -b -P render_mini.py -- MODEL.stl --rotate 0,0,0     # already upright
    blender -b -P render_mini.py -- MODEL.stl --color 0.8,0.54,0.35 --azimuth -15

ORIENTATION PICKER (contact sheet of candidate rotations):
    blender -b -P render_mini.py -- MODEL.stl --contact-sheet
    blender -b -P render_mini.py -- MODEL.stl --contact-sheet --sheet-cols 3 --out sheet.png
  Renders a grid of the model at several rotations and prints, on stdout, a line:
    CONTACT_SHEET {"cols":3,"rows":3,"tile":420,"rotations":[[0,0,0],[90,0,0],...]}
  The tiles are laid out left-to-right, top-to-bottom in that exact order, so a UI
  can map a clicked cell index -> rotation and feed it back as --rotate.

Notes:
  * Pass every part of one mini as separate positional paths; they're joined.
  * Default --rotate is 90,0,0 (stands up based creatures like DTL sculpts).
    Floating / diorama-fragment poses may need a different value (use --contact-sheet).
  * Exit code is 0 on success, non-zero on failure — easy to check from Rust.
"""

import bpy, sys, os, math, json, tempfile
from mathutils import Vector

# ----------------------------- locked recipe -----------------------------
LOOK = dict(
    # Creamier and less saturated than the original (0.80, 0.54, 0.35):
    # stacked with warm lights the old value rendered orange/"digital"
    # next to formal product renders.
    base_color   = (0.85, 0.65, 0.43),   # pale warm resin
    roughness    = 0.52,
    sss_weight   = 0.35,
    sss_radius   = (0.70, 0.35, 0.20),   # reddish scatter
    sss_scale    = 0.12,
    key   = dict(color=(1.0,0.82,0.55), energy=1100, size=10, loc=( 4,-4,6)),
    fill  = dict(color=(1.0,0.78,0.55), energy=110,  size=12, loc=(-5,-2,3)),  # low = deep shadows
    rim   = dict(color=(1.0,0.80,0.60), energy=500,  size=5,  loc=( 0, 5,5)),
    cam_lens = 60.0,
    samples  = 96,
    res      = 1600,
    exposure = 0.0,
    # Look-variant constants, gathered here so the WHOLE recipe lives in one
    # overridable place. "resin" = glossy coat + speckle + dim studio
    # reflections; "rich" = harder key, deep shadows, gentle contrast curve.
    resin = dict(
        coat_weight    = 0.3,
        coat_roughness = 0.12,
        noise_scale    = 450.0,
        noise_detail   = 3.0,
        bump_strength  = 0.035,
        world_color    = (0.9, 0.88, 0.85),
        world_strength = 0.12,
        # Classic energies, but the rim runs slightly COOL against the warm
        # key — the subtle temperature split real product shots have. The
        # key itself follows LOOK["key"]["color"].
        fill_color     = (0.95, 0.93, 0.90),
        rim_color      = (0.85, 0.90, 1.0),
    ),
    rich = dict(
        # Light color stays close to white — the warmth should come from
        # the resin, not from orange lamps stacking onto an orange material.
        key_color        = (1.0, 0.92, 0.80),
        fill_color       = (1.0, 0.90, 0.78),
        rim_color        = (1.0, 0.92, 0.82),
        # Key energy stays at 1.0: with the pale resin a boosted key blows
        # the lit side out to clipping. Hardness comes from the smaller
        # size alone; the low fill is what deepens the shadow side.
        key_energy_mult  = 1.0,
        key_size_mult    = 0.55,
        fill_energy_mult = 0.3,
        sss_weight_mult  = 0.6,
        gamma            = 0.9,
        exposure_shift   = -0.25,
    ),
)

# Candidate rotations for the orientation picker. Order is stable — a UI maps
# clicked cell index -> this list. Covers the usual "which way is up" cases.
CANDIDATE_ROTATIONS = [
    (0,0,0),   (90,0,0),   (-90,0,0),
    (180,0,0), (0,90,0),   (0,-90,0),
    (90,0,90), (90,0,-90), (0,0,90),
]

# ----------------------------- arg parsing --------------------------------
def parse():
    argv = sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else []
    cfg = dict(paths=[], out=None, rotate=(90,0,0), color=LOOK["base_color"],
               azimuth=-15.0, elev=0.22, zoom=1.15, res=LOOK["res"], samples=LOOK["samples"],
               look="flat", contact=False, sheet_cols=3, sheet_res=420, sheet_samples=24,
               align=False)
    i = 0
    while i < len(argv):
        a = argv[i]
        if   a == "--out":           i+=1; cfg["out"]=argv[i]
        elif a == "--rotate":        i+=1; cfg["rotate"]=tuple(float(x) for x in argv[i].split(","))
        elif a == "--color":         i+=1; cfg["color"]=tuple(float(x) for x in argv[i].split(","))
        elif a == "--azimuth":       i+=1; cfg["azimuth"]=float(argv[i])
        elif a == "--elev":          i+=1; cfg["elev"]=float(argv[i])
        elif a == "--zoom":          i+=1; cfg["zoom"]=float(argv[i])
        elif a == "--res":           i+=1; cfg["res"]=int(argv[i])
        elif a == "--samples":       i+=1; cfg["samples"]=int(argv[i])
        elif a == "--look":          i+=1; cfg["look"]=argv[i]
        elif a == "--contact-sheet": cfg["contact"]=True
        elif a == "--align-parts":   cfg["align"]=True
        elif a == "--sheet-cols":    i+=1; cfg["sheet_cols"]=int(argv[i])
        elif a == "--sheet-res":     i+=1; cfg["sheet_res"]=int(argv[i])
        else:                        cfg["paths"].append(a)
        i += 1
    if not cfg["paths"]:
        raise SystemExit("No STL path given. Usage: blender -b -P render_mini.py -- model.stl")
    return cfg

# ----------------------------- scene build --------------------------------
def clear():
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for c in (bpy.data.meshes, bpy.data.materials, bpy.data.cameras, bpy.data.lights):
        for b in list(c):
            if b.users == 0: c.remove(b)

def stack_on_base(groups):
    """Re-seat parts exported around different origins (--align-parts).

    STL carries no shared origin, so when a creator re-exports one part the
    files drift apart and the join floats the mini through its base. The
    part named *base* is the ground truth: its THINNEST bbox axis is the
    model's up axis whatever orientation it was exported in (bases are
    flat), every other part gets centered over it and seated on its top.
    Must mirror stackOnBase in StlViewport.vue so preview == render.
    """
    base = next((g for g in groups if "base" in os.path.basename(g[0]).lower()), None)
    if base is None or len(groups) < 2: return
    def bounds(objs):
        pts = [o.matrix_world @ Vector(c) for o in objs for c in o.bound_box]
        return ([min(p[i] for p in pts) for i in range(3)],
                [max(p[i] for p in pts) for i in range(3)])
    bmin, bmax = bounds(base[1])
    extents = [bmax[i]-bmin[i] for i in range(3)]
    up = extents.index(min(extents))
    across = [i for i in range(3) if i != up]
    base_center = [(bmin[i]+bmax[i])/2 for i in range(3)]
    base_top = bmax[up]
    for path, objs in groups:
        if path == base[0]: continue
        omin, omax = bounds(objs)
        ocenter = [(omin[i]+omax[i])/2 for i in range(3)]
        delta = [0.0, 0.0, 0.0]
        for i in across: delta[i] = base_center[i] - ocenter[i]
        delta[up] = base_top - omin[up]
        for o in objs:
            o.location = (o.location[0]+delta[0], o.location[1]+delta[1], o.location[2]+delta[2])
    bpy.context.view_layer.update()

def import_join(paths, align=False):
    groups = []
    for p in paths:
        p = os.path.normpath(p)
        if not os.path.isfile(p): raise SystemExit(f"File not found: {p}")
        before = set(bpy.data.objects)
        if hasattr(bpy.ops.wm, "stl_import"): bpy.ops.wm.stl_import(filepath=p)
        else: bpy.ops.import_mesh.stl(filepath=p)
        added = [o for o in bpy.data.objects if o not in before and o.type=="MESH"]
        groups.append((p, added))
    new = [o for _, objs in groups for o in objs]
    if not new: raise SystemExit("STL import produced no mesh.")
    if align: stack_on_base(groups)
    bpy.ops.object.select_all(action="DESELECT")
    for o in new: o.select_set(True)
    bpy.context.view_layer.objects.active = new[0]
    if len(new) > 1: bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    obj.name = "Model"; bpy.ops.object.shade_smooth()
    return obj

def normalize(obj, rotate):
    obj.rotation_euler = tuple(math.radians(a) for a in rotate)
    bpy.ops.object.transform_apply(location=False, rotation=True, scale=False)
    bpy.ops.object.origin_set(type="ORIGIN_CENTER_OF_VOLUME")
    bpy.context.view_layer.update()
    s = 2.0 / (max(obj.dimensions) or 1.0); obj.scale = (s,s,s)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.location = (0,0,0); bpy.context.view_layer.update()
    zmin = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
    obj.location.z -= zmin; bpy.context.view_layer.update()

def resin_material(obj, color, look="flat"):
    m = bpy.data.materials.new("Resin"); m.use_nodes = True
    b = m.node_tree.nodes.get("Principled BSDF")
    b.inputs["Base Color"].default_value = tuple(color)+(1.0,)
    b.inputs["Metallic"].default_value = 0.0
    b.inputs["Roughness"].default_value = LOOK["roughness"]
    # SSS wraps light around into the shadow side; the rich look keeps just
    # enough for the resin read while letting shadows actually go dark
    sss_weight = LOOK["sss_weight"] * (LOOK["rich"]["sss_weight_mult"] if look == "rich" else 1.0)
    for name, val in (("Subsurface Weight", sss_weight),
                      ("Subsurface Radius", LOOK["sss_radius"]),
                      ("Subsurface Scale",  LOOK["sss_scale"])):
        if name in b.inputs: b.inputs[name].default_value = val
    if look == "resin":
        # Cured resin is satin with a tighter glossy layer on top — the
        # dual-lobe "gloss over matte" a single roughness can't give
        for name, val in (("Coat Weight", LOOK["resin"]["coat_weight"]),
                          ("Coat Roughness", LOOK["resin"]["coat_roughness"])):
            if name in b.inputs: b.inputs[name].default_value = val
        # Faint surface speckle: micro-noise bump breaks the highlights up
        # so they sparkle like a physical print instead of CAD-smooth
        nt = m.node_tree
        noise = nt.nodes.new("ShaderNodeTexNoise")
        noise.inputs["Scale"].default_value = LOOK["resin"]["noise_scale"]
        noise.inputs["Detail"].default_value = LOOK["resin"]["noise_detail"]
        bump = nt.nodes.new("ShaderNodeBump")
        bump.inputs["Strength"].default_value = LOOK["resin"]["bump_strength"]
        nt.links.new(noise.outputs["Fac"], bump.inputs["Height"])
        nt.links.new(bump.outputs["Normal"], b.inputs["Normal"])
    obj.data.materials.clear(); obj.data.materials.append(m)

def lights(look="flat"):
    # "rich" = the promo-grade tonal shift: a harder (smaller), stronger key
    # against a low fill, so the form rolls from pale cream through warm
    # midtones into deep shadow. All constants live in LOOK["rich"] /
    # LOOK["resin"] — see the recipe comments there for the why.
    rich = LOOK["rich"]; resin = LOOK["resin"]
    key_scale, key_size, fill_scale = (
        (rich["key_energy_mult"], rich["key_size_mult"], rich["fill_energy_mult"])
        if look == "rich" else (1.0, 1.0, 1.0))
    rich_colors = {
        "Key":  rich["key_color"],
        "Fill": rich["fill_color"],
        "Rim":  rich["rim_color"],
    }
    resin_colors = {
        "Key":  LOOK["key"]["color"],
        "Fill": resin["fill_color"],
        "Rim":  resin["rim_color"],
    }
    def mk(spec, name, energy_scale=1.0, size_scale=1.0):
        d = bpy.data.lights.new(name, "AREA"); d.energy=spec["energy"]*energy_scale; d.size=spec["size"]*size_scale
        if look == "rich":   d.color = rich_colors[name]
        elif look == "resin": d.color = resin_colors[name]
        else:                d.color = spec["color"]
        o = bpy.data.objects.new(name, d); o.location = spec["loc"]
        bpy.context.collection.objects.link(o)
        v = Vector((0,0,0.6)) - Vector(spec["loc"])
        o.rotation_euler = v.to_track_quat("-Z","Y").to_euler()
    mk(LOOK["key"],"Key",key_scale,key_size); mk(LOOK["fill"],"Fill",fill_scale); mk(LOOK["rim"],"Rim")

def black_world(look="flat"):
    w = bpy.data.worlds.new("World"); bpy.context.scene.world = w; w.use_nodes = True
    nt = w.node_tree
    bg = nt.nodes.get("Background")
    bg.inputs["Color"].default_value = (0,0,0,1); bg.inputs["Strength"].default_value = 0.0
    if look == "resin":
        # The backdrop stays pure black for CAMERA rays, but the surface
        # gets to reflect a dim neutral studio — with a void-black world the
        # speculars contain only three lamps, which is the big "CG" tell.
        env = nt.nodes.new("ShaderNodeBackground")
        env.inputs["Color"].default_value = tuple(LOOK["resin"]["world_color"]) + (1.0,)
        env.inputs["Strength"].default_value = LOOK["resin"]["world_strength"]
        lp = nt.nodes.new("ShaderNodeLightPath")
        mix = nt.nodes.new("ShaderNodeMixShader")
        out = nt.nodes.get("World Output")
        nt.links.new(lp.outputs["Is Camera Ray"], mix.inputs["Fac"])
        nt.links.new(env.outputs["Background"], mix.inputs[1])   # non-camera rays
        nt.links.new(bg.outputs["Background"], mix.inputs[2])    # camera rays: black
        nt.links.new(mix.outputs["Shader"], out.inputs["Surface"])

def camera(obj, azimuth, elev, zoom):
    cd = bpy.data.cameras.new("Camera"); cd.lens = LOOK["cam_lens"]
    cam = bpy.data.objects.new("Camera", cd); bpy.context.collection.objects.link(cam)
    bpy.context.scene.camera = cam
    coords = [obj.matrix_world @ v.co for v in obj.data.vertices]
    mn = Vector((min(c.x for c in coords),min(c.y for c in coords),min(c.z for c in coords)))
    mx = Vector((max(c.x for c in coords),max(c.y for c in coords),max(c.z for c in coords)))
    bbc = (mn+mx)*0.5
    az = math.radians(azimuth)
    # Exact fit: project all 8 bbox corners through the camera and solve the
    # distance at which every corner is inside the (square) frame. Unlike a
    # bounding-sphere fit this cannot clip and does not depend on model
    # shape. The in-app preview (StlViewport.vue) runs the IDENTICAL
    # algorithm — keep them in sync for WYSIWYG framing.
    d = Vector((math.sin(az), -math.cos(az), elev)).normalized()  # target -> camera
    fwd = -d
    right = fwd.cross(Vector((0.0, 0.0, 1.0))).normalized()
    up = right.cross(fwd)
    half = math.tan(cam.data.angle / 2)
    D = 0.0
    for cx in (mn.x, mx.x):
        for cy in (mn.y, mx.y):
            for cz in (mn.z, mx.z):
                e = Vector((cx, cy, cz)) - bbc
                needed = e.dot(d) + max(abs(e.dot(right)), abs(e.dot(up))) / half
                D = max(D, needed)
    D *= zoom
    cam.location = bbc + d * D
    cam.rotation_euler = (bbc-Vector(cam.location)).to_track_quat('-Z','Y').to_euler()
    cam.data.dof.use_dof = False

def setup_render(res, samples, look="flat"):
    sc = bpy.context.scene
    sc.render.engine = "CYCLES"; sc.cycles.samples = samples
    try: sc.cycles.use_denoising = True
    except Exception: pass
    try:
        prefs = bpy.context.preferences.addons["cycles"].preferences; prefs.get_devices()
        for dt in ("OPTIX","CUDA","HIP","METAL","ONEAPI"):
            try:
                prefs.compute_device_type = dt
                if any(d.type==dt for d in prefs.devices):
                    for d in prefs.devices: d.use=True
                    sc.cycles.device="GPU"; break
            except Exception: continue
    except Exception: pass
    # Standard (not AgX) on purpose: AgX desaturates the warm resin tones,
    # which is the opposite of the formal product-render look
    sc.view_settings.view_transform = "Standard"
    try: sc.view_settings.look = "None"
    except Exception: pass
    sc.view_settings.exposure = LOOK["exposure"]
    if look == "rich":
        # gamma < 1 is a cheap contrast curve: deepens shadows while the
        # near-white key side barely moves. Kept gentle — pushing it also
        # over-saturates midtones, which reads "digital"
        sc.view_settings.gamma = LOOK["rich"]["gamma"]
        # pull the highlights back off the clipping point
        sc.view_settings.exposure = LOOK["exposure"] + LOOK["rich"]["exposure_shift"]
    sc.render.resolution_x = res; sc.render.resolution_y = res
    sc.render.resolution_percentage = 100
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_mode = "RGBA"
    sc.render.film_transparent = False

def build_and_render(cfg, rotate, out, res, samples):
    """Full pipeline for one image at a given rotation."""
    clear()
    obj = import_join(cfg["paths"], cfg["align"])
    normalize(obj, rotate)
    resin_material(obj, cfg["color"], cfg["look"])
    lights(cfg["look"]); black_world(cfg["look"])
    camera(obj, cfg["azimuth"], cfg["elev"], cfg["zoom"])
    setup_render(res, samples, cfg["look"])
    bpy.context.scene.render.filepath = os.path.abspath(out).replace("\\","/")
    bpy.ops.render.render(write_still=True)

# ----------------------------- contact sheet ------------------------------
def make_contact_sheet(cfg, out):
    import numpy as np
    cands = CANDIDATE_ROTATIONS
    cols = max(1, cfg["sheet_cols"]); rows = (len(cands)+cols-1)//cols
    tile = cfg["sheet_res"]
    tmp = tempfile.mkdtemp(prefix="minisheet_")
    tiles = []
    for i, rot in enumerate(cands):
        fp = os.path.join(tmp, f"tile_{i}.png")
        build_and_render(cfg, rot, fp, tile, cfg["sheet_samples"])
        tiles.append(fp)
    # composite with numpy (Blender bundles numpy)
    arrs = []
    for fp in tiles:
        im = bpy.data.images.load(fp); w, h = im.size
        a = np.array(im.pixels[:], dtype=np.float32).reshape(h, w, 4)[::-1]  # top-down
        arrs.append(a); bpy.data.images.remove(im)
    h, w, _ = arrs[0].shape
    canvas = np.zeros((h*rows, w*cols, 4), dtype=np.float32); canvas[..., 3] = 1.0
    for i, a in enumerate(arrs):
        r, c = i//cols, i%cols
        canvas[r*h:(r+1)*h, c*w:(c+1)*w] = a
    sheet = bpy.data.images.new("contact_sheet", w*cols, h*rows)
    sheet.pixels = canvas[::-1].ravel()
    sheet.filepath_raw = os.path.abspath(out).replace("\\","/")
    sheet.file_format = "PNG"; sheet.save()
    # machine-readable mapping for the calling tool
    print("CONTACT_SHEET " + json.dumps(
        {"cols": cols, "rows": rows, "tile": tile,
         "rotations": [list(r) for r in cands]}))

# ----------------------------- main ---------------------------------------
def main():
    cfg = parse()
    first = os.path.normpath(cfg["paths"][0])
    if cfg["contact"]:
        out = cfg["out"] or (os.path.splitext(first)[0] + "_sheet.png")
        print(f"[render_mini] contact sheet -> {out}")
        make_contact_sheet(cfg, out)
    else:
        out = cfg["out"] or (os.path.splitext(first)[0] + ".png")
        print(f"[render_mini] rendering -> {out}")
        build_and_render(cfg, cfg["rotate"], out, cfg["res"], cfg["samples"])
    print("[render_mini] done.")

if __name__ == "__main__":
    main()
