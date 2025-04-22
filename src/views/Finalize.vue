<template>
<button
    class="btn btn-success"
    @click="finalizeRelease"
    :disabled="!modelCount"
>
    Finalize & Export Release
</button>
</template>

<script setup lang="ts">
import { useToastStore } from "../stores/toastStore.ts";
import { commands } from "../bindings.ts";
import { useReleasesStore } from "../stores/releasesStore";

const toastStore = useToastStore();
const { release, clearRelease, modelCount } = useReleasesStore();

const finalizeRelease = async () => {
  if (release) {
    try {
      const result = await commands.finalizeRelease(release.release_dir);
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
