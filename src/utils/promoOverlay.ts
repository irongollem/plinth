/* Branding bake: draws the studio's logo/text overlay onto the finished
   render. The webview does the compositing (not Rust) on purpose — the
   preview already renders these exact fonts with the browser's text
   engine, so reusing it is the only way the baked image is guaranteed to
   match what the user positioned. Rust-side rasterizing would mean
   bundling four font families and re-implementing kerning/tracking to
   chase the same result. */

export type CornerPos = "tl" | "tr" | "bl" | "br";
export type TextPos = "tl" | "tc" | "tr" | "bl" | "bc" | "br";

export type OverlaySpec = {
  logoOn: boolean;
  logoPos: CornerPos;
  textOn: boolean;
  textPos: TextPos;
  title: string;
  credit: string;
  /** CSS font-family stack for the title, e.g. `'Archivo', sans-serif` */
  fontCss: string;
  /** Title size in design px (the studio slider value, 12–48) */
  size: number;
};

export type OverlayLayout = {
  /** Uniform scale from design space to output pixels */
  scale: number;
  /** Square box the logo is contain-fitted into; null = no logo drawn */
  logo: { x: number; y: number; box: number } | null;
  /** Text block metrics; null = nothing to draw */
  text: {
    x: number;
    align: CanvasTextAlign;
    /** top of the title line (textBaseline "top"); null = title empty */
    titleY: number | null;
    /** top of the credit line; null = credit empty */
    creditY: number | null;
    titlePx: number;
    creditPx: number;
    /** letter-spacing for the credit line, in output px */
    tracking: number;
  } | null;
};

/* The studio preview is treated as a 512px design space: all offsets and
   font sizes scale linearly with output width, so a 512 draft and a 2048
   final carry the SAME composition — only sharper. */
const DESIGN_WIDTH = 512;
const MARGIN = 18; // top/side inset, matches the preview's 18px
const BOTTOM_MARGIN = 24; // preview's 46px includes viewport chrome; the image needs less
const LOGO_BOX = 52; // preview placeholder is 52px (w-13)
const CREDIT_PX = 9.5;
const LINE_GAP = 4;
/** Title line-height factor — canvas has no line box, so we make one. */
const TITLE_LINE = 1.15;

export const overlayLayout = (
  width: number,
  height: number,
  spec: OverlaySpec,
): OverlayLayout => {
  const scale = width / DESIGN_WIDTH;
  const margin = MARGIN * scale;
  const bottom = BOTTOM_MARGIN * scale;

  let logo: OverlayLayout["logo"] = null;
  if (spec.logoOn) {
    const box = LOGO_BOX * scale;
    logo = {
      x: spec.logoPos.endsWith("l") ? margin : width - margin - box,
      y: spec.logoPos.startsWith("t") ? margin : height - bottom - box,
      box,
    };
  }

  let text: OverlayLayout["text"] = null;
  // An empty title is a placeholder in the preview ("Untitled") but must
  // never be baked into a real promo — empty lines simply drop out.
  const hasTitle = spec.textOn && spec.title.trim().length > 0;
  const hasCredit = spec.textOn && spec.credit.trim().length > 0;
  if (hasTitle || hasCredit) {
    const titlePx = spec.size * scale;
    const creditPx = CREDIT_PX * scale;
    const gap = LINE_GAP * scale;
    const titleH = hasTitle ? titlePx * TITLE_LINE : 0;
    const creditH = hasCredit ? creditPx : 0;
    const blockH = titleH + (hasTitle && hasCredit ? gap : 0) + creditH;
    const top = spec.textPos.startsWith("t")
      ? margin
      : height - bottom - blockH;
    const column = spec.textPos[1];
    text = {
      x: column === "l" ? margin : column === "c" ? width / 2 : width - margin,
      align: column === "l" ? "left" : column === "c" ? "center" : "right",
      titleY: hasTitle ? top : null,
      creditY: hasCredit ? top + titleH + (hasTitle ? gap : 0) : null,
      titlePx,
      creditPx,
      tracking: creditPx * 0.18, // the preview's tracking-[0.18em]
    };
  }

  return { scale, logo, text };
};

/** Average luminance (0–1) of a region, for picking a readable ink color. */
const regionLuminance = (
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  w: number,
  h: number,
): number => {
  const data = ctx.getImageData(
    Math.max(0, Math.floor(x)),
    Math.max(0, Math.floor(y)),
    Math.max(1, Math.floor(w)),
    Math.max(1, Math.floor(h)),
  ).data;
  let sum = 0;
  for (let i = 0; i < data.length; i += 4) {
    sum += 0.2126 * data[i] + 0.7152 * data[i + 1] + 0.0722 * data[i + 2];
  }
  return sum / (data.length / 4) / 255;
};

/**
 * Draw the overlay onto a canvas that already holds the render.
 * Ink color is picked per text block by sampling what's underneath —
 * dark text on the light studio backdrop, light text if someone renders
 * against a dark look later. No color knob to get wrong.
 */
export const drawOverlay = (
  ctx: CanvasRenderingContext2D,
  width: number,
  height: number,
  spec: OverlaySpec,
  logoImage: CanvasImageSource | null,
): void => {
  const layout = overlayLayout(width, height, spec);

  if (layout.logo && logoImage) {
    const { x, y, box } = layout.logo;
    const iw = Number((logoImage as HTMLImageElement).naturalWidth ?? box);
    const ih = Number((logoImage as HTMLImageElement).naturalHeight ?? box);
    // contain-fit, centered in the box, never upscaled past the box
    const fit = Math.min(box / iw, box / ih);
    const w = iw * fit;
    const h = ih * fit;
    ctx.drawImage(logoImage, x + (box - w) / 2, y + (box - h) / 2, w, h);
  }

  const text = layout.text;
  if (!text) return;

  const blockTop = text.titleY ?? text.creditY ?? 0;
  const blockBottom =
    (text.creditY ?? text.titleY ?? 0) +
    (text.creditY !== null ? text.creditPx : text.titlePx);
  const sampleW = width * 0.4;
  const sampleX =
    text.align === "left"
      ? text.x
      : text.align === "center"
        ? text.x - sampleW / 2
        : text.x - sampleW;
  const luminance = regionLuminance(
    ctx,
    sampleX,
    blockTop,
    sampleW,
    Math.max(1, blockBottom - blockTop),
  );
  const ink = luminance > 0.55 ? "#1f2429" : "#f4f4f5";

  ctx.textAlign = text.align;
  ctx.textBaseline = "top";
  ctx.fillStyle = ink;

  if (text.titleY !== null) {
    ctx.font = `700 ${text.titlePx}px ${spec.fontCss}`;
    ctx.fillText(spec.title.trim(), text.x, text.titleY);
  }
  if (text.creditY !== null) {
    ctx.font = `500 ${text.creditPx}px 'IBM Plex Mono', monospace`;
    ctx.globalAlpha = 0.7;
    // letterSpacing shipped in WebKit/Chromium recently enough that the
    // bundled webview may lack it — the credit just loses its tracking
    if ("letterSpacing" in ctx) {
      ctx.letterSpacing = `${text.tracking}px`;
    }
    ctx.fillText(spec.credit.trim(), text.x, text.creditY);
    if ("letterSpacing" in ctx) {
      ctx.letterSpacing = "0px";
    }
    ctx.globalAlpha = 1;
  }
};
