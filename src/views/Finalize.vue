<template>
  <View>
    <template #left>
      <form @submit.prevent="finalizeRelease">
        <ModelOverview v-if="release && release.model_references.length > 0" />
        <button
          class="btn btn-success"
          :disabled="!modelCount || isCompressing"
          v-if="!isCompressing"
        >
          Finalize & Export Release
        </button>
        <button
          v-else
          type="button"
          class="btn btn-danger"
          @click="cancelCompression"
        >
          Cancel Compression
        </button>
        <div class="text-sm text-gray-600 mt-2" v-if="!isCompressing">
        </div>
      </form>
    </template>

    <template #right v-if="compressionStatus">
      <div class="p-4 bg-gray-100 rounded-lg">
        <h2 class="text-lg font-semibold mb-4">
          {{ getStatusTitle() }}
        </h2>

        <!-- For active compression -->
        <template v-if="isCompressing && compressionStatus">
          <template v-if="isProgressStatus(compressionStatus)">
            <ProgressBar :progress="compressionStatus.Progress.percent_size" />

            <div class="mt-4 space-y-2 text-sm">
              <div class="flex justify-between">
                <span>Files:</span>
                <span>{{ compressionStatus.Progress.processed_files }}/{{ compressionStatus.Progress.total_files }}</span>
              </div>

              <div class="flex justify-between">
                <span>Size:</span>
                <span>{{ formatBytes(compressionStatus.Progress.processed_size_kb) }}/{{ formatBytes(compressionStatus.Progress.total_size_kb) }}</span>
              </div>

              <div v-if="compressionStatus.Progress.current_file" class="mt-1">
                <span class="text-gray-600">Currently processing:</span>
                <div class="font-mono text-xs truncate bg-gray-200 p-1 mt-1 rounded">
                  {{ compressionStatus.Progress.current_file }}
                </div>
              </div>
            </div>
          </template>
        </template>

        <!-- For completed compression -->
        <template v-else-if="compressionStatus && isCompletedStatus(compressionStatus)">
          <div class="mt-2 p-3 bg-green-100 text-green-800 rounded">
            <div>Successfully compressed {{ compressionStatus.Completed.total_files }} files</div>
            <div>Total size: {{ formatBytes(compressionStatus.Completed.total_size_kb) }}</div>
            <div>Time elapsed: {{ formatTime(compressionStatus.Completed.elapsed_seconds) }}</div>
          </div>
        </template>

        <!-- For failed compression -->
        <template v-else-if="compressionStatus && isFailedStatus(compressionStatus)">
          <div class="mt-2 p-3 bg-red-100 text-red-800 rounded">
            <div class="font-bold">Compression failed</div>
            <div class="mt-1">{{ compressionStatus.Failed.error }}</div>
            <button
              class="btn btn-sm btn-outline mt-3"
              @click="resetStatus"
            >
              Try Again
            </button>
          </div>
        </template>

        <!-- For cancelled compression -->
        <template v-else-if="compressionStatus && isCancelledStatus(compressionStatus)">
          <div class="mt-2 p-3 bg-yellow-100 text-yellow-800 rounded">
            <div>Compression was cancelled.</div>
            <button
              class="btn btn-sm btn-outline mt-3"
              @click="resetStatus"
            >
              Try Again
            </button>
          </div>
        </template>

        <!-- For started status -->
        <template v-else-if="compressionStatus && isStartedStatus(compressionStatus)">
          <div class="mt-2 p-3 bg-blue-100 text-blue-800 rounded">
            <div>Starting compression...</div>
            <div>Files to process: {{ compressionStatus.Started.total_files }}</div>
            <div>Total size: {{ formatBytes(compressionStatus.Started.total_size_kb) }}</div>
          </div>
        </template>
      </div>
    </template>
  </View>
</template>

<script setup lang="ts">
import { commands, events } from "../bindings.ts";
import { useToastStore } from "../stores/toastStore.ts";
import { useReleasesStore } from "../stores/releasesStore";
import { useCompressionStatus } from "../composables/useCompressionStatus";
import View from "../components/View.vue";
import ModelOverview from "../components/ModelOverview.vue";
import ProgressBar from "../components/ProgressBar.vue";
import { onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";

const toastStore = useToastStore();
const { release, clearRelease, modelCount, releaseDir } = useReleasesStore();
const {
  compressionStatus,
  activeJobId,
  isCompressing,
  resetStatus,
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
    compressionStatus.value = event.payload;

    // Auto-clear on completion
    if (compressionStatus.value && isCompletedStatus(compressionStatus.value)) {
      activeJobId.value = "";
      clearRelease();
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

const finalizeRelease = async () => {
  if (releaseDir) {
    try {
      resetStatus();
      const result = await commands.finalizeRelease(releaseDir);
      if (result.status === "ok") {
        activeJobId.value = result.data;
        toastStore.addToast("Compression started", "info");
      } else {
        console.error("Finalize release failed:", result.error);
        toastStore.addToast(
          `Failed to start compression: ${
            typeof result.error === "string"
              ? result.error
              : JSON.stringify(result.error)
          }`,
          "error",
          0,
        );
      }
    } catch (error) {
      console.error("Finalize exception:", error);
      toastStore.addToast(
        `Exception during finalization: ${error}`,
        "error",
        0,
      );
    }
  }
};

const cancelCompression = async () => {
  if (activeJobId.value) {
    try {
      const result = await commands.cancelCompression(activeJobId.value);
      if (result.status === "ok") {
        toastStore.addToast("Cancellation requested", "info");
      } else {
        toastStore.addToast(`Failed to cancel: ${result.error}`, "error");
      }
    } catch (error) {
      toastStore.addToast(`Error cancelling compression: ${error}`, "error");
    }
  }
};
</script>
