import { describe, expect, it } from "vitest";
import type { PlinthParams } from "../bindings";
import { insetShrink, shrinkKind } from "./cutFootprint";

// Rust twin: src-tauri/src/basecutter/cutters.rs (top_face_of + its unit
// tests). These constants must stay pinned to that implementation — see
// docs/BASECUTTER.md "The plinth".
const PLINTH: Pick<PlinthParams, "height_mm" | "taper_deg"> = {
  height_mm: 3.7,
  taper_deg: 15,
};

describe("cutFootprint", () => {
  it("shrinks a 32mm circle to ~30.017mm", () => {
    const shrink = insetShrink(PLINTH);
    const cut = shrinkKind({ kind: "circle", diameter_mm: 32 }, shrink);
    if (cut.kind !== "circle") throw new Error("expected a circle back");
    expect(Math.abs(cut.diameter_mm - 30.017)).toBeLessThanOrEqual(0.01);
  });

  it("shrinks a rect by 2*inset per axis", () => {
    const shrink = insetShrink(PLINTH);
    const cut = shrinkKind(
      { kind: "rect", width_mm: 25, depth_mm: 40 },
      shrink,
    );
    expect(cut).toEqual({
      kind: "rect",
      width_mm: 25 - shrink,
      depth_mm: 40 - shrink,
    });
  });

  it("shrinks an ellipse by 2*inset per axis", () => {
    const shrink = insetShrink(PLINTH);
    const cut = shrinkKind(
      { kind: "ellipse", major_mm: 90, minor_mm: 52 },
      shrink,
    );
    expect(cut).toEqual({
      kind: "ellipse",
      major_mm: 90 - shrink,
      minor_mm: 52 - shrink,
    });
  });

  it("never shrinks below zero", () => {
    const shrink = insetShrink(PLINTH);
    const cut = shrinkKind({ kind: "circle", diameter_mm: 1 }, shrink);
    expect(cut).toEqual({ kind: "circle", diameter_mm: 0 });
  });
});
