
<template>
  <View>
    <template #left>
      <h1 class="text-xl font-bold">Settings</h1>
      <form class="mt-4 space-y-6" @change="debouncedSave">
        <FileSelect
          id="scratch_dir"
          label="Temporary Files Directory"
          dir-mode
          v-model="settings.scratch_dir"
          tooltip="Your files will be temporarily stored here before being compressed."
          @blur="debouncedSave"
          @keydown.enter="debouncedSave"
        />

        <FileSelect
          id="target_dir"
          label="Target Directory"
          dir-mode
          v-model="settings.target_dir"
          tooltip="Your compressed files will be saved here."
          @blur="debouncedSave"
          @keydown.enter="debouncedSave"
        />
      </form>
    </template>
  </View>
</template>

<script setup lang="ts">
import { ref, onMounted } from "vue";
import { commands, type Settings } from "../bindings.ts";
import View from "../components/View.vue";
import FileSelect from "../components/FileSelect.vue";
import { useToastStore } from "../stores/toastStore";

const toastStore = useToastStore();

const settings = ref<Settings>({
  scratch_dir: null,
  target_dir: null,
  compression_type: "Zip",
  chunk_size: null,
});

let saveTimeout: number | null = null;
const debouncedSave = () => {
  if (saveTimeout) clearTimeout(saveTimeout);
  saveTimeout = setTimeout(() => {
    saveSettings();
  }, 500) as unknown as number;
};

onMounted(async () => {
  try {
    const savedSettings = await commands.getSettings();
    if (savedSettings.status === "ok") {
      savedSettings.data.compression_type =
        savedSettings.data.compression_type || "Zip";
      settings.value = savedSettings.data;
      toastStore.addToast("Settings loaded successfully", "success", 3000);
    } else {
      toastStore.addToast(
        `Failed to load settings: ${savedSettings.error}`,
        "error",
        0,
      );
    }
  } catch (error) {
    console.error("Failed to load settings:", error);
    toastStore.addToast(`Failed to load settings: ${error}`, "error", 0);
  }
});

const saveSettings = async () => {
  try {
    const result = await commands.setSettings(settings.value);
    if (result.status === "ok") {
      toastStore.addToast("Settings saved successfully", "success", 3000);
    }
    if (result.status === "error") {
      console.error("Failed to save settings:", result.error);
      toastStore.addToast(
        `Failed to save settings: ${result.error}`,
        "error",
        0,
      );
    }
  } catch (error) {
    console.error("Error saving settings:", error);
    toastStore.addToast(`Error saving settings: ${error}`, "error", 0);
  }
};
</script>
