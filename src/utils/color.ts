/**
 * sRGB <-> linear color math shared by the render studio and the advanced
 * look schema. Blender's LOOK recipe stores light/material colors as LINEAR
 * RGB triples; the webview's <input type="color"> speaks sRGB hex — every
 * color crossing that boundary goes through here.
 */

export type LinearRgb = [number, number, number];

export const srgbToLinear = (c: number): number =>
  c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;

export const linearToSrgb = (c: number): number =>
  c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055;

/** "#rrggbb" -> linear triple. Exact (unrounded) — round at the call site. */
export const hexToLinear = (hex: string): LinearRgb => {
  const h = hex.replace("#", "");
  const r = Number.parseInt(h.slice(0, 2), 16) / 255;
  const g = Number.parseInt(h.slice(2, 4), 16) / 255;
  const b = Number.parseInt(h.slice(4, 6), 16) / 255;
  return [srgbToLinear(r), srgbToLinear(g), srgbToLinear(b)];
};

/** Linear triple -> "#rrggbb", clamped. The round-trip through 8-bit sRGB
 * loses precision, so "same color?" checks must compare hex, not linear. */
export const linearToHex = (rgb: readonly number[]): string =>
  `#${rgb
    .map((c) => {
      const s = Math.round(Math.min(1, Math.max(0, linearToSrgb(c))) * 255);
      return s.toString(16).padStart(2, "0");
    })
    .join("")}`;
