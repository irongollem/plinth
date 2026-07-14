import { toRaw } from "vue";

/**
 * Deep-copy a value that may be (or contain) a Vue reactive proxy.
 *
 * `structuredClone` throws `DataCloneError` on Proxies, and anything read
 * out of a `ref`/`reactive` in a template or watcher IS a Proxy — while the
 * same object obtained before wrapping is not. That asymmetry produced a
 * bug that only fired on user interaction: cloning a preset worked in
 * onMounted (raw command result) and silently threw on every chip click
 * (proxied via the ref), leaving stale params behind a freshly-highlighted
 * chip. `toRaw` unwraps to the underlying target (Vue wraps nested objects
 * lazily on access, so the target's own nested values are already raw),
 * making the clone safe from either side of the reactivity boundary.
 */
export function cloneRaw<T>(value: T): T {
  return structuredClone(toRaw(value));
}
