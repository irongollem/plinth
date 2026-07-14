/**
 * Nominal -> cut footprint derivation — the frontend's TS mirror of Rust's
 * `top_face_of` (src-tauri/src/basecutter/cutters.rs). See
 * docs/BASECUTTER.md "The plinth": `cut = nominal - 2 * inset`, where
 * `inset = height * tan(taper)`. The two implementations must never
 * disagree, so both the viewport overlay and its pinning test
 * (cutFootprint.test.ts) import this one module instead of each keeping
 * their own copy.
 */
import type { CutterKind, PlinthParams } from "../bindings";

/** Per-axis shrink (mm) applied to a nominal footprint to get the cut
 * footprint: `2 * height_mm * tan(taper_deg)`. */
export const insetShrink = (
  plinth: Pick<PlinthParams, "height_mm" | "taper_deg">,
): number => {
  const inset = plinth.height_mm * Math.tan((plinth.taper_deg * Math.PI) / 180);
  return 2 * inset;
};

/** Shrink a nominal cutter kind by `shrink` mm per axis (clamped at 0). */
export const shrinkKind = (kind: CutterKind, shrink: number): CutterKind => {
  switch (kind.kind) {
    case "circle":
      return {
        kind: "circle",
        diameter_mm: Math.max(0, kind.diameter_mm - shrink),
      };
    case "ellipse":
      return {
        kind: "ellipse",
        major_mm: Math.max(0, kind.major_mm - shrink),
        minor_mm: Math.max(0, kind.minor_mm - shrink),
      };
    case "rect":
      return {
        kind: "rect",
        width_mm: Math.max(0, kind.width_mm - shrink),
        depth_mm: Math.max(0, kind.depth_mm - shrink),
      };
  }
};
