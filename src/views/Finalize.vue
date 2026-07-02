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
        <div class="text-sm text-gray-600 mt-2" v-if="!isCompressing"></div>
      </form>
    </template>

    <template #right>
      <CompressionStatus @reset="resetStatus" />
    </template>
  </View>
</template>

<script setup lang="ts">
import { commands } from "../bindings.ts";
import CompressionStatus from "../components/CompressionStatus.vue";
import ModelOverview from "../components/ModelOverview.vue";
import View from "../components/View.vue";
import { useCompressionStatus } from "../composables/useCompressionStatus";
import { useReleasesStore } from "../stores/releasesStore";
import { useToastStore } from "../stores/toastStore.ts";

const toastStore = useToastStore();
const { release, modelCount, releaseDir } = useReleasesStore();
const { activeJobId, isCompressing, resetStatus } = useCompressionStatus();

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
