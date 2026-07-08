<template>
  <main class="h-full overflow-y-auto p-7">
    <div class="max-w-150 flex flex-col gap-4">
      <div class="font-bold text-[17px]">Settings</div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >CATALOG FOLDERS</span
        >
        <div
          class="flex flex-col gap-1 bg-base-200 border border-base-content/10 rounded-lg px-2.5 py-1.5"
        >
          <span
            v-for="root in catalogRoots"
            :key="root"
            class="font-mono text-[12px] text-base-content/60 truncate"
            :title="root"
            >{{ root
            }}<span
              v-if="root === settings.catalog_primary_root"
              class="text-warning"
              title="Primary — Clean up moves every folder's models into this one"
            >
              ★ primary</span
            ></span
          >
          <span
            v-if="!catalogRoots.length"
            class="font-mono text-[12px] text-base-content/40"
            >No folders yet</span
          >
        </div>
        <span class="text-[10.5px] text-base-content/40"
          >Add, scan, and remove folders from the Catalog tab — one designer
          folder at a time works best for huge collections.</span
        >
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
          <button
            v-if="verdict && verdict !== 'Ok' && !isDownloading"
            type="button"
            class="btn btn-xs btn-primary"
            @click="startDownload"
          >
            Download {{ managedVersion }}
          </button>
        </div>
        <div v-if="isDownloading" class="flex items-center gap-2">
          <progress
            class="progress progress-primary flex-1"
            :value="downloadPercent"
            max="100"
          ></progress>
          <span class="font-mono text-[10px] text-base-content/50">{{
            downloadPhase ? `${downloadPhase}…` : `${downloadPercent}%`
          }}</span>
        </div>
        <p
          v-if="blenderStatusText"
          class="text-[10.5px] font-mono"
          :class="blenderStatusClass"
        >
          {{ blenderStatusText }}
        </p>
        <p v-if="downloadError" class="text-[10.5px] font-mono text-error">
          {{ downloadError }}
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
          Open in slicer lets you tick which of the model's files to send to
          whatever app your system opens them with (pre-sliced scenes are
          pre-selected). Reveal folder shows them in Finder/Explorer instead —
          handy if you switch between slicers.
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >PACKED MODELS</span
        >
        <div
          class="flex gap-1 bg-base-200 border border-base-content/10 rounded-full p-0.75 w-55"
        >
          <button
            type="button"
            class="flex-1 text-center font-semibold text-[11px] py-1.5 rounded-full cursor-pointer"
            :class="
              packCleanup
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="settings.pack_cleanup_after = true"
          >
            Clean up after use
          </button>
          <button
            type="button"
            class="flex-1 text-center font-semibold text-[11px] py-1.5 rounded-full cursor-pointer"
            :class="
              !packCleanup
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="settings.pack_cleanup_after = false"
          >
            Keep extracted
          </button>
        </div>
        <label class="flex items-center gap-2 text-[11px]">
          <span class="text-base-content/60">Compression level</span>
          <input
            :value="settings.pack_level ?? 3"
            type="number"
            min="1"
            max="19"
            class="input input-xs w-16 font-mono"
            @change="
              settings.pack_level =
                Number.parseInt(
                  ($event.target as HTMLInputElement).value,
                  10,
                ) || null
            "
          />
          <span class="text-base-content/40">zstd, default 3</span>
        </label>
        <p class="text-[10.5px] text-base-content/40">
          Printing or previewing a packed model extracts just the needed files
          from its archive. Clean up after use removes those temporary copies
          again once the action is done — the closest thing to printing straight
          from the bundle. Higher compression levels pack smaller but slower;
          extraction speed is unaffected.
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >SCALE FIGURE</span
        >
        <div
          class="flex items-center gap-2.5 bg-base-200 border border-base-content/10 rounded-lg px-2.5 py-1.5"
        >
          <span
            class="font-mono text-[12px] text-base-content/60 flex-1 truncate"
          >
            {{ settings.scale_reference_path || "No figure chosen" }}
          </span>
          <button type="button" class="btn btn-xs" @click="browseScaleRef">
            Browse…
          </button>
          <button
            v-if="settings.scale_reference_path"
            type="button"
            class="btn btn-xs btn-ghost"
            @click="settings.scale_reference_path = null"
          >
            clear
          </button>
        </div>
        <label class="flex items-center gap-2 text-[11px]">
          <span class="text-base-content/60">Stands</span>
          <input
            :value="settings.scale_reference_height_mm ?? 28"
            type="number"
            min="1"
            max="500"
            step="0.5"
            class="input input-xs w-16 font-mono"
            @change="
              settings.scale_reference_height_mm =
                Number.parseFloat(($event.target as HTMLInputElement).value) ||
                null
            "
          />
          <span class="text-base-content/40">mm tall next to the model</span>
        </label>
        <p class="text-[10.5px] text-base-content/40">
          A reference figure rendered in grey beside your model at true relative
          size — the "banana for scale". Any STL works (a 28&nbsp;mm standing
          person reads best); toggle it per render in the studio.
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >CREATOR LICENCE</span
        >
        <div
          class="flex items-center gap-2.5 bg-base-200 border border-base-content/10 rounded-lg px-2.5 py-1.5"
        >
          <span
            class="font-mono text-[12px] text-base-content/60 flex-1 truncate"
          >
            {{ settings.licence_path || "No licence file chosen" }}
          </span>
          <button type="button" class="btn btn-xs" @click="browseLicence">
            Browse…
          </button>
          <button
            v-if="settings.licence_path"
            type="button"
            class="btn btn-xs btn-ghost"
            @click="settings.licence_path = null"
          >
            clear
          </button>
        </div>
        <p class="text-[10.5px] text-base-content/40">
          Your licence terms as a file (PDF, txt, md…). The release builder
          offers to include it in every release you pack — it travels inside the
          release.3pk, named licence, so your customers always receive your
          terms alongside the models.
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
import { useBlenderProvision } from "../composables/useBlenderProvision";
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
  catalog_roots: null,
  catalog_primary_root: null,
  known_designers: null,
  print_action: null,
  release_field_defaults: null,
  pack_level: null,
  pack_cleanup_after: null,
  blender_setup_acknowledged: null,
});

// Display only — the Catalog tab manages the list. Falls back to the
// legacy single root for a store that predates multi-root.
const catalogRoots = computed(
  () =>
    settings.value.catalog_roots ??
    (settings.value.catalog_root ? [settings.value.catalog_root] : []),
);

// Unset means the default behavior: hand files straight to the slicer
const printAction = computed(
  () => settings.value.print_action ?? "open-in-slicer",
);

// Unset means the default: extracted working copies are taken back after use
const packCleanup = computed(() => settings.value.pack_cleanup_after ?? true);

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

// Shared with the first-run dialog and the Render tab — one verdict, three
// surfaces. The status line derives from it, so a download finishing (even
// one started elsewhere) updates this tab without a manual re-detect.
const {
  check: blenderCheck,
  checking: blenderChecking,
  verdict,
  managedVersion,
  runCheck,
  isDownloading,
  percent: downloadPercent,
  phase: downloadPhase,
  errorMessage: downloadError,
  startDownload,
} = useBlenderProvision();

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

const browseScaleRef = async () => {
  const files = await selectFiles({
    multiple: false,
    title: "Select the scale figure STL",
    accept: ".stl",
  });
  if (files?.length) {
    settings.value.scale_reference_path = files[0].path;
  }
};

const browseLicence = async () => {
  const files = await selectFiles({
    multiple: false,
    title: "Select your licence file",
  });
  if (files?.length) {
    settings.value.licence_path = files[0].path;
  }
};

const checkBlender = async () => {
  await runCheck();
};

const blenderStatusText = computed(() => {
  if (blenderChecking.value) return "Checking...";
  const check = blenderCheck.value;
  if (!check) return "";
  const managed = check.is_managed ? " (managed by stl-pack)" : "";
  if (!check.info)
    return "Blender not found. Download it here or point to an install.";
  switch (check.verdict) {
    case "Outdated":
      return `△ ${check.info.version} works, but previews are tuned for Blender ${check.managed_version}`;
    case "TooOld":
      return `✗ ${check.info.version} is below the 4.2 minimum — rendering is disabled`;
    default:
      return `✓ Found ${check.info.version} at ${check.info.path}${managed}`;
  }
});

const blenderStatusClass = computed(() => {
  switch (verdict.value) {
    case "Ok":
      return "text-success";
    case "Outdated":
      return "text-warning";
    default:
      return "text-error";
  }
});

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
    // The Catalog tab owns the roots list (and the setup dialog owns the
    // Blender acknowledgement) and this tab may have loaded before they
    // wrote — saving the stale copy would drop their changes. Re-read the
    // authoritative values right before writing.
    const fresh = await commands.getSettings();
    const payload =
      fresh.status === "ok"
        ? {
            ...settings.value,
            catalog_root: fresh.data.catalog_root,
            catalog_roots: fresh.data.catalog_roots,
            catalog_primary_root: fresh.data.catalog_primary_root,
            blender_setup_acknowledged: fresh.data.blender_setup_acknowledged,
          }
        : settings.value;
    const result = await commands.setSettings(payload);
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
