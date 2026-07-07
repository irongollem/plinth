import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  type BatchRenderStatus,
  type BatchRenderTarget,
  commands,
  events,
} from "../bindings";

/**
 * Tracks the batch preview render (many models, one Blender launch), driven
 * by the BatchRenderStatus event stream. Same shape as usePackStatus; the
 * extra `modelFinishedCount` ticks per completed model because previews land
 * incrementally — the grid can refresh without waiting for the whole sweep.
 */
export function useBatchRender() {
  const batchStatus = ref<BatchRenderStatus | null>(null);
  const batchJobId = ref("");
  /** Bumped on ANY terminal state — a cancelled sweep still rendered some. */
  const batchFinishedCount = ref(0);
  /** Bumped per finished model (ok or failed). */
  const modelFinishedCount = ref(0);
  /** Per-model failures collected for the summary toast. */
  const modelErrors = ref<string[]>([]);

  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.batchRenderStatus.listen((event) => {
      batchStatus.value = event.payload;
      if ("Started" in event.payload) {
        batchJobId.value = event.payload.Started.job_id;
        modelErrors.value = [];
      }
      if ("ModelFinished" in event.payload) {
        modelFinishedCount.value++;
        const finished = event.payload.ModelFinished;
        if (!finished.ok) {
          modelErrors.value.push(
            `${finished.dir_path}: ${finished.error ?? "render failed"}`,
          );
        }
      }
      if (
        "Completed" in event.payload ||
        "Failed" in event.payload ||
        "Cancelled" in event.payload
      ) {
        batchJobId.value = "";
        batchFinishedCount.value++;
      }
    });
  });

  onUnmounted(() => {
    unlisten?.();
  });

  const isBatchRendering = computed(
    () =>
      !!batchJobId.value ||
      (batchStatus.value !== null &&
        ("Started" in batchStatus.value || "Progress" in batchStatus.value)),
  );

  const batchProgress = computed(() =>
    batchStatus.value && "Progress" in batchStatus.value
      ? batchStatus.value.Progress
      : null,
  );

  const batchSummary = computed(() =>
    batchStatus.value && "Completed" in batchStatus.value
      ? batchStatus.value.Completed
      : null,
  );

  const batchError = computed(() =>
    batchStatus.value && "Failed" in batchStatus.value
      ? batchStatus.value.Failed.error
      : null,
  );

  const batchCancelled = computed(() =>
    batchStatus.value && "Cancelled" in batchStatus.value
      ? batchStatus.value.Cancelled
      : null,
  );

  const startBatch = async (targets: BatchRenderTarget[]) => {
    batchStatus.value = null;
    const result = await commands.startBatchRender(targets);
    if (result.status === "ok") batchJobId.value = result.data;
    return result;
  };

  const cancelBatch = async () => {
    // batch jobs live in the same registry as studio renders on purpose:
    // one cancel command serves both
    if (batchJobId.value) await commands.cancelRender(batchJobId.value);
  };

  return {
    isBatchRendering,
    batchProgress,
    batchSummary,
    batchError,
    batchCancelled,
    batchFinishedCount,
    modelFinishedCount,
    modelErrors,
    startBatch,
    cancelBatch,
  };
}
