import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  commands,
  events,
  type DuplicateStatus,
  type ScanStatus,
} from "../bindings";

/**
 * Tracks the catalog's background jobs (disk scan + duplicate analysis)
 * driven by the Rust scan-status / duplicate-status event streams.
 */
export function useCatalogJobs() {
  const scanStatus = ref<ScanStatus | null>(null);
  const scanJobId = ref("");
  const dupStatus = ref<DuplicateStatus | null>(null);
  const dupJobId = ref("");
  /** Bumped when a scan finishes so views can refresh their queries. */
  const scanCompletedCount = ref(0);
  const dupCompletedCount = ref(0);

  let unlistenScan: UnlistenFn | null = null;
  let unlistenDup: UnlistenFn | null = null;

  onMounted(async () => {
    unlistenScan = await events.scanStatus.listen((event) => {
      scanStatus.value = event.payload;
      if (
        "Completed" in event.payload ||
        "Failed" in event.payload ||
        "Cancelled" in event.payload
      ) {
        scanJobId.value = "";
        if ("Completed" in event.payload) scanCompletedCount.value++;
      }
    });
    unlistenDup = await events.duplicateStatus.listen((event) => {
      dupStatus.value = event.payload;
      if (
        "Completed" in event.payload ||
        "Failed" in event.payload ||
        "Cancelled" in event.payload
      ) {
        dupJobId.value = "";
        if ("Completed" in event.payload) dupCompletedCount.value++;
      }
    });
  });

  onUnmounted(() => {
    unlistenScan?.();
    unlistenDup?.();
  });

  const isScanning = computed(
    () =>
      !!scanJobId.value ||
      (scanStatus.value !== null &&
        ("Started" in scanStatus.value || "Progress" in scanStatus.value)),
  );

  const isFindingDuplicates = computed(
    () =>
      !!dupJobId.value ||
      (dupStatus.value !== null &&
        ("Started" in dupStatus.value || "Progress" in dupStatus.value)),
  );

  const scanProgress = computed(() =>
    scanStatus.value && "Progress" in scanStatus.value
      ? scanStatus.value.Progress
      : null,
  );

  const scanError = computed(() =>
    scanStatus.value && "Failed" in scanStatus.value
      ? scanStatus.value.Failed.error
      : null,
  );

  const dupProgress = computed(() =>
    dupStatus.value && "Progress" in dupStatus.value
      ? dupStatus.value.Progress
      : null,
  );

  const dupSummary = computed(() =>
    dupStatus.value && "Completed" in dupStatus.value
      ? dupStatus.value.Completed
      : null,
  );

  const startScan = async (root: string) => {
    scanStatus.value = null;
    const result = await commands.startCatalogScan(root);
    if (result.status === "ok") scanJobId.value = result.data;
    return result;
  };

  const startDuplicateScan = async () => {
    dupStatus.value = null;
    const result = await commands.startDuplicateScan();
    if (result.status === "ok") dupJobId.value = result.data;
    return result;
  };

  const cancelScan = async () => {
    if (scanJobId.value) await commands.cancelCatalogJob(scanJobId.value);
  };

  const cancelDuplicateScan = async () => {
    if (dupJobId.value) await commands.cancelCatalogJob(dupJobId.value);
  };

  return {
    isScanning,
    scanProgress,
    scanError,
    scanCompletedCount,
    startScan,
    cancelScan,
    isFindingDuplicates,
    dupProgress,
    dupSummary,
    dupCompletedCount,
    startDuplicateScan,
    cancelDuplicateScan,
  };
}
