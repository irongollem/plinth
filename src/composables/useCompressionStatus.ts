import { computed, ref } from "vue";
import type {
  CancelledStatus,
  CompletedStatus,
  CompressionStatus,
  FailedStatus,
  ProgressStatus,
  StartedStatus,
} from "../bindings";

// Module-scoped so every component using this composable shares ONE status:
// Finalize.vue starts the job and CompressionStatus.vue receives the events;
// with per-call refs they would each track their own copy and disagree.
const compressionStatus = ref<CompressionStatus | null>(null);
const activeJobId = ref<string>("");

export function useCompressionStatus() {
  // Type guards
  const isStartedStatus = (
    status: CompressionStatus,
  ): status is { Started: StartedStatus } => "Started" in status;

  const isProgressStatus = (
    status: CompressionStatus,
  ): status is { Progress: ProgressStatus } => "Progress" in status;

  const isCompletedStatus = (
    status: CompressionStatus,
  ): status is { Completed: CompletedStatus } => "Completed" in status;

  const isFailedStatus = (
    status: CompressionStatus,
  ): status is { Failed: FailedStatus } => "Failed" in status;

  const isCancelledStatus = (
    status: CompressionStatus,
  ): status is { Cancelled: CancelledStatus } => "Cancelled" in status;

  const getStartedData = (
    status: CompressionStatus | null,
  ): StartedStatus | null =>
    status && isStartedStatus(status) ? status.Started : null;

  const getProgressData = (
    status: CompressionStatus | null,
  ): ProgressStatus | null =>
    status && isProgressStatus(status) ? status.Progress : null;

  const getCompletedData = (
    status: CompressionStatus | null,
  ): CompletedStatus | null =>
    status && isCompletedStatus(status) ? status.Completed : null;

  const getFailedData = (
    status: CompressionStatus | null,
  ): FailedStatus | null =>
    status && isFailedStatus(status) ? status.Failed : null;

  // Computed property to check if compression is in progress
  const isCompressing = computed(() => {
    return (
      !!activeJobId.value ||
      (compressionStatus.value !== null &&
        isProgressStatus(compressionStatus.value))
    );
  });

  // Reset the status
  const resetStatus = () => {
    compressionStatus.value = null;
    activeJobId.value = "";
  };

  const getStatus = () => {
    if (!compressionStatus.value) return "None";
    if (isStartedStatus(compressionStatus.value)) return "Started";
    if (isProgressStatus(compressionStatus.value)) return "Progress";
    if (isCompletedStatus(compressionStatus.value)) return "Completed";
    if (isFailedStatus(compressionStatus.value)) return "Failed";
    if (isCancelledStatus(compressionStatus.value)) return "Cancelled";
  };

  // Get user-friendly title based on current status
  const getStatusTitle = () => {
    switch (getStatus()) {
      case "None":
        return "Ready to Compress";
      case "Started":
        return "Starting Compression...";
      case "Progress":
        return "Compressing Files...";
      case "Completed":
        return "Compression Complete";
      case "Failed":
        return "Compression Failed";
      case "Cancelled":
        return "Compression Cancelled";
    }
  };

  // Format bytes to human-readable string
  const formatBytes = (kb: number) => {
    if (kb === 0) return "0 KB";

    const sizes = ["KB", "MB", "GB"];
    const i = Math.floor(Math.log(kb) / Math.log(1024));
    return `${(kb / 1024 ** i).toFixed(2)} ${sizes[i]}`;
  };

  // Format seconds to minutes and seconds string
  const formatTime = (seconds: number) => {
    const mins = Math.floor(seconds / 60);
    const secs = Math.floor(seconds % 60);
    return `${mins}m ${secs}s`;
  };

  return {
    compressionStatus,
    activeJobId,
    isCompressing,
    resetStatus,
    getStatus,
    getStatusTitle,
    formatBytes,
    formatTime,
    // Type guards
    isStartedStatus,
    isProgressStatus,
    isCompletedStatus,
    isFailedStatus,
    isCancelledStatus,
    // Data accessors
    getStartedData,
    getProgressData,
    getCompletedData,
    getFailedData,
  };
}
