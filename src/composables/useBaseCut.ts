import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  type BaseCutJob,
  type BaseCutStatus,
  commands,
  events,
} from "../bindings";
import { jobIdOf } from "../utils/events";

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
  /** `false` = the plug/plinth union left loose shells behind (normal mode
   * only); `null`/absent in topper mode or when the union fused cleanly. */
  fused?: boolean | null;
  /** Loose-shell count backing `fused`, present alongside it. */
  shells?: number | null;
  /** Present only when the job's requested `topper_mm` fell outside the
   * script's clamp range — the effective value it used instead. */
  topper_mm_clamped?: number | null;
  /** `true` = this placement carried a magnet spec that topper mode
   * ignored. */
  magnet_ignored?: boolean | null;
  /** Slice-mode scatter shells omitted because Blender could not produce a
   * closed, rim-bounded intersection for them. */
  scatter_skipped?: number | null;
  /** The cut's `.glb` twin path (VTT GLB export design doc "Base cut"),
   * glb-mode jobs only — `null`/absent in the default (non-glb) mode. */
  glb_path?: string | null;
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
      const eventJobId = jobIdOf(payload);
      // Once a job is in flight, ignore events from any other (a stale
      // listener from a previous run that hasn't unmounted yet, say).
      if (jobId.value && eventJobId !== jobId.value) return;
      status.value = payload;

      if ("Started" in payload) {
        // Started owns `total` — the single source of truth for the job
        // size (start() no longer guesses it from the placement list).
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
          fused: done.fused,
          shells: done.shells,
          topper_mm_clamped: done.topper_mm_clamped,
          magnet_ignored: done.magnet_ignored,
          scatter_skipped: done.scatter_skipped,
          glb_path: done.glb_path,
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

  /** True once a Cancelled event lands — the "user stopped this, it didn't
   * break" case (see BaseCutStatus's doc comment). */
  const cancelled = computed(
    () => status.value !== null && "Cancelled" in status.value,
  );

  /** One place for the fields start() and reset() both clear, so the two
   * can't drift out of sync on what counts as "no job". */
  const resetState = () => {
    status.value = null;
    jobId.value = "";
    total.value = 0;
    results.value = [];
    validationWarning.value = null;
  };

  const start = async (job: BaseCutJob) => {
    resetState();
    const result = await commands.startBaseCut(job);
    if (result.status === "ok") jobId.value = result.data;
    return result;
  };

  const cancel = async () => {
    if (!jobId.value) return;
    await commands.cancelBaseCut(jobId.value);
  };

  const reset = () => {
    resetState();
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
    cancelled,
    start,
    cancel,
    reset,
  };
}
