<template>
  <form @submit.prevent="saveModelData" @keydown.enter.prevent>
  <View>
    <template #left>
      <h1 class="text-xl font-bold">Model info</h1>
        <TextInput
            id="model-name"
            v-model="model.name"
            label="Model Name"
            placeholder="Enter model name..."
        />

        <TextArea id="description" placeholder="Enter the description (Optional)..." label="Description" v-model="model.description" />

      <TextInput
        id="group"
        label="Group"
        placeholder="Enter group name (Optional)..."
        v-model="model.group"
        :options="groups"
      />

        <TagInput id="tags" v-model="model.tags" label="Tags" placeholder="Write tags here..." />

        <FileSelect
            id="model-files"
            label="Model Files"
            multiple
            accept=".stl,.obj,.chitubox,.lys,.3mf,.blend,.gcode"
            v-model="modelFiles"
            :enabled="model.name.length > 0"
        />

        <ul v-if="modelFiles.length > 0" class="list">
          <li v-for="modelFile in modelFiles" :key="modelFile.path" class="list-row">
            <div>
              <img class="size-8 rounded-box" :src="logoForFileName(modelFile.name)" alt="File icon" />
            </div>
            <div>
              {{modelFile.name}}
            </div>
            <div>
              <button type="button" class="btn btn-xs btn-error" @click="modelFiles.splice(modelFiles.indexOf(modelFile), 1)">Remove</button>
            </div>
          </li>
        </ul>

        <div class="flex justify-between w-full mb-4">
          <button class="btn btn-primary" type="submit" :disabled="!formComplete || isStoring">
            <template v-if="isStoring">
              <span class="loading loading-spinner"></span>
              <span>Storing...</span>
            </template>
            <span v-else>Save Model</span>
          </button>
          <button class="btn btn-error" type="button" @click="clearModel">Clear Model</button>
        </div>
    </template>

    <template #right>
      <ImageSelect v-model="images" />
    </template>
  </View>
  </form>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { computed, ref } from "vue";
import type { Ref } from "vue";
import { type StlModel, commands } from "../bindings.ts";
import FileSelect from "../components/FileSelect.vue";
import ImageSelect from "../components/ImageSelect.vue";
import TagInput from "../components/TagInput.vue";
import TextArea from "../components/TextArea.vue";
import TextInput from "../components/TextInput.vue";
import View from "../components/View.vue";
import type { SelectedFile } from "../composables/useFileSelect";
import { useReleasesStore } from "../stores/releasesStore.ts";
import { useToastStore } from "../stores/toastStore.ts";
import { logoForFileName } from "../types.ts";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
// storeToRefs keeps these live: a plain destructure freezes releaseDir at
// first mount, and this KeepAlive'd component would then save a second
// release's models into the FIRST release's directory
const { groups, releaseDir } = storeToRefs(releasesStore);
const { addModel } = releasesStore;
const model: Ref<StlModel> = ref({
  id: null,
  name: "",
  description: null,
  tags: [],
  images: [],
  model_files: [],
  group: null,
});
const images = ref<SelectedFile[]>([]);
const modelFiles = ref<SelectedFile[]>([]);

const isStoring = ref(false);

const formComplete = computed(
  () =>
    model.value.name && modelFiles.value.length > 0 && images.value.length > 0,
);

const saveModelData = async () => {
  if (!formComplete.value) {
    toastStore.addToast("Please make sure the form is complete", "error", 0);
    return;
  }

  isStoring.value = true;
  try {
    if (!releaseDir.value) {
      throw new Error("Release directory name is missing");
    }

    const savedModelTupleResult = await commands.addModel(
      model.value,
      releaseDir.value,
      modelFiles.value.map((f) => f.path),
      images.value.map((f) => f.path),
    );
    if (savedModelTupleResult.status === "ok") {
      toastStore.addToast("Model saved successfully", "success");
      addModel(...savedModelTupleResult.data);
      clearModel();
    } else {
      toastStore.reportError(
        "Failed to save model",
        savedModelTupleResult.error,
      );
    }
  } catch (error) {
    toastStore.reportError("Failed to save model", error);
  } finally {
    isStoring.value = false;
  }
};

const clearModel = () => {
  model.value = {
    id: null,
    name: "",
    description: null,
    tags: [],
    images: [],
    model_files: [],
    group: null,
  };
  images.value = [];
  modelFiles.value = [];
};
</script>
