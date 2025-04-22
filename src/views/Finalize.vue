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
</View>
</template>

<script setup lang="ts">
import { useToastStore } from "../stores/toastStore.ts";
import { commands } from "../bindings.ts";
import { useReleasesStore } from "../stores/releasesStore";
import View from "../components/View.vue";
import ModelOverview from "../components/ModelOverview.vue";

const toastStore = useToastStore();
const { release, clearRelease, modelCount, releaseDir } = useReleasesStore();

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
