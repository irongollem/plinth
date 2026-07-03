<template>
  <div
    class="p-4 bg-base-100 border-1 border-base-content/15 rounded-lg flex flex-col items-center"
  >
    <h2 class="text-lg font-semibold mb-4">
      {{ getStatusTitle() }}
    </h2>

    <!-- For active compression -->
    <template v-if="compressionStatus && isProgressStatus(compressionStatus)">
      <ProgressBar :progress="compressionStatus.Progress.percent_size" />

      <div class="space-y-2 w-full">
        <div class="flex justify-between">
          <span>Files:</span>
          <span>
            {{ compressionStatus.Progress.processed_files }}
            /
            {{ compressionStatus.Progress.total_files }}
          </span>
        </div>

        <div class="flex justify-between">
          <span>Size:</span>
          <span>
            {{ formatBytes(compressionStatus.Progress.processed_size_kb) }}
            /
            {{ formatBytes(compressionStatus.Progress.total_size_kb) }}
          </span>
        </div>

        <div
          v-if="compressionStatus.Progress.current_file"
          class="flex justify-between"
        >
          <span>Currently processing:</span>
          <span>
            {{ compressionStatus.Progress.current_file }}
          </span>
        </div>
      </div>
    </template>

    <!-- For completed compression -->
    <template
      v-else-if="compressionStatus && isCompletedStatus(compressionStatus)"
    >
      <div class="space-y-2 w-full">
        <div class="flex justify-between">
          <span>Files:</span>
          <span>
            {{ compressionStatus.Completed.total_files }}
          </span>
        </div>
        <div class="flex justify-between">
          <span>Total size:</span>
          <span>
            {{ formatBytes(compressionStatus.Completed.total_size_kb) }}
          </span>
        </div>
        <div class="flex justify-between">
          <span>Time elapsed:</span>
          <span>
            {{ formatTime(compressionStatus.Completed.elapsed_seconds) }}
          </span>
        </div>
        <div class="flex justify-center mt-4">
          <button class="btn btn-sm btn-primary" @click="openCompletedFolder">
            Open Folder
          </button>
        </div>
      </div>
    </template>

    <!-- For failed compression -->
    <template
      v-else-if="compressionStatus && isFailedStatus(compressionStatus)"
    >
      <div class="text-red-800 w-full text-center">
        <div class="mb-2">{{ compressionStatus.Failed.error }}</div>
        <button class="btn btn-sm btn-error" @click="$emit('reset')">
          Try Again
        </button>
      </div>
    </template>

    <!-- For cancelled compression -->
    <template
      v-else-if="compressionStatus && isCancelledStatus(compressionStatus)"
    >
      <div class="w-full text-center">
        <button class="btn btn-sm btn-secondary" @click="$emit('reset')">
          Try Again
        </button>
      </div>
    </template>

    <!-- For started status -->
    <template
      v-else-if="compressionStatus && isStartedStatus(compressionStatus)"
    >
      <div class="mt-2 p-3 bg-blue-100 text-blue-800 rounded">
        <div>Starting compression...</div>
        <div>Files to process: {{ compressionStatus.Started.total_files }}</div>
        <div>
          Total size: {{ formatBytes(compressionStatus.Started.total_size_kb) }}
        </div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import type { UnlistenFn } from "@tauri-apps/api/event";
import { openPath } from "@tauri-apps/plugin-opener";
import { onMounted, onUnmounted, ref } from "vue";
import { events } from "../bindings";
import {
  compressionJobId,
  useCompressionStatus,
} from "../composables/useCompressionStatus";
import { useToastStore } from "../stores/toastStore";
import ProgressBar from "./ProgressBar.vue";

const emit = defineEmits<{
  (e: "reset"): void;
  (e: "completed"): void;
}>();

const toastStore = useToastStore();
const {
  compressionStatus,
  activeJobId,
  getStatusTitle,
  formatBytes,
  formatTime,
  isStartedStatus,
  isProgressStatus,
  isCompletedStatus,
  isFailedStatus,
  isCancelledStatus,
} = useCompressionStatus();

const listenerTracker = ref<UnlistenFn>();

onMounted(async () => {
  listenerTracker.value = await events.compressionStatus.listen((event) => {
    // Ignore stragglers from an earlier job (e.g. cancel then restart):
    // without this, job 1's late Cancelled/Completed would clear job 2's
    // state and toast the wrong outcome
    if (
      activeJobId.value &&
      compressionJobId(event.payload) !== activeJobId.value
    ) {
      return;
    }
    compressionStatus.value = event.payload;

    // Auto-clear on completion
    if (compressionStatus.value && isCompletedStatus(compressionStatus.value)) {
      activeJobId.value = "";
      emit("completed");
      toastStore.addToast(
        "Release finalized and exported successfully",
        "success",
      );
    }

    // Handle errors
    if (compressionStatus.value && isFailedStatus(compressionStatus.value)) {
      activeJobId.value = "";
      toastStore.addToast(
        `Compression failed: ${compressionStatus.value.Failed.error}`,
        "error",
        0,
      );
    }

    // Handle cancellation
    if (compressionStatus.value && isCancelledStatus(compressionStatus.value)) {
      activeJobId.value = "";
      toastStore.addToast("Compression was cancelled", "info");
    }
  });
});

onUnmounted(() => {
  if (listenerTracker.value) {
    listenerTracker.value();
  }
});

const openCompletedFolder = async () => {
  if (
    compressionStatus.value &&
    isCompletedStatus(compressionStatus.value) &&
    compressionStatus.value.Completed.folder_path
  ) {
    try {
      await openPath(compressionStatus.value.Completed.folder_path);
    } catch (error) {
      console.error("Failed to open folder:", error);
    }
  }
};
</script>
