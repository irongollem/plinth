/**
 * Validation for user-edited Base Cutter placement names (BaseCutter.vue's
 * inline rename). Pure — no Vue reactivity — so it's unit-testable on its
 * own and reusable from the rename handler without dragging component state
 * along.
 *
 * Mirrors basecutter::commands::validate_placements' duplicate-name guard
 * (Rust rejects two placements sharing a non-empty name because
 * base_cut.py names each output STL after its placement — see that
 * function's doc comment) so the frontend catches the same mistake at edit
 * time rather than only after start_base_cut rejects the whole job.
 *
 * Also enforces filesystem-safety Rust doesn't need to: base_cut.py writes
 * "{name}.stl" directly into out_dir, and the userbase is mostly Windows
 * (see storage-setup notes), so a name must survive as a Windows file name.
 * This does NOT check Windows' reserved device names (CON, PRN, COM1, ...)
 * — out-of-scope for this pass, and unlikely for a base-cutter placement
 * name in practice.
 */

// Characters Windows forbids in a file/directory name, plus C0 control
// characters (never valid in a Windows name either, and never something a
// user meant to type into this field).
// eslint-disable-next-line no-control-regex
const FORBIDDEN_CHARS = /[<>:"/\\|?*\x00-\x1f]/;

export const validatePlacementName = (
  draft: string,
  otherNames: (string | null | undefined)[],
): string | null => {
  const trimmed = draft.trim();
  if (!trimmed) return "Name can't be empty";
  if (FORBIDDEN_CHARS.test(trimmed)) {
    return `Can't contain any of < > : " / \\ | ? *`;
  }
  // Windows silently strips a trailing dot when the file actually lands on
  // disk — trailing whitespace is already gone via trim() above, so the
  // dot is the one thing left that could make the on-disk name diverge
  // from what the uniqueness check below just validated.
  if (trimmed.endsWith(".")) {
    return "Can't end with a dot — Windows strips it from the file name";
  }
  const lower = trimmed.toLowerCase();
  if (otherNames.some((name) => (name ?? "").trim().toLowerCase() === lower)) {
    return `'${trimmed}' is already used by another placement`;
  }
  return null;
};
