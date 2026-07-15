import { describe, expect, it } from "vitest";
import {
  mintNames,
  placementNamePrefix,
  validatePlacementName,
  validatePlacementNamePrefix,
} from "./placementName";

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

describe("validatePlacementNamePrefix", () => {
  it("accepts an empty draft — the feature is off, not invalid", () => {
    expect(validatePlacementNamePrefix("")).toBeNull();
    expect(validatePlacementNamePrefix("   ")).toBeNull();
  });

  it("accepts a normal prefix", () => {
    expect(validatePlacementNamePrefix("donkey")).toBeNull();
  });

  it.each([...'<>:"/\\|?*'])(
    "rejects the Windows-forbidden character %s",
    (ch) => {
      expect(validatePlacementNamePrefix(`donkey${ch}`)).toMatch(
        /Can't contain/,
      );
    },
  );

  it("rejects a trailing dot", () => {
    expect(validatePlacementNamePrefix("donkey.")).toMatch(/dot/);
  });

  it("never checks uniqueness — a prefix is shared by a whole batch on purpose", () => {
    // No otherNames parameter at all: this is the behavioral proof that the
    // uniqueness rule from validatePlacementName doesn't apply here.
    expect(validatePlacementNamePrefix("donkey")).toBeNull();
  });
});

describe("placementNamePrefix", () => {
  it("uses the cutter-id scheme when no user prefix is set", () => {
    expect(placementNamePrefix("", "28.5", "round-28.5")).toBe("round28.5-");
  });

  it("uses the prefix+size scheme when a user prefix is set", () => {
    expect(placementNamePrefix("donkey", "28.5", "round-28.5")).toBe(
      "donkey-28.5mm-",
    );
  });

  it("swaps × for x in a multi-dimension size label (filesystem-safe)", () => {
    expect(placementNamePrefix("donkey", "60×35", "rect-60x35")).toBe(
      "donkey-60x35mm-",
    );
  });
});

describe("mintNames", () => {
  it("mints 1..count when nothing existing matches the prefix", () => {
    expect(mintNames("round285-", [], 3)).toEqual([
      "round285-1",
      "round285-2",
      "round285-3",
    ]);
  });

  it("continues 1-past the highest existing suffix under that exact prefix", () => {
    expect(mintNames("round285-", ["round285-1", "round285-2"], 2)).toEqual([
      "round285-3",
      "round285-4",
    ]);
  });

  it("never reuses a suffix still in use, even after a middle deletion", () => {
    // {1,2,3} with "2" deleted: a naive survivor-count would mint "2" again,
    // colliding with the still-live "round285-3".
    expect(mintNames("round285-", ["round285-1", "round285-3"], 1)).toEqual([
      "round285-4",
    ]);
  });

  it("ignores names under a different prefix", () => {
    expect(
      mintNames("donkey-28.5mm-", ["round285-1", "round285-2"], 1),
    ).toEqual(["donkey-28.5mm-1"]);
  });

  it("ignores null/undefined entries in existingNames", () => {
    expect(mintNames("round285-", [null, undefined], 1)).toEqual([
      "round285-1",
    ]);
  });
});
