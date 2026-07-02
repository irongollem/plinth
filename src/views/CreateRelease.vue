<template>
  <form @submit.prevent="saveRelease">
    <View>
      <template #left>
        <h1 class="text-xl font-bold">Release info</h1>
        <TextInput
          id="designer"
          label="Designer"
          placeholder="Name of the designer..."
          v-model="release.designer"
          required
        />

        <TextInput
          id="release"
          label="Release"
          placeholder="Name of the release..."
          v-model="release.name"
          required
        />

        <MonthYearInput
          id="releaseDate"
          label="Release date"
          v-model="release.date"
          required
        />

        <TextArea
          id="description"
          label="Description"
          placeholder="Enter the description (Optional)..."
          v-model="release.description"
        />
        <FileSelect
          id="extraFiles"
          label="Additional content (licence, pdf's etc.)"
          multiple
          accept="pdf, md, zip"
          v-model="extraFiles"
        />

        <Switch v-model="openOnSafe" :label="`Open temporary directory in ${fileExplorerName} after creation`" />

        <div class="flex justify-between w-full mb-4">
          <button
            class="btn"
            type="submit"
            :disabled="!formComplete || isStoring"
          >
            <template v-if="isStoring">
              <span class="loading loading-spinner"></span>
              <span>Storing...</span>
            </template>
            <span v-else>Create Release</span>
          </button>
          <button type="button" class="btn btn-error" @click="clearRelease">
            Clear Release
          </button>
        </div>
      </template>

      <template #right>
        <ImageSelect v-model="releaseImages" />
      </template>
    </View>
  </form>
</template>

<script setup lang="ts">
import { openPath } from "@tauri-apps/plugin-opener";
import { computed, ref } from "vue";

import { type Release, commands } from "../bindings";
import FileSelect from "../components/FileSelect.vue";
import ImageSelect from "../components/ImageSelect.vue";
import MonthYearInput from "../components/MonthYearInput.vue";
import Switch from "../components/Switch.vue";
import TextArea from "../components/TextArea.vue";
import TextInput from "../components/TextInput.vue";
import View from "../components/View.vue";
import type { SelectedFile } from "../composables/useFileSelect";
import { useOS } from "../composables/useOS";
import { useReleasesStore } from "../stores/releasesStore.ts";
import { useToastStore } from "../stores/toastStore.ts";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { fileExplorerName } = useOS();

const openOnSafe = ref(false);
const release = ref<Release>({
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

const isStoring = ref(false);

const clearRelease = () => {
  release.value = {
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

const formComplete = computed(
  () => release.value.name && release.value.designer && release.value.date,
);

const saveRelease = async () => {
  if (!formComplete.value) {
    toastStore.addToast("Please enter a name for the release", "error", 0);
    return;
  }
  isStoring.value = true;
  try {
    const result = await commands.createRelease(
      release.value,
      releaseImages.value.map((image) => image.path),
      extraFiles.value.map((file) => file.path),
    );
    if (result.status === "ok") {
      // The backend computes the real directory name (and persists it in
      // release.json); mirror it locally instead of re-deriving it here
      release.value.release_dir =
        result.data.split(/[/\\]/).pop() ?? result.data;
      releasesStore.updateRelease(release.value);
      releasesStore.setReleaseDir(result.data);
      releasesStore.setActiveTab("addStl");
      if (openOnSafe.value) {
        await openPath(result.data);
      }
    } else {
      toastStore.reportError("Failed to create release", result.error);
    }
  } catch (error) {
    toastStore.reportError("Failed to create release", error);
  } finally {
    isStoring.value = false;
  }
};
</script>
