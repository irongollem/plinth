/**
 * Bounded undo-stack helper for Base Cutter's placement/group edits
 * (docs/BASECUTTER.md phase 6 follow-up: an undo button for placement
 * changes). Pure array logic only; BaseCutter.vue owns the actual reactive
 * stack and the snapshot contents (placements + groups + selectedIndex,
 * cloneRaw'd off the live reactive state before every discrete mutation —
 * or once per drag/rotate GESTURE rather than once per pointermove, see
 * the `gestureInFlight` flag in BaseCutter.vue's viewport wiring).
 *
 * Undo-only for now: `pushBounded` only ever grows/caps a single stack.
 * Adding redo later would need its own forward stack (cleared on every new
 * push, the usual undo/redo rule) — nothing here forecloses that, it's
 * just not built until it's asked for.
 */

/** Push `item` onto `stack`, dropping the OLDEST entry once length exceeds
 * `max` — undo always steps back one action; it just eventually runs out
 * rather than growing unboundedly. Returns a NEW array (never mutates
 * `stack`), so callers can assign it straight back into a Vue ref. */
export const pushBounded = <T>(stack: T[], item: T, max: number): T[] => {
  const next = [...stack, item];
  return next.length > max ? next.slice(next.length - max) : next;
};

/** Pop the most recent entry off `stack` — `item` is `undefined` (and
 * `rest` unchanged) if the stack was empty. Never mutates the input. */
export const popLast = <T>(stack: T[]): { item: T | undefined; rest: T[] } => {
  if (!stack.length) return { item: undefined, rest: stack };
  return { item: stack[stack.length - 1], rest: stack.slice(0, -1) };
};
