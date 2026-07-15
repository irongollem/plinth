/**
 * Group math for Base Cutter "unit blocks" (docs/BASECUTTER.md phase 6:
 * regiment placement). Pure functions only — no Vue reactivity, no
 * `bindings.ts` involvement. Group membership itself is frontend
 * VIEW STATE, never serialized to the backend (see BaseCutter.vue's
 * `groups` ref); this module only supplies the math the view applies to
 * `Placement` objects it already owns.
 */
import type { Placement } from "../bindings";

/** The subset of a Placement a group operation actually reads/writes. */
export type GroupMember = Pick<Placement, "x_mm" | "y_mm" | "rotation_deg">;

/** Mean of member centers — the pivot point for a group rotate. Empty input
 * returns the origin rather than NaN; callers never invoke this on an empty
 * group in practice (a group always has >= 2 members, see BaseCutter.vue's
 * dissolve-at-1 rule), but a pure function shouldn't NaN on a bad input. */
export const centroidOf = (
  members: GroupMember[],
): { x: number; y: number } => {
  if (!members.length) return { x: 0, y: 0 };
  const sum = members.reduce(
    (acc, m) => ({ x: acc.x + m.x_mm, y: acc.y + m.y_mm }),
    { x: 0, y: 0 },
  );
  return { x: sum.x / members.length, y: sum.y / members.length };
};

/** Normalize a degree value into [0, 360) — same convention as the existing
 * per-placement rotate buttons in BaseCutter.vue. */
export const normalizeDeg = (deg: number): number => ((deg % 360) + 360) % 360;

/**
 * Shortest signed angular delta from `from` to `to`, in (-180, 180]. Plain
 * subtraction (`to - from`) breaks across the 0/360 seam: stepping a
 * placement at 355° by +5° wraps its stored rotation_deg to 0°, and a naive
 * `0 - 355 = -355` reads as "spin most of the way around backwards" instead
 * of the intended tiny forward nudge. Every caller that recovers a delta
 * from two ABSOLUTE rotation values (the viewport's handle drag, the
 * [ / ] keys) must go through this, not raw subtraction.
 */
export const angularDelta = (from: number, to: number): number => {
  let d = (to - from) % 360;
  if (d > 180) d -= 360;
  if (d <= -180) d += 360;
  return d;
};

/**
 * Rotate a formation of members around (centerX, centerY) by `deltaDeg`:
 * every member's position orbits the center (bearing math) AND every
 * member's own `rotation_deg` advances by the same delta (normalized to
 * [0, 360)) — a real unit block pivots as one rigid body, so a formation
 * rotate must move both position and facing together or the ranks would
 * slide out of alignment relative to each other. Returns NEW member
 * objects (spread of the input) — callers write the results back onto
 * their own placement objects; this function never mutates its input.
 */
export const rotateGroup = <T extends GroupMember>(
  members: T[],
  centerX: number,
  centerY: number,
  deltaDeg: number,
): T[] => {
  const rad = (deltaDeg * Math.PI) / 180;
  const cos = Math.cos(rad);
  const sin = Math.sin(rad);
  return members.map((m) => {
    const dx = m.x_mm - centerX;
    const dy = m.y_mm - centerY;
    return {
      ...m,
      x_mm: centerX + dx * cos - dy * sin,
      y_mm: centerY + dx * sin + dy * cos,
      rotation_deg: normalizeDeg(m.rotation_deg + deltaDeg),
    };
  });
};

/**
 * dx/dy between two positions — the delta a group MOVE spreads to every
 * other member. Deliberately just a diff, not an accumulator: the viewport
 * emits a fresh `update` (with the member's new absolute x_mm/y_mm) on
 * every pointermove during a drag, so the caller must compute this against
 * the member's PRE-update position each event and re-derive it fresh next
 * event — accumulating it across events, or computing it against a stale
 * "drag start" position, would double-apply the same motion to the other
 * members as the drag continues.
 */
export const moveDelta = (
  oldPos: { x_mm: number; y_mm: number },
  newPos: { x_mm: number; y_mm: number },
): { dx: number; dy: number } => ({
  dx: newPos.x_mm - oldPos.x_mm,
  dy: newPos.y_mm - oldPos.y_mm,
});

/**
 * Recompute a `selectedIndex` after removing `removedIndices` (any order,
 * any count) from the placements array it indexes into — extends
 * BaseCutter.vue's original single-delete compensation (null out the
 * selection if it was the removed item, else shift it down by however many
 * removed items sat before it) to simultaneous multi-member removals, e.g.
 * deleting an entire group in one action.
 */
export const reindexSelection = (
  selectedIndex: number | null,
  removedIndices: number[],
): number | null => {
  if (selectedIndex === null) return null;
  if (removedIndices.includes(selectedIndex)) return null;
  const before = removedIndices.filter((i) => i < selectedIndex).length;
  return selectedIndex - before;
};
