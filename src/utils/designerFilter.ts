/**
 * The "no designer" facet travels as a filter VALUE, not as a designer
 * name, so it needs a spelling both sides agree on and a label the user
 * should see instead of that spelling.
 *
 * Paired with `catalog::db::UNIDENTIFIED_DESIGNER_FILTER`; specta doesn't
 * export consts, so the test beside this file pins the wire value.
 */
export const UNIDENTIFIED_DESIGNER_FILTER = "__plinth_unidentified_designer__";

export const isUnidentifiedFilter = (filter: string | null | undefined) =>
  filter === UNIDENTIFIED_DESIGNER_FILTER;

/** What a designer filter is called on screen. */
export const designerFilterLabel = (filter: string) =>
  isUnidentifiedFilter(filter) ? "Unidentified" : filter;
