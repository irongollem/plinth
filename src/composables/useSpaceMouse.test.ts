import { describe, expect, it } from "vitest";
import { cleanAxis, isSpaceMouse } from "./useSpaceMouse";

describe("isSpaceMouse", () => {
  it("matches 3Dconnexion devices by name and vendor id", () => {
    for (const id of [
      "SpaceMouse Compact (Vendor: 256f Product: c635)",
      "3Dconnexion SpaceNavigator",
      "Space Pilot Pro",
      "some pad (vendor: 046d product: c62b)",
    ]) {
      expect(isSpaceMouse({ id })).toBe(true);
    }
  });

  it("ignores ordinary game controllers", () => {
    for (const id of [
      "Xbox Wireless Controller (Vendor: 045e Product: 02fd)",
      "Sony DualSense (Vendor: 054c Product: 0ce6)",
      "",
    ]) {
      expect(isSpaceMouse({ id })).toBe(false);
    }
  });
});

describe("cleanAxis", () => {
  it("zeroes resting noise inside the deadzone", () => {
    expect(cleanAxis(0)).toBe(0);
    expect(cleanAxis(0.05)).toBe(0);
    expect(cleanAxis(-0.05)).toBe(0);
  });

  it("preserves sign and ramps from the deadzone edge, not from zero", () => {
    expect(cleanAxis(1)).toBeCloseTo(1);
    expect(cleanAxis(-1)).toBeCloseTo(-1);
    // Just past the 0.06 deadzone the output is near 0, not a jump
    expect(cleanAxis(0.07)).toBeGreaterThan(0);
    expect(cleanAxis(0.07)).toBeLessThan(0.02);
  });
});
