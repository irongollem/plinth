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

/** The char/dot rules `validatePlacementName` and `validatePlacementNamePrefix`
 * share — factored out so the two can't drift on what's filesystem-unsafe.
 * Takes an already-trimmed, already-known-non-empty string; emptiness means
 * different things to the two callers (a name can't be empty, a prefix
 * being empty just means the feature is off), so each checks that itself. */
const forbiddenCharOrTrailingDotIssue = (trimmed: string): string | null => {
  if (FORBIDDEN_CHARS.test(trimmed)) {
    return `Can't contain any of < > : " / \\ | ? *`;
  }
  // Windows silently strips a trailing dot when the file actually lands on
  // disk — trailing whitespace is already gone via trim() above, so the
  // dot is the one thing left that could make the on-disk name diverge
  // from what a caller's own further checks (e.g. uniqueness) validated.
  if (trimmed.endsWith(".")) {
    return "Can't end with a dot — Windows strips it from the file name";
  }
  return null;
};

export const validatePlacementName = (
  draft: string,
  otherNames: (string | null | undefined)[],
): string | null => {
  const trimmed = draft.trim();
  if (!trimmed) return "Name can't be empty";
  const issue = forbiddenCharOrTrailingDotIssue(trimmed);
  if (issue) return issue;
  const lower = trimmed.toLowerCase();
  if (otherNames.some((name) => (name ?? "").trim().toLowerCase() === lower)) {
    return `'${trimmed}' is already used by another placement`;
  }
  return null;
};

/**
 * Validates the LAYOUT step's optional name-prefix field (BaseCutter.vue) —
 * the same filesystem-safety rules as `validatePlacementName`, MINUS the
 * uniqueness check: a prefix isn't a whole placement name, it's shared by a
 * whole batch on purpose (`placementNamePrefix`/`mintNames` below dedupe the
 * MINTED names, not the prefix itself). An empty draft is valid — it means
 * the feature is off, not an error — so this returns `null` for it instead
 * of "can't be empty".
 */
export const validatePlacementNamePrefix = (draft: string): string | null => {
  const trimmed = draft.trim();
  if (!trimmed) return null;
  return forbiddenCharOrTrailingDotIssue(trimmed);
};

/**
 * The base prefix a batch of minted placement names will share, before the
 * trailing "<n>" (see `mintNames`). Two schemes: WITH a user name-prefix
 * (LAYOUT step's optional field, validated by `validatePlacementNamePrefix`),
 * "<prefix>-<sizeLabel>mm-" (e.g. "donkey-28.5mm-1"); `sizeLabel`'s own "×"
 * (BaseCutter.vue's ellipse/rect dimension labels, e.g. "60×35") is swapped
 * for "x" here since the result becomes a filename downstream (base_cut.py
 * writes "{name}.stl") on a mostly-Windows userbase (see
 * docs/BASECUTTER.md's storage notes) where "×" is an avoidable landmine.
 * WITHOUT a user prefix: the original cutter-id scheme, "<id-sans-dashes>-".
 */
export const placementNamePrefix = (
  userPrefix: string,
  sizeLabel: string,
  cutterId: string,
): string =>
  userPrefix
    ? `${userPrefix}-${sizeLabel.replace(/×/g, "x")}mm-`
    : `${cutterId.replace(/-/g, "")}-`;

/**
 * Mints `count` collision-free placement names sharing `prefix`, numbered
 * 1-past the highest existing numeric suffix already in use under that
 * EXACT prefix — deliberately NOT a count of survivors: deleting the middle
 * name out of a {1,2,3} run and reusing "2" would hand a fresh placement the
 * same name as one that's still live, and base_cut.py names each output STL
 * after its placement, so two live placements sharing a name silently
 * overwrite one output with another. Taking 1 + max(existing suffixes)
 * instead never reuses a name still in use. `existingNames` is every current
 * placement's name as a plain array (BaseCutter.vue passes
 * `placements.value.map(p => p.name)`) — kept plain rather than reading
 * Vue's reactive array directly, so this stays pure and unit-testable.
 */
export const mintNames = (
  prefix: string,
  existingNames: (string | null | undefined)[],
  count: number,
): string[] => {
  let maxSuffix = 0;
  for (const name of existingNames) {
    if (!name?.startsWith(prefix)) continue;
    const suffix = Number(name.slice(prefix.length));
    if (Number.isFinite(suffix)) maxSuffix = Math.max(maxSuffix, suffix);
  }
  return Array.from(
    { length: count },
    (_, i) => `${prefix}${maxSuffix + 1 + i}`,
  );
};
