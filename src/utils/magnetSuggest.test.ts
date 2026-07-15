import { describe, expect, it } from "vitest";
import type { CutterKind, MagnetSpec, PlinthParams } from "../bindings";
import { insetShrink, shrinkKind } from "./cutFootprint";
import {
  bossFits,
  bossOuterDiameterMm,
  bossPositionsFit,
  magnetPositionsMm,
  suggestedMagnetCount,
  suggestMagnet,
} from "./magnetSuggest";

// Rust/Python twin: src-tauri/resources/base_cut.py:328
//   r_boss = magnet["diameter_mm"] / 2.0 + clearance + wall
// (doubled here for a diameter). No separate boss-wall constant exists in
// the script — the boss reuses the plinth's own wall_mm — so this mirror
// must stay pinned to PlinthParams.wall_mm, not a hardcoded number. If the
// script ever grows a distinct boss-wall knob, this test should fail loud.
const PLINTH: PlinthParams = {
  height_mm: 3.7,
  taper_deg: 15,
  hollow: true,
  wall_mm: 1.2,
  top_mm: 1.2,
  magnet_clearance_mm: 0.15,
};

const cutOf = (nominal: CutterKind): CutterKind =>
  shrinkKind(nominal, insetShrink(PLINTH));

describe("bossOuterDiameterMm", () => {
  it("matches base_cut.py:328's r_boss * 2 for a 6x2 magnet", () => {
    // r_boss = 6/2 + 0.15 + 1.2 = 4.35 -> diameter 8.7
    const magnet: Pick<MagnetSpec, "diameter_mm"> = { diameter_mm: 6 };
    expect(bossOuterDiameterMm(magnet, PLINTH)).toBeCloseTo(8.7, 9);
  });
});

describe("magnetPositionsMm", () => {
  it("returns a single centered position for count 1", () => {
    const round32: CutterKind = { kind: "circle", diameter_mm: 32 };
    expect(magnetPositionsMm(round32, 1)).toEqual([{ x: 0, y: 0 }]);
  });

  it("spaces 2 positions along an ellipse's major axis at long/(count+1)", () => {
    // base_cut.py's _magnet_positions: spacing = long_dim / (count + 1),
    // positions symmetric around the origin.
    const oval: CutterKind = { kind: "ellipse", major_mm: 120, minor_mm: 92 };
    const positions = magnetPositionsMm(oval, 2);
    const spacing = 120 / 3; // = 40
    expect(positions).toEqual([
      { x: -0.5 * spacing, y: 0 },
      { x: 0.5 * spacing, y: 0 },
    ]);
  });

  it("spaces positions along a rect's longer side", () => {
    // 50x100 "chariot": depth (100) is the long axis, so offsets land on y.
    const rect: CutterKind = { kind: "rect", width_mm: 50, depth_mm: 100 };
    const positions = magnetPositionsMm(rect, 3);
    // spacing = 100 / (3 + 1) = 25
    expect(positions).toEqual([
      { x: 0, y: -25 },
      { x: 0, y: 0 },
      { x: 0, y: 25 },
    ]);
  });

  it("clamps count to MAX_MAGNET_COUNT (4)", () => {
    const rect: CutterKind = { kind: "rect", width_mm: 50, depth_mm: 200 };
    expect(magnetPositionsMm(rect, 9)).toHaveLength(4);
  });
});

describe("bossPositionsFit", () => {
  it("fits a single small boss on a 32mm round", () => {
    const round32: CutterKind = { kind: "circle", diameter_mm: 32 };
    const cut = cutOf(round32);
    expect(bossPositionsFit(round32, cut, 1, 8.7)).toBe(true);
  });

  it("rejects an oversized single boss on a 20mm square", () => {
    const square20: CutterKind = { kind: "rect", width_mm: 20, depth_mm: 20 };
    const cut = cutOf(square20);
    // The cut footprint is ~18.02mm square; a 20mm boss diameter can't
    // clear it, a 17.7mm one (barely) can.
    expect(bossPositionsFit(square20, cut, 1, 20)).toBe(false);
    expect(bossPositionsFit(square20, cut, 1, 17.7)).toBe(true);
  });

  it("rejects a 2-boss layout whose end positions poke past a narrow ellipse's minor axis", () => {
    // A long, narrow ellipse: spacing pushes the bosses toward the ends,
    // where the minor axis (10mm) can't clear even a small boss.
    const thin: CutterKind = { kind: "ellipse", major_mm: 100, minor_mm: 10 };
    const cut = cutOf(thin);
    expect(bossPositionsFit(thin, cut, 2, 8.7)).toBe(false);
  });

  it("accepts a 2-boss layout that clears a wide oval", () => {
    const oval: CutterKind = { kind: "ellipse", major_mm: 120, minor_mm: 92 };
    const cut = cutOf(oval);
    expect(bossPositionsFit(oval, cut, 2, 8.7)).toBe(true);
  });
});

describe("bossFits", () => {
  it("defaults to a single centered boss", () => {
    const round32: CutterKind = { kind: "circle", diameter_mm: 32 };
    expect(bossFits({ diameter_mm: 5 }, round32, PLINTH)).toBe(true);
  });

  it("checks the requested count", () => {
    const thin: CutterKind = { kind: "ellipse", major_mm: 100, minor_mm: 10 };
    expect(bossFits({ diameter_mm: 5 }, thin, PLINTH, 2)).toBe(false);
    expect(bossFits({ diameter_mm: 5 }, thin, PLINTH, 1)).toBe(true);
  });
});

describe("suggestedMagnetCount", () => {
  // Thresholds pinned against docs/BASECUTTER.md's seed library, computed
  // on the CUT (taper-shrunk) long dimension — see cutFootprint's
  // insetShrink (shrink ~= 1.983mm at the default plinth):
  //   round-25 (cut ~23.0mm)      -> 1  (small round)
  //   square-25 (cut ~23.0mm)     -> 1  (small square)
  //   oval-60x35 (cut ~58.0mm)    -> 2  (the plan's own example)
  //   rect-50x100 "chariot" (cut ~98.0mm) -> 3
  //   oval-170x105 (cut ~168.0mm) -> 4  (capped at MAX_MAGNET_COUNT)
  it("small rounds/squares suggest 1", () => {
    expect(suggestedMagnetCount(23.0)).toBe(1);
  });

  it("a 60x35 oval suggests 2", () => {
    const oval: CutterKind = { kind: "ellipse", major_mm: 60, minor_mm: 35 };
    const cut = cutOf(oval);
    expect(suggestedMagnetCount((cut as { major_mm: number }).major_mm)).toBe(
      2,
    );
  });

  it("a 50x100 rect suggests 3", () => {
    const rect: CutterKind = { kind: "rect", width_mm: 50, depth_mm: 100 };
    const cut = cutOf(rect);
    expect(suggestedMagnetCount((cut as { depth_mm: number }).depth_mm)).toBe(
      3,
    );
  });

  it("a 170x105 oval suggests the cap of 4", () => {
    const oval: CutterKind = { kind: "ellipse", major_mm: 170, minor_mm: 105 };
    const cut = cutOf(oval);
    expect(suggestedMagnetCount((cut as { major_mm: number }).major_mm)).toBe(
      4,
    );
  });
});

describe("suggestMagnet", () => {
  const inventory: MagnetSpec[] = [
    { diameter_mm: 5, height_mm: 1, count: 1 },
    { diameter_mm: 5, height_mm: 2, count: 1 },
    { diameter_mm: 6, height_mm: 2, count: 1 },
    { diameter_mm: 8, height_mm: 3, count: 1 },
    { diameter_mm: 10, height_mm: 2, count: 1 },
  ];

  it("picks the largest-diameter magnet, count 1, for a 32mm round", () => {
    const round32: CutterKind = { kind: "circle", diameter_mm: 32 };
    const suggestion = suggestMagnet(round32, PLINTH, inventory);
    expect(suggestion).toEqual({
      spec: { diameter_mm: 10, height_mm: 2, count: 1 },
      count: 1,
    });
  });

  it("suggests count 2 for a 60x35 oval, still the largest magnet that clears both bosses", () => {
    const oval: CutterKind = { kind: "ellipse", major_mm: 60, minor_mm: 35 };
    const suggestion = suggestMagnet(oval, PLINTH, inventory);
    // spec is the inventory row untouched (count stays the inventory
    // invariant 1); the suggested POCKET count only ever rides in the
    // sibling field — see suggestMagnet's return comment.
    expect(suggestion).toEqual({
      spec: { diameter_mm: 10, height_mm: 2, count: 1 },
      count: 2,
    });
  });

  it("steps the count down when the target count's boss positions don't fit", () => {
    // 170x10 ellipse: cut major ~168mm calls for the count-4 cap by the
    // long-dimension rule, but the minor axis (cut ~8mm) is too narrow for
    // any off-center boss to clear the curved ellipse boundary — counts
    // 4, 3, and 2 all fail, leaving only a single centered pocket, and
    // only the smallest-diameter magnets are narrow enough to fit even
    // that (see magnetSuggest.test.ts's inventory-fit exploration).
    const thin: CutterKind = { kind: "ellipse", major_mm: 170, minor_mm: 10 };
    const suggestion = suggestMagnet(thin, PLINTH, inventory);
    expect(suggestion).toEqual({
      spec: { diameter_mm: 5, height_mm: 2, count: 1 },
      count: 1,
    });
  });

  it("returns null when nothing in the inventory fits even a single pocket", () => {
    const round32: CutterKind = { kind: "circle", diameter_mm: 32 };
    const suggestion = suggestMagnet(round32, PLINTH, [
      { diameter_mm: 40, height_mm: 5, count: 1 },
    ]);
    expect(suggestion).toBeNull();
  });

  it("returns null for an empty inventory", () => {
    const round32: CutterKind = { kind: "circle", diameter_mm: 32 };
    expect(suggestMagnet(round32, PLINTH, [])).toBeNull();
  });
});
