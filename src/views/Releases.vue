<script setup lang="ts">
import { openPath } from "@tauri-apps/plugin-opener";
import { storeToRefs } from "pinia";
import { computed, onMounted, ref, watch } from "vue";
import type { Ref } from "vue";
import {
  type Release,
  type ReleaseSummary,
  type StlModel,
  commands,
} from "../bindings.ts";
import CompressionStatus from "../components/CompressionStatus.vue";
import FileSelect from "../components/FileSelect.vue";
import ImageSelect from "../components/ImageSelect.vue";
import Switch from "../components/Switch.vue";
import TagInput from "../components/TagInput.vue";
import TextArea from "../components/TextArea.vue";
import TextInput from "../components/TextInput.vue";
import MonthYearInput from "../components/MonthYearInput.vue";
import { useCompressionStatus } from "../composables/useCompressionStatus";
import { filesFromPaths } from "../composables/useFileSelect";
import type { SelectedFile } from "../composables/useFileSelect";
import { useOS } from "../composables/useOS";
import { type ReleaseStep, useReleasesStore } from "../stores/releasesStore.ts";
import { useToastStore } from "../stores/toastStore.ts";
import { logoForFileName } from "../types.ts";
import { formatFileSize } from "../utils/format";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { release, releaseDir, modelCount, pendingModelImages } =
  storeToRefs(releasesStore);
const { fileExplorerName } = useOS();
const { activeJobId, isCompressing, resetStatus } = useCompressionStatus();

const stepDefs: { step: ReleaseStep; label: string }[] = [
  { step: 1, label: "Release info" },
  { step: 2, label: "Models" },
  { step: 3, label: "Render" },
  { step: 4, label: "Finalize" },
];
const stepState = (step: ReleaseStep) => {
  const active = releasesStore.releaseStep === step;
  const done =
    step < releasesStore.releaseStep ||
    (step === 1 && releasesStore.releaseExists);
  const reachable = step === 1 || releasesStore.releaseExists;
  return { active, done, reachable };
};

/* ---------------- left rail: past releases (read-only, from catalog scans) --------------- */
const pastReleases = ref<ReleaseSummary[]>([]);
onMounted(async () => {
  const result = await commands.getCatalogReleases();
  if (result.status === "ok") pastReleases.value = result.data;
});

/* ---------------------------- step 1: release info ---------------------------- */
const openOnSave = ref(false);
const releaseForm = ref<Release>({
  name: "",
  designer: "",
  description: "",
  date: "",
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
    designer: "",
    description: "",
    date: "",
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
    const result = await commands.createRelease(
      releaseForm.value,
      releaseImages.value.map((image) => image.path),
      extraFiles.value.map((file) => file.path),
    );
    if (result.status === "ok") {
      // The backend computes the real directory name; mirror it locally
      releaseForm.value.release_dir =
        result.data.split(/[/\\]/).pop() ?? result.data;
      releasesStore.updateRelease(releaseForm.value);
      releasesStore.setReleaseDir(result.data);
      releasesStore.setReleaseStep(2);
      if (openOnSave.value) await openPath(result.data);
    } else {
      toastStore.reportError("Failed to create release", result.error);
    }
  } catch (error) {
    toastStore.reportError("Failed to create release", error);
  } finally {
    isSavingInfo.value = false;
  }
};

/* ------------------------------- step 2: models -------------------------------- */
const model: Ref<StlModel> = ref({
  id: null,
  name: "",
  description: null,
  tags: [],
  images: [],
  model_files: [],
  group: null,
});
const modelImages = ref<SelectedFile[]>([]);
const modelFiles = ref<SelectedFile[]>([]);
const isStoringModel = ref(false);

watch(pendingModelImages, async (paths) => {
  if (!paths.length) return;
  const rendered = await filesFromPaths(paths);
  const known = new Set(modelImages.value.map((image) => image.path));
  modelImages.value = [
    ...modelImages.value,
    ...rendered.filter((image) => !known.has(image.path)),
  ];
  pendingModelImages.value = [];
});

const modelFormComplete = computed(
  () =>
    model.value.name &&
    modelFiles.value.length > 0 &&
    modelImages.value.length > 0,
);

const saveModel = async () => {
  if (!modelFormComplete.value) {
    toastStore.addToast("Please make sure the form is complete", "error", 0);
    return;
  }
  isStoringModel.value = true;
  try {
    if (!releaseDir.value) throw new Error("Release directory name is missing");
    const result = await commands.addModel(
      model.value,
      releaseDir.value,
      modelFiles.value.map((f) => f.path),
      modelImages.value.map((f) => f.path),
    );
    if (result.status === "ok") {
      toastStore.addToast("Model saved successfully", "success");
      releasesStore.addModel(...result.data);
      model.value = {
        id: null,
        name: "",
        description: null,
        tags: [],
        images: [],
        model_files: [],
        group: null,
      };
      modelImages.value = [];
      modelFiles.value = [];
    } else {
      toastStore.reportError("Failed to save model", result.error);
    }
  } catch (error) {
    toastStore.reportError("Failed to save model", error);
  } finally {
    isStoringModel.value = false;
  }
};

/* ---------------------------- step 3: render summary ---------------------------- */
const openRenderStudio = (paths?: string[]) => {
  if (paths?.length) {
    releasesStore.requestRender(paths);
  } else {
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
  if (!releaseDir.value) {
    toastStore.addToast(
      "No release directory yet — create a release first.",
      "error",
    );
    return;
  }
  isStartingExport.value = true;
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
      class="w-[264px] shrink-0 border-r border-base-content/10 p-3.5 flex flex-col gap-2 overflow-y-auto"
    >
      <div class="flex items-baseline justify-between px-1 pb-1.5">
        <span class="font-bold text-[15px]">Release builder</span>
        <button
          type="button"
          class="font-semibold text-[11px] text-primary cursor-pointer"
          @click="startNewDraft"
        >
          + New
        </button>
      </div>

      <button
        v-if="release"
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
            >step {{ releasesStore.releaseStep }} of 4</span
          >
        </div>
        <div class="font-semibold text-[13px] mt-1">
          {{ release.name || "Untitled release" }}
        </div>
        <div class="font-mono text-[10.5px] text-base-content/60 mt-0.5">
          {{ release.designer || "—" }} · {{ modelCount }} models
        </div>
      </button>
      <div
        v-else
        class="text-center text-xs text-base-content/40 border border-dashed border-base-content/15 rounded-box px-3 py-6"
      >
        No draft yet — fill in Release info to start one.
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
              class="w-[18px] h-[18px] shrink-0 rounded-full font-bold text-[10px] flex items-center justify-center border box-border"
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
            class="w-[26px] h-px bg-base-content/10 mx-0.5"
          ></span>
        </template>
        <span class="flex-1"></span>
      </div>

      <!-- STEP 1: INFO -->
      <div
        v-if="releasesStore.releaseStep === 1"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <form
          class="max-w-[560px] flex flex-col gap-3.5"
          @submit.prevent="saveReleaseInfo"
          @keydown.enter.prevent
        >
          <div class="font-bold text-[17px]">Release info</div>
          <div class="grid grid-cols-2 gap-3">
            <TextInput
              id="designer"
              label="Designer"
              placeholder="Name of the designer..."
              v-model="releaseForm.designer"
              required
            />
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
              <span v-else>Save &amp; continue → Models</span>
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

      <!-- STEP 2: MODELS -->
      <div
        v-if="releasesStore.releaseStep === 2"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <div class="flex gap-6">
          <form
            class="flex-1 max-w-[480px] flex flex-col gap-3"
            @submit.prevent="saveModel"
            @keydown.enter.prevent
          >
            <div class="font-bold text-[17px]">Add a model</div>
            <TextInput
              id="model-name"
              label="Model name"
              placeholder="Enter model name..."
              v-model="model.name"
            />
            <div class="grid grid-cols-2 gap-3">
              <TextInput
                id="group"
                label="Group"
                placeholder="e.g. Heroes"
                v-model="model.group"
                :options="releasesStore.groups"
              />
              <TagInput
                id="tags"
                v-model="model.tags"
                label="Tags"
                placeholder="bust, presupported..."
              />
            </div>
            <FileSelect
              id="model-files"
              label="Model files — .stl .obj .3mf .chitubox .lys"
              multiple
              accept=".stl,.obj,.chitubox,.lys,.3mf,.blend,.gcode"
              v-model="modelFiles"
            />
            <button
              type="submit"
              class="btn btn-primary max-w-[180px]"
              :disabled="!modelFormComplete || isStoringModel"
            >
              <template v-if="isStoringModel">
                <span class="loading loading-spinner loading-sm"></span>
                Storing...
              </template>
              <span v-else>Save model</span>
            </button>
          </form>
          <div class="w-[300px] shrink-0">
            <ImageSelect v-model="modelImages" />
          </div>
        </div>

        <div v-if="releasesStore.models.length" class="mt-6 max-w-[804px]">
          <div
            class="font-mono font-semibold text-[10px] tracking-[0.12em] text-base-content/40 border-b border-base-content/10 pb-1.5"
          >
            IN THIS RELEASE — {{ modelCount }} MODELS
          </div>
          <div
            v-for="d in releasesStore.models"
            :key="d.id ?? d.name"
            class="flex items-center gap-3 border-b border-base-content/5 py-2"
          >
            <img
              :src="logoForFileName(d.model_files[0] ?? '')"
              class="w-9 h-9 rounded-box"
              alt=""
            />
            <span class="flex-1 font-medium text-[13px]">{{ d.name }}</span>
            <span class="font-mono text-[10.5px] text-base-content/60"
              >{{ d.model_files.length }} files ·
              {{ d.images.length }} images</span
            >
            <button
              type="button"
              class="btn btn-xs btn-ghost"
              @click="releasesStore.removeModel(d)"
            >
              Remove
            </button>
          </div>
        </div>

        <div class="flex gap-2.5 mt-6">
          <button
            type="button"
            class="btn btn-primary"
            @click="releasesStore.setReleaseStep(3)"
          >
            Continue → Render
          </button>
        </div>
      </div>

      <!-- STEP 3: RENDER -->
      <div
        v-if="releasesStore.releaseStep === 3"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <div class="max-w-[640px] flex flex-col gap-3.5">
          <div class="font-bold text-[17px]">Promo renders</div>
          <p class="text-[12.5px] text-base-content/60 -mt-1.5">
            Render a promo image per model in the Render studio, then send it
            back here as the model's catalog image.
          </p>
          <div v-if="releasesStore.models.length">
            <div
              class="flex font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40 border-b border-base-content/10 pb-1.5"
            >
              <span class="flex-1">MODEL</span>
              <span class="w-[120px]">STATUS</span>
              <span class="w-[110px]"></span>
            </div>
            <div
              v-for="d in releasesStore.models"
              :key="d.id ?? d.name"
              class="flex items-center border-b border-base-content/5 py-2.5"
            >
              <span class="flex-1 font-medium text-[13px]">{{ d.name }}</span>
              <span
                class="w-[120px] font-mono text-[11px]"
                :class="
                  d.images.length ? 'text-success' : 'text-base-content/40'
                "
                >{{ d.images.length ? "✓ has image" : "no image yet" }}</span
              >
              <button
                type="button"
                class="w-[110px] font-semibold text-[11px] text-base-content/60 cursor-pointer text-left"
                @click="openRenderStudio(d.model_files)"
              >
                open studio →
              </button>
            </div>
          </div>
          <div v-else class="text-sm text-base-content/40">
            Add models in step 2 first.
          </div>
          <div
            class="flex items-center gap-3.5 bg-base-200 border border-base-content/10 rounded-box px-3.5 py-3"
          >
            <span class="font-mono text-[11px] text-base-content/60"
              >Renders are opened in the Render studio and sent back per
              model</span
            >
            <span class="flex-1"></span>
            <button
              type="button"
              class="font-semibold text-[11.5px] text-primary-content bg-primary rounded-full px-3.5 py-1.5 cursor-pointer"
              @click="openRenderStudio()"
            >
              Open render studio
            </button>
          </div>
          <div class="flex gap-2.5">
            <button
              type="button"
              class="btn btn-primary"
              @click="releasesStore.setReleaseStep(4)"
            >
              Continue → Finalize
            </button>
            <button
              type="button"
              class="btn btn-ghost"
              @click="releasesStore.setReleaseStep(2)"
            >
              ← Back
            </button>
          </div>
        </div>
      </div>

      <!-- STEP 4: FINALIZE -->
      <div
        v-if="releasesStore.releaseStep === 4"
        class="flex-1 overflow-y-auto px-6 py-5"
      >
        <div class="max-w-[640px] flex flex-col gap-3.5">
          <div class="font-bold text-[17px]">Finalize &amp; export</div>
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
              Finalize &amp; export .3pk
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
            @click="releasesStore.setReleaseStep(3)"
          >
            ← Back
          </button>
        </div>
      </div>
    </div>
  </main>
</template>
