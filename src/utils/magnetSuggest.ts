/**
 * Magnet suggestion rule (docs/BASECUTTER.md "Hollow, with magnet mounts":
 * "the tool suggests the largest inventory magnet whose boss fits the
 * base's top face, the user can override per placement"). Frontend-only —
 * the backend never picks a magnet (or a count) for a placement, it only
 * carries whatever the user (or this suggestion) put there.
 *
 * Bigger bases want more than one magnet: base_cut.py's build_plinth
 * spaces `count` boss/pocket pairs along the placement's long axis
 * (_magnet_positions, resources/base_cut.py) instead of one centered
 * pocket. This module mirrors that positioning so the fit check — and the
 * suggested count itself — matches what the script will actually carve.
 */
import type { CutterKind, MagnetSpec, PlinthParams } from "../bindings";
import { insetShrink, shrinkKind } from "./cutFootprint";
import { footprintDims } from "./cutterKinds";

type Vec2 = { x: number; y: number };

/** Mirrors base_cut.py's MAX_MAGNET_COUNT — past this it stops being a
 * plausible mounting pattern for any base in the seed library. */
export const MAX_MAGNET_COUNT = 4;

/**
 * Boss outer diameter, mirroring base_cut.py's build_plinth boss loop
 * (resources/base_cut.py:328, `r_boss = magnet["diameter_mm"] / 2.0 +
 * clearance + wall`, doubled for a diameter): pocket = magnet diameter +
 * 2*clearance, boss adds `wall` around the pocket on every side. There is
 * no separate "boss wall" constant in the script — it reuses the plinth's
 * own cavity wall thickness (`plinth.wall_mm`), the same value the hollow
 * shell's walls are built from, so this mirror takes the same field.
 */
export const bossOuterDiameterMm = (
  magnet: Pick<MagnetSpec, "diameter_mm">,
  plinth: Pick<PlinthParams, "wall_mm" | "magnet_clearance_mm">,
): number =>
  magnet.diameter_mm + 2 * plinth.magnet_clearance_mm + 2 * plinth.wall_mm;

/**
 * (unit direction, length_mm) of a footprint's longest axis — TS mirror of
 * base_cut.py's long_axis(), derived from footprintDims' width/depth:
 * circles (width === depth) and ellipses (width is the major axis) both
 * take +X — the same arbitrary-but-harmless default the script uses for
 * circles — and a deep rect flips to +Y.
 */
const longAxis = (kind: CutterKind): { direction: Vec2; lengthMm: number } => {
  const { width, depth } = footprintDims(kind);
  return width >= depth
    ? { direction: { x: 1, y: 0 }, lengthMm: width }
    : { direction: { x: 0, y: 1 }, lengthMm: depth };
};

/**
 * x/y offsets (mm) for `count` boss/pocket pairs, mirroring base_cut.py's
 * _magnet_positions verbatim: count 1 is a single centered pocket; count
 * >= 2 spaces `count` pairs along the NOMINAL footprint's long axis at
 * `spacing = long_dim / (count + 1)`, symmetric around the origin. Uses
 * the nominal footprint (not the derived cut footprint) for the spacing
 * itself — same as the script — because the offsets apply uniformly at
 * every height of the tapered body; whether a given offset actually
 * clears the (smaller) cut footprint is a separate fit question, answered
 * by `bossPositionsFit` below.
 */
export const magnetPositionsMm = (
  nominal: CutterKind,
  count: number,
): Vec2[] => {
  const n = Math.max(1, Math.min(Math.trunc(count), MAX_MAGNET_COUNT));
  if (n === 1) return [{ x: 0, y: 0 }];
  const { direction, lengthMm } = longAxis(nominal);
  const spacing = lengthMm / (n + 1);
  return Array.from({ length: n }, (_, i) => {
    const offset = (i - (n - 1) / 2) * spacing;
    // `+ 0` folds a stray -0 (direction.{x,y} is 0 or 1, times a negative
    // offset) back to 0 — cosmetic, but keeps positions diffable/testable.
    return { x: direction.x * offset + 0, y: direction.y * offset + 0 };
  });
};

/** Does a circle of `radiusMm` centered at `point` stay fully inside the
 * CUT footprint's shape? Circle and rect have exact closed forms; the
 * ellipse case samples points around the boss's own perimeter and checks
 * each against the ellipse equation — mirrors the script's own habit of
 * discretizing curves (CIRCLE_SEGMENTS) rather than deriving a closed-form
 * offset-ellipse curve, and is exact whenever the boss center sits on an
 * axis (always true here — see magnetPositionsMm) since the sampled
 * extremes then include the true worst-case point. */
const pointFitsInCutFootprint = (
  cut: CutterKind,
  point: Vec2,
  radiusMm: number,
): boolean => {
  switch (cut.kind) {
    case "circle":
      return Math.hypot(point.x, point.y) + radiusMm <= cut.diameter_mm / 2;
    case "rect":
      return (
        Math.abs(point.x) + radiusMm <= cut.width_mm / 2 &&
        Math.abs(point.y) + radiusMm <= cut.depth_mm / 2
      );
    case "ellipse": {
      const a = cut.major_mm / 2;
      const b = cut.minor_mm / 2;
      const SAMPLES = 32;
      for (let i = 0; i < SAMPLES; i++) {
        const theta = (i / SAMPLES) * 2 * Math.PI;
        const x = point.x + radiusMm * Math.cos(theta);
        const y = point.y + radiusMm * Math.sin(theta);
        if ((x / a) ** 2 + (y / b) ** 2 > 1) return false;
      }
      return true;
    }
  }
};

/** Do all `count` boss positions (spaced per magnetPositionsMm, against
 * the NOMINAL footprint) fit inside the placement's derived CUT footprint?
 * The cut footprint is used for the fit test — not the nominal — because
 * the top face is the tightest cross-section the boss has to clear (walls
 * taper inward going up; see docs/BASECUTTER.md "The plinth"). */
export const bossPositionsFit = (
  nominal: CutterKind,
  cut: CutterKind,
  count: number,
  bossDiameterMm: number,
): boolean => {
  const radius = bossDiameterMm / 2;
  return magnetPositionsMm(nominal, count).every((p) =>
    pointFitsInCutFootprint(cut, p, radius),
  );
};

/** Does this magnet's boss(es) fit the placement, at the given count
 * (default 1, a single centered pocket)? */
export const bossFits = (
  magnet: Pick<MagnetSpec, "diameter_mm">,
  nominal: CutterKind,
  plinth: PlinthParams,
  count = 1,
): boolean => {
  const cut = shrinkKind(nominal, insetShrink(plinth));
  return bossPositionsFit(
    nominal,
    cut,
    count,
    bossOuterDiameterMm(magnet, plinth),
  );
};

/**
 * Suggested magnet count from the CUT footprint's long dimension — a rule
 * over geometry, not a per-base-size lookup table (docs/BASECUTTER.md: "no
 * hardcoded base->magnet table anywhere"). Thresholds are pinned in
 * magnetSuggest.test.ts against the seed library: small rounds/squares
 * land at 1, a 60x35 oval and similar mid-size bases at 2, big ovals/rects
 * (105x70, 50x100) at 3, and the largest (170x105, 160mm round) at 4 —
 * capped at MAX_MAGNET_COUNT to match the script.
 */
export const suggestedMagnetCount = (cutLongDimensionMm: number): number => {
  if (cutLongDimensionMm >= 140) return 4;
  if (cutLongDimensionMm >= 90) return 3;
  if (cutLongDimensionMm >= 55) return 2;
  return 1;
};

/**
 * The suggestion rule: start from the count the CUT footprint's long
 * dimension calls for (suggestedMagnetCount), and — since a long-but-narrow
 * base can call for more bosses than its short axis can actually clear —
 * step the count down until at least one inventory magnet's boss fits at
 * every position, then take the largest (by diameter, then height) magnet
 * that fits at that count. Returns null only when nothing fits even a
 * single centered pocket, or the inventory is empty. The caller must never
 * auto-apply this, only badge it (docs/BASECUTTER.md: "the user can
 * override per placement").
 */
export const suggestMagnet = (
  nominal: CutterKind,
  plinth: PlinthParams,
  inventory: MagnetSpec[],
): { spec: MagnetSpec; count: number } | null => {
  const cut = shrinkKind(nominal, insetShrink(plinth));
  const targetCount = suggestedMagnetCount(longAxis(cut).lengthMm);

  for (let count = targetCount; count >= 1; count--) {
    const fitting = inventory.filter((m) =>
      bossFits(m, nominal, plinth, count),
    );
    if (!fitting.length) continue;
    const largest = fitting.reduce((best, m) =>
      m.diameter_mm > best.diameter_mm ||
      (m.diameter_mm === best.diameter_mm && m.height_mm > best.height_mm)
        ? m
        : best,
    );
    // `spec` is the inventory row untouched — its `count` is the inventory
    // invariant 1, NOT the suggested pocket count. The pocket count lives
    // only in the sibling `count` field; a caller applying the suggestion
    // must set both explicitly (see BaseCutter's applySuggestedMagnet),
    // which keeps "inventory row" and "per-placement pocket spec" from
    // silently blurring through the shared MagnetSpec shape.
    return { spec: largest, count };
  }
  return null;
};
