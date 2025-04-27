<template>
<View>
  <template #left>
  <form @submit.prevent="finalizeRelease">
    <ModelOverview v-if="release && release.model_references.length > 0" />
    <button class="btn btn-success" :disabled="!modelCount">
        Finalize & Export Release
    </button>
    {{ releaseDir }}
  </form>
  </template>
  <template v-if="progress?.percent_size" #right>
    <ProgressBar :progress="progress?.percent_size" />
    {{progress.processed_files}}/{{progress.total_files}} files compressed
    <br/>
    {{progress.processed_size}}/{{progress.total_size}} bytes compressed
  </template>
</View>
</template>

<script setup lang="ts">
import { commands, type CompressionProgessEvent, events } from "../bindings.ts";
import { useToastStore } from "../stores/toastStore.ts";
import { useReleasesStore } from "../stores/releasesStore";
import View from "../components/View.vue";
import ModelOverview from "../components/ModelOverview.vue";
import ProgressBar from "../components/ProgressBar.vue";
import { onMounted, onUnmounted, ref } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";

const toastStore = useToastStore();
const { release, clearRelease, modelCount, releaseDir } = useReleasesStore();

const progress = ref<CompressionProgessEvent>();
const listenerTracker = ref<UnlistenFn>();

onMounted(async () => {
  listenerTracker.value = await events.compressionProgessEvent.listen(
    (event) => {
      const progressLocal = event.payload;
      progress.value = progressLocal;
    },
  );
});

onUnmounted(() => {
  listenerTracker.value?.();
});

const finalizeRelease = async () => {
  if (releaseDir) {
    try {
      const result = await commands.finalizeRelease(releaseDir);
      if (result.status === "ok") {
        clearRelease();
        toastStore.addToast(
          "Release finalized and exported successfully",
          "success",
        );
      } else {
        console.error("Finalize release failed:", result.error);
        toastStore.addToast(
          `Failed to finalize release: ${
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
</script>
