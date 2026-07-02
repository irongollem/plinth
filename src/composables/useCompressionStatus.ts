import { computed, ref } from "vue";
import type {
  CancelledStatus,
  CompletedStatus,
  CompressionStatus,
  FailedStatus,
  ProgressStatus,
  StartedStatus,
} from "../bindings";
import { formatBytes, formatTime } from "../utils/format";

// Module-scoped so every component using this composable shares ONE status:
// Finalize.vue starts the job and CompressionStatus.vue receives the events;
// with per-call refs they would each track their own copy and disagree.
const compressionStatus = ref<CompressionStatus | null>(null);
const activeJobId = ref<string>("");

// Type guards (pure — defined once at module scope)
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

/** The job_id carried by any CompressionStatus event. */
export const compressionJobId = (status: CompressionStatus): string => {
  if (isStartedStatus(status)) return status.Started.job_id;
  if (isProgressStatus(status)) return status.Progress.job_id;
  if (isCompletedStatus(status)) return status.Completed.job_id;
  if (isFailedStatus(status)) return status.Failed.job_id;
  return status.Cancelled.job_id;
};

const isCompressing = computed(() => {
  return (
    !!activeJobId.value ||
    (compressionStatus.value !== null &&
      isProgressStatus(compressionStatus.value))
  );
});

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

export function useCompressionStatus() {
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
  };
}
