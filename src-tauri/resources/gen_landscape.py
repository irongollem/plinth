# Parametric landscape generator — bakes a heightfield terrain STL from a
# JSON parameter set. See docs/BASECUTTER.md "The landscape generator
# (phase 6)": the point is that every starter terrain (cobblestone street,
# sandy dunes, rocky ground, lava flow) is a heightfield, and a displaced
# grid with a skirt and bottom cap is watertight by construction — no
# undercuts, no designer sculpt required, no bundled STL assets.
#
# PRESETS LIVE IN RUST (basecutter::generator::get_landscape_presets), NOT
# HERE — this script only knows parameters, never preset names.
#
# Params JSON (path after `--params`), all lengths in mm, Z-up:
# {
#   "out": "/path/to/landscape.stl",
#   "seed": 12345,
#   "width_mm": 120.0, "depth_mm": 80.0,
#   "resolution_mm": 0.75,      # grid step; floored to 0.4 (see MIN_RESOLUTION_MM)
#   "carrier_mm": 2.0,          # flat plate thickness under the sculpted relief
#   "relief_mm": 6.0,           # sculpted height ABOVE the carrier (max - min)
#   "layers": {
#     "noise":    { "enabled": true, "scale": 0.05, "octaves": 4,
#                   "ridged": false, "amount": 1.0 },
#     "ripples":  { "enabled": false, "wavelength_mm": 8.0, "direction_deg": 0.0,
#                   "amount": 1.0, "waviness": 0.3 },
#     "stones":   { "enabled": false, "cell_mm": 12.0, "gap_mm": 1.2,
#                   "dome": 0.6, "jitter": 0.15, "amount": 1.0 },
#     "boulders": { "enabled": false, "count": 6, "min_mm": 8.0, "max_mm": 20.0,
#                   "amount": 1.0 },
#     "flow":     { "enabled": false, "channel_width_mm": 10.0,
#                   "meander_scale": 0.3, "bank_height": 1.0, "amount": 1.0 },
#     "camber":   { "enabled": false, "amount": 1.0 }
#   }
# }
#
# Every layer is additive (all summable); the combined field is then
# renormalized so its minimum sits at 0 and its maximum equals relief_mm,
# and the whole plate is lifted by carrier_mm on top of that — the carrier
# is a flat spacer, never counted as relief. See generate()'s docstring for
# the exact geometry recipe.
#
# Determinism: mathutils.noise has no built-in seed parameter — it is a
# pure function of position — so seed variance comes entirely from adding
# a seed-derived offset vector to the sample position before every noise
# call (see _seed_offset), and from seeding python's own random.Random for
# anything stochastic (boulder placement, stone jitter uses a position hash
# instead — see _hash01). Same seed + same params always bakes the same
# mesh, byte-for-byte modulo STL float rounding.
#
# stdout protocol (parsed by basecutter::generator):
#   GENERATING {"seed":...}
#   GENERATED {"out":..., "dims_mm":[x,y,z], "verts":N, "manifold":bool}
#   GENERATION_FAILED {"reason":...}
# Exit code: 0 on success; a caught generation error prints GENERATION_FAILED
# then sys.exit(1); an uncaught exception (bad params JSON, missing "out",
# ...) propagates and is turned into a non-zero exit by Blender's own
# `--python-exit-code 1` (the same convention as base_cut.py/render_mini.py).

import json
import math
import random
import sys
import traceback

import bmesh
import bpy
from mathutils import Vector, noise

# Floor on the grid step (docs/BASECUTTER.md phase 6): below this the vertex
# count explodes on a normal-sized plate for no visible print-quality gain —
# FDM/resin layer lines are coarser than a 0.4mm grid.
MIN_RESOLUTION_MM = 0.4
DEFAULT_RESOLUTION_MM = 0.75
DEFAULT_CARRIER_MM = 2.0

# Merge-by-distance / degenerate-face thresholds — same values as
# base_cut.py's cleanup_and_check for the same reason (booleans aren't even
# involved here, but the STL float32 roundtrip can still turn a near-zero
# sliver into an exact zero, dropping a pinhole in the shell).
MERGE_DIST_MM = 0.001

# Salts keep each layer's seed-derived randomness independent of which OTHER
# layers are enabled — every layer draws from its own seed^salt stream, so
# toggling "ripples" off never reshuffles "noise"'s offset.
_SALT_NOISE = 0x1
_SALT_RIPPLES = 0x2
_SALT_FLOW = 0x3
_SALT_BOULDERS = 0x4


def tok(name, payload=None):
    line = name if payload is None else name + " " + json.dumps(payload)
    print(line, flush=True)


# ------------------------------------------------------------- determinism

def _seed_offset(seed, salt):
    """A stable-per-(seed, salt) 3D offset added to sample positions before
    every mathutils.noise call — the mechanism that makes an otherwise
    seedless noise function vary with `seed` (see the module docstring)."""
    rng = random.Random((int(seed) & 0xFFFFFFFF) ^ (salt * 0x9E3779B1))
    return Vector((rng.uniform(-1000.0, 1000.0) for _ in range(3)))


def _hash01(seed, ix, iy, salt=0):
    """Deterministic float in [0, 1) from (seed, cell coords, salt) — a
    position hash, not a stream, so it doesn't care what order cells are
    visited in (needed for stones' jittered cell centers and per-stone
    height variance, both of which must be stable across the whole grid)."""
    h = (int(seed) * 374761393) ^ (int(ix) * 668265263) ^ (int(iy) * 2147483647) ^ (salt * 2246822519)
    h &= 0xFFFFFFFF
    h = (h ^ (h >> 13)) * 1274126177 & 0xFFFFFFFF
    h ^= h >> 16
    return (h & 0xFFFFFFFF) / 4294967295.0


# ------------------------------------------------------------ style layers

def _fractal(x, y, offset, scale, octaves, ridged, lacunarity=2.0, persistence=0.5):
    """Stacked-octave noise, in the spirit of mathutils.noise.fractal but
    with a per-octave ridged transform (docs/BASECUTTER.md: "ridged: bool
    (abs/1-abs for sharp crests)") — a single abs() applied to the finished
    sum would soften into rolling hills, not crests; folding it into every
    octave keeps high-frequency detail sharp too. `scale` is a frequency
    multiplier (bigger scale = smaller, more numerous features), matching
    Blender's own Noise Texture node convention. Returns roughly [-1, 1]
    (or [0, 1] when ridged) — exact range doesn't matter since the whole
    heightfield gets renormalized afterwards."""
    total = 0.0
    amplitude = 1.0
    freq = 1.0
    max_amp = 0.0
    for _ in range(max(1, int(octaves))):
        p = Vector((x * scale * freq + offset.x, y * scale * freq + offset.y, offset.z))
        n = noise.noise(p)
        if ridged:
            n = 1.0 - abs(n)
        total += n * amplitude
        max_amp += amplitude
        amplitude *= persistence
        freq *= lacunarity
    return (total / max_amp) if max_amp > 1e-9 else 0.0


def _noise_layer(seed, params):
    offset = _seed_offset(seed, _SALT_NOISE)
    scale = params.get("scale", 0.05)
    octaves = params.get("octaves", 4)
    ridged = bool(params.get("ridged", False))
    amount = params.get("amount", 1.0)
    return lambda x, y: _fractal(x, y, offset, scale, octaves, ridged) * amount


def _ripples_layer(seed, params):
    """Windswept sand: a sine wave along `direction_deg`, its phase distorted
    by a slow noise field when `waviness` > 0 so the ripples meander rather
    than ruling dead-straight lines."""
    offset = _seed_offset(seed, _SALT_RIPPLES)
    wavelength = max(0.05, params.get("wavelength_mm", 8.0))
    direction = math.radians(params.get("direction_deg", 0.0))
    amount = params.get("amount", 1.0)
    waviness = params.get("waviness", 0.0)
    dx, dy = math.cos(direction), math.sin(direction)

    def fn(x, y):
        phase = 2.0 * math.pi * (x * dx + y * dy) / wavelength
        if waviness:
            p = Vector((x * 0.03 + offset.x, y * 0.03 + offset.y, offset.z))
            phase += waviness * noise.noise(p) * math.pi
        return math.sin(phase) * amount

    return fn


def _cell_center(seed, cell_mm, ix, iy):
    """Center of Voronoi cell (ix, iy), jittered off the regular grid so
    cobblestones don't look like graph paper. The 0.7 jitter fraction (of
    half a cell) is a fixed part of the algorithm, not a user parameter —
    `stones.jitter` controls per-stone HEIGHT variance instead (see
    docs/BASECUTTER.md's stones layer params)."""
    jx = _hash01(seed, ix, iy, 1) - 0.5
    jy = _hash01(seed, ix, iy, 2) - 0.5
    return Vector(((ix + 0.5 + jx * 0.7) * cell_mm, (iy + 0.5 + jy * 0.7) * cell_mm))


def _stones_layer(seed, params):
    """2D Voronoi by brute-force nearest/second-nearest search over a
    jittered 3x3 neighborhood of cell centers (no scipy available inside
    Blender's python) — see docs/BASECUTTER.md's stones layer. Each stone
    domes up from its cell center; within `gap_mm` of the cell border (i.e.
    where the distance to the 2nd-nearest center is within gap_mm of the
    distance to the nearest) the surface drops to a recessed mortar floor
    instead."""
    cell_mm = max(0.5, params.get("cell_mm", 12.0))
    gap_mm = max(0.0, params.get("gap_mm", 1.2))
    dome = min(1.0, max(0.0, params.get("dome", 0.6)))
    jitter = params.get("jitter", 0.15)
    amount = params.get("amount", 1.0)
    radius = max(0.05, cell_mm * 0.5 - gap_mm * 0.5)

    def fn(x, y):
        ix0 = math.floor(x / cell_mm)
        iy0 = math.floor(y / cell_mm)
        p = Vector((x, y))
        best_d = math.inf
        second_d = math.inf
        best_ix = best_iy = 0
        for dj in (-1, 0, 1):
            for di in (-1, 0, 1):
                ix, iy = ix0 + di, iy0 + dj
                d = (p - _cell_center(seed, cell_mm, ix, iy)).length
                if d < best_d:
                    second_d = best_d
                    best_d, best_ix, best_iy = d, ix, iy
                elif d < second_d:
                    second_d = d
        if (second_d - best_d) < gap_mm:
            return -0.3 * amount  # recessed mortar between stones
        t = min(1.0, best_d / radius)
        flat_top = 1.0
        rounded = math.sqrt(max(0.0, 1.0 - t * t))
        shape = flat_top * (1.0 - dome) + rounded * dome
        height_scale = 1.0 + jitter * (_hash01(seed, best_ix, best_iy, 3) * 2.0 - 1.0)
        return shape * height_scale * amount

    return fn


def _boulders_layer(seed, width_mm, depth_mm, params):
    """N seeded gaussian bumps — count/min/max diameter, one weight for the
    whole layer. A dedicated random.Random stream (seed ^ a fixed salt) so
    boulder placement never shifts when other layers' params change.

    Combined by MAX, not sum: two overlapping boulders should look like two
    domes touching, not a single tower twice as tall. A sum would also make
    the final normalize-to-relief_mm step read the busiest cluster as "the"
    peak and flatten every solitary boulder elsewhere on the plate down to
    near nothing."""
    count = max(0, int(params.get("count", 6)))
    min_mm = params.get("min_mm", 8.0)
    max_mm = params.get("max_mm", 20.0)
    amount = params.get("amount", 1.0)
    rng = random.Random((int(seed) & 0xFFFFFFFF) ^ _SALT_BOULDERS)
    boulders = []
    for _ in range(count):
        cx = rng.uniform(-width_mm / 2.0, width_mm / 2.0)
        cy = rng.uniform(-depth_mm / 2.0, depth_mm / 2.0)
        diameter = rng.uniform(min(min_mm, max_mm), max(min_mm, max_mm))
        boulders.append((cx, cy, max(0.1, diameter / 2.0)))

    def fn(x, y):
        peak = 0.0
        for cx, cy, r in boulders:
            sigma = max(0.1, r * 0.6)
            dist2 = (x - cx) ** 2 + (y - cy) ** 2
            bump = math.exp(-dist2 / (2.0 * sigma * sigma))
            if bump > peak:
                peak = bump
        return peak * amount

    return fn


def _flow_layer(seed, params):
    """Lava/river channel field: low-frequency noise, absolute-valued so its
    zero-crossings become winding channel centerlines and its peaks become
    raised banks between them (docs/BASECUTTER.md: "smooth ropey flow
    channels between raised crusted banks"). `bank_height` sharpens the
    channel-to-bank transition (a power curve) rather than scaling the
    layer overall — `amount` is still the one knob every layer shares for
    that."""
    offset = _seed_offset(seed, _SALT_FLOW)
    channel_width = max(0.5, params.get("channel_width_mm", 10.0))
    meander = max(0.01, params.get("meander_scale", 0.3))
    bank_height = max(0.05, params.get("bank_height", 1.0))
    amount = params.get("amount", 1.0)
    freq = 1.0 / channel_width

    def fn(x, y):
        p = Vector((x * freq + offset.x, y * freq * meander + offset.y, offset.z))
        base = abs(noise.noise(p))
        shaped = base ** max(0.2, 1.0 / bank_height)
        return shaped * amount

    return fn


def _camber_layer(width_mm, params):
    """Parabolic crown across the plate's width (cobblestone streets are
    highest at the centerline, sloping to the gutters at the edges)."""
    amount = params.get("amount", 1.0)
    half = width_mm / 2.0

    def fn(x, y):
        if half <= 0:
            return 0.0
        t = x / half
        return (1.0 - t * t) * amount

    return fn


def build_layer_fns(seed, width_mm, depth_mm, layers):
    """One f(x, y) -> contribution callable per ENABLED layer, in a fixed
    order — the order only affects summation float rounding, never the
    seed-derived randomness each layer draws (every layer's RNG/offset is
    salted independently, see the module docstring)."""
    fns = []
    if layers.get("noise", {}).get("enabled"):
        fns.append(_noise_layer(seed, layers["noise"]))
    if layers.get("ripples", {}).get("enabled"):
        fns.append(_ripples_layer(seed, layers["ripples"]))
    if layers.get("stones", {}).get("enabled"):
        fns.append(_stones_layer(seed, layers["stones"]))
    if layers.get("boulders", {}).get("enabled"):
        fns.append(_boulders_layer(seed, width_mm, depth_mm, layers["boulders"]))
    if layers.get("flow", {}).get("enabled"):
        fns.append(_flow_layer(seed, layers["flow"]))
    if layers.get("camber", {}).get("enabled"):
        fns.append(_camber_layer(width_mm, layers["camber"]))
    return fns


# ------------------------------------------------------------ mesh building

def new_object(name, bm):
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.collection.objects.link(obj)
    return obj


def build_heightfield(xs, ys, heights):
    """Displaced grid + boundary skirt + bottom cap, watertight by
    construction (docs/BASECUTTER.md phase 6). `heights[j][i]` is the
    world-space Z for grid point (xs[i], ys[j]).

    The bottom cap is a single n-gon over the full perimeter loop — the
    same convention base_cut.py's loft_solid uses for its top/bottom caps
    (a proven pattern in this codebase for STL-exportable closed solids),
    used here instead of bmesh.ops.holes_fill so cap construction doesn't
    depend on the fill operator picking a good triangulation of an
    arbitrarily long, mostly-collinear boundary loop.
    """
    nx, ny = len(xs), len(ys)
    bm = bmesh.new()

    top = [[bm.verts.new((xs[i], ys[j], heights[j][i])) for i in range(nx)] for j in range(ny)]

    # Top surface: one quad per grid cell. (v00, v10, v11, v01) winds CCW
    # when viewed from +Z for increasing (x, y) — recalc_face_normals below
    # makes the final orientation authoritative regardless.
    for j in range(ny - 1):
        for i in range(nx - 1):
            bm.faces.new((top[j][i], top[j][i + 1], top[j + 1][i + 1], top[j + 1][i]))

    # Perimeter loop, CCW: bottom edge (i: 0->nx-1 @ j=0), right edge
    # (j: 1->ny-1 @ i=nx-1), top edge (i: nx-2->0 @ j=ny-1), left edge
    # (j: ny-2->1 @ i=0).
    perimeter_top = []
    perimeter_top.extend(top[0][i] for i in range(nx))
    perimeter_top.extend(top[j][nx - 1] for j in range(1, ny))
    perimeter_top.extend(top[ny - 1][i] for i in range(nx - 2, -1, -1))
    perimeter_top.extend(top[j][0] for j in range(ny - 2, 0, -1))

    perimeter_bottom = [bm.verts.new((v.co.x, v.co.y, 0.0)) for v in perimeter_top]

    n = len(perimeter_top)
    for k in range(n):
        a, b = perimeter_top[k], perimeter_top[(k + 1) % n]
        c, d = perimeter_bottom[(k + 1) % n], perimeter_bottom[k]
        bm.faces.new((a, b, c, d))

    bm.faces.new(tuple(reversed(perimeter_bottom)))

    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return new_object("landscape", bm)


def bbox_dims(verts):
    """Single-pass streaming min/max (see base_cut.py's identical helper for
    why: avoids materializing three full coordinate lists on a large grid)."""
    min_x = min_y = min_z = math.inf
    max_x = max_y = max_z = -math.inf
    for v in verts:
        co = v.co
        min_x, max_x = min(min_x, co.x), max(max_x, co.x)
        min_y, max_y = min(min_y, co.y), max(max_y, co.y)
        min_z, max_z = min(min_z, co.z), max(max_z, co.z)
    if min_x is math.inf:
        return [0.0, 0.0, 0.0]
    return [max_x - min_x, max_y - min_y, max_z - min_z]


def cleanup_and_check(obj):
    """Merge stray verts, fix normals, return (manifold, dims_mm, verts).
    Same recipe (and same rationale) as base_cut.py's cleanup_and_check."""
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=MERGE_DIST_MM)
    bmesh.ops.dissolve_degenerate(bm, edges=bm.edges, dist=MERGE_DIST_MM)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    manifold = all(e.is_manifold for e in bm.edges)
    dims = bbox_dims(bm.verts)
    verts = len(bm.verts)
    bm.to_mesh(obj.data)
    bm.free()
    return manifold, [round(d, 3) for d in dims], verts


# ---------------------------------------------------------------------- job

def generate(params):
    width_mm = float(params["width_mm"])
    depth_mm = float(params["depth_mm"])
    resolution_mm = max(MIN_RESOLUTION_MM, float(params.get("resolution_mm", DEFAULT_RESOLUTION_MM)))
    carrier_mm = float(params.get("carrier_mm", DEFAULT_CARRIER_MM))
    relief_mm = float(params.get("relief_mm", 6.0))
    seed = int(params.get("seed", 0))
    layers = params.get("layers", {}) or {}
    out = params["out"]

    nx = max(2, round(width_mm / resolution_mm) + 1)
    ny = max(2, round(depth_mm / resolution_mm) + 1)
    xs = [-width_mm / 2.0 + i * (width_mm / (nx - 1)) for i in range(nx)]
    ys = [-depth_mm / 2.0 + j * (depth_mm / (ny - 1)) for j in range(ny)]

    layer_fns = build_layer_fns(seed, width_mm, depth_mm, layers)

    raw = [[0.0] * nx for _ in range(ny)]
    h_min, h_max = math.inf, -math.inf
    for j, y in enumerate(ys):
        row = raw[j]
        for i, x in enumerate(xs):
            h = 0.0
            for fn in layer_fns:
                h += fn(x, y)
            row[i] = h
            if h < h_min:
                h_min = h
            if h > h_max:
                h_max = h

    span = h_max - h_min
    heights = [[0.0] * nx for _ in range(ny)]
    for j in range(ny):
        for i in range(nx):
            normalized = ((raw[j][i] - h_min) / span * relief_mm) if span > 1e-9 else 0.0
            heights[j][i] = carrier_mm + normalized

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete()

    obj = build_heightfield(xs, ys, heights)
    manifold, dims, vert_count = cleanup_and_check(obj)

    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.wm.stl_export(filepath=out, export_selected_objects=True)

    return out, dims, vert_count, manifold


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    params_path = argv[argv.index("--params") + 1]
    with open(params_path, encoding="utf-8") as f:
        params = json.load(f)

    seed = int(params.get("seed", 0))
    tok("GENERATING", {"seed": seed})
    try:
        out, dims, verts, manifold = generate(params)
    except Exception as e:  # noqa: BLE001 — reported as a token, not a crash
        traceback.print_exc()
        tok("GENERATION_FAILED", {"reason": str(e)})
        sys.exit(1)

    tok("GENERATED", {"out": out, "dims_mm": dims, "verts": verts, "manifold": manifold})


main()
