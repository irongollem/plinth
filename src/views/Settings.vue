
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

        <div class="form-control mb-2">
          <label class="floating-label" for="blender_path">
            <span class="label">Blender Location</span>
          </label>
          <div class="flex">
            <input
              id="blender_path"
              type="text"
              readonly
              :value="settings.blender_path"
              class="input flex-1"
              placeholder="Auto-detect (PATH, /Applications, BLENDER_BIN)"
            />
            <div
              class="tooltip"
              data-tip="Blender is used to render promo images of your models. Leave empty to auto-detect."
            >
              <button type="button" class="btn" @click="browseBlender">Browse</button>
            </div>
            <button type="button" class="btn" @click="checkBlender">Detect</button>
          </div>
          <p v-if="blenderStatus" class="mt-1 text-xs" :class="blenderFound ? 'text-success' : 'text-error'">
            {{ blenderStatus }}
          </p>
        </div>

        <div class="mb-4">
          <label for="max_compression_threads" class="block text-sm font-medium text-gray-700">
            Max Compression Threads
            <span class="text-xs text-gray-500">(Detected {{ availableCores }} cores)</span>
          </label>
          <div class="mt-1 flex items-center">
            <input
              id="max_compression_threads"
              type="range"
              min="1"
              :max="availableCores"
              v-model.number="settings.max_compression_threads"
              class="mr-2 w-full h-2 rounded-lg appearance-none cursor-pointer bg-gray-200"
              @change="debouncedSave"
            />
            <span class="text-sm text-gray-600">{{ settings.max_compression_threads || 'Auto' }}</span>
          </div>
          <p class="mt-1 text-xs text-gray-500">
            Maximum CPU cores to use for compression. Lower for better system responsiveness, higher for faster compression.
            Default is automatically calculated ({{ defaultThreadCount }}).
          </p>
        </div>
      </form>
    </template>
  </View>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { type Settings, commands } from "../bindings.ts";
import FileSelect from "../components/FileSelect.vue";
import View from "../components/View.vue";
import { useFileSelect } from "../composables/useFileSelect";
import { useToastStore } from "../stores/toastStore";

const toastStore = useToastStore();
const { selectFiles } = useFileSelect();

const settings = ref<Settings>({
  scratch_dir: null,
  target_dir: null,
  compression_type: "Zip",
  chunk_size: null,
  max_compression_threads: null,
  blender_path: null,
});

const blenderStatus = ref("");
const blenderFound = ref(false);

const browseBlender = async () => {
  const files = await selectFiles({
    multiple: false,
    title: "Select Blender executable",
  });
  if (files?.length) {
    settings.value.blender_path = files[0].path;
    await saveSettings();
    await checkBlender();
  }
};

const checkBlender = async () => {
  blenderStatus.value = "Checking...";
  const result = await commands.detectBlender();
  if (result.status === "ok") {
    blenderFound.value = true;
    blenderStatus.value = `Found ${result.data.version} at ${result.data.path}`;
  } else {
    blenderFound.value = false;
    blenderStatus.value =
      "Blender not found. Install Blender 4.x+ or point to its location.";
  }
};

const availableCores = ref(navigator.hardwareConcurrency || 4);
const defaultThreadCount = computed(() =>
  Math.max(1, availableCores.value - 1),
);

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
