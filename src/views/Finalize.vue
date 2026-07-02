<template>
  <View>
    <template #left>
      <form @submit.prevent="finalizeRelease">
        <ModelOverview v-if="release && release.model_references.length > 0" />
        <button
          class="btn btn-success"
          :disabled="!modelCount || isCompressing || isStarting"
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
import { storeToRefs } from "pinia";
import { ref } from "vue";
import { commands } from "../bindings.ts";
import CompressionStatus from "../components/CompressionStatus.vue";
import ModelOverview from "../components/ModelOverview.vue";
import View from "../components/View.vue";
import { useCompressionStatus } from "../composables/useCompressionStatus";
import { useReleasesStore } from "../stores/releasesStore";
import { useToastStore } from "../stores/toastStore.ts";

const toastStore = useToastStore();
// storeToRefs keeps these reactive — a plain destructure snapshots the
// values at first mount and never updates (this component is KeepAlive'd)
const { release, modelCount, releaseDir } = storeToRefs(useReleasesStore());
const { activeJobId, isCompressing, resetStatus } = useCompressionStatus();

const isStarting = ref(false);

const finalizeRelease = async () => {
  if (!releaseDir.value) {
    toastStore.addToast(
      "No release directory yet — create a release first.",
      "error",
    );
    return;
  }
  // Guard the invoke round-trip: without it a double-click starts two jobs
  isStarting.value = true;
  try {
    resetStatus();
    const result = await commands.finalizeRelease(releaseDir.value);
    if (result.status === "ok") {
      activeJobId.value = result.data;
      toastStore.addToast("Compression started", "info");
    } else {
      toastStore.reportError("Failed to start compression", result.error);
    }
  } catch (error) {
    toastStore.reportError("Exception during finalization", error);
  } finally {
    isStarting.value = false;
  }
};

const cancelCompression = async () => {
  if (activeJobId.value) {
    try {
      const result = await commands.cancelCompression(activeJobId.value);
      if (result.status === "ok") {
        toastStore.addToast("Cancellation requested", "info");
      } else {
        toastStore.reportError("Failed to cancel", result.error);
      }
    } catch (error) {
      toastStore.reportError("Error cancelling compression", error);
    }
  }
};
</script>
