import { describe, expect, it } from "vitest";
import {
  angularDelta,
  centroidOf,
  moveDelta,
  normalizeDeg,
  reindexSelection,
  renameMember,
  rotateGroup,
} from "./placementGroups";

describe("centroidOf", () => {
  it("is the mean of member centers", () => {
    const c = centroidOf([
      { x_mm: 0, y_mm: 0, rotation_deg: 0 },
      { x_mm: 10, y_mm: 0, rotation_deg: 0 },
      { x_mm: 5, y_mm: 15, rotation_deg: 0 },
    ]);
    expect(c.x).toBeCloseTo(5);
    expect(c.y).toBeCloseTo(5);
  });

  it("returns the origin for an empty group rather than NaN", () => {
    expect(centroidOf([])).toEqual({ x: 0, y: 0 });
  });
});

describe("normalizeDeg", () => {
  it("leaves an in-range value unchanged", () => {
    expect(normalizeDeg(90)).toBe(90);
    expect(normalizeDeg(0)).toBe(0);
  });

  it("wraps values >= 360", () => {
    expect(normalizeDeg(360)).toBe(0);
    expect(normalizeDeg(370)).toBe(10);
    expect(normalizeDeg(725)).toBe(5);
  });

  it("wraps negative values into [0, 360)", () => {
    expect(normalizeDeg(-10)).toBe(350);
    expect(normalizeDeg(-370)).toBe(350);
  });
});

describe("angularDelta", () => {
  it("is a plain difference within range", () => {
    expect(angularDelta(10, 15)).toBe(5);
    expect(angularDelta(15, 10)).toBe(-5);
  });

  it("takes the short way across the 0/360 seam", () => {
    // 355 -> 0 is a +5 nudge forward, not a -355 spin backward.
    expect(angularDelta(355, 0)).toBe(5);
    // 0 -> 355 is a -5 nudge backward, not a +355 spin forward.
    expect(angularDelta(0, 355)).toBe(-5);
  });

  it("returns 0 for an unchanged angle", () => {
    expect(angularDelta(42, 42)).toBe(0);
  });

  it("stays within (-180, 180] — an exact half-turn is +180, not -180", () => {
    expect(angularDelta(0, 180)).toBe(180);
    expect(angularDelta(180, 0)).toBe(180);
  });
});

describe("rotateGroup", () => {
  it("is a no-op at delta 0", () => {
    const members = [
      { x_mm: 10, y_mm: 0, rotation_deg: 0 },
      { x_mm: -10, y_mm: 0, rotation_deg: 90 },
    ];
    const out = rotateGroup(members, 0, 0, 0);
    expect(out[0].x_mm).toBeCloseTo(10);
    expect(out[0].y_mm).toBeCloseTo(0);
    expect(out[0].rotation_deg).toBe(0);
    expect(out[1].rotation_deg).toBe(90);
  });

  it("orbits a member 90 degrees around a non-origin centroid", () => {
    // Member sits 10mm to the +X of the centroid at (5, 5); orbiting +90
    // degrees should land it 10mm to the +Y of the centroid.
    const out = rotateGroup([{ x_mm: 15, y_mm: 5, rotation_deg: 0 }], 5, 5, 90);
    expect(out[0].x_mm).toBeCloseTo(5);
    expect(out[0].y_mm).toBeCloseTo(15);
    expect(out[0].rotation_deg).toBe(90);
  });

  it("orbits negative deltas the opposite way", () => {
    const out = rotateGroup(
      [{ x_mm: 15, y_mm: 5, rotation_deg: 0 }],
      5,
      5,
      -90,
    );
    expect(out[0].x_mm).toBeCloseTo(5);
    expect(out[0].y_mm).toBeCloseTo(-5);
    expect(out[0].rotation_deg).toBe(270);
  });

  it("normalizes rotation_deg through a wraparound", () => {
    const out = rotateGroup(
      [{ x_mm: 0, y_mm: 0, rotation_deg: 350 }],
      0,
      0,
      20,
    );
    expect(out[0].rotation_deg).toBeCloseTo(10);
  });

  it("keeps a member AT the centroid stationary but still spins it", () => {
    const out = rotateGroup([{ x_mm: 5, y_mm: 5, rotation_deg: 0 }], 5, 5, 45);
    expect(out[0].x_mm).toBeCloseTo(5);
    expect(out[0].y_mm).toBeCloseTo(5);
    expect(out[0].rotation_deg).toBe(45);
  });

  it("does not mutate its input", () => {
    const members = [{ x_mm: 10, y_mm: 0, rotation_deg: 0 }];
    rotateGroup(members, 0, 0, 90);
    expect(members[0]).toEqual({ x_mm: 10, y_mm: 0, rotation_deg: 0 });
  });

  it("carries extra fields through untouched (e.g. a full Placement)", () => {
    const members = [
      {
        x_mm: 10,
        y_mm: 0,
        rotation_deg: 0,
        cutter: { kind: "circle" as const, diameter_mm: 32 },
        magnet: null,
        name: "round32-1",
      },
    ];
    const out = rotateGroup(members, 0, 0, 90);
    expect(out[0].name).toBe("round32-1");
    expect(out[0].cutter).toEqual({ kind: "circle", diameter_mm: 32 });
  });
});

describe("moveDelta", () => {
  it("is the plain dx/dy between two positions", () => {
    expect(moveDelta({ x_mm: 0, y_mm: 0 }, { x_mm: 5, y_mm: -3 })).toEqual({
      dx: 5,
      dy: -3,
    });
  });

  it("is zero for an unchanged position", () => {
    expect(moveDelta({ x_mm: 12, y_mm: 8 }, { x_mm: 12, y_mm: 8 })).toEqual({
      dx: 0,
      dy: 0,
    });
  });

  it("computed against successive pre-update positions never double-counts a drag", () => {
    // Simulates 3 pointermove events dragging a member from (0,0) to
    // (9,9) in steps of 3 — each delta must be computed against the
    // PREVIOUS event's position, not the drag's original start, or the
    // total applied to other members would overshoot.
    let pos = { x_mm: 0, y_mm: 0 };
    const steps = [
      { x_mm: 3, y_mm: 3 },
      { x_mm: 6, y_mm: 6 },
      { x_mm: 9, y_mm: 9 },
    ];
    let totalDx = 0;
    let totalDy = 0;
    for (const next of steps) {
      const { dx, dy } = moveDelta(pos, next);
      totalDx += dx;
      totalDy += dy;
      pos = next;
    }
    expect(totalDx).toBeCloseTo(9);
    expect(totalDy).toBeCloseTo(9);
  });
});

describe("renameMember", () => {
  type Group = { id: string; label: string; names: string[] };

  it("rewrites the renamed member's entry in its group's names", () => {
    const groups: Group[] = [
      { id: "g1", label: "regiment 1", names: ["round32-1", "round32-2"] },
    ];
    const out = renameMember(groups, "round32-1", "Left Flank");
    expect(out[0].names).toEqual(["Left Flank", "round32-2"]);
  });

  it("leaves other groups untouched", () => {
    const groups: Group[] = [
      { id: "g1", label: "regiment 1", names: ["round32-1", "round32-2"] },
      { id: "g2", label: "regiment 2", names: ["square25-1", "square25-2"] },
    ];
    const out = renameMember(groups, "square25-1", "Right Flank");
    expect(out[0]).toBe(groups[0]); // untouched group is the SAME reference
    expect(out[1].names).toEqual(["Right Flank", "square25-2"]);
  });

  it("is a no-op (same array reference) when the name isn't grouped", () => {
    const groups: Group[] = [
      { id: "g1", label: "regiment 1", names: ["round32-1", "round32-2"] },
    ];
    const out = renameMember(groups, "not-a-member", "whatever");
    expect(out).toBe(groups);
  });

  it("does not mutate the input group objects", () => {
    const groups: Group[] = [
      { id: "g1", label: "regiment 1", names: ["round32-1", "round32-2"] },
    ];
    renameMember(groups, "round32-1", "Left Flank");
    expect(groups[0].names).toEqual(["round32-1", "round32-2"]);
  });
});

describe("reindexSelection", () => {
  it("returns null for a null selection", () => {
    expect(reindexSelection(null, [0, 1])).toBeNull();
  });

  it("returns null when the selected index itself was removed", () => {
    expect(reindexSelection(2, [2])).toBeNull();
    expect(reindexSelection(2, [0, 2, 5])).toBeNull();
  });

  it("shifts the selection down by the count of removed indices before it", () => {
    expect(reindexSelection(5, [1, 3])).toBe(3);
    expect(reindexSelection(5, [7, 9])).toBe(5); // both after selection
  });

  it("matches the single-delete compensation deletePlacement used to do inline", () => {
    // selected === index -> null
    expect(reindexSelection(3, [3])).toBeNull();
    // selected > index -> decrement
    expect(reindexSelection(3, [1])).toBe(2);
    // selected < index -> unchanged
    expect(reindexSelection(3, [5])).toBe(3);
  });

  it("handles removedIndices in any order", () => {
    expect(reindexSelection(10, [8, 2, 5])).toBe(7);
  });
});
