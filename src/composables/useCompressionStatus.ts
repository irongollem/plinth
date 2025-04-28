import type {
  CompressionStatus,
  StartedStatus,
  ProgressStatus,
  CompletedStatus,
  FailedStatus,
  CancelledStatus,
} from "../bindings";
import { ref, computed } from "vue";

export function useCompressionStatus() {
  const compressionStatus = ref<CompressionStatus | null>(null);
  const activeJobId = ref<string>("");

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

  // Get user-friendly title based on current status
  const getStatusTitle = () => {
    if (!compressionStatus.value) return "Ready to Compress";

    if (isStartedStatus(compressionStatus.value))
      return "Starting Compression...";
    if (isProgressStatus(compressionStatus.value))
      return "Compressing Files...";
    if (isCompletedStatus(compressionStatus.value))
      return "Compression Complete";
    if (isFailedStatus(compressionStatus.value)) return "Compression Failed";
    if (isCancelledStatus(compressionStatus.value))
      return "Compression Cancelled";

    return "Ready to Compress";
  };

  // Format bytes to human-readable string
  const formatBytes = (bytes: number) => {
    if (bytes === 0) return "0 Bytes";

    const sizes = ["Bytes", "KB", "MB", "GB"];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    return `${(bytes / 1024 ** i).toFixed(2)} ${sizes[i]}`;
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
