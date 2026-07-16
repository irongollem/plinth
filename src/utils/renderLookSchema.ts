/**
 * Declarative schema for the render studio's advanced look controls.
 *
 * MUST mirror LOOK in src-tauri/resources/render_mini.py — same dot-paths,
 * same defaults. The script deep-merges a JSON override object onto LOOK,
 * so every knob here is a path into that dict. Single source of truth for
 * rendering the controls, validating persisted state, and sanitizing
 * imported look files.
 *
 * Deliberately absent: base_color, res and samples. The studio already
 * sends those as CLI flags (--color/--res/--samples), and flags outrank
 * --config — a knob for them here would be dead.
 */

import { linearToHex } from "./color";

export type Vec3 = [number, number, number];
export type LookValue = number | Vec3;
/** Only the diff from defaults, keyed by dot-path ("key.energy"). */
export type LookOverrides = Record<string, LookValue>;

export interface LookKnob {
  path: string;
  label: string;
  /** "color" is a linear-RGB Vec3 in overrides, hex in the UI. */
  kind: "number" | "color" | "vec3";
  default: LookValue;
  min?: number;
  max?: number;
  step?: number;
  hint?: string;
}

export interface LookGroup {
  title: string;
  /** Which studio look actually reads these knobs (undefined = all). */
  appliesTo?: "resin" | "rich" | "marmoset";
  knobs: LookKnob[];
}

const light = (
  name: "key" | "fill" | "rim",
  color: Vec3,
  energy: number,
  loc: Vec3,
  size: number,
  hint?: string,
): LookKnob[] => [
  {
    path: `${name}.color`,
    label: "Color",
    kind: "color",
    default: color,
    hint: hint ?? "Classic look — Resin and Rich use their own colors below",
  },
  {
    path: `${name}.energy`,
    label: "Energy",
    kind: "number",
    default: energy,
    min: 0,
    max: 20000,
    step: 10,
  },
  {
    path: `${name}.size`,
    label: "Size",
    kind: "number",
    default: size,
    min: 0.1,
    max: 50,
    step: 0.1,
    hint: "Bigger area light = softer shadows",
  },
  {
    path: `${name}.loc`,
    label: "Position",
    kind: "vec3",
    default: loc,
    min: -20,
    max: 20,
    step: 0.5,
    hint: "Lights re-aim at the model after moving",
  },
];

export const LOOK_GROUPS: LookGroup[] = [
  {
    title: "Material",
    knobs: [
      {
        path: "roughness",
        label: "Roughness",
        kind: "number",
        default: 0.52,
        min: 0,
        max: 1,
        step: 0.01,
      },
      {
        path: "sss_weight",
        label: "Subsurface weight",
        kind: "number",
        default: 0.35,
        min: 0,
        max: 1,
        step: 0.01,
        hint: "How much light wraps into the shadow side",
      },
      {
        path: "sss_radius",
        label: "Subsurface radius",
        kind: "vec3",
        default: [0.7, 0.35, 0.2],
        min: 0,
        max: 2,
        step: 0.01,
        hint: "Per-channel scatter depth (R,G,B)",
      },
      {
        path: "sss_scale",
        label: "Subsurface scale",
        kind: "number",
        default: 0.12,
        min: 0,
        max: 1,
        step: 0.01,
      },
    ],
  },
  {
    title: "Key light",
    knobs: light(
      "key",
      [1, 0.82, 0.55],
      1100,
      [4, -4, 6],
      10,
      "Classic and Resin looks — Rich uses its own color below",
    ),
  },
  {
    title: "Fill light",
    knobs: light("fill", [1, 0.78, 0.55], 110, [-5, -2, 3], 12),
  },
  { title: "Rim light", knobs: light("rim", [1, 0.8, 0.6], 500, [0, 5, 5], 5) },
  {
    title: "Camera & post",
    knobs: [
      {
        path: "cam_lens",
        label: "Lens (mm)",
        kind: "number",
        default: 60,
        min: 20,
        max: 200,
        step: 1,
      },
      {
        path: "exposure",
        label: "Exposure",
        kind: "number",
        default: 0,
        min: -3,
        max: 3,
        step: 0.05,
      },
    ],
  },
  {
    title: "Resin look extras",
    appliesTo: "resin",
    knobs: [
      {
        path: "resin.coat_weight",
        label: "Coat weight",
        kind: "number",
        default: 0.3,
        min: 0,
        max: 1,
        step: 0.01,
        hint: "The glossy layer over the satin base",
      },
      {
        path: "resin.coat_roughness",
        label: "Coat roughness",
        kind: "number",
        default: 0.12,
        min: 0,
        max: 1,
        step: 0.01,
      },
      {
        path: "resin.noise_scale",
        label: "Speckle scale",
        kind: "number",
        default: 450,
        min: 10,
        max: 2000,
        step: 10,
      },
      {
        path: "resin.noise_detail",
        label: "Speckle detail",
        kind: "number",
        default: 3,
        min: 0,
        max: 15,
        step: 0.5,
      },
      {
        path: "resin.bump_strength",
        label: "Speckle strength",
        kind: "number",
        default: 0.035,
        min: 0,
        max: 0.5,
        step: 0.005,
      },
      {
        path: "resin.world_color",
        label: "Studio reflection",
        kind: "color",
        default: [0.9, 0.88, 0.85],
        hint: "Reflected by the surface, invisible to the camera",
      },
      {
        path: "resin.world_strength",
        label: "Reflection strength",
        kind: "number",
        default: 0.12,
        min: 0,
        max: 1,
        step: 0.01,
      },
      {
        path: "resin.fill_color",
        label: "Fill color",
        kind: "color",
        default: [0.95, 0.93, 0.9],
      },
      {
        path: "resin.rim_color",
        label: "Rim color",
        kind: "color",
        default: [0.85, 0.9, 1],
        hint: "Runs cool against the warm key on purpose",
      },
    ],
  },
  {
    title: "Rich look extras",
    appliesTo: "rich",
    knobs: [
      {
        path: "rich.key_color",
        label: "Key color",
        kind: "color",
        default: [1, 0.9, 0.76],
      },
      {
        path: "rich.key_loc",
        label: "Key position",
        kind: "vec3",
        default: [4.5, -5.5, 3],
        min: -20,
        max: 20,
        step: 0.5,
        hint: "Low raking light reveals shallow terrain relief",
      },
      {
        path: "rich.fill_color",
        label: "Fill color",
        kind: "color",
        default: [0.92, 0.86, 0.78],
      },
      {
        path: "rich.rim_color",
        label: "Rim color",
        kind: "color",
        default: [0.9, 0.92, 1],
      },
      {
        path: "rich.key_energy_mult",
        label: "Key energy ×",
        kind: "number",
        default: 0.82,
        min: 0.1,
        max: 3,
        step: 0.05,
      },
      {
        path: "rich.key_size_mult",
        label: "Key size ×",
        kind: "number",
        default: 1.05,
        min: 0.1,
        max: 2,
        step: 0.05,
        hint: "Smaller key = harder shadows",
      },
      {
        path: "rich.fill_energy_mult",
        label: "Fill energy ×",
        kind: "number",
        default: 0.38,
        min: 0,
        max: 2,
        step: 0.05,
      },
      {
        path: "rich.rim_energy_mult",
        label: "Rim energy ×",
        kind: "number",
        default: 0.45,
        min: 0,
        max: 3,
        step: 0.05,
      },
      {
        path: "rich.rim_size_mult",
        label: "Rim size ×",
        kind: "number",
        default: 1.4,
        min: 0.1,
        max: 2,
        step: 0.05,
      },
      {
        path: "rich.sss_weight_mult",
        label: "Subsurface ×",
        kind: "number",
        default: 0.55,
        min: 0,
        max: 1,
        step: 0.05,
      },
      {
        path: "rich.exposure_shift",
        label: "Exposure shift",
        kind: "number",
        default: -0.05,
        min: -2,
        max: 1,
        step: 0.05,
      },
      {
        path: "rich.world_color",
        label: "Studio environment",
        kind: "color",
        default: [0.78, 0.76, 0.74],
        hint: "Broad light for low relief, invisible to the camera",
      },
      {
        path: "rich.world_strength",
        label: "Environment strength",
        kind: "number",
        default: 0.13,
        min: 0,
        max: 1,
        step: 0.01,
      },
    ],
  },
  {
    title: "Marmoset look extras",
    appliesTo: "marmoset",
    knobs: [
      {
        path: "marmoset.key_color",
        label: "Key color",
        kind: "color",
        default: [1, 0.78, 0.52],
      },
      {
        path: "marmoset.fill_color",
        label: "Fill color",
        kind: "color",
        default: [0.86, 0.62, 0.4],
      },
      {
        path: "marmoset.rim_color",
        label: "Rim color",
        kind: "color",
        default: [1, 0.56, 0.3],
      },
      {
        path: "marmoset.key_energy_mult",
        label: "Key energy ×",
        kind: "number",
        default: 1.08,
        min: 0.1,
        max: 3,
        step: 0.05,
      },
      {
        path: "marmoset.key_size_mult",
        label: "Key size ×",
        kind: "number",
        default: 0.42,
        min: 0.1,
        max: 2,
        step: 0.05,
        hint: "Smaller key = harder shadows",
      },
      {
        path: "marmoset.fill_energy_mult",
        label: "Fill energy ×",
        kind: "number",
        default: 0.16,
        min: 0,
        max: 2,
        step: 0.01,
      },
      {
        path: "marmoset.rim_energy_mult",
        label: "Rim energy ×",
        kind: "number",
        default: 0.75,
        min: 0,
        max: 3,
        step: 0.05,
      },
      {
        path: "marmoset.rim_size_mult",
        label: "Rim size ×",
        kind: "number",
        default: 0.55,
        min: 0.1,
        max: 2,
        step: 0.05,
      },
      {
        path: "marmoset.sss_weight_mult",
        label: "Subsurface ×",
        kind: "number",
        default: 0.3,
        min: 0,
        max: 1,
        step: 0.01,
      },
      {
        path: "marmoset.exposure_shift",
        label: "Exposure shift",
        kind: "number",
        default: -0.2,
        min: -2,
        max: 1,
        step: 0.05,
      },
      {
        path: "marmoset.world_color",
        label: "Studio environment",
        kind: "color",
        default: [0.72, 0.62, 0.52],
        hint: "Reflected into cavities, invisible to the camera",
      },
      {
        path: "marmoset.world_strength",
        label: "Environment strength",
        kind: "number",
        default: 0.14,
        min: 0,
        max: 1,
        step: 0.01,
      },
    ],
  },
  {
    title: "Translucent resin material",
    knobs: [
      {
        path: "translucent.sss_weight",
        label: "Scatter weight",
        kind: "number",
        default: 0.48,
        min: 0,
        max: 1,
        step: 0.01,
        hint: "Used only when Translucent resin is enabled",
      },
      {
        path: "translucent.sss_radius",
        label: "Scatter color radius",
        kind: "vec3",
        default: [1, 0.32, 0.12],
        min: 0,
        max: 2,
        step: 0.01,
      },
      {
        path: "translucent.sss_scale",
        label: "Scatter depth",
        kind: "number",
        default: 0.18,
        min: 0,
        max: 1,
        step: 0.01,
      },
      {
        path: "translucent.transmission",
        label: "Transmission",
        kind: "number",
        default: 0.04,
        min: 0,
        max: 0.5,
        step: 0.01,
        hint: "Kept low so cured resin glows without looking like glass",
      },
      {
        path: "translucent.backlight_mult",
        label: "Rear light ×",
        kind: "number",
        default: 1.65,
        min: 0,
        max: 4,
        step: 0.05,
      },
      {
        path: "translucent.backlight_color",
        label: "Rear light color",
        kind: "color",
        default: [1, 0.34, 0.08],
      },
    ],
  },
];

export const LOOK_KNOBS: ReadonlyMap<string, LookKnob> = new Map(
  LOOK_GROUPS.flatMap((g) => g.knobs).map((k) => [k.path, k]),
);

const isVec3 = (v: unknown): v is Vec3 =>
  Array.isArray(v) &&
  v.length === 3 &&
  v.every((n) => typeof n === "number" && Number.isFinite(n));

const clamp = (v: number, knob: LookKnob) =>
  Math.min(
    knob.max ?? Number.POSITIVE_INFINITY,
    Math.max(knob.min ?? Number.NEGATIVE_INFINITY, v),
  );

/** Is this value (still) the knob's default? Colors compare as HEX — the
 * 8-bit sRGB round-trip loses precision, so a linear-float compare would
 * turn every color ever touched into a phantom tweak. */
export const isKnobDefault = (knob: LookKnob, value: LookValue): boolean => {
  if (typeof knob.default === "number") {
    return typeof value === "number" && Math.abs(value - knob.default) < 1e-9;
  }
  if (!isVec3(value)) return false;
  if (knob.kind === "color")
    return linearToHex(value) === linearToHex(knob.default);
  return knob.default.every((d, i) => Math.abs(value[i] - d) < 1e-9);
};

/**
 * Validate an untrusted overrides record (localStorage blob or imported
 * look file) against the schema. Unknown paths and malformed values are
 * dropped and reported; numbers are clamped into the knob's range.
 */
export const sanitizeOverrides = (
  raw: unknown,
): { overrides: LookOverrides; dropped: string[] } => {
  const overrides: LookOverrides = {};
  const dropped: string[] = [];
  if (raw && typeof raw === "object" && !Array.isArray(raw)) {
    for (const [path, value] of Object.entries(raw)) {
      const knob = LOOK_KNOBS.get(path);
      if (!knob) {
        dropped.push(path);
        continue;
      }
      if (
        knob.kind === "number" &&
        typeof value === "number" &&
        Number.isFinite(value)
      ) {
        overrides[path] = clamp(value, knob);
      } else if (knob.kind !== "number" && isVec3(value)) {
        const lo =
          knob.kind === "color" ? 0 : (knob.min ?? Number.NEGATIVE_INFINITY);
        const hi =
          knob.kind === "color" ? 1 : (knob.max ?? Number.POSITIVE_INFINITY);
        overrides[path] = value.map((v) =>
          Math.min(hi, Math.max(lo, v)),
        ) as Vec3;
      } else {
        dropped.push(path);
        continue;
      }
      // A clamped/round-tripped value can land back on the default — a
      // stored "tweak" that changes nothing is just badge noise
      if (isKnobDefault(knob, overrides[path])) delete overrides[path];
    }
  }
  return { overrides, dropped };
};

/** Dot-path overrides -> the nested object render_mini.py merges onto LOOK. */
export const overridesToNested = (
  overrides: LookOverrides,
): Record<string, unknown> => {
  const nested: Record<string, unknown> = {};
  for (const [path, value] of Object.entries(overrides)) {
    const keys = path.split(".");
    let cursor = nested;
    for (const key of keys.slice(0, -1)) {
      cursor[key] = cursor[key] ?? {};
      cursor = cursor[key] as Record<string, unknown>;
    }
    cursor[keys[keys.length - 1]] = value;
  }
  return nested;
};
