# Parametric landscape generator — bakes a heightfield terrain STL from a
# JSON parameter set. See docs/BASECUTTER.md "The landscape generator
# (phase 6)": the point is that every starter terrain (cobblestone street,
# sandy dunes, rocky ground, lava flow, forest floor) is a heightfield, and
# a displaced grid with a skirt and bottom cap is watertight by
# construction — no undercuts, no designer sculpt required, no bundled STL
# assets.
#
# PRESETS LIVE IN RUST (basecutter::generator::get_landscape_presets), NOT
# HERE — this script only knows parameters, never preset names.
#
# Params JSON (path after `--params`), all lengths in mm, Z-up:
# {
#   "out": "/path/to/landscape.stl",
#   "seed": 12345,
#   "width_mm": 120.0, "depth_mm": 80.0,
#   "resolution_mm": 0.75,      # grid step; floor 0.1 (resin-grade), and
#                                # coarsened to fit MAX_GRID_VERTS on big
#                                # plates — GENERATED reports the effective value
#   "feature_scale": 1.0,       # zooms the TERRAIN (stone cells, ripple
#                                # wavelengths, boulder sizes — every layer's
#                                # characteristic length), clamped 0.25-4;
#                                # orthogonal to resolution_mm's mesh density
#   "carrier_mm": 2.0,          # flat plate thickness under the sculpted relief
#   "relief_mm": 6.0,           # sculpted height ABOVE the carrier (max - min)
#   "layers": {
#     "noise":    { "enabled": true, "scale": 0.05, "octaves": 4,
#                   "ridged": false, "amount": 1.0 },
#     "ripples":  { "enabled": false, "wavelength_mm": 8.0, "direction_deg": 0.0,
#                   "amount": 1.0, "waviness": 0.3 },
#     "stones":   { "enabled": false, "cell_mm": 4.0, "gap_mm": 0.5,
#                   "dome": 0.6, "jitter": 0.15, "cluster": 0.0,
#                   "rough": 0.0, "amount": 1.0 },
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
#   GENERATED {"out":..., "dims_mm":[x,y,z], "verts":N, "manifold":bool,
#              "resolution_mm":effective}
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

# Floor on the grid step. 0.1mm is resin territory (resin XY resolution
# ~0.05mm; FDM stops seeing lateral detail around a 0.4mm line width) —
# the floor exists only to keep a typo like 0.01 from freezing the bake.
# The REAL guard is MAX_GRID_VERTS below: cost is quadratic in the step,
# so a fine step on a big plate is capped by area, not by a fixed number.
MIN_RESOLUTION_MM = 0.1
DEFAULT_RESOLUTION_MM = 0.75
DEFAULT_CARRIER_MM = 2.0

# Vertex budget for the displaced grid. ~2M verts bakes in tens of seconds
# (pure-Python per-vertex layer evaluation is the bottleneck, not Blender)
# and stays comfortable for the viewport and the boolean cutter. When the
# requested step would exceed this on the requested plate, the step is
# coarsened to fit and the GENERATED payload reports the effective value.
MAX_GRID_VERTS = 2_000_000

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
_SALT_CLUSTER = 0x5
_SALT_EDGE = 0x6


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
    """Windswept sand: a sine wave along `direction_deg`, phase-distorted by
    a slow noise field so the crests meander, and amplitude-modulated by a
    second slow field so ripples strengthen and fade in patches. Both are
    what real sand does — a constant-amplitude, near-straight sine reads as
    machined corduroy, not dunes (the first sandy bake did exactly that:
    ±0.5π of wobble across a 9mm wavelength is visually a ruled line)."""
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
            phase += waviness * noise.noise(p) * 2.0 * math.pi
        # Patchy strength: never fully dead (floor 0.25), never uniform.
        m = Vector((x * 0.02 + offset.y, y * 0.02 + offset.z, offset.x))
        patch = 0.625 + 0.375 * noise.noise(m)
        return math.sin(phase) * patch * amount

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


def _smoothstep(t):
    t = min(1.0, max(0.0, t))
    return t * t * (3.0 - 2.0 * t)


def _stones_layer(seed, params, resolution_mm):
    """2D Voronoi by brute-force nearest/second-nearest search over a
    jittered 3x3 neighborhood of cell centers (no scipy available inside
    Blender's python) — see docs/BASECUTTER.md's stones layer. Each stone
    domes up from its cell center and drops to a recessed mortar floor at
    the cell border.

    The stone->mortar transition is a smoothstep band, never a hard cliff,
    and the band is sized to the GRID (>= ~2 grid steps): a height jump
    narrower than the sampling resolution can only render as a staircase
    along diagonal borders — the aliasing that made the first cut bases
    look pixelated. Softening in the heightfield is the fix; refining the
    grid alone just shrinks the pixels."""
    cell_mm = max(0.5, params.get("cell_mm", 4.0))
    gap_mm = max(0.0, params.get("gap_mm", 0.5))
    dome = min(1.0, max(0.0, params.get("dome", 0.6)))
    jitter = params.get("jitter", 0.15)
    amount = params.get("amount", 1.0)
    # cluster (0..1): 0 = every cell is a stone with uniform gaps (cobbles);
    # towards 1, a slow coherence field decides per cell — low cells drown
    # to the floor entirely (open lakes) and the gap between two strongly-
    # crusted neighbors closes up, fusing them into one large mass. This is
    # what separates "lava crust" from "cobblestone street": the same
    # Voronoi, unevenly distributed.
    cluster = min(1.0, max(0.0, params.get("cluster", 0.0)))
    # rough (0..1): high-frequency wobble on the border distance — ragged,
    # broken plate outlines instead of clean Voronoi edges. Feature size is
    # ~1-2mm, so it only resolves when the grid is fine enough (which is
    # exactly the behavior wanted: raggedness is a resolution-permitting
    # detail, never aliasing).
    rough = min(1.0, max(0.0, params.get("rough", 0.0)))
    cluster_offset = _seed_offset(seed, _SALT_CLUSTER)
    edge_offset = _seed_offset(seed, _SALT_EDGE)
    # Distance from the Voronoi border where the stone reaches full height:
    # half the mortar gap is mortar floor, then a resolution-scaled shoulder.
    # The shoulder must clear ~1 grid step to kill aliasing but stay small
    # against the stone radius — at 2x resolution a 4mm cobble was ALL
    # shoulder and read as dimples, not setts.
    edge = gap_mm * 0.5
    shoulder = max(0.25, resolution_mm * 1.25)
    radius = max(0.05, cell_mm * 0.5 - edge)
    mortar = -0.3

    def crustiness(ix, iy):
        """Slow coherent field sampled at the CELL CENTER — neighbors get
        correlated values, so crust survives in patches, not salt-and-pepper."""
        c = _cell_center(seed, cell_mm, ix, iy)
        p = Vector((
            c.x * 0.02 + cluster_offset.x,
            c.y * 0.02 + cluster_offset.y,
            cluster_offset.z,
        ))
        return 0.5 + 0.5 * noise.noise(p)

    def fn(x, y):
        ix0 = math.floor(x / cell_mm)
        iy0 = math.floor(y / cell_mm)
        p = Vector((x, y))
        best_d = math.inf
        second_d = math.inf
        best_ix = best_iy = 0
        second_ix = second_iy = 0
        for dj in (-1, 0, 1):
            for di in (-1, 0, 1):
                ix, iy = ix0 + di, iy0 + dj
                d = (p - _cell_center(seed, cell_mm, ix, iy)).length
                if d < best_d:
                    second_d = best_d
                    second_ix, second_iy = best_ix, best_iy
                    best_d, best_ix, best_iy = d, ix, iy
                elif d < second_d:
                    second_d = d
                    second_ix, second_iy = ix, iy
        # ~distance to the Voronoi border between nearest and 2nd-nearest.
        border = (second_d - best_d) * 0.5
        gap_eff = edge
        if cluster > 0.0:
            c1 = crustiness(best_ix, best_iy)
            if c1 < cluster * 0.55:
                return mortar * amount  # this whole cell drowned — open lake
            c2 = crustiness(second_ix, second_iy)
            # Both neighbors solidly crusted -> the border between them
            # heals shut and they read as one mass. Ramp starts just above
            # the drown threshold so surviving neighborhoods actually fuse
            # instead of staying a polite tiled street.
            fuse = _smoothstep((min(c1, c2) - 0.45) / 0.25)
            gap_eff = edge * (1.0 - 0.9 * fuse * cluster)
        if rough > 0.0:
            e = Vector((
                x * 0.8 + edge_offset.x,
                y * 0.8 + edge_offset.y,
                edge_offset.z,
            ))
            border += rough * cell_mm * 0.12 * noise.noise(e)
        t = min(1.0, best_d / radius)
        rounded = math.sqrt(max(0.0, 1.0 - t * t))
        shape = (1.0 - dome) + rounded * dome
        height_scale = 1.0 + jitter * (_hash01(seed, best_ix, best_iy, 3) * 2.0 - 1.0)
        stone = shape * height_scale
        rise = _smoothstep((border - gap_eff) / shoulder)
        return (mortar + (stone - mortar) * rise) * amount

    return fn


def _boulders_layer(seed, width_mm, depth_mm, params, resolution_mm):
    """N seeded ROCKS — angular broken chunks, not smooth domes. A gaussian
    bump (the original implementation) is a perfect circle in plan and an
    all-shoulder, no-plateau profile — at miniature scale that reads as a
    pimple, not a rock. Fixing that took three attempts worth recording:

    Attempt 1 — a single warped-radius profile (radius = f(theta), or even
    the radial distance domain-warped by 2D noise before measuring) still
    failed: any profile that is ONE superellipse measured from ONE center
    is, by construction, a family of nested self-similar shells around that
    center. Warping barely changes that when the warp's wavelength is
    comparable to the boulder's own radius (the common case) — the result
    reads as a fluted bundt cake / flower, ridges running dead straight
    from crown to base, not a rock.

    Attempt 2 — a small UNION of 2-3 superellipse LOBES with different
    centers, combined by MAX. This does break exact radial symmetry (two
    shells around two different points intersect along a real, non-radial
    seam), and the footprint stops being a plain ellipse. But it still
    rendered as a smooth rounded "witch hat" (see the phase's verification
    renders): `1 - t**p_exp` is C1-continuous everywhere except exactly at
    a seam, and where two lobes of similar height cross, their SLOPES are
    similar too — a crease with near-equal slopes on both sides reads as
    smooth to the eye and to raking light, even though it is technically
    non-smooth. Rounded lobes can union into a bumpier rounded shape; they
    cannot union into a flat one. Real broken rock is flat FACES meeting at
    sharp ridges — that needs at least one genuinely PLANAR ingredient, and
    nothing planar existed anywhere in the stack.

    The fix: lobes still set the boulder's overall SILHOUETTE (footprint
    size/aspect/irregularity — kept from attempt 2, values below), but the
    TOP is now carved by intersecting that silhouette envelope with a small
    number of randomly tilted FACET PLANES, combined by MIN (the boolean-
    geometry mental model: a rock is what's left after several flat cuts
    through a rounded blank). A plane is linear, so unlike any power-law
    lobe its slope is constant right up to a hard crease where it meets the
    next plane or the envelope — THAT crease is a real edge, not a
    near-tangent blend, because two different constant slopes essentially
    never match. Few planes (4-6) with slope magnitude and origin drawn per
    boulder keep the facets FEW AND LARGE (a chunky flat face reads as
    "broken rock"; a dozen small ones just reads as noise) — the "fewer,
    bigger, more distinct" shape this module has been reaching for since
    attempt 1. The envelope is still what bounds the footprint and gives
    the rock a footing that tapers into the surrounding terrain instead of
    an infinite wedge (planes alone are unbounded).

    Per lobe, same two ideas as attempt 2 are kept for the envelope:
    - Rock profile, not dome: height follows `1 - t**p_exp` (t = normalized
      distance from the lobe's own center, 0..1); the higher exponent range
      here (vs. the original 2.5-4.8) keeps more of the lobe near full
      height and firms up the footprint's own edge, since the envelope
      itself is now also a visible boundary (where a facet plane runs
      higher than the envelope everywhere, the envelope's own edge is what
      you see). The last bit of the drop is smoothstep-softened over a
      band sized to resolution_mm (same antialiasing reasoning as
      _stones_layer's `shoulder`) so the SILHOUETTE edge doesn't stairstep
      — the facet CREASES inside it are deliberately left hard (see below).
    - Surface roughness: ON TOP of the carved surface, a further noise
      field displaces it, amplitude scaled to the boulder's OWN height and
      masked by the combined profile so it fades at the silhouette edge.
      Now RIDGED (folded like _fractal's ridged octaves) instead of raw
      Perlin — plain Perlin grain is smooth bumps, which is exactly the
      "not angular" complaint one level up; folding it creates small sharp
      creases in the grain too, consistent with the facets. Wavelength is
      floored to ~2.5 grid steps (never finer — that can only alias, not
      render as rock grain) and its amplitude is kept modest since the
      facets, not the grain, now carry the primary "broken rock" read.

    Facet creases are intentionally NOT smoothstep-softened the way the
    envelope's outer edge is: a real broken rock face meets its neighbor at
    a true sharp edge, and softening it would just recreate attempt 2's
    problem in miniature. A grid step of ~1mm sampling a facet spanning
    several mm of a ~10-25mm-radius boulder resolves the crease as a real
    polyline, not noise — and any residual single-step stairstepping on a
    crease that is SUPPOSED to be sharp reads as more rock, not less (this
    is the inverse of _stones_layer's aliasing problem, where the cliff was
    never supposed to be there).

    Every per-boulder, per-lobe, and per-facet random (lobe/facet count,
    offset, aspect, rotation, exponent, weight, slope, height) is drawn, in
    a fixed order, from ONE seeded stream (seed ^ salt) — deterministic
    from the seed and the boulder's position in that stream, so identical
    twins never happen but the same seed always bakes the same rocks.

    Whole boulders are combined by MAX, not sum: two overlapping boulders
    should look like two rocks touching, not a single tower twice as tall.
    A sum would also make the final normalize-to-relief_mm step read the
    busiest cluster as "the" peak and flatten every solitary boulder
    elsewhere on the plate down to near nothing."""
    count = max(0, int(params.get("count", 6)))
    min_mm = params.get("min_mm", 8.0)
    max_mm = params.get("max_mm", 20.0)
    amount = params.get("amount", 1.0)
    rng = random.Random((int(seed) & 0xFFFFFFFF) ^ _SALT_BOULDERS)

    def _make_lobe(ou, ov, r0, rot_extra, resolution_mm, weight):
        aspect = rng.uniform(0.75, 1.3)
        lrx = r0 * aspect
        lry = r0 / aspect
        # Higher floor than the original 2.5-4.8: the envelope is a
        # plateau-then-cliff, not a dome, on purpose. In whatever angular
        # sector a facet plane's descent is weakest (see _make_facets —
        # with only a few planes there's always SOME sector where none of
        # them point squarely outward), the envelope is what's left
        # carving that sector's outline, and the first faceted bake still
        # showed a visible round "collar" there (see the phase's
        # verification renders) — a slow power-law falloff over the outer
        # ~30% of the radius. A steeper exponent confines that falloff to a
        # narrower band close to the edge, so even an un-faceted sector
        # reads as a firm-edged plateau dropping to the terrain, not a dome
        # shoulder. (Pushing this much higher than 4.5-8, tried during
        # tuning, over-corrected: combined with tightly-clustered facet
        # origins it made every boulder read as a shallow cone instead of a
        # broken chunk — see _make_facets' `op_dist` for the other half of
        # that fix.)
        p_exp = rng.uniform(4.5, 8.0)
        # Edge shoulder width, in units of `t` (normalized 0..1 distance) —
        # a fixed mm width (same floor/scale as _stones_layer's `shoulder`)
        # turned into a fraction of THIS lobe's own radius, capped so a
        # tiny lobe on a coarse grid doesn't lose its whole plateau to the
        # antialiasing band.
        shoulder_t = min(0.45, max(resolution_mm * 1.25, 0.3) / max(lrx, lry))
        lca, lsa = math.cos(rot_extra), math.sin(rot_extra)
        return (ou, ov, lrx, lry, lca, lsa, p_exp, shoulder_t, weight)

    def _make_facets(r0):
        """4-6 tilted half-space planes, roughly spread around the boulder
        (a base angle per facet plus jitter, so facets don't cluster on one
        side) with per-facet height/origin variety so some read as one big
        flat face and others as a smaller chip near an edge. Each plane is
        `h0 - tilt/r0 * ((u-opx)*cos(ang) + (v-opy)*sin(ang))` in the
        boulder's own local (u, v) — min'd together (and against the
        envelope) in `fn` below, which is what turns a rounded blank into a
        faceted one (see the docstring above).

        `tilt` is DERIVED from a target zero-crossing distance rather than
        drawn directly: the first version drew slope magnitude straight
        from a fixed range, and in whatever direction a boulder happened to
        have no steeply-descending facet, that facet's height stayed near
        its (near-1.0) origin value almost all the way to the rim — a flat
        plateau in profile terms, which the envelope then rounds off at the
        very edge, reading right back as a smooth shoulder in that sector
        (caught by walking the camera around the boulder in the phase's
        verification renders, not visible from a single angle). Deriving
        `tilt` from `h0` and a target distance instead guarantees EVERY
        facet actually reaches zero somewhere inside its own direction, so
        no sector is left flat.

        `op_dist` — how far a facet's own local origin sits off the
        boulder's center — is deliberately wide (up to half the radius),
        not a tight cluster near the middle: a first pass kept every
        facet's origin close to center, and with a guaranteed nearby
        zero-crossing on top, every plane became a wedge converging on
        roughly the same apex — a shallow CONE with a jagged crown, round
        in silhouette from most angles, not a broken chunk. Scattering the
        origins across the footprint instead makes different facets peak
        and vanish in different places, which is what actually produces
        faces of visibly different size and position rather than uniform
        pie slices (verified by walking the camera around; this is the
        version that reads as broken rock from every angle tried, not just
        one)."""
        num_facets = rng.randint(4, 6)
        facets = []
        for i in range(num_facets):
            base_ang = (2.0 * math.pi * i / num_facets) + rng.uniform(
                -0.5, 0.5
            ) * (2.0 * math.pi / num_facets)
            # Plane height at its own local origin — allowed above AND
            # below the nominal 1.0 peak so some facets crest higher than
            # others (an uneven, broken-looking top) and some undercut low
            # enough to chip a corner down toward the envelope's own edge.
            h0 = rng.uniform(0.7, 1.2)
            # Distance (in boulder radii, from THIS facet's own origin —
            # see op_dist below) at which its height reaches zero along its
            # own descent direction. Wide range: short = a small chip near
            # its own (possibly off-center) origin, long = a broad face
            # that barely dips within the footprint, deferring to whichever
            # other facet or the envelope is lowest there.
            zero_at = rng.uniform(0.7, 2.0)
            tilt = h0 / zero_at
            op_ang = rng.uniform(0.0, 2.0 * math.pi)
            op_dist = rng.uniform(0.0, 0.5) * r0
            opx, opy = math.cos(op_ang) * op_dist, math.sin(op_ang) * op_dist
            facets.append((math.cos(base_ang), math.sin(base_ang), tilt, h0, opx, opy))
        return facets

    boulders = []
    for _ in range(count):
        cx = rng.uniform(-width_mm / 2.0, width_mm / 2.0)
        cy = rng.uniform(-depth_mm / 2.0, depth_mm / 2.0)
        diameter = rng.uniform(min(min_mm, max_mm), max(min_mm, max_mm))
        r0 = max(0.1, diameter / 2.0)
        rot = rng.uniform(0.0, 2.0 * math.pi)
        # Per-boulder height variance — identical heights read as stamped
        # copies, not rubble.
        height = rng.uniform(0.7, 1.15)

        lobes = [_make_lobe(0.0, 0.0, r0, 0.0, resolution_mm, 1.0)]
        max_extent = max(lobes[0][2], lobes[0][3])
        # 1-2 smaller "knob" lobes offset from the main center — footprint
        # irregularity (two rocks fused, not a plain ellipse). The TOP's
        # angularity now comes from the facet planes below, not from these
        # seams — see the docstring's attempt 2 for why lobe seams alone
        # weren't enough.
        for _ in range(rng.randint(1, 2)):
            ang = rng.uniform(0.0, 2.0 * math.pi)
            dist = rng.uniform(0.15, 0.45) * r0
            ou, ov = math.cos(ang) * dist, math.sin(ang) * dist
            sub_r0 = rng.uniform(0.4, 0.7) * r0
            sub_rot = rng.uniform(-0.7, 0.7)
            sub_weight = rng.uniform(0.6, 0.95)
            lobe = _make_lobe(ou, ov, sub_r0, sub_rot, resolution_mm, sub_weight)
            lobes.append(lobe)
            max_extent = max(max_extent, dist + max(lobe[2], lobe[3]))

        facets = _make_facets(r0)

        # Plateau surface roughness: amplitude as a fraction of THIS
        # boulder's own height (see `height` above); wavelength floored so
        # it can't alias (never finer than ~2.5 grid steps) and scales with
        # the boulder's own size so big rocks show bigger grain. Kept
        # modest (vs. the original 0.10-0.20) since the facets now carry
        # most of the "broken rock" read; too much grain on top would blur
        # the creases back toward smooth.
        rough_amount = rng.uniform(0.05, 0.12)
        rough_off = Vector((rng.uniform(-1000.0, 1000.0) for _ in range(3)))
        rough_wavelength = max(resolution_mm * 2.5, r0 * 0.18)
        ca, sa = math.cos(rot), math.sin(rot)
        # Cheap reject radius for the per-point loop below.
        bound = max_extent * 1.15
        boulders.append((
            cx, cy, ca, sa, lobes, facets, r0, height,
            rough_amount, rough_off, rough_wavelength, bound,
        ))

    def fn(x, y):
        peak = 0.0
        for (cx, cy, ca, sa, lobes, facets, r0, height,
             rough_amount, rough_off, rough_wavelength, bound) in boulders:
            dx, dy = x - cx, y - cy
            if dx * dx + dy * dy > bound * bound:
                continue
            u = dx * ca + dy * sa
            v = -dx * sa + dy * ca
            envelope = 0.0
            for (ou, ov, lrx, lry, lca, lsa, p_exp, shoulder_t, weight) in lobes:
                lu, lv = u - ou, v - ov
                luR = lu * lca + lv * lsa
                lvR = -lu * lsa + lv * lca
                t = math.sqrt((luR / lrx) ** 2 + (lvR / lry) ** 2)
                if t >= 1.0:
                    continue
                core = max(0.0, 1.0 - t ** p_exp)
                edge = _smoothstep(min(1.0, (1.0 - t) / shoulder_t))
                lobe_profile = core * edge * weight
                if lobe_profile > envelope:
                    envelope = lobe_profile
            if envelope <= 0.0:
                continue
            # Intersect with every facet plane (MIN, not smoothstep-blended
            # — the crease is meant to be a real edge, see the docstring).
            facet_min = math.inf
            for (fca, fsa, tilt, h0, opx, opy) in facets:
                pu, pv = u - opx, v - opy
                plane_h = h0 - (tilt / r0) * (pu * fca + pv * fsa)
                if plane_h < facet_min:
                    facet_min = plane_h
            profile = envelope if facet_min > envelope else facet_min
            if profile <= 0.0:
                continue
            rp = Vector((
                x / rough_wavelength + rough_off.x,
                y / rough_wavelength + rough_off.y,
                rough_off.z,
            ))
            # Ridged (folded) grain, not raw Perlin — see the docstring:
            # plain noise is smooth bumps, folding it creates small sharp
            # creases consistent with the facets instead of fighting them.
            ridged = 1.0 - abs(noise.noise(rp))
            rough = (ridged - 0.5) * 2.0 * rough_amount * profile
            bump = (profile + rough) * height
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


def build_layer_fns(seed, width_mm, depth_mm, layers, resolution_mm):
    """One f(x, y) -> contribution callable per ENABLED layer, in a fixed
    order — the order only affects summation float rounding, never the
    seed-derived randomness each layer draws (every layer's RNG/offset is
    salted independently, see the module docstring). `resolution_mm` lets
    layers with hard height transitions (stones) size their smoothing band
    to the grid so edges never alias into staircases."""
    fns = []
    if layers.get("noise", {}).get("enabled"):
        fns.append(_noise_layer(seed, layers["noise"]))
    if layers.get("ripples", {}).get("enabled"):
        fns.append(_ripples_layer(seed, layers["ripples"]))
    if layers.get("stones", {}).get("enabled"):
        fns.append(_stones_layer(seed, layers["stones"], resolution_mm))
    if layers.get("boulders", {}).get("enabled"):
        fns.append(_boulders_layer(seed, width_mm, depth_mm, layers["boulders"], resolution_mm))
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


def _split_quad(bm, v00, v10, v11, v01):
    """Triangulate one grid quad along whichever 3D diagonal is SHORTER,
    after displacement — not a fixed corner-to-corner convention.

    Leaving the quad as a single bmesh face and letting Blender's STL
    exporter tessellate it on export always splits the same way (fan from
    the face's first vertex, i.e. always the v00-v11 diagonal). On a flat
    plate that's invisible; on a displaced heightfield every quad slashes
    in the same fixed direction regardless of which way the surface
    actually curves, so curved features (cobble domes, dune ridges) pick up
    a uniform diagonal crease pattern that catches light as a "woven"
    faceting artifact — the fix reported by zoomed cobblestone renders.

    Comparing the two diagonals (v00-v11 vs v10-v01) and following the
    shorter one instead makes the triangulation follow curvature: the split
    lands across whichever pair of opposite corners is actually closer
    together post-displacement, which is where a curved quad wants its
    crease to fall to best approximate the surface. This is a pure function
    of (already seed-deterministic) vertex heights, so it adds no
    randomness — same seed still bakes the same mesh, triangle-for-triangle.

    Winding doesn't need to be tracked here: bmesh.ops.recalc_face_normals
    (called once over the whole mesh after building it) makes the final
    orientation authoritative regardless of how each triangle was wound.
    """
    d_ac = (v00.co - v11.co).length_squared
    d_bd = (v10.co - v01.co).length_squared
    if d_ac <= d_bd:
        bm.faces.new((v00, v10, v11))
        bm.faces.new((v00, v11, v01))
    else:
        bm.faces.new((v00, v10, v01))
        bm.faces.new((v10, v11, v01))


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

    # Top surface: two triangles per grid cell, split along the shorter of
    # the quad's two 3D diagonals (see _split_quad) — this is the surface
    # the STL exporter would otherwise auto-triangulate with a fixed
    # diagonal, which is exactly the faceting artifact this fixes. The
    # skirt and bottom cap below stay quads/n-gon: they're flat or
    # near-flat by construction (a vertical wall, a z=0 plane), so a fixed
    # split there is invisible.
    for j in range(ny - 1):
        for i in range(nx - 1):
            _split_quad(bm, top[j][i], top[j][i + 1], top[j + 1][i + 1], top[j + 1][i])

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

def _scaled_layers(layers, k):
    """Apply the plate-level feature_scale: multiply every layer's
    characteristic LENGTH by k (and divide noise's frequency-style `scale`),
    so one knob zooms the terrain itself — distinct from resolution_mm,
    which only changes mesh density. k=1 returns the dict untouched."""
    if abs(k - 1.0) < 1e-9:
        return layers
    scaled = json.loads(json.dumps(layers))  # deep copy, layers are plain JSON
    if "noise" in scaled and scaled["noise"]:
        n = scaled["noise"]
        n["scale"] = n.get("scale", 0.05) / k
    if "ripples" in scaled and scaled["ripples"]:
        r = scaled["ripples"]
        r["wavelength_mm"] = r.get("wavelength_mm", 8.0) * k
    if "stones" in scaled and scaled["stones"]:
        s = scaled["stones"]
        s["cell_mm"] = s.get("cell_mm", 4.0) * k
        s["gap_mm"] = s.get("gap_mm", 0.5) * k
    if "boulders" in scaled and scaled["boulders"]:
        b = scaled["boulders"]
        b["min_mm"] = b.get("min_mm", 8.0) * k
        b["max_mm"] = b.get("max_mm", 20.0) * k
    if "flow" in scaled and scaled["flow"]:
        f = scaled["flow"]
        f["channel_width_mm"] = f.get("channel_width_mm", 10.0) * k
    return scaled


def generate(params):
    width_mm = float(params["width_mm"])
    depth_mm = float(params["depth_mm"])
    feature_scale = min(4.0, max(0.25, float(params.get("feature_scale", 1.0))))
    requested_mm = max(
        MIN_RESOLUTION_MM, float(params.get("resolution_mm", DEFAULT_RESOLUTION_MM))
    )
    # The guard is a vertex BUDGET, not the step itself: a 0.1mm step is
    # legitimate resin detail on a small plate and a memory bomb on a huge
    # one. verts ~= (w/res)*(d/res), so the finest step that fits the
    # budget scales with sqrt(area).
    fits_budget_mm = math.sqrt((width_mm * depth_mm) / MAX_GRID_VERTS)
    resolution_mm = max(requested_mm, fits_budget_mm)
    carrier_mm = float(params.get("carrier_mm", DEFAULT_CARRIER_MM))
    relief_mm = float(params.get("relief_mm", 6.0))
    seed = int(params.get("seed", 0))
    layers = _scaled_layers(params.get("layers", {}) or {}, feature_scale)
    out = params["out"]

    nx = max(2, round(width_mm / resolution_mm) + 1)
    ny = max(2, round(depth_mm / resolution_mm) + 1)
    xs = [-width_mm / 2.0 + i * (width_mm / (nx - 1)) for i in range(nx)]
    ys = [-depth_mm / 2.0 + j * (depth_mm / (ny - 1)) for j in range(ny)]

    layer_fns = build_layer_fns(seed, width_mm, depth_mm, layers, resolution_mm)

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

    return out, dims, vert_count, manifold, resolution_mm


def main():
    argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    params_path = argv[argv.index("--params") + 1]
    with open(params_path, encoding="utf-8") as f:
        params = json.load(f)

    seed = int(params.get("seed", 0))
    tok("GENERATING", {"seed": seed})
    try:
        out, dims, verts, manifold, effective_res = generate(params)
    except Exception as e:  # noqa: BLE001 — reported as a token, not a crash
        traceback.print_exc()
        tok("GENERATION_FAILED", {"reason": str(e)})
        sys.exit(1)

    # resolution_mm is the EFFECTIVE grid step — it differs from the request
    # when the vertex budget coarsened it (see MAX_GRID_VERTS).
    tok(
        "GENERATED",
        {
            "out": out,
            "dims_mm": dims,
            "verts": verts,
            "manifold": manifold,
            "resolution_mm": round(effective_res, 3),
        },
    )


main()
