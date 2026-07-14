/**
 * Shared helpers for specta-generated Tauri event payloads. Every status
 * event (BaseCutStatus, RenderStatus, ...) is a union of single-key objects
 * shaped `{ Variant: { job_id: string, ... } }` — whichever variant arrived,
 * its one value carries job_id. This replaces the nested-ternary
 * "'X' in payload ? payload.X.job_id : ..." chain that used to live in each
 * status composable (one per event type) with a single generic accessor.
 */
export function jobIdOf<T extends Record<string, { job_id: string }>>(
  payload: T,
): string {
  return Object.values(payload)[0].job_id;
}
