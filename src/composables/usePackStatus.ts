import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import { commands, events, type PackStatus } from "../bindings";

/**
 * Tracks compressed-at-rest pack/unpack jobs, driven by the Rust
 * pack-status event stream. Same shape as useCatalogJobs: start returns a
 * job_id, progress arrives as events, cancel goes through the shared
 * catalog-job registry.
 */
export function usePackStatus() {
  const packStatus = ref<PackStatus | null>(null);
  const packJobId = ref("");
  /** Bumped on ANY terminal state — even a cancelled or failed batch has
   *  already packed/unpacked some folders, so views must refresh. */
  const packFinishedCount = ref(0);

  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.packStatus.listen((event) => {
      packStatus.value = event.payload;
      // ensure_model_files is awaited (no job_id return value), so the
      // Started event is where cancellation learns the id
      if ("Started" in event.payload) {
        packJobId.value = event.payload.Started.job_id;
      }
      if (
        "Completed" in event.payload ||
        "Failed" in event.payload ||
        "Cancelled" in event.payload
      ) {
        packJobId.value = "";
        packFinishedCount.value++;
      }
    });
  });

  onUnmounted(() => {
    unlisten?.();
  });

  const isPacking = computed(
    () =>
      !!packJobId.value ||
      (packStatus.value !== null &&
        ("Started" in packStatus.value || "Progress" in packStatus.value)),
  );

  const packProgress = computed(() =>
    packStatus.value && "Progress" in packStatus.value
      ? packStatus.value.Progress
      : null,
  );

  const packError = computed(() =>
    packStatus.value && "Failed" in packStatus.value
      ? packStatus.value.Failed.error
      : null,
  );

  const packSummary = computed(() =>
    packStatus.value && "Completed" in packStatus.value
      ? packStatus.value.Completed
      : null,
  );

  const packCancelled = computed(() =>
    packStatus.value && "Cancelled" in packStatus.value
      ? packStatus.value.Cancelled
      : null,
  );

  /** "pack" or "unpack" — which direction the current/last job ran. */
  const packAction = computed(() => {
    if (!packStatus.value) return null;
    if ("Started" in packStatus.value) return packStatus.value.Started.action;
    if ("Completed" in packStatus.value)
      return packStatus.value.Completed.action;
    return null;
  });

  const startPack = async (modelDirs: string[]) => {
    packStatus.value = null;
    const result = await commands.packModels(modelDirs);
    if (result.status === "ok") packJobId.value = result.data;
    return result;
  };

  const startUnpack = async (modelDirs: string[]) => {
    packStatus.value = null;
    const result = await commands.unpackModels(modelDirs);
    if (result.status === "ok") packJobId.value = result.data;
    return result;
  };

  const cancelPack = async () => {
    if (packJobId.value) await commands.cancelCatalogJob(packJobId.value);
  };

  return {
    isPacking,
    packProgress,
    packError,
    packSummary,
    packCancelled,
    packAction,
    packFinishedCount,
    startPack,
    startUnpack,
    cancelPack,
  };
}
