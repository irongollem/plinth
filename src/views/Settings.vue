<template>
  <main class="h-full overflow-y-auto p-7">
    <div class="max-w-150 flex flex-col gap-4">
      <div class="font-bold text-[17px]">Settings</div>

      <div class="flex flex-col gap-1.5">
        <FileSelect
          id="catalog_root"
          label="Catalog root — scanned for models"
          dir-mode
          v-model="settings.catalog_root"
          tooltip="The folder scanned to build your catalog."
        />
      </div>

      <div class="flex flex-col gap-1.5">
        <FileSelect
          id="scratch_dir"
          label="Temporary files directory"
          dir-mode
          v-model="settings.scratch_dir"
          tooltip="Your files will be temporarily stored here before being compressed."
        />
        <span class="text-[10.5px] text-base-content/40"
          >Files are staged here before compression.</span
        >
      </div>

      <div class="flex flex-col gap-1.5">
        <FileSelect
          id="target_dir"
          label="Target directory — finished .3pk releases"
          dir-mode
          v-model="settings.target_dir"
          tooltip="Your compressed files will be saved here."
        />
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >BLENDER LOCATION</span
        >
        <div
          class="flex items-center gap-2.5 bg-base-200 border border-base-content/10 rounded-lg px-2.5 py-1.5"
        >
          <span
            class="font-mono text-[12px] text-base-content/60 flex-1 truncate"
          >
            {{
              settings.blender_path ||
              "Auto-detect (PATH, /Applications, BLENDER_BIN)"
            }}
          </span>
          <button type="button" class="btn btn-xs" @click="browseBlender">
            Browse…
          </button>
          <button type="button" class="btn btn-xs" @click="checkBlender">
            Detect
          </button>
        </div>
        <p
          v-if="blenderStatus"
          class="text-[10.5px] font-mono"
          :class="blenderFound ? 'text-success' : 'text-error'"
        >
          {{ blenderStatus }}
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >MAX COMPRESSION THREADS — {{ availableCores }} CORES DETECTED</span
        >
        <div class="flex items-center gap-3">
          <input
            id="max_compression_threads"
            type="range"
            min="1"
            :max="availableCores"
            v-model.number="settings.max_compression_threads"
            class="range range-primary range-sm flex-1"
          />
          <span class="font-mono font-semibold text-[13px] w-6 text-right">
            {{ settings.max_compression_threads || "Auto" }}
          </span>
        </div>
        <p class="text-[10.5px] text-base-content/40">
          Lower for better system responsiveness, higher for faster compression.
          Default is automatically calculated ({{ defaultThreadCount }}).
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >KNOWN DESIGNERS — RECOGNIZED IN FOLDER NAMES WHEN SCANNING</span
        >
        <div
          class="flex flex-wrap gap-1.5 items-center bg-base-200 border border-base-content/10 rounded-lg p-2"
        >
          <span
            v-for="designer in settings.known_designers ?? []"
            :key="designer"
            class="font-mono text-[11px] text-base-content/70 border border-base-content/15 rounded-full px-2.5 py-0.5 flex items-center gap-1"
          >
            {{ designer }}
            <button
              type="button"
              class="opacity-50 hover:opacity-100"
              @click="removeDesigner(designer)"
            >
              ✕
            </button>
          </span>
          <form class="join" @submit.prevent="addDesigner">
            <input
              v-model="newDesigner"
              type="text"
              class="input input-xs join-item w-40 font-mono"
              placeholder="+ add designer"
            />
          </form>
        </div>
        <p class="text-[10.5px] text-base-content/40">
          Infers a model's designer from its folder path when there's no release
          metadata. Matching ignores case, spaces and punctuation. Applies on
          the next scan.
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >PRINT BUTTON</span
        >
        <div
          class="flex gap-1 bg-base-200 border border-base-content/10 rounded-full p-0.75 w-55"
        >
          <button
            type="button"
            class="flex-1 text-center font-semibold text-[11px] py-1.5 rounded-full cursor-pointer"
            :class="
              printAction === 'open-in-slicer'
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="settings.print_action = 'open-in-slicer'"
          >
            Open in slicer
          </button>
          <button
            type="button"
            class="flex-1 text-center font-semibold text-[11px] py-1.5 rounded-full cursor-pointer"
            :class="
              printAction === 'reveal-folder'
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="settings.print_action = 'reveal-folder'"
          >
            Reveal folder
          </button>
        </div>
        <p class="text-[10.5px] text-base-content/40">
          Open in slicer sends the model's files straight to whatever app your
          system opens STL files with. Reveal folder shows them in
          Finder/Explorer instead — handy if you switch between slicers.
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >APPEARANCE</span
        >
        <div
          class="flex gap-1 bg-base-200 border border-base-content/10 rounded-full p-0.75 w-55"
        >
          <button
            type="button"
            class="flex-1 text-center font-semibold text-[11px] py-1.5 rounded-full cursor-pointer"
            :class="
              themeStore.isDark()
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="themeStore.setDark"
          >
            Dark
          </button>
          <button
            type="button"
            class="flex-1 text-center font-semibold text-[11px] py-1.5 rounded-full cursor-pointer"
            :class="
              !themeStore.isDark()
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="themeStore.setLight"
          >
            Light
          </button>
        </div>
      </div>
    </div>
  </main>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { type Settings, commands } from "../bindings.ts";
import FileSelect from "../components/FileSelect.vue";
import { useFileSelect } from "../composables/useFileSelect";
import { useThemeStore } from "../stores/themeStore";
import { useToastStore } from "../stores/toastStore";

const toastStore = useToastStore();
const { selectFiles } = useFileSelect();
const themeStore = useThemeStore();

const settings = ref<Settings>({
  scratch_dir: null,
  target_dir: null,
  compression_type: "Zip",
  chunk_size: null,
  max_compression_threads: null,
  blender_path: null,
  catalog_root: null,
  known_designers: null,
  print_action: null,
});

// Unset means the default behavior: hand files straight to the slicer
const printAction = computed(
  () => settings.value.print_action ?? "open-in-slicer",
);

/* The scanner's designer lexicon, editable here; seeded server-side with
   sensible defaults. Mutating the array triggers the deep-watch auto-save. */
const newDesigner = ref("");
const addDesigner = () => {
  const name = newDesigner.value.trim();
  newDesigner.value = "";
  if (!name) return;
  const list = settings.value.known_designers ?? [];
  if (!list.some((d) => d.toLowerCase() === name.toLowerCase())) {
    settings.value.known_designers = [...list, name];
  }
};
const removeDesigner = (name: string) => {
  settings.value.known_designers = (
    settings.value.known_designers ?? []
  ).filter((d) => d !== name);
};

const blenderStatus = ref("");
const blenderFound = ref(false);

const browseBlender = async () => {
  const files = await selectFiles({
    multiple: false,
    title: "Select Blender executable",
  });
  if (files?.length) {
    settings.value.blender_path = files[0].path;
    await checkBlender();
  }
};

const checkBlender = async () => {
  blenderStatus.value = "Checking...";
  const result = await commands.detectBlender();
  if (result.status === "ok") {
    blenderFound.value = true;
    blenderStatus.value = `✓ Found ${result.data.version} at ${result.data.path}`;
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

// Watch the settings object itself instead of relying on native form
// events: the directory pickers update the model via a Vue emit, which
// fires no DOM change/blur event, so form listeners never saw them.
const settingsLoaded = ref(false);
watch(
  settings,
  () => {
    if (settingsLoaded.value) debouncedSave();
  },
  { deep: true },
);

onMounted(async () => {
  try {
    const savedSettings = await commands.getSettings();
    if (savedSettings.status === "ok") {
      savedSettings.data.compression_type =
        savedSettings.data.compression_type || "Zip";
      settings.value = savedSettings.data;
      toastStore.addToast("Settings loaded successfully", "success", 3000);
    } else {
      toastStore.reportError("Failed to load settings", savedSettings.error);
    }
  } catch (error) {
    toastStore.reportError("Failed to load settings", error);
  } finally {
    // Enable auto-save only after the initial load has populated the form
    setTimeout(() => {
      settingsLoaded.value = true;
    }, 0);
  }
});

const saveSettings = async () => {
  try {
    const result = await commands.setSettings(settings.value);
    if (result.status === "ok") {
      toastStore.addToast("Settings saved successfully", "success", 3000);
    }
    if (result.status === "error") {
      toastStore.reportError("Failed to save settings", result.error);
    }
  } catch (error) {
    toastStore.reportError("Error saving settings", error);
  }
};
</script>
