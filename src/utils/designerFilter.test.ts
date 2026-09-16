import { describe, expect, it } from "vitest";

import {
  designerFilterLabel,
  isUnidentifiedFilter,
  UNIDENTIFIED_DESIGNER_FILTER,
} from "./designerFilter";

describe("designer filter", () => {
  it("spells the wire value the way the backend does", () => {
    // Rust's catalog::db::UNIDENTIFIED_DESIGNER_FILTER, verbatim. If one
    // side is renamed without the other the filter silently matches
    // nothing, so the literal is asserted rather than trusted.
    expect(UNIDENTIFIED_DESIGNER_FILTER).toBe(
      "__plinth_unidentified_designer__",
    );
  });

  it("recognizes the unidentified facet and nothing else", () => {
    expect(isUnidentifiedFilter(UNIDENTIFIED_DESIGNER_FILTER)).toBe(true);
    expect(isUnidentifiedFilter("DTL")).toBe(false);
    expect(isUnidentifiedFilter("")).toBe(false);
    expect(isUnidentifiedFilter(null)).toBe(false);
  });

  it("never shows the sentinel to the user", () => {
    expect(designerFilterLabel(UNIDENTIFIED_DESIGNER_FILTER)).toBe(
      "Unidentified",
    );
    expect(designerFilterLabel("DTL")).toBe("DTL");
  });
});
