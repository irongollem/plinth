import { acceptHMRUpdate, defineStore } from "pinia";
import { computed, ref } from "vue";
import type {
  ModelLocation,
  ModelReference,
  Release,
  StlModel,
} from "../bindings";
import { useToastStore } from "./toastStore.ts";

export type DraftReleaseModel = StlModel & {
  source_dir?: string;
  source_group?: string;
  pose?: string | null;
  scale?: string | null;
  support_status?: string | null;
};

// Top-level sidebar sections. The old flat release/addStl/finalize tabs are
// gone — those now live as steps INSIDE "releases" (see releaseStep below).
export type Tab = "catalog" | "releases" | "render" | "settings";

/** The release-builder flow: Models (including renders) -> Details -> Pack. */
export type ReleaseStep = 1 | 2 | 3;

export const useReleasesStore = defineStore("releases", () => {
  const toastStore = useToastStore();
  const release = ref<Release | undefined>();
  const models = ref<DraftReleaseModel[]>([]);
  const releaseDir = ref<string | undefined>();
  const activeTab = ref<Tab>("catalog");
  const releaseStep = ref<ReleaseStep>(1);
  // Cross-tab handoff: the catalog can push STL parts into the Render tab
  const renderParts = ref<string[]>([]);
  // When the handoff came from a catalog model, its dir_path rides along so
  // the finished render can be written back as that model's preview
  const renderPreviewTarget = ref<string | null>(null);
  // A render launched from the builder is attached directly to this staged
  // model. This lets rendering happen before a release directory exists.
  const renderDraftTarget = ref<string | null>(null);
  // ...and the Render tab can push finished promo images into Add STL
  const pendingModelImages = ref<string[]>([]);

  const setActiveTab = (tab: Tab) => {
    activeTab.value = tab;
  };

  /** Jump straight to a release step (used by the stepper header/sidebar). */
  const setReleaseStep = (step: ReleaseStep) => {
    releaseStep.value = step;
    activeTab.value = "releases";
  };

  const requestRender = (
    paths: string[],
    previewTargetDir?: string,
    draftTargetId?: string,
  ) => {
    renderParts.value = paths;
    renderPreviewTarget.value = previewTargetDir ?? null;
    renderDraftTarget.value = draftTargetId ?? null;
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

  /** Stage source paths in memory; they are copied only after details are saved. */
  const stageModel = (model: StlModel) => {
    // Do not rely on Web Crypto here: some Tauri WebViews do not expose
    // crypto.randomUUID(), which made catalog additions fail silently.
    const draftId = `draft-${Date.now()}-${models.value.length}`;
    const staged = { ...model, id: model.id ?? draftId };
    models.value.push(staged);
    return staged;
  };

  const attachImageToModel = (id: string, path: string) => {
    const target = models.value.find((model) => model.id === id);
    if (target && !target.images.includes(path)) target.images.push(path);
  };

  const removeModel = (model: StlModel) => {
    // Match by id in EACH list independently: names are not unique across
    // groups, and coupling the two arrays by index deletes the wrong
    // reference the moment they drift
    const index = models.value.findIndex((m) => m.id === model.id);
    if (index !== -1) {
      models.value.splice(index, 1);
    }
    const refIndex =
      release.value?.model_references.findIndex(
        (reference) => reference.id === model.id,
      ) ?? -1;
    if (refIndex !== -1 && release.value) {
      release.value.model_references.splice(refIndex, 1);
    }
  };

  const modelCount = computed(
    () =>
      new Set(models.value.map((model) => model.source_group ?? model.id)).size,
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
    releaseDir.value = undefined;
    releaseStep.value = 1;
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
    releaseStep,
    setReleaseStep,
    renderParts,
    renderPreviewTarget,
    renderDraftTarget,
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
    stageModel,
    attachImageToModel,
    removeModel,
    clearModels,
  };
});

// Preserve newly added state/actions when this store changes during `vite dev`.
if (import.meta.hot) {
  import.meta.hot.accept(acceptHMRUpdate(useReleasesStore, import.meta.hot));
}
