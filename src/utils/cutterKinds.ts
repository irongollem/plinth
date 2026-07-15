/**
 * The one place that reads CutterKind's per-kind dimension fields. Every
 * other module derives from `footprintDims` instead of switching over the
 * union itself — before this existed, six near-identical switches lived
 * across placementGenerators / magnetSuggest / cutFootprint /
 * LandscapeViewport, and a new kind (or a renamed field) had to be threaded
 * through all of them by hand, failing silently in whichever one was
 * missed. (cutFootprint.shrinkKind keeps its own switch: it RECONSTRUCTS a
 * tagged CutterKind rather than reading dims, which no {width, depth}
 * helper can express.)
 */
import type { Cutter, CutterKind } from "../bindings";

/**
 * A footprint's local-axis bounding dimensions (mm), unrotated — the
 * width/depth convention the viewport and generators share: circles are
 * diameter x diameter, ellipses are major (x) x minor (y), rects are
 * width x depth verbatim.
 */
export const footprintDims = (
  kind: CutterKind,
): { width: number; depth: number } => {
  switch (kind.kind) {
    case "circle":
      return { width: kind.diameter_mm, depth: kind.diameter_mm };
    case "ellipse":
      return { width: kind.major_mm, depth: kind.minor_mm };
    case "rect":
      return { width: kind.width_mm, depth: kind.depth_mm };
  }
};

/** The cutter library split by shape family — what both the Base Cutter
 * palette and Render's "stand on base" picker group their options by. */
export const groupCutters = (
  library: Cutter[],
): { rounds: Cutter[]; ovals: Cutter[]; rects: Cutter[] } => ({
  rounds: library.filter((c) => c.kind.kind === "circle"),
  ovals: library.filter((c) => c.kind.kind === "ellipse"),
  rects: library.filter((c) => c.kind.kind === "rect"),
});
