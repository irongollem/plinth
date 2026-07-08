<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { openPath } from "@tauri-apps/plugin-opener";
import { storeToRefs } from "pinia";
import { computed, onMounted, ref, watch } from "vue";
import {
  type Release,
  type ReleaseDraftSummary,
  type ReleaseSummary,
  commands,
} from "../bindings.ts";
import CompressionStatus from "../components/CompressionStatus.vue";
import FileSelect from "../components/FileSelect.vue";
import Switch from "../components/Switch.vue";
import TextArea from "../components/TextArea.vue";
import TextInput from "../components/TextInput.vue";
import MonthYearInput from "../components/MonthYearInput.vue";
import { useCompressionStatus } from "../composables/useCompressionStatus";
import {
  filesFromPaths,
  type SelectedFile,
} from "../composables/useFileSelect";
import { useOS } from "../composables/useOS";
import {
  type DraftReleaseModel,
  type ReleaseStep,
  useReleasesStore,
} from "../stores/releasesStore.ts";
import { useToastStore } from "../stores/toastStore.ts";
import { logoForFileName } from "../types.ts";
import { formatFileSize } from "../utils/format";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { release, releaseDir, modelCount } = storeToRefs(releasesStore);
const { fileExplorerName } = useOS();
const { activeJobId, isCompressing, resetStatus, compressionStatus } =
  useCompressionStatus();

const stepDefs: { step: ReleaseStep; label: string }[] = [
  { step: 1, label: "Models" },
  { step: 2, label: "Release details" },
  { step: 3, label: "Pack" },
];
const stepState = (step: ReleaseStep) => {
  const active = releasesStore.releaseStep === step;
  const done = step < releasesStore.releaseStep;
  const reachable = step === 1 || modelCount.value > 0;
  return { active, done, reachable };
};

/* ---------------- left rail: past releases (read-only, from catalog scans) --------------- */
const pastReleases = ref<ReleaseSummary[]>([]);
onMounted(async () => {
  const result = await commands.getCatalogReleases();
  if (result.status === "ok") pastReleases.value = result.data;
});

/* ---------------- left rail: unfinished drafts sitting in the scratch dir ---------------
   A release directory only exists after step 2 (createRelease), so anything
   here got that far but never finished packing — crash, quit, or a
   localStorage snapshot that didn't survive. Reading it back needs the
   backend: release.json only carries {id, path} per model, the actual
   curation lives in each model's own model.json sidecar. */
const releaseDrafts = ref<ReleaseDraftSummary[]>([]);
const loadDrafts = async () => {
  const result = await commands.listReleaseDrafts();
  if (result.status === "ok") releaseDrafts.value = result.data;
};
onMounted(loadDrafts);

// A successful pack deletes the scratch folder (compression_jobs) — drop it
// from the resume list without waiting for the next full remount
watch(compressionStatus, (status) => {
  if (status && "Completed" in status) loadDrafts();
});

// The active draft's own folder (if it made it past step 2) shouldn't also
// show up as a separate "resume" option
const resumableDrafts = computed(() =>
  releaseDrafts.value.filter((d) => d.release_dir !== releaseDir.value),
);

const isResumingDraft = ref(false);
const resumeDraft = async (draft: ReleaseDraftSummary) => {
  isResumingDraft.value = true;
  try {
    const result = await commands.loadReleaseDraft(draft.release_dir);
    if (result.status !== "ok") throw result.error;
    const [loadedRelease, loadedModels] = result.data;
    releasesStore.loadDraft(loadedRelease, loadedModels, draft.release_dir);
    toastStore.addToast(
      `Resumed "${draft.name}" (${loadedModels.length} of ${draft.model_count} models restored)`,
      loadedModels.length < draft.model_count ? "warning" : "info",
    );
  } catch (error) {
    toastStore.reportError("Failed to resume draft", error);
  } finally {
    isResumingDraft.value = false;
  }
};

/* ---------------------------- step 1: release info ---------------------------- */
/** MonthYearInput format ("M/YYYY") — new releases default to this month. */
const currentMonthYear = () =>
  `${new Date().getMonth() + 1}/${new Date().getFullYear()}`;

/* Field values the user checked "remember" on (persisted in settings),
   keyed by field id. Today that's just the designer name. */
const fieldDefaults = ref<Partial<Record<string, string>>>({});
const rememberDesigner = ref(false);

const openOnSave = ref(false);
const releaseForm = ref<Release>({
  name: "",
  designer: "",
  description: "",
  date: currentMonthYear(),
  version: "1.0.0",
  model_references: [],
  groups: [],
  release_dir: "",
  images: [],
  other_files: [],
});
const extraFiles = ref<SelectedFile[]>([]);
const releaseImages = ref<SelectedFile[]>([]);
const isSavingInfo = ref(false);

const formComplete = computed(
  () =>
    releaseForm.value.name &&
    releaseForm.value.designer &&
    releaseForm.value.date,
);

const clearReleaseForm = () => {
  releaseForm.value = {
    name: "",
    designer: fieldDefaults.value.designer ?? "",
    description: "",
    date: currentMonthYear(),
    version: "1.0.0",
    model_references: [],
    groups: [],
    release_dir: "",
    images: [],
    other_files: [],
  };
  extraFiles.value = [];
  releaseImages.value = [];
};

/* Recover/continue: the staged models live in the store (which snapshots
   itself), but details typed here would die with the window — mirror them
   to localStorage so a restart resumes mid-form. */
const FORM_STORAGE_KEY = "plinth.releaseFormDraft";
watch(
  [releaseForm, extraFiles, releaseImages],
  () => {
    localStorage.setItem(
      FORM_STORAGE_KEY,
      JSON.stringify({
        v: 1,
        form: releaseForm.value,
        extraPaths: extraFiles.value.map((file) => file.path),
        imagePaths: releaseImages.value.map((file) => file.path),
      }),
    );
  },
  { deep: true },
);

onMounted(async () => {
  const settings = await commands.getSettings();
  if (settings.status === "ok") {
    fieldDefaults.value = settings.data.release_field_defaults ?? {};
    rememberDesigner.value = "designer" in fieldDefaults.value;
  }

  // Details already committed to the store (restored draft) win; otherwise
  // fall back to the last unsaved form snapshot.
  if (release.value) {
    releaseForm.value = { ...release.value };
  } else {
    try {
      const raw = localStorage.getItem(FORM_STORAGE_KEY);
      const saved = raw ? JSON.parse(raw) : null;
      if (saved?.v === 1) {
        releaseForm.value = saved.form;
        // Rebuilt from paths: SelectedFile carries a preview method that
        // doesn't survive JSON; files deleted since simply drop out
        extraFiles.value = await filesFromPaths(saved.extraPaths ?? []);
        releaseImages.value = await filesFromPaths(saved.imagePaths ?? []);
      }
    } catch {
      localStorage.removeItem(FORM_STORAGE_KEY);
    }
  }
  if (!releaseForm.value.designer) {
    releaseForm.value.designer = fieldDefaults.value.designer ?? "";
  }
  if (!releaseForm.value.date) releaseForm.value.date = currentMonthYear();
});

/** Sync the remembered designer with the checkbox at save time. */
const persistFieldDefaults = async () => {
  const current = await commands.getSettings();
  if (current.status !== "ok") return;
  const defaults = { ...current.data.release_field_defaults };
  if (rememberDesigner.value && releaseForm.value.designer) {
    defaults.designer = releaseForm.value.designer;
  } else {
    delete defaults.designer;
  }
  fieldDefaults.value = defaults;
  await commands.setSettings({
    ...current.data,
    release_field_defaults: Object.keys(defaults).length ? defaults : null,
  });
};

const startNewDraft = () => {
  releasesStore.clearRelease();
  clearReleaseForm();
};

const saveReleaseInfo = async () => {
  if (!formComplete.value) {
    toastStore.addToast("Please enter a name for the release", "error", 0);
    return;
  }
  isSavingInfo.value = true;
  try {
    // stray whitespace must not reach release.json — these values become
    // folder names and cross-user metadata on pack
    releaseForm.value.name = releaseForm.value.name.trim();
    releaseForm.value.designer = releaseForm.value.designer.trim();
    releaseForm.value.description = releaseForm.value.description.trim();
    await persistFieldDefaults();
    releasesStore.updateRelease(releaseForm.value);
    releasesStore.setReleaseStep(3);
  } catch (error) {
    toastStore.reportError("Failed to save release details", error);
  } finally {
    isSavingInfo.value = false;
  }
};

/* ---------------------- step 1: selected catalog models ----------------------- */
const selectedDraftId = ref<string | null>(null);
const selectedDraft = computed(() =>
  releasesStore.models.find((model) => model.id === selectedDraftId.value),
);
const draftGroups = computed(() => {
  const grouped = new Map<string, DraftReleaseModel[]>();
  for (const model of releasesStore.models) {
    const key = model.source_group ?? model.group ?? model.name;
    grouped.set(key, [...(grouped.get(key) ?? []), model]);
  }
  return [...grouped.entries()].map(([name, variants]) => ({ name, variants }));
});
const removeDraftGroup = (name: string) => {
  releasesStore.models = releasesStore.models.filter(
    (model) => (model.source_group ?? model.group ?? model.name) !== name,
  );
};

/* Release-only tags: same chip UI as the catalog, but edits live in the
   staged draft and never touch the catalog's model_tags. */
const newDraftTag = ref("");
const addDraftTag = () => {
  const draft = selectedDraft.value;
  const tag = newDraftTag.value.trim().toLowerCase().replace(/\s+/g, "_");
  newDraftTag.value = "";
  if (draft && tag && !draft.tags.includes(tag)) draft.tags.push(tag);
};
const removeDraftTag = (tag: string) => {
  const draft = selectedDraft.value;
  if (draft) draft.tags = draft.tags.filter((t) => t !== tag);
};

watch(
  () => releasesStore.models,
  (models) => {
    if (!models.some((model) => model.id === selectedDraftId.value)) {
      selectedDraftId.value = models[0]?.id ?? null;
    }
  },
  { deep: true, immediate: true },
);

/* ---------------------------- step 3: render summary ---------------------------- */
const openRenderStudio = (draft?: DraftReleaseModel) => {
  // model_files can carry slicer scenes (.lys) and other sidecars; the
  // render engine imports STL only, so hand over just those
  const paths = draft?.model_files;
  const stls = paths?.filter((p) => p.toLowerCase().endsWith(".stl")) ?? [];
  if (stls.length) {
    releasesStore.requestRender(stls, undefined, draft?.id?.toString());
  } else {
    if (paths?.length) {
      toastStore.addToast("This model has no .stl parts to render", "warning");
    }
    releasesStore.setActiveTab("render");
  }
};

/* -------------------------------- step 4: finalize -------------------------------- */
const isStartingExport = ref(false);
const totalFileCount = computed(() =>
  releasesStore.models.reduce((sum, m) => sum + m.model_files.length, 0),
);
const totalImageCount = computed(() =>
  releasesStore.models.reduce((sum, m) => sum + m.images.length, 0),
);

const finalizeRelease = async () => {
  if (!release.value) {
    toastStore.addToast("Add the release details before packing.", "error");
    return;
  }
  isStartingExport.value = true;
  try {
    // Disk is the export boundary: no release directory or copied model files
    // are created until the user explicitly chooses to pack.
    if (!releaseDir.value) {
      const stagedModels = [...releasesStore.models];
      const created = await commands.createRelease(
        release.value,
        releaseImages.value.map((image) => image.path),
        extraFiles.value.map((file) => file.path),
      );
      if (created.status !== "ok") throw created.error;

      releasesStore.setReleaseDir(created.data);
      releasesStore.clearModels();
      // One batch call: the backend lays the whole draft out canonically
      // (members sharing a leaf merge — two poses of one model come back
      // as ONE model with file-level poses) and writes release.json once.
      // Draft ids are local "draft-…" strings, not UUIDs — the backend
      // assigns real ids, so they must not cross the boundary. In-place is
      // fine: clearModels() above already detached these from the store.
      for (const staged of stagedModels) staged.id = null;
      const added = await commands.addModels(stagedModels, created.data);
      if (added.status !== "ok") throw added.error;
      for (const [model, sidecarPath] of added.data) {
        releasesStore.addModel(model, sidecarPath);
      }
      if (openOnSave.value) await openPath(created.data);
    }
    resetStatus();
    const result = await commands.finalizeRelease(releasesStore.releaseDir!);
    if (result.status === "ok") {
      activeJobId.value = result.data;
      toastStore.addToast("Compression started", "info");
    } else {
      toastStore.reportError("Failed to start compression", result.error);
    }
  } catch (error) {
    toastStore.reportError("Exception during finalization", error);
  } finally {
    isStartingExport.value = false;
  }
};

const cancelCompression = async () => {
  if (!activeJobId.value) return;
  const result = await commands.cancelCompression(activeJobId.value);
  if (result.status === "ok") {
    toastStore.addToast("Cancellation requested", "info");
  } else {
    toastStore.reportError("Failed to cancel", result.error);
  }
};
</script>

<template>
  <main class="flex h-full min-w-0">
    <!-- release list rail -->
    <div
      class="w-66 shrink-0 border-r border-base-content/10 p-3.5 flex flex-col gap-2 overflow-y-auto"
    >
      <div class="flex items-baseline justify-between px-1 pb-1.5">
        <span class="font-bold text-[15px]">Release builder</span>
        <button
          type="button"
          class="font-semibold text-[11px] text-primary cursor-pointer"
          @click="startNewDraft"
        >
          Reset draft
        </button>
      </div>

      <button
        v-if="modelCount"
        type="button"
        class="text-left bg-base-200 border border-primary rounded-box px-3 py-2.5 cursor-pointer"
        @click="releasesStore.setReleaseStep(releasesStore.releaseStep)"
      >
        <div class="flex gap-1.5 items-center">
          <span
            class="font-mono font-semibold text-[9px] tracking-[0.12em] text-primary"
            >DRAFT</span
          >
          <span class="font-mono text-[10px] text-base-content/40 ml-auto"
            >step {{ releasesStore.releaseStep }} of 3</span
          >
        </div>
        <div class="font-semibold text-[13px] mt-1">
          {{ release?.name || "Untitled release" }}
        </div>
        <div class="font-mono text-[10.5px] text-base-content/60 mt-0.5">
          {{ release?.designer || "—" }} · {{ modelCount }} models
        </div>
      </button>
      <div
        v-else
        class="text-center text-xs text-base-content/40 border border-dashed border-base-content/15 rounded-box px-3 py-6"
      >
        No models yet — choose some from the catalog or add them here.
      </div>

      <div v-if="resumableDrafts.length" class="mt-2 flex flex-col gap-2">
        <span
          class="font-mono text-[9px] tracking-[0.12em] text-base-content/40 px-1"
          >UNFINISHED — NEVER PACKED</span
        >
        <button
          v-for="d in resumableDrafts"
          :key="d.release_dir"
          type="button"
          class="text-left bg-base-200 border border-base-content/10 rounded-box px-3 py-2.5 cursor-pointer hover:border-primary/50 disabled:opacity-50 disabled:cursor-not-allowed"
          :disabled="isResumingDraft"
          @click="resumeDraft(d)"
        >
          <div class="font-semibold text-[13px]">
            {{ d.name || "Untitled release" }}
          </div>
          <div class="font-mono text-[10.5px] text-base-content/60 mt-0.5">
            {{ d.designer || "unknown designer" }} · {{ d.model_count }}
            models
          </div>
        </button>
      </div>

      <div v-if="pastReleases.length" class="mt-2 flex flex-col gap-2">
        <span
          class="font-mono text-[9px] tracking-[0.12em] text-base-content/40 px-1"
          >FROM YOUR CATALOG</span
        >
        <div
          v-for="r in pastReleases"
          :key="r.release_name"
          class="bg-base-200 border border-base-content/10 rounded-box px-3 py-2.5"
        >
          <div class="font-semibold text-[13px]">{{ r.release_name }}</div>
          <div class="font-mono text-[10.5px] text-base-content/60 mt-0.5">
            {{ r.designer || "unknown designer" }} · {{ r.model_count }} models
            ·
            {{ formatFileSize(r.total_size_bytes) }}
          </div>
        </div>
      </div>
    </div>

    <!-- workspace -->
    <div class="flex-1 min-w-0 flex flex-col overflow-hidden">
      <!-- stepper header -->
      <div class="flex items-center gap-0 px-6 pt-4 shrink-0">
        <template v-for="(s, i) in stepDefs" :key="s.step">
          <button
            type="button"
            class="flex items-center gap-2 cursor-pointer px-2.5 py-1.5 rounded-full"
            :class="[
              stepState(s.step).active ? 'bg-base-content/5' : '',
              !stepState(s.step).reachable
                ? 'opacity-40 cursor-not-allowed'
                : '',
            ]"
            :disabled="!stepState(s.step).reachable"
            @click="releasesStore.setReleaseStep(s.step)"
          >
            <span
              class="w-4.5 h-4.5 shrink-0 rounded-full font-bold text-[10px] flex items-center justify-center border box-border"
              :class="
                stepState(s.step).done
                  ? 'bg-success border-success text-success-content'
                  : stepState(s.step).active
                    ? 'bg-primary border-primary text-primary-content'
                    : 'border-base-content/20 text-base-content/40'
              "
            >
              {{ stepState(s.step).done ? "✓" : s.step }}
            </span>
            <span
              class="text-[12px]"
              :class="
                stepState(s.step).active
                  ? 'font-semibold'
                  : 'font-normal text-base-content/60'
              "
              >{{ s.label }}</span
            >
          </button>
          <span
            v-if="i < stepDefs.length - 1"
            class="w-6.5 h-px bg-base-content/10 mx-0.5"
          ></span>
        </template>
        <span class="flex-1"></span>
      </div>

      <!-- STEP 2: RELEASE DETAILS -->
      <div
        v-if="releasesStore.releaseStep === 2"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <form
          class="max-w-140 flex flex-col gap-3.5"
          @submit.prevent="saveReleaseInfo"
          @keydown.enter.prevent
        >
          <div class="font-bold text-[17px]">Release details</div>
          <p class="text-[12.5px] text-base-content/60 -mt-1.5">
            Name and describe the collection now that you can see what is in it.
          </p>
          <div class="grid grid-cols-2 gap-3">
            <div class="flex flex-col gap-1.5">
              <TextInput
                id="designer"
                label="Designer"
                placeholder="Name of the designer..."
                v-model="releaseForm.designer"
                required
              />
              <label
                class="flex items-center gap-1.5 text-[11px] text-base-content/60 cursor-pointer"
              >
                <input
                  type="checkbox"
                  class="checkbox checkbox-xs"
                  v-model="rememberDesigner"
                />
                Remember for future releases
              </label>
            </div>
            <MonthYearInput
              id="releaseDate"
              label="Release date"
              v-model="releaseForm.date"
              required
            />
          </div>
          <TextInput
            id="release-name"
            label="Release name"
            placeholder="Name of the release..."
            v-model="releaseForm.name"
            required
          />
          <TextArea
            id="description"
            label="Description"
            placeholder="Enter the description (Optional)..."
            v-model="releaseForm.description"
          />
          <FileSelect
            id="extraFiles"
            label="Additional content (license, PDFs)"
            multiple
            accept="pdf, md, zip"
            v-model="extraFiles"
          />
          <Switch
            v-model="openOnSave"
            :label="`Open temporary directory in ${fileExplorerName} after creation`"
          />
          <div class="flex gap-2.5 mt-1.5">
            <button
              type="submit"
              class="btn btn-primary"
              :disabled="!formComplete || isSavingInfo"
            >
              <template v-if="isSavingInfo">
                <span class="loading loading-spinner loading-sm"></span>
                Saving...
              </template>
              <span v-else>Save &amp; continue → Pack</span>
            </button>
            <button
              type="button"
              class="btn btn-ghost"
              @click="clearReleaseForm"
            >
              Clear
            </button>
          </div>
        </form>
      </div>

      <!-- STEP 1: MODELS + OPTIONAL RENDERS -->
      <div
        v-if="releasesStore.releaseStep === 1"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <div class="font-bold text-[17px]">Selected models</div>
        <p class="text-[12.5px] text-base-content/60 mt-1">
          Add models from the catalog. Changes here are release-only and never
          modify the catalog.
        </p>

        <div
          v-if="draftGroups.length"
          class="grid grid-cols-[minmax(360px,1fr)_320px] gap-5 mt-5 max-w-245"
        >
          <div class="flex flex-col gap-3">
            <section
              v-for="group in draftGroups"
              :key="group.name"
              class="border border-base-content/10 rounded-box overflow-hidden"
            >
              <div class="flex items-center px-3 py-2 bg-base-200">
                <span class="font-semibold text-[13px] flex-1">{{
                  group.name
                }}</span>
                <span class="font-mono text-[10px] text-base-content/45 mr-2">
                  {{ group.variants.length }} pose{{
                    group.variants.length === 1 ? "" : "s"
                  }}
                </span>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
                  @click="removeDraftGroup(group.name)"
                >
                  Remove
                </button>
              </div>
              <button
                v-for="variant in group.variants"
                :key="variant.id ?? variant.name"
                type="button"
                class="w-full flex items-center gap-3 px-3 py-2 border-t border-base-content/5 text-left cursor-pointer"
                :class="
                  selectedDraftId === variant.id
                    ? 'bg-primary/10'
                    : 'hover:bg-base-content/5'
                "
                @click="selectedDraftId = variant.id"
              >
                <img
                  v-if="variant.images[0]"
                  :src="convertFileSrc(variant.images[0])"
                  class="w-10 h-10 rounded-box object-cover"
                  alt=""
                />
                <img
                  v-else
                  :src="logoForFileName(variant.model_files[0] ?? '')"
                  class="w-10 h-10 rounded-box"
                  alt=""
                />
                <span class="flex-1 min-w-0">
                  <span class="block font-medium text-[13px] truncate">{{
                    variant.pose || variant.name
                  }}</span>
                  <span
                    class="block font-mono text-[10px] text-base-content/45"
                  >
                    {{ variant.support_status || "unknown supports" }} ·
                    {{ variant.model_files.length }} files
                  </span>
                </span>
                <span
                  class="font-mono text-[10px]"
                  :class="
                    variant.images.length ? 'text-success' : 'text-warning'
                  "
                >
                  {{ variant.images.length ? "✓ render" : "no render" }}
                </span>
              </button>
            </section>
          </div>

          <aside
            v-if="selectedDraft"
            class="border border-base-content/10 rounded-box p-3.5 flex flex-col gap-3 self-start"
          >
            <img
              v-if="selectedDraft.images[0]"
              :src="convertFileSrc(selectedDraft.images[0])"
              class="w-full aspect-4/3 object-cover rounded-box"
              alt=""
            />
            <div
              v-else
              class="w-full aspect-4/3 rounded-box bg-base-200 flex items-center justify-center text-sm text-base-content/40"
            >
              No render for this pose
            </div>
            <TextInput
              id="draft-name"
              label="Release name"
              v-model="selectedDraft.name"
            />
            <TextArea
              id="draft-description"
              label="Description"
              v-model="selectedDraft.description"
            />
            <div class="flex flex-col gap-1">
              <span class="font-mono text-[9px] text-base-content/40"
                >TAGS</span
              >
              <div class="flex flex-wrap gap-1.5 items-center">
                <span
                  v-for="tag in selectedDraft.tags"
                  :key="tag"
                  class="font-mono text-[10px] text-base-content/60 border border-base-content/15 rounded-full px-2.5 py-0.5 flex items-center gap-1"
                >
                  {{ tag }}
                  <button
                    type="button"
                    class="opacity-50 hover:opacity-100"
                    @click="removeDraftTag(tag)"
                  >
                    ✕
                  </button>
                </span>
                <form class="join" @submit.prevent="addDraftTag">
                  <input
                    v-model="newDraftTag"
                    type="text"
                    class="input input-xs join-item w-24"
                    placeholder="+ tag"
                  />
                </form>
              </div>
              <span class="font-mono text-[9px] text-base-content/40">
                release-only — the catalog stays untouched
              </span>
            </div>
            <div
              class="grid grid-cols-2 gap-2 font-mono text-[10px] text-base-content/50"
            >
              <span>POSE · {{ selectedDraft.pose || "—" }}</span>
              <span>SCALE · {{ selectedDraft.scale || "—" }}</span>
            </div>
            <button
              type="button"
              class="btn btn-sm btn-secondary"
              :disabled="
                !selectedDraft.model_files.some((path) =>
                  path.toLowerCase().endsWith('.stl'),
                )
              "
              @click="openRenderStudio(selectedDraft)"
            >
              {{
                selectedDraft.images.length
                  ? "Replace render"
                  : "Render this pose"
              }}
            </button>
          </aside>
        </div>
        <div
          v-else
          class="mt-6 max-w-160 border border-dashed border-base-content/15 rounded-box p-8 text-center text-sm text-base-content/45"
        >
          No models selected. Add them from the catalog to begin.
        </div>

        <div class="flex gap-2.5 mt-6">
          <button
            type="button"
            class="btn btn-primary"
            :disabled="!modelCount"
            @click="releasesStore.setReleaseStep(2)"
          >
            Continue → Release details
          </button>
        </div>
      </div>

      <!-- STEP 3: PACK -->
      <div
        v-if="releasesStore.releaseStep === 3"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <div class="max-w-160 flex flex-col gap-3.5">
          <div class="font-bold text-[17px]">Pack release</div>
          <div class="grid grid-cols-3 gap-2.5">
            <div
              class="bg-base-200 border border-base-content/10 rounded-box px-3.5 py-3"
            >
              <div class="font-bold text-[20px]">{{ modelCount }}</div>
              <div class="font-mono text-[10px] text-base-content/40 mt-0.5">
                MODELS
              </div>
            </div>
            <div
              class="bg-base-200 border border-base-content/10 rounded-box px-3.5 py-3"
            >
              <div class="font-bold text-[20px]">{{ totalFileCount }}</div>
              <div class="font-mono text-[10px] text-base-content/40 mt-0.5">
                FILES
              </div>
            </div>
            <div
              class="bg-base-200 border border-base-content/10 rounded-box px-3.5 py-3"
            >
              <div class="font-bold text-[20px]">{{ totalImageCount }}</div>
              <div class="font-mono text-[10px] text-base-content/40 mt-0.5">
                IMAGES
              </div>
            </div>
          </div>

          <form @submit.prevent="finalizeRelease">
            <button
              class="btn btn-primary"
              :disabled="!modelCount || isCompressing || isStartingExport"
              v-if="!isCompressing"
            >
              Pack &amp; export .3pk
            </button>
            <button
              v-else
              type="button"
              class="btn btn-error"
              @click="cancelCompression"
            >
              Cancel Compression
            </button>
          </form>
          <CompressionStatus @reset="resetStatus" />

          <button
            type="button"
            class="btn btn-ghost self-start"
            @click="releasesStore.setReleaseStep(2)"
          >
            ← Back
          </button>
        </div>
      </div>
    </div>
  </main>
</template>
