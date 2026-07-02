import { defineStore } from "pinia";
import { computed, ref } from "vue";
import type {
  ModelLocation,
  ModelReference,
  Release,
  StlModel,
} from "../bindings";
import { useToastStore } from "./toastStore.ts";

export type Tab =
  | "settings"
  | "catalog"
  | "release"
  | "addStl"
  | "render"
  | "finalize";
export const useReleasesStore = defineStore("releases", () => {
  const toastStore = useToastStore();
  const release = ref<Release | undefined>();
  const models = ref<StlModel[]>([]);
  const releaseDir = ref<string | undefined>();
  const activeTab = ref<Tab>("release");
  // Cross-tab handoff: the catalog can push STL parts into the Render tab
  const renderParts = ref<string[]>([]);
  // ...and the Render tab can push finished promo images into Add STL
  const pendingModelImages = ref<string[]>([]);

  const setActiveTab = (tab: Tab) => {
    activeTab.value = tab;
  };

  const requestRender = (paths: string[]) => {
    renderParts.value = paths;
    activeTab.value = "render";
  };

  const queueModelImage = (path: string) => {
    if (!pendingModelImages.value.includes(path)) {
      pendingModelImages.value = [...pendingModelImages.value, path];
    }
  };

  const releaseExists = computed(() => !!release.value);
  const updateRelease = (newRelease: Release) => {
    release.value = {
      ...newRelease,
      model_references: [...(newRelease.model_references || [])],
    };
    // Keep the local model list consistent with the new reference list —
    // e.g. creating a fresh release must not keep the previous one's models
    models.value = models.value.filter((model) =>
      release.value?.model_references.some(
        (reference) => reference.id === model.id,
      ),
    );
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

    // Match by id in EACH list independently: names are not unique across
    // groups, and coupling the two arrays by index deletes the wrong
    // reference the moment they drift
    const index = models.value.findIndex((m) => m.id === model.id);
    if (index !== -1) {
      models.value.splice(index, 1);
    }
    const refIndex = release.value.model_references.findIndex(
      (reference) => reference.id === model.id,
    );
    if (refIndex !== -1) {
      release.value.model_references.splice(refIndex, 1);
    }
  };

  const modelCount = computed(
    () => release.value?.model_references?.length || 0,
  );

  const clearModels = () => {
    // Unconditional: clearRelease clears the release ref too, and a guard
    // on release.value would skip wiping models[] depending on call order
    models.value = [];
    if (release.value) {
      release.value.model_references = [];
    }
  };

  const setReleaseDir = (dir: string) => {
    releaseDir.value = dir;
  };

  const clearRelease = () => {
    clearModels();
    release.value = undefined;
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
    renderParts,
    requestRender,
    pendingModelImages,
    queueModelImage,
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
