import { describe, expect, it } from "vitest";
import { validatePlacementName } from "./placementName";

describe("validatePlacementName", () => {
  it("accepts a normal name with no collisions", () => {
    expect(
      validatePlacementName("Left Arm", ["Right Arm", "Torso"]),
    ).toBeNull();
  });

  it("rejects an empty name", () => {
    expect(validatePlacementName("", [])).toMatch(/empty/);
    expect(validatePlacementName("   ", [])).toMatch(/empty/);
  });

  it.each([...'<>:"/\\|?*'])(
    "rejects the Windows-forbidden character %s",
    (ch) => {
      expect(validatePlacementName(`round32${ch}`, [])).toMatch(
        /Can't contain/,
      );
    },
  );

  it("rejects control characters", () => {
    expect(validatePlacementName("round32", [])).toMatch(/Can't contain/);
  });

  it("rejects a trailing dot", () => {
    expect(validatePlacementName("round32.", [])).toMatch(/dot/);
  });

  it("trims surrounding whitespace before checking anything else", () => {
    expect(validatePlacementName("  round32  ", [])).toBeNull();
  });

  it("rejects a case-insensitive duplicate against another placement", () => {
    expect(validatePlacementName("Round32", ["round32"])).toMatch(
      /already used/,
    );
    expect(validatePlacementName("ROUND32", ["round32"])).toMatch(
      /already used/,
    );
  });

  it("allows a name equal to its own current value (not compared against itself)", () => {
    // Callers pass otherNames = every OTHER placement's name, never this
    // one's own — so re-committing an unchanged name is always valid.
    expect(validatePlacementName("round32", ["square25"])).toBeNull();
  });

  it("ignores null/undefined entries in otherNames (unnamed placements)", () => {
    expect(validatePlacementName("round32", [null, undefined])).toBeNull();
  });

  it("is case- and whitespace-tolerant on the OTHER side of the comparison too", () => {
    expect(validatePlacementName("round32", [" Round32 "])).toMatch(
      /already used/,
    );
  });
});
