/**
 * Placement generators (docs/BASECUTTER.md phase 6): "one click for a 5x2
 * regiment of one cutter, or 'N random bases of size X' auto-scattered
 * without overlap". Pure frontend, pure math — the job already takes a
 * placement list (basecutter::job::BaseCutJob), so a generator is just a
 * function that produces more `Placement`s for the view to push onto its
 * array. Neither generator assigns a `name`; BaseCutter.vue's `nextName`
 * stays the single place that mints collision-free names.
 */
import type { Cutter, CutterKind, Placement } from "../bindings";
import { footprintDims } from "./cutterKinds";

/** A placement without its name — what both generators hand back for the
 * view to name via `nextName` before pushing onto `placements`. */
export type GeneratedPlacement = Omit<Placement, "name">;

/** Axis-aligned world bounds (landscape mm coordinates) — the shape
 * `LandscapeViewport`'s `loaded` event emits, structurally. */
export type Bounds = { minX: number; maxX: number; minY: number; maxY: number };

type Vec2 = { x: number; y: number };

/**
 * Radius of the smallest circle centered on the placement's own center that
 * fully contains its NOMINAL footprint, at ANY rotation — exact, not an
 * approximation: a circle's own radius, an ellipse's semi-major axis, and a
 * rect's half-diagonal are each already the true worst-case distance from
 * center to edge (for circle/ellipse that's footprintDims' width/2; only a
 * rect's corner reaches past its axis extents). Because the radius is
 * rotation-invariant, it doubles as a cheap, ROTATION-SAFE stand-in for the
 * footprint in both the containment and the overlap tests below — cheap,
 * but CONSERVATIVE: two placements whose bounding circles overlap might
 * still be geometrically clear of each other at their actual rotations
 * (e.g. two rects meeting corner-to-corner at 45 degrees), so this can
 * reject some valid placements. It never accepts an actually-overlapping
 * pair, which is the side to err on.
 */
const boundingRadiusMm = (kind: CutterKind): number => {
  const { width, depth } = footprintDims(kind);
  return kind.kind === "rect" ? Math.hypot(width, depth) / 2 : width / 2;
};

/**
 * Grid of `rows` x `cols` placements of one cutter, centered on `center`,
 * unrotated (rotation_deg: 0 — how real unit blocks rank up: every member
 * faces the same way). `pitch = nominal_dimension + gapMm` per axis: at
 * gapMm 0 that's exactly nominal-edge-to-nominal-edge tiling for squares
 * and rects (docs/BASECUTTER.md "The plinth": "ranked square bases touch —
 * and tile flush — at their bottom edges"); rounds/ovals use the same rule
 * against their bounding width/depth (diameter, or major x minor), which
 * doesn't tile flush the way a square does but keeps the same one-formula
 * pitch math across every kind. Non-positive rows/cols yield an empty grid
 * rather than throwing — the UI clamps its inputs to >= 1, but a generator
 * shouldn't trust a caller's arithmetic.
 */
export const regimentPlacements = (
  cutter: Cutter,
  rows: number,
  cols: number,
  gapMm: number,
  center: Vec2,
): GeneratedPlacement[] => {
  const nRows = Math.max(0, Math.trunc(rows));
  const nCols = Math.max(0, Math.trunc(cols));
  if (nRows === 0 || nCols === 0) return [];

  const { width, depth } = footprintDims(cutter.kind);
  const pitchX = width + gapMm;
  const pitchY = depth + gapMm;

  const placements: GeneratedPlacement[] = [];
  for (let r = 0; r < nRows; r++) {
    const y = center.y + (r - (nRows - 1) / 2) * pitchY;
    for (let c = 0; c < nCols; c++) {
      const x = center.x + (c - (nCols - 1) / 2) * pitchX;
      placements.push({
        cutter: cutter.kind,
        x_mm: x,
        y_mm: y,
        rotation_deg: 0,
        magnet: null,
      });
    }
  }
  return placements;
};

/**
 * The world-space bounding box a regiment would occupy — same pitch math
 * as `regimentPlacements`, without building the placement list. Lets the
 * view warn "regiment extends past the landscape" before committing to it
 * (docs/BASECUTTER.md phase 6 doesn't demand the regiment be rejected —
 * cuts outside the sculpt simply fail per-cut with a reason, that's the
 * pipeline's job — this is purely an early, inline heads-up).
 */
export const regimentExtent = (
  cutter: Cutter,
  rows: number,
  cols: number,
  gapMm: number,
  center: Vec2,
): Bounds => {
  const nRows = Math.max(0, Math.trunc(rows));
  const nCols = Math.max(0, Math.trunc(cols));
  const { width, depth } = footprintDims(cutter.kind);
  const halfW = nCols > 0 ? ((nCols - 1) * (width + gapMm) + width) / 2 : 0;
  const halfH = nRows > 0 ? ((nRows - 1) * (depth + gapMm) + depth) / 2 : 0;
  return {
    minX: center.x - halfW,
    maxX: center.x + halfW,
    minY: center.y - halfH,
    maxY: center.y + halfH,
  };
};

/** Deterministic seeded PRNG (mulberry32): same seed -> same sequence,
 * forever — what makes `scatterPlacements` unit-testable without mocking
 * `Math.random`, and what a future "reroll" button could reuse the way
 * `genParams.seed` already works for landscape generation. */
export const mulberry32 = (seed: number): (() => number) => {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
};

// Attempts are budgeted per requested placement, not per success: once a
// landscape is nearly full, later draws fail far more often than early
// ones, so a flat total budget (rather than "N successes or bust") is what
// lets the scatter give up gracefully — returning fewer placements — even
// mid-way through a crowded run, instead of spinning forever or landing an
// unbounded loop.
const MAX_ATTEMPTS_PER_PLACEMENT = 200;

/**
 * Up to `count` non-overlapping placements of one cutter, scattered inside
 * `bounds`, keeping the whole NOMINAL footprint inside the bounds and clear
 * of both `existing` placements and the ones generated in this same call.
 * Rejection sampling: draw a random point + (for non-circles) a random
 * rotation, accept it if the conservative bounding-circle test (see
 * `boundingRadiusMm`) clears every other placement, else redraw. Returns
 * FEWER than `count` rather than looping forever once the area is too
 * full to fit any more — the caller (BaseCutter.vue) surfaces that as "N
 * of M placed", never a hard error.
 */
export const scatterPlacements = (
  cutter: Cutter,
  count: number,
  bounds: Bounds,
  existing: Pick<Placement, "cutter" | "x_mm" | "y_mm">[],
  rng: () => number,
): GeneratedPlacement[] => {
  const n = Math.max(0, Math.trunc(count));
  if (n === 0) return [];

  const radius = boundingRadiusMm(cutter.kind);
  const minX = bounds.minX + radius;
  const maxX = bounds.maxX - radius;
  const minY = bounds.minY + radius;
  const maxY = bounds.maxY - radius;
  if (minX > maxX || minY > maxY) return []; // doesn't fit even once

  const obstacles = existing.map((p) => ({
    x: p.x_mm,
    y: p.y_mm,
    r: boundingRadiusMm(p.cutter),
  }));
  const placed: GeneratedPlacement[] = [];

  const clearsAll = (x: number, y: number) =>
    obstacles.every((o) => Math.hypot(x - o.x, y - o.y) >= radius + o.r) &&
    placed.every((p) => Math.hypot(x - p.x_mm, y - p.y_mm) >= radius + radius);

  let attempts = 0;
  const maxAttempts = n * MAX_ATTEMPTS_PER_PLACEMENT;
  while (placed.length < n && attempts < maxAttempts) {
    attempts++;
    const x = minX + rng() * (maxX - minX);
    const y = minY + rng() * (maxY - minY);
    if (!clearsAll(x, y)) continue;
    const rotation_deg = cutter.kind.kind === "circle" ? 0 : rng() * 360;
    placed.push({
      cutter: cutter.kind,
      x_mm: x,
      y_mm: y,
      rotation_deg,
      magnet: null,
    });
  }
  return placed;
};
