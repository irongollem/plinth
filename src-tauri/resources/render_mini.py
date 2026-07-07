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

LOOK OVERRIDES (--config, for people who know their way around a light rig):
    blender -b -P render_mini.py -- MODEL.stl --config look.json
    blender -b -P render_mini.py -- MODEL.stl --config '{"key":{"energy":1500}}'
  The argument is a JSON file path or inline JSON (leading '{'). It deep-merges
  onto the LOOK recipe below — any subset of its keys, e.g.
    {"roughness": 0.4, "key": {"energy": 1500}, "rich": {"gamma": 0.85}}
  Precedence: LOOK defaults < --config < explicit CLI flags (--color etc.).
  Unknown keys and type mismatches are warned about and skipped, never fatal.
  Light-color semantics per look: flat uses key/fill/rim.color; resin uses
  resin.fill_color/rim_color (its key follows key.color); rich uses rich.*.
  The contact sheet inherits the config too (the merge happens before parse).

SCALE REFERENCE ("banana for scale" — true relative size next to the model):
    blender -b -P render_mini.py -- MODEL.stl --scale-ref man.stl --scale-ref-height 28
  The reference imports beside the model in neutral grey, scaled so it stands
  --scale-ref-height mm tall in the model's own mm space (a 28mm scale guy
  stays chest-height to a 90mm ogre). Included in framing, excluded from
  MEASURED. Assumed sliced-ready (Z up). Missing file = warn and skip.

BATCH MODE (many minis, one Blender launch — startup cost paid once):
    blender -b -P render_mini.py -- --batch manifest.json
  manifest.json = {"entries":[{"parts":["a.stl","base.stl"],"out":"a.png",
  "rotate":[90,0,0], ...optional per-entry overrides (color/look/res/samples/
  azimuth/elev/zoom/align/config)}]}. Progress is machine-readable on stdout:
    BATCH_START {"total":N} / BATCH_MODEL {"index":i,"out":...} /
    MEASURED {"index":i,"dims_mm":[x,y,z],"parts":n} /
    BATCH_DONE {"index":i,"ok":true|false[,"error":...]}
  A failed entry (missing/corrupt STL) reports ok:false and the batch moves on.

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
def load_config(arg):
    """--config accepts inline JSON (leading '{') or a path to a JSON file."""
    try:
        obj = json.loads(arg) if arg.lstrip().startswith("{") else json.load(open(arg, encoding="utf-8"))
    except (OSError, ValueError) as e:
        raise SystemExit(f"[render_mini] config error: {e}")
    if not isinstance(obj, dict):
        raise SystemExit("[render_mini] config error: top level must be a JSON object")
    return obj

def merge_config(dst, src, path=""):
    """Deep-merge user overrides onto the LOOK recipe, in place.

    Unknown keys and type mismatches warn and skip — a look file from a
    newer/older app version must degrade gracefully, not kill the render.
    JSON has no tuples, so lists coerce where the default is a tuple; int
    defaults (samples/res) round, float defaults stay float.
    """
    for key, val in src.items():
        where = f"{path}.{key}" if path else key
        if key not in dst:
            print(f"[render_mini] config: ignored unknown key '{where}'")
            continue
        cur = dst[key]
        number = isinstance(val, (int, float)) and not isinstance(val, bool)
        if isinstance(cur, dict):
            if isinstance(val, dict): merge_config(cur, val, where)
            else: print(f"[render_mini] config: ignored '{where}' (expected an object)")
        elif isinstance(cur, tuple):
            if (isinstance(val, list) and len(val) == len(cur)
                    and all(isinstance(x, (int, float)) and not isinstance(x, bool) for x in val)):
                dst[key] = tuple(float(x) for x in val)
            else:
                print(f"[render_mini] config: ignored '{where}' (expected {len(cur)} numbers)")
        elif isinstance(cur, int) and not isinstance(cur, bool):
            if number: dst[key] = int(round(val))
            else: print(f"[render_mini] config: ignored '{where}' (expected a number)")
        elif isinstance(cur, float):
            if number: dst[key] = float(val)
            else: print(f"[render_mini] config: ignored '{where}' (expected a number)")
        else:
            dst[key] = val

def parse():
    argv = sys.argv[sys.argv.index("--")+1:] if "--" in sys.argv else []
    # PASS 1 — apply --config onto LOOK before anything reads it: the cfg
    # defaults below bake base_color/res/samples in at parse time, so a
    # merge that ran later would silently lose those overrides.
    for i, a in enumerate(argv):
        if a == "--config" and i + 1 < len(argv):
            merge_config(LOOK, load_config(argv[i + 1]))
    cfg = dict(paths=[], out=None, rotate=(90,0,0), color=LOOK["base_color"],
               azimuth=-15.0, elev=0.22, zoom=1.15, res=LOOK["res"], samples=LOOK["samples"],
               look="flat", contact=False, sheet_cols=3, sheet_res=420, sheet_samples=24,
               align=False, batch=None, scale_ref=None, scale_ref_height=28.0)
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
        elif a == "--config":        i+=1  # consumed in pass 1 above
        elif a == "--contact-sheet": cfg["contact"]=True
        elif a == "--align-parts":   cfg["align"]=True
        elif a == "--sheet-cols":    i+=1; cfg["sheet_cols"]=int(argv[i])
        elif a == "--sheet-res":     i+=1; cfg["sheet_res"]=int(argv[i])
        elif a == "--batch":         i+=1; cfg["batch"]=argv[i]
        elif a == "--scale-ref":     i+=1; cfg["scale_ref"]=argv[i]
        elif a == "--scale-ref-height": i+=1; cfg["scale_ref_height"]=float(argv[i])
        else:                        cfg["paths"].append(a)
        i += 1
    if not cfg["paths"] and not cfg["batch"]:
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
    # STL is authored in mm; these are the model's true printed dimensions,
    # captured on the ONE line where they still exist — the next statement
    # scales everything to a 2.0-unit stage and discards absolute size.
    dims_mm = tuple(obj.dimensions)
    s = 2.0 / (max(obj.dimensions) or 1.0); obj.scale = (s,s,s)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    obj.location = (0,0,0); bpy.context.view_layer.update()
    zmin = min((obj.matrix_world @ v.co).z for v in obj.data.vertices)
    obj.location.z -= zmin; bpy.context.view_layer.update()
    return dims_mm

def add_scale_reference(cfg, obj, dims_mm):
    """Import the reference figure ("banana for scale") beside the model.

    True relative size, not equal framing: normalize() made the model's
    longest side 2.0 stage units, so one stage unit = max(dims_mm)/2 mm.
    The reference is scaled to stand scale_ref_height mm tall in that SAME
    mm space — a 28 mm scale guy next to a 90 mm ogre stays chest-height.
    The reference STL is assumed sliced-ready (Z up); it is deliberately
    absent from MEASURED (it isn't part of the model) but included in the
    camera framing so it never clips.
    """
    path = cfg.get("scale_ref")
    if not path:
        return None
    path = os.path.normpath(path)
    if not os.path.isfile(path):
        print(f"[render_mini] scale reference not found, skipping: {path}")
        return None
    before = set(bpy.data.objects)
    if hasattr(bpy.ops.wm, "stl_import"): bpy.ops.wm.stl_import(filepath=path)
    else: bpy.ops.import_mesh.stl(filepath=path)
    added = [o for o in bpy.data.objects if o not in before and o.type == "MESH"]
    if not added:
        print("[render_mini] scale reference import produced no mesh, skipping")
        return None
    bpy.ops.object.select_all(action="DESELECT")
    for o in added: o.select_set(True)
    bpy.context.view_layer.objects.active = added[0]
    if len(added) > 1: bpy.ops.object.join()
    ref = bpy.context.view_layer.objects.active
    ref.name = "ScaleRef"; bpy.ops.object.shade_smooth()
    bpy.ops.object.origin_set(type="ORIGIN_CENTER_OF_VOLUME")
    bpy.context.view_layer.update()

    mm_per_unit = (max(dims_mm) or 1.0) / 2.0
    native_h = ref.dimensions.z or 1.0
    s = (cfg["scale_ref_height"] / mm_per_unit) / native_h
    ref.scale = (s, s, s)
    bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
    # seat on the floor and stand it just off the model's shadow-side edge
    ref.location = (0, 0, 0); bpy.context.view_layer.update()
    zmin = min((ref.matrix_world @ v.co).z for v in ref.data.vertices)
    ref.location.z -= zmin
    gap = 0.18
    ref.location.x = -(obj.dimensions.x / 2 + ref.dimensions.x / 2 + gap)
    bpy.context.view_layer.update()
    return ref

def resin_material(obj, color, look="flat"):
    # use_nodes is deprecated (always True, no-op) since Blender 5.0 — new
    # materials already carry the default Principled BSDF + Output nodes
    m = bpy.data.materials.new("Resin")
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
    # same deprecated-use_nodes story as resin_material() above
    w = bpy.data.worlds.new("World"); bpy.context.scene.world = w
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

def camera(objs, azimuth, elev, zoom):
    cd = bpy.data.cameras.new("Camera"); cd.lens = LOOK["cam_lens"]
    cam = bpy.data.objects.new("Camera", cd); bpy.context.collection.objects.link(cam)
    bpy.context.scene.camera = cam
    # frame everything on stage (model + optional scale reference)
    coords = [o.matrix_world @ v.co for o in objs for v in o.data.vertices]
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

def build_and_render(cfg, rotate, out, res, samples, index=0, measure=False):
    """Full pipeline for one image at a given rotation."""
    clear()
    obj = import_join(cfg["paths"], cfg["align"])
    dims_mm = normalize(obj, rotate)
    if measure:
        # Machine-readable, like CONTACT_SHEET below. flush matters: Python
        # stdout is block-buffered when piped, and the Rust side attributes
        # progress to models by these lines arriving in order.
        print("MEASURED " + json.dumps(
            {"index": index,
             "dims_mm": [round(d, 1) for d in dims_mm],
             "parts": len(cfg["paths"])}), flush=True)
    ref = add_scale_reference(cfg, obj, dims_mm)
    resin_material(obj, cfg["color"], cfg["look"])
    if ref is not None:
        # neutral grey: the reference must read as a ruler, not a product
        resin_material(ref, (0.52, 0.54, 0.58), cfg["look"])
    lights(cfg["look"]); black_world(cfg["look"])
    camera([obj] + ([ref] if ref is not None else []),
           cfg["azimuth"], cfg["elev"], cfg["zoom"])
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

# ----------------------------- batch mode ---------------------------------
def run_batch(cfg, manifest_path):
    """Render many minis in ONE Blender launch (--batch manifest.json).

    Each `blender -b` costs seconds of startup before the first sample — for
    a library-wide preview sweep that's the dominant cost. The manifest is
    {"entries":[{"parts":[...],"out":...,"rotate":[x,y,z], ...overrides}]};
    entries render sequentially, clear() resetting the scene between them
    (the same loop the contact sheet has always used). One bad STL must not
    kill the batch: import_join raises SystemExit, so that is caught per
    entry and reported as a machine-readable BATCH_DONE ok:false.

    A future scale-reference figure ("banana for scale") slots in here: one
    more manifest key naming a reference STL imported next to the model and
    excluded from the 2.0-unit normalization.
    """
    import copy
    with open(manifest_path, encoding="utf-8") as fh:
        entries = json.load(fh)["entries"]
    base_look = copy.deepcopy(LOOK)
    print("BATCH_START " + json.dumps({"total": len(entries)}), flush=True)
    for i, e in enumerate(entries):
        print("BATCH_MODEL " + json.dumps({"index": i, "out": e["out"]}), flush=True)
        try:
            # per-entry LOOK overrides must not leak into the next entry
            LOOK.clear(); LOOK.update(copy.deepcopy(base_look))
            if e.get("config"): merge_config(LOOK, e["config"])
            ecfg = dict(cfg)
            ecfg["paths"] = e["parts"]
            ecfg["align"] = bool(e.get("align", cfg["align"]))
            for k in ("look", "res", "samples", "azimuth", "elev", "zoom",
                      "scale_ref", "scale_ref_height"):
                if k in e: ecfg[k] = e[k]
            if "color" in e: ecfg["color"] = tuple(e["color"])
            rotate = tuple(e.get("rotate", (90, 0, 0)))
            build_and_render(ecfg, rotate, e["out"], ecfg["res"], ecfg["samples"],
                             index=i, measure=True)
            print("BATCH_DONE " + json.dumps({"index": i, "ok": True}), flush=True)
        except (SystemExit, Exception) as ex:
            print("BATCH_DONE " + json.dumps(
                {"index": i, "ok": False, "error": str(ex) or type(ex).__name__}),
                flush=True)

# ----------------------------- main ---------------------------------------
def main():
    cfg = parse()
    if cfg["batch"]:
        print(f"[render_mini] batch -> {cfg['batch']}")
        run_batch(cfg, cfg["batch"])
        print("[render_mini] done.")
        return
    first = os.path.normpath(cfg["paths"][0])
    if cfg["contact"]:
        out = cfg["out"] or (os.path.splitext(first)[0] + "_sheet.png")
        print(f"[render_mini] contact sheet -> {out}")
        make_contact_sheet(cfg, out)
    else:
        out = cfg["out"] or (os.path.splitext(first)[0] + ".png")
        print(f"[render_mini] rendering -> {out}")
        build_and_render(cfg, cfg["rotate"], out, cfg["res"], cfg["samples"],
                         measure=True)
    print("[render_mini] done.")

if __name__ == "__main__":
    main()
