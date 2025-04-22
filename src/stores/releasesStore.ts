import { defineStore } from "pinia";
import { ref, computed } from "vue";
import type {
  ModelLocation,
  ModelReference,
  Release,
  StlModel,
} from "../bindings";
import { useToastStore } from "./toastStore.ts";

export type Tab = "settings" | "release" | "addStl" | "finalize";
export const useReleasesStore = defineStore("releases", () => {
  const toastStore = useToastStore();
  const release = ref<Release | undefined>();
  const models = ref<StlModel[]>([]);
  const releaseDir = ref<string | undefined>();
  const activeTab = ref<Tab>("release");
  const groups = ref<Set<string>>(new Set());

  const setActiveTab = (tab: Tab) => {
    activeTab.value = tab;
  };

  const releaseExists = computed(() => !!release.value);
  const updateRelease = (newRelease: Release) => {
    release.value = {
      ...newRelease,
      model_references: [...(newRelease.model_references || [])],
    };
  };

  const addModel = (model: StlModel, path: string) => {
    if (!release.value) {
      toastStore.addToast(
        "Release not initialized. Please create a release first.",
        "error",
      );
      return;
    }

    models.value.push(model);
    if (model.group) {
      groups.value.add(model.group);
    }
    release.value.model_references.push(<ModelReference>{
      id: model.id,
      location: <ModelLocation>{ Local: path },
    });
  };

  const removeModel = (model: StlModel) => {
    if (!release.value || !release.value.model_references || !models.value)
      return;

    const index = models.value.findIndex((m) => m.name === model.name);
    if (index !== -1) {
      models.value.splice(index, 1);
      release.value.model_references.splice(index, 1);
    }
  };

  const modelCount = computed(
    () => release.value?.model_references?.length || 0,
  );

  const clearModels = () => {
    if (!release.value) return;
    release.value.model_references = [];
    models.value = [];
  };

  const setReleaseDir = (dir: string) => {
    releaseDir.value = dir;
  };

  const clearRelease = () => {
    if (!release.value) return;
    release.value = undefined;
    groups.value.clear();
    clearModels();
  };

  return {
    activeTab,
    setActiveTab,
    release,
    releaseDir,
    releaseExists,
    clearRelease,
    setReleaseDir,
    updateRelease,
    models,
    modelCount,
    groups,
    addModel,
    removeModel,
    clearModels,
  };
});
