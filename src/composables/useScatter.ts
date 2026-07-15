import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  type ScatterJob,
  type ScatterStatus,
  commands,
  events,
} from "../bindings";
import { jobIdOf } from "../utils/events";

/**
 * Tracks a scatter job driven by the Rust scatter pipeline (start_scatter /
 * cancel_scatter + scatter-status events). Mirrors useLandscapeGen: subscribe
 * once, expose reactive job state, filter events by job id once one is in
 * flight. See docs/SCATTER.md "Pinned interfaces".
 */
export function useScatter() {
  const status = ref<ScatterStatus | null>(null);
  const jobId = ref("");

  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.scatterStatus.listen((event) => {
      const payload = event.payload;
      const eventJobId = jobIdOf(payload);
      // Once a job is in flight, ignore events from any other (a stale
      // listener from a previous run that hasn't unmounted yet, say).
      if (jobId.value && eventJobId !== jobId.value) return;
      status.value = payload;

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

  const isRunning = computed(
    () =>
      !!jobId.value ||
      (status.value !== null &&
        !("Finished" in status.value) &&
        !("Failed" in status.value) &&
        !("Cancelled" in status.value)),
  );

  /** {placed, total} once the script has reported at least one Progress
   * tick — null before that (Started carries no counts yet). */
  const progress = computed(() =>
    status.value && "Progress" in status.value
      ? {
          placed: status.value.Progress.placed,
          total: status.value.Progress.total,
        }
      : null,
  );

  const finished = computed(() =>
    status.value && "Finished" in status.value ? status.value.Finished : null,
  );

  const failedMessage = computed(() =>
    status.value && "Failed" in status.value
      ? status.value.Failed.message
      : null,
  );

  const failedStdoutTail = computed(() =>
    status.value && "Failed" in status.value
      ? status.value.Failed.stdout_tail
      : null,
  );

  const cancelled = computed(
    () => status.value !== null && "Cancelled" in status.value,
  );

  /** One place for the fields start() and reset() both clear, so the two
   * can't drift out of sync on what counts as "no job". */
  const resetState = () => {
    status.value = null;
    jobId.value = "";
  };

  const start = async (job: ScatterJob) => {
    resetState();
    const result = await commands.startScatter(job);
    if (result.status === "ok") jobId.value = result.data;
    return result;
  };

  const cancel = async () => {
    if (!jobId.value) return;
    await commands.cancelScatter(jobId.value);
  };

  const reset = () => {
    resetState();
  };

  return {
    status,
    isRunning,
    progress,
    finished,
    failedMessage,
    failedStdoutTail,
    cancelled,
    start,
    cancel,
    reset,
  };
}
