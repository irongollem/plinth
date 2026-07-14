import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  type BaseCutJob,
  type BaseCutStatus,
  commands,
  events,
} from "../bindings";

/** One finished (or failed) cut, folded from CutDone/CutFailed events into a
 * single flat shape the results list can render without branching on which
 * variant arrived. */
export type BaseCutResult = {
  index: number;
  ok: boolean;
  out_path?: string;
  dims_mm?: [number, number, number];
  manifold?: boolean;
  reason?: string;
};

/**
 * Tracks a Base Cutter job driven by the Rust job pipeline (start_base_cut /
 * cancel_base_cut + base-cut-status events). Mirrors useBatchRender /
 * useRenderStatus: subscribe once, expose reactive job state, filter events
 * by job id once one is in flight.
 */
export function useBaseCut() {
  const status = ref<BaseCutStatus | null>(null);
  const jobId = ref("");
  const total = ref(0);
  const results = ref<BaseCutResult[]>([]);
  /** Non-fatal note from the validation pass (see BaseCutValidationReport). */
  const validationWarning = ref<string | null>(null);

  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.baseCutStatus.listen((event) => {
      const payload = event.payload;
      const eventJobId =
        "Started" in payload
          ? payload.Started.job_id
          : "Validating" in payload
            ? payload.Validating.job_id
            : "Validated" in payload
              ? payload.Validated.job_id
              : "CutStarted" in payload
                ? payload.CutStarted.job_id
                : "CutDone" in payload
                  ? payload.CutDone.job_id
                  : "CutFailed" in payload
                    ? payload.CutFailed.job_id
                    : "Finished" in payload
                      ? payload.Finished.job_id
                      : payload.Failed.job_id;
      // Once a job is in flight, ignore events from any other (a stale
      // listener from a previous run that hasn't unmounted yet, say).
      if (jobId.value && eventJobId !== jobId.value) return;
      status.value = payload;

      if ("Started" in payload) {
        total.value = payload.Started.total;
        results.value = [];
        validationWarning.value = null;
      }
      if ("Validated" in payload) {
        validationWarning.value = payload.Validated.report.warning ?? null;
      }
      if ("CutDone" in payload) {
        const done = payload.CutDone;
        results.value.push({
          index: done.index,
          ok: true,
          out_path: done.out_path,
          dims_mm: done.dims_mm,
          manifold: done.manifold,
        });
      }
      if ("CutFailed" in payload) {
        const failed = payload.CutFailed;
        results.value.push({
          index: failed.index,
          ok: false,
          reason: failed.reason,
        });
      }
      if ("Finished" in payload || "Failed" in payload) {
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
        !("Failed" in status.value)),
  );

  /** 0-based index of the cut currently in progress (best-effort — only
   * meaningful while running). */
  const currentIndex = computed(() => {
    if (!status.value) return 0;
    if ("CutStarted" in status.value) return status.value.CutStarted.index;
    if ("CutDone" in status.value) return status.value.CutDone.index;
    if ("CutFailed" in status.value) return status.value.CutFailed.index;
    return 0;
  });

  const percent = computed(() => {
    if (!total.value) return 0;
    return Math.min(
      100,
      Math.round((results.value.length * 100) / total.value),
    );
  });

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

  const finishedSummary = computed(() =>
    status.value && "Finished" in status.value ? status.value.Finished : null,
  );

  const start = async (job: BaseCutJob) => {
    status.value = null;
    results.value = [];
    validationWarning.value = null;
    total.value = job.placements.length;
    const result = await commands.startBaseCut(job);
    if (result.status === "ok") jobId.value = result.data;
    return result;
  };

  const cancel = async () => {
    if (!jobId.value) return;
    await commands.cancelBaseCut(jobId.value);
  };

  const reset = () => {
    status.value = null;
    jobId.value = "";
    total.value = 0;
    results.value = [];
    validationWarning.value = null;
  };

  return {
    status,
    isRunning,
    total,
    currentIndex,
    percent,
    results,
    validationWarning,
    failedMessage,
    failedStdoutTail,
    finishedSummary,
    start,
    cancel,
    reset,
  };
}
