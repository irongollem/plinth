import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  type MdFinished,
  type MinihoardDownloadStatus,
  type MinihoardError,
  commands,
  events,
} from "../bindings";
import { jobIdOf } from "../utils/events";

/** One object's progress in the queue panel, folded from the
 * ObjectStart/FileProgress/ObjectDone/ObjectFailed event stream into a
 * single row the template can render without branching per-variant. */
export type MinihoardQueueItem = {
  id: number;
  name: string;
  index: number;
  total: number;
  bytesDone: number;
  /** Null when the server didn't send Content-Length — render an
   * indeterminate/bytes-only state instead of a percent. */
  bytesTotal: number | null;
  done: boolean;
  failed: boolean;
  reason: string | null;
};

/**
 * Tracks a Minihoard download job driven by start_minihoard_download /
 * cancel_minihoard_download + minihoard-download-status events. Mirrors
 * useBaseCut: subscribe once, expose reactive queue state, filter events by
 * job id once one is in flight.
 */
export function useMinihoardDownload() {
  const status = ref<MinihoardDownloadStatus | null>(null);
  const jobId = ref("");
  const total = ref(0);
  /** Arrival order of ObjectStart ids — object keys with numeric-looking
   * names iterate in ascending numeric order in JS, not insertion order,
   * so the queue panel needs its own ordering list rather than relying on
   * Object.keys(byId). */
  const order = ref<number[]>([]);
  const byId = ref<Record<number, MinihoardQueueItem>>({});
  /** ids that reached ObjectDone during the run in flight (or the last
   * completed one) — the view uses this on Finished to flip `downloaded`
   * on the buffered list without re-listing. */
  const doneIds = ref<number[]>([]);

  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.minihoardDownloadStatus.listen((event) => {
      const payload = event.payload;
      const eventJobId = jobIdOf(payload);
      // Once a job is in flight, ignore events from any other (a stale
      // listener from a previous run that hasn't unmounted yet, say).
      if (jobId.value && eventJobId !== jobId.value) return;
      status.value = payload;

      if ("Started" in payload) {
        total.value = payload.Started.total;
      }
      if ("ObjectStart" in payload) {
        const { id, name, index, total: objTotal } = payload.ObjectStart;
        order.value.push(id);
        byId.value[id] = {
          id,
          name,
          index,
          total: objTotal,
          bytesDone: 0,
          bytesTotal: null,
          done: false,
          failed: false,
          reason: null,
        };
      }
      if ("FileProgress" in payload) {
        const { id, bytes_done, bytes_total } = payload.FileProgress;
        const item = byId.value[id];
        if (item) {
          item.bytesDone = bytes_done;
          item.bytesTotal = bytes_total;
        }
      }
      if ("ObjectDone" in payload) {
        const { id } = payload.ObjectDone;
        const item = byId.value[id];
        if (item) item.done = true;
        doneIds.value.push(id);
      }
      if ("ObjectFailed" in payload) {
        const { id, reason } = payload.ObjectFailed;
        const item = byId.value[id];
        if (item) {
          item.failed = true;
          item.reason = reason;
        }
      }
      if (
        "Finished" in payload ||
        "Failed" in payload ||
        "Cancelled" in payload
      ) {
        jobId.value = "";
      }
    });
  });

  onUnmounted(() => {
    unlisten?.();
    unlisten = null;
  });

  const queue = computed<MinihoardQueueItem[]>(() =>
    order.value.map((id) => byId.value[id]).filter((item) => !!item),
  );

  const isRunning = computed(
    () =>
      !!jobId.value ||
      (status.value !== null &&
        !("Finished" in status.value) &&
        !("Failed" in status.value) &&
        !("Cancelled" in status.value)),
  );

  const finishedSummary = computed<MdFinished | null>(() =>
    status.value && "Finished" in status.value ? status.value.Finished : null,
  );

  const failedError = computed<MinihoardError | null>(() =>
    status.value && "Failed" in status.value ? status.value.Failed.error : null,
  );

  const cancelled = computed(
    () => status.value !== null && "Cancelled" in status.value,
  );

  /** One place for the fields start() and reset() both clear, so the two
   * can't drift out of sync on what counts as "no job". */
  const resetState = () => {
    status.value = null;
    jobId.value = "";
    total.value = 0;
    order.value = [];
    byId.value = {};
    doneIds.value = [];
  };

  const start = async (binaryPath: string, ids: number[]) => {
    resetState();
    const result = await commands.startMinihoardDownload(binaryPath, ids);
    if (result.status === "ok") jobId.value = result.data;
    return result;
  };

  const cancel = async () => {
    if (!jobId.value) return;
    await commands.cancelMinihoardDownload(jobId.value);
  };

  const reset = () => {
    resetState();
  };

  return {
    status,
    jobId,
    total,
    queue,
    doneIds,
    isRunning,
    finishedSummary,
    failedError,
    cancelled,
    start,
    cancel,
    reset,
  };
}
