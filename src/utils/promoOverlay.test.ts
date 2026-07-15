import { describe, expect, it } from "vitest";
import { type OverlaySpec, overlayLayout } from "./promoOverlay";

const spec = (overrides: Partial<OverlaySpec> = {}): OverlaySpec => ({
  logoOn: true,
  logoPos: "tr",
  textOn: true,
  textPos: "bl",
  title: "Bog Hag",
  credit: "BESTIARUM · 2026",
  fontCss: "'Archivo', sans-serif",
  size: 20,
  ...overrides,
});

describe("overlayLayout", () => {
  it("scales the whole composition linearly with output size", () => {
    const small = overlayLayout(512, 512, spec());
    const large = overlayLayout(2048, 2048, spec());
    // 4x the pixels -> exactly 4x every metric: a draft and a final
    // render must carry the SAME composition
    expect(large.scale).toBe(small.scale * 4);
    expect(large.logo?.box).toBe(small.logo!.box * 4);
    expect(large.text?.titlePx).toBe(small.text!.titlePx * 4);
    expect(large.text?.creditPx).toBe(small.text!.creditPx * 4);
    expect(large.text?.x).toBe(small.text!.x * 4);
  });

  it("sizes the subtitle from the selected title size", () => {
    const small = overlayLayout(512, 512, spec({ size: 20 })).text!;
    const large = overlayLayout(512, 512, spec({ size: 40 })).text!;
    expect(small.creditPx).toBe(11);
    expect(large.creditPx).toBe(22);
    expect(large.creditPx).toBe(small.creditPx * 2);
  });

  it("places the logo in each corner inside the margins", () => {
    const w = 512;
    const h = 512;
    const tl = overlayLayout(w, h, spec({ logoPos: "tl" })).logo!;
    expect(tl).toMatchObject({ x: 18, y: 18 });
    const br = overlayLayout(w, h, spec({ logoPos: "br" })).logo!;
    expect(br.x).toBe(w - 18 - br.box);
    expect(br.y).toBe(h - 24 - br.box);
  });

  it("stacks title above credit and anchors bottom blocks to the bottom", () => {
    const { text } = overlayLayout(512, 512, spec({ textPos: "bl" }));
    expect(text!.align).toBe("left");
    expect(text!.titleY).not.toBeNull();
    expect(text!.creditY).toBeGreaterThan(text!.titleY!);
    // credit line's bottom edge sits exactly on the bottom margin
    expect(text!.creditY! + text!.creditPx).toBeCloseTo(512 - 24);
  });

  it("centers and right-aligns by column", () => {
    const bc = overlayLayout(512, 512, spec({ textPos: "bc" })).text!;
    expect(bc).toMatchObject({ x: 256, align: "center" });
    const tr = overlayLayout(512, 512, spec({ textPos: "tr" })).text!;
    expect(tr).toMatchObject({ x: 512 - 18, align: "right" });
    expect(tr.titleY).toBe(18);
  });

  it("never bakes placeholder text: empty lines drop out entirely", () => {
    // empty title -> credit alone hugs the bottom margin
    const creditOnly = overlayLayout(512, 512, spec({ title: "  " })).text!;
    expect(creditOnly.titleY).toBeNull();
    expect(creditOnly.creditY! + creditOnly.creditPx).toBeCloseTo(512 - 24);
    // both empty -> no text block at all, even with the toggle on
    const none = overlayLayout(512, 512, spec({ title: "", credit: "" }));
    expect(none.text).toBeNull();
    // toggle off wins over content
    const off = overlayLayout(512, 512, spec({ textOn: false }));
    expect(off.text).toBeNull();
  });

  it("omits the logo when toggled off", () => {
    expect(overlayLayout(512, 512, spec({ logoOn: false })).logo).toBeNull();
  });
});
