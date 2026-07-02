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
    base_color   = (0.80, 0.54, 0.35),   # warm resin
    roughness    = 0.52,
    sss_weight   = 0.35,
    sss_radius   = (0.70, 0.35, 0.20),   # reddish scatter
    sss_scale    = 0.12,
    key   = dict(color=(1.0,0.82,0.55), energy=1100, size=10, loc=( 4,-4,6)),
    fill  = dict(color=(1.0,0.78,0.55), energy=110,  size=12, loc=(-5,-2,3)),  # low = deep shadows
    rim   = dict(color=(1.0,0.80,0.60), energy=500,  size=5,  loc=( 0, 5,5)),
    cam_lens = 60,
    samples  = 96,
    res      = 1600,
    exposure = 0.0,
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
               look="flat", contact=False, sheet_cols=3, sheet_res=420, sheet_samples=24)
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

def import_join(paths):
    before = set(bpy.data.objects)
    for p in paths:
        p = os.path.normpath(p)
        if not os.path.isfile(p): raise SystemExit(f"File not found: {p}")
        if hasattr(bpy.ops.wm, "stl_import"): bpy.ops.wm.stl_import(filepath=p)
        else: bpy.ops.import_mesh.stl(filepath=p)
    new = [o for o in bpy.data.objects if o not in before and o.type=="MESH"]
    if not new: raise SystemExit("STL import produced no mesh.")
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
    sss_weight = LOOK["sss_weight"] * (0.6 if look == "rich" else 1.0)
    for name, val in (("Subsurface Weight", sss_weight),
                      ("Subsurface Radius", LOOK["sss_radius"]),
                      ("Subsurface Scale",  LOOK["sss_scale"])):
        if name in b.inputs: b.inputs[name].default_value = val
    obj.data.materials.clear(); obj.data.materials.append(m)

def lights(look="flat"):
    # "rich" = the promo-grade tonal shift: a harder (smaller), stronger key
    # against a near-absent fill, so the form rolls from pale cream through
    # saturated warm midtones into deep shadow instead of flattening out.
    key_scale, key_size, fill_scale = (1.2, 0.55, 0.2) if look == "rich" else (1.0, 1.0, 1.0)
    def mk(spec, name, energy_scale=1.0, size_scale=1.0):
        d = bpy.data.lights.new(name, "AREA"); d.energy=spec["energy"]*energy_scale; d.size=spec["size"]*size_scale
        d.color = spec["color"]
        o = bpy.data.objects.new(name, d); o.location = spec["loc"]
        bpy.context.collection.objects.link(o)
        v = Vector((0,0,0.6)) - Vector(spec["loc"])
        o.rotation_euler = v.to_track_quat("-Z","Y").to_euler()
    mk(LOOK["key"],"Key",key_scale,key_size); mk(LOOK["fill"],"Fill",fill_scale); mk(LOOK["rim"],"Rim")

def black_world():
    w = bpy.data.worlds.new("World"); bpy.context.scene.world = w; w.use_nodes = True
    bg = w.node_tree.nodes.get("Background")
    bg.inputs["Color"].default_value = (0,0,0,1); bg.inputs["Strength"].default_value = 0.0

def camera(obj, azimuth, elev, zoom):
    cd = bpy.data.cameras.new("Camera"); cd.lens = LOOK["cam_lens"]
    cam = bpy.data.objects.new("Camera", cd); bpy.context.collection.objects.link(cam)
    bpy.context.scene.camera = cam
    coords = [obj.matrix_world @ v.co for v in obj.data.vertices]
    mn = Vector((min(c.x for c in coords),min(c.y for c in coords),min(c.z for c in coords)))
    mx = Vector((max(c.x for c in coords),max(c.y for c in coords),max(c.z for c in coords)))
    bbc = (mn+mx)*0.5
    # Bounding-box half-diagonal, NOT max vertex distance: the in-app
    # preview (StlViewport.vue) fits its camera with the exact same number,
    # so preview framing and render framing stay WYSIWYG. Keep in sync.
    radius = (mx - mn).length * 0.5
    D = radius/math.tan(cam.data.angle/2)*zoom; az = math.radians(azimuth)
    cam.location = bbc + Vector((math.sin(az), -math.cos(az), elev)).normalized()*D
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
        # gamma < 1 is a cheap contrast curve: deepens shadows and midtone
        # saturation while the near-white key side barely moves
        sc.view_settings.gamma = 0.85
    sc.render.resolution_x = res; sc.render.resolution_y = res
    sc.render.resolution_percentage = 100
    sc.render.image_settings.file_format = "PNG"
    sc.render.image_settings.color_mode = "RGBA"
    sc.render.film_transparent = False

def build_and_render(cfg, rotate, out, res, samples):
    """Full pipeline for one image at a given rotation."""
    clear()
    obj = import_join(cfg["paths"])
    normalize(obj, rotate)
    resin_material(obj, cfg["color"], cfg["look"])
    lights(cfg["look"]); black_world()
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
