/**
 * STL worker-decode lifecycle, shared by StlViewport and LandscapeViewport.
 * Parsing + mergeVertices run in a Web Worker so a multi-million-triangle
 * mesh never freezes the UI; a superseded decode is aborted by terminating
 * the worker (the only way to stop CPU-bound JS), which rejects its pending
 * promise so the caller's own token check discards the stale result.
 * Geometry assembly stays with each viewport — they differ (single mesh vs.
 * multi-part join/normalize) — this composable only owns the worker.
 */
import type {
  StlDecodeResponse,
  StlPartPayload,
} from "../utils/stlGeometry.worker.ts";

/** Rejection message for an aborted decode — not a real failure, just a
 * newer request taking over; callers should swallow it silently. */
export const SUPERSEDED = "superseded";

/**
 * A file's bytes as a transferable ArrayBuffer. When the Uint8Array spans
 * its whole backing buffer (the common case — one file, one allocation) the
 * buffer transfers directly with no copy; an offset/length view (e.g. a
 * slice into a larger allocation) falls back to `slice`, which copies only
 * that view's bytes rather than handing over memory outside it. Module-scope
 * (not per-instance): it captures nothing from useStlDecode's closure.
 */
export const toTransferableBuffer = (bytes: Uint8Array): ArrayBuffer => {
  if (bytes.byteOffset === 0 && bytes.byteLength === bytes.buffer.byteLength) {
    return bytes.buffer as ArrayBuffer;
  }
  return bytes.buffer.slice(
    bytes.byteOffset,
    bytes.byteOffset + bytes.byteLength,
  ) as ArrayBuffer;
};

export function useStlDecode() {
  let worker: Worker | null = null;
  let pending: {
    id: number;
    resolve: (parts: StlPartPayload[]) => void;
    reject: (error: Error) => void;
  } | null = null;

  const spawnWorker = () => {
    const w = new Worker(
      new URL("../utils/stlGeometry.worker.ts", import.meta.url),
      { type: "module" },
    );
    w.addEventListener("message", (event: MessageEvent<StlDecodeResponse>) => {
      if (!pending || event.data.id !== pending.id) return;
      const { resolve, reject } = pending;
      pending = null;
      if (event.data.error) reject(new Error(event.data.error));
      else resolve(event.data.parts);
    });
    return w;
  };

  /** Reject whatever decode is in flight and drop the worker with it. */
  const abortDecode = () => {
    if (!pending) return;
    worker?.terminate();
    worker = null;
    pending.reject(new Error(SUPERSEDED));
    pending = null;
  };

  const decodeInWorker = (
    id: number,
    buffers: ArrayBuffer[],
    opts?: { splitComponents?: boolean },
  ) => {
    abortDecode();
    worker ??= spawnWorker();
    return new Promise<StlPartPayload[]>((resolve, reject) => {
      pending = { id, resolve, reject };
      // buffers transfer, not copy — the worker owns them from here
      worker?.postMessage(
        { id, buffers, splitComponents: opts?.splitComponents },
        buffers,
      );
    });
  };

  /** Tear down the worker for good (component unmount). */
  const dispose = () => {
    abortDecode();
    worker?.terminate();
    worker = null;
  };

  return { decodeInWorker, abortDecode, dispose };
}
