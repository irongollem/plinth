import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  type LandscapeGenStatus,
  type LandscapeParams,
  commands,
  events,
} from "../bindings";
import { jobIdOf } from "../utils/events";

/**
 * Tracks a landscape-generator job driven by the Rust generator pipeline
 * (start_landscape_generation / cancel_landscape_generation +
 * landscape-gen-status events). Mirrors useBaseCut: subscribe once, expose
 * reactive job state, filter events by job id once one is in flight. See
 * docs/BASECUTTER.md "The landscape generator (phase 6)".
 */
export function useLandscapeGen() {
  const status = ref<LandscapeGenStatus | null>(null);
  const jobId = ref("");

  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.landscapeGenStatus.listen((event) => {
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

  const start = async (params: LandscapeParams, presetId: string | null) => {
    resetState();
    const result = await commands.startLandscapeGeneration(params, presetId);
    if (result.status === "ok") jobId.value = result.data;
    return result;
  };

  const cancel = async () => {
    if (!jobId.value) return;
    await commands.cancelLandscapeGeneration(jobId.value);
  };

  const reset = () => {
    resetState();
  };

  return {
    status,
    isRunning,
    finished,
    failedMessage,
    failedStdoutTail,
    cancelled,
    start,
    cancel,
    reset,
  };
}
