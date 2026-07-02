import { defineStore } from "pinia";
import { computed, ref } from "vue";
import type {
  ModelLocation,
  ModelReference,
  Release,
  StlModel,
} from "../bindings";
import { useToastStore } from "./toastStore.ts";

export type Tab = "settings" | "release" | "addStl" | "render" | "finalize";
export const useReleasesStore = defineStore("releases", () => {
  const toastStore = useToastStore();
  const release = ref<Release | undefined>();
  const models = ref<StlModel[]>([]);
  const releaseDir = ref<string | undefined>();
  const activeTab = ref<Tab>("release");

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
    clearModels();
  };

  const groups = computed(() =>
    Array.from(
      new Set<string>(
        models.value.map((model) => model.group).filter(Boolean) as string[],
      ),
    ),
  );

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
