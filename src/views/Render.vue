<template>
  <main
    class="bg-gray-800 text-gray-100 flex flex-col md:flex-row gap-6 p-6 pb-2 h-full rounded-b-lg"
  >
    <!-- Controls -->
    <section
      class="md:w-96 shrink-0 overflow-y-auto pr-2 max-h-full mb-6 space-y-4"
    >
      <h1 class="text-xl font-bold">Render</h1>

      <div
        v-if="blenderStatus === 'missing'"
        class="alert alert-warning text-sm"
      >
        <span>
          Blender was not found. Install Blender 4.x+ or set its location in
          settings.
        </span>
        <button
          class="btn btn-xs"
          @click="releasesStore.setActiveTab('settings')"
        >
          Open Settings
        </button>
      </div>
      <div v-else-if="blenderInfo" class="text-xs opacity-60">
        {{ blenderInfo.version }}
      </div>

      <FileSelect
        id="render-parts"
        label="Model Parts (joined into one mini)"
        multiple
        accept=".stl"
        v-model="parts"
      />

      <div>
        <h2 class="font-semibold mb-1">Orientation</h2>
        <p class="text-xs opacity-60 mb-2">
          Drag the model until it stands upright. Rotation
          <span class="font-mono">{{ rotationLabel }}</span>
        </p>
        <div class="flex flex-wrap gap-1">
          <button class="btn btn-xs" @click="viewport?.setRotation([90, 0, 0])">
            Stand up
          </button>
          <button class="btn btn-xs" @click="viewport?.rotateWorld('x', 90)">
            X +90°
          </button>
          <button class="btn btn-xs" @click="viewport?.rotateWorld('y', 90)">
            Y +90°
          </button>
          <button class="btn btn-xs" @click="viewport?.rotateWorld('z', 90)">
            Z +90°
          </button>
          <button class="btn btn-xs" @click="viewport?.resetRotation()">
            Reset
          </button>
        </div>
      </div>

      <div class="grid grid-cols-2 gap-3">
        <div>
          <label class="label text-sm" for="render-resolution"
            >Resolution</label
          >
          <select
            id="render-resolution"
            class="select select-sm w-full"
            v-model.number="resolution"
          >
            <option :value="512">512 px</option>
            <option :value="1024">1024 px</option>
            <option :value="1600">1600 px</option>
            <option :value="2048">2048 px</option>
          </select>
        </div>
        <div>
          <label class="label text-sm" for="render-samples">Quality</label>
          <select
            id="render-samples"
            class="select select-sm w-full"
            v-model.number="samples"
          >
            <option :value="32">Draft (32)</option>
            <option :value="96">Standard (96)</option>
            <option :value="256">High (256)</option>
          </select>
        </div>
      </div>

      <div class="flex items-center gap-3">
        <div>
          <label class="label text-sm" for="render-look">Look</label>
          <select id="render-look" class="select select-sm" v-model="look">
            <option value="rich">Rich (promo contrast)</option>
            <option value="flat">Flat (even lighting)</option>
          </select>
        </div>
        <div>
          <label class="label text-sm" for="render-color">Resin color</label>
          <input
            id="render-color"
            type="color"
            class="block h-8 w-16 cursor-pointer"
            v-model="colorHex"
          />
        </div>
        <label class="label cursor-pointer gap-2 text-sm mt-4">
          <input
            type="checkbox"
            class="checkbox checkbox-sm"
            v-model="matchCamera"
          />
          Match preview camera
        </label>
      </div>

      <div>
        <label class="label text-sm" for="render-output">Output</label>
        <div class="flex">
          <input
            id="render-output"
            type="text"
            readonly
            class="input input-sm flex-1"
            :value="outputPath || defaultOutputPath"
            placeholder="Select model parts first..."
          />
          <button
            class="btn btn-sm"
            :disabled="!parts.length"
            @click="chooseOutput"
          >
            Save as...
          </button>
        </div>
      </div>

      <div class="flex items-center gap-3">
        <button
          class="btn btn-primary flex-grow"
          :disabled="
            !parts.length || isRendering || blenderStatus === 'missing'
          "
          @click="render"
        >
          <template v-if="isRendering">
            <span class="loading loading-spinner"></span>
            <span>Rendering...</span>
          </template>
          <span v-else>Render promo image</span>
        </button>
        <button v-if="isRendering" class="btn btn-error" @click="cancel">
          Cancel
        </button>
      </div>

      <div v-if="isRendering" class="flex items-center gap-3">
        <ProgressBar :progress="percent" />
        <span class="text-sm opacity-70">Cycles render in progress</span>
      </div>

      <div
        v-if="errorMessage"
        class="alert alert-error text-xs whitespace-pre-wrap"
      >
        {{ errorMessage }}
      </div>

      <div v-if="resultPath" class="space-y-2">
        <h2 class="font-semibold">
          Result
          <span v-if="elapsedSeconds" class="text-xs opacity-60">
            ({{ elapsedSeconds.toFixed(1) }}s)
          </span>
        </h2>
        <button
          type="button"
          class="w-full cursor-zoom-in"
          @click="showResult = true"
        >
          <img
            :src="resultUrl ?? undefined"
            alt="Rendered promo image"
            class="rounded-box w-full"
          />
        </button>
        <div class="flex gap-2">
          <button class="btn btn-sm flex-1" @click="showResult = true">
            View large
          </button>
          <button class="btn btn-sm flex-1" @click="openPath(resultPath)">
            Open file
          </button>
        </div>
        <button
          v-if="releasesStore.releaseExists"
          class="btn btn-sm btn-secondary w-full"
          @click="sendToAddStl"
        >
          Use as model image in Add STL
        </button>
      </div>
    </section>

    <!-- Viewport / result -->
    <aside class="flex-1 max-h-full mb-6 relative">
      <StlViewport
        ref="viewport"
        :parts="partPaths"
        :color="colorLinear"
        @rotation="onRotation"
        @view="onView"
        @loaded="onLoaded"
        @error="onViewportError"
      />
      <!-- Finished render takes over the viewport so it can't be missed -->
      <div
        v-if="showResult && resultUrl"
        class="absolute inset-0 bg-black rounded-box flex flex-col z-10"
      >
        <div class="flex items-center gap-2 p-2">
          <span class="text-sm font-semibold opacity-80">
            Render result
            <span v-if="elapsedSeconds" class="opacity-50">
              — {{ elapsedSeconds.toFixed(1) }}s
            </span>
          </span>
          <span class="flex-1"></span>
          <button
            v-if="resultPath"
            class="btn btn-xs"
            @click="openPath(resultPath)"
          >
            Open file
          </button>
          <button class="btn btn-xs btn-primary" @click="showResult = false">
            Back to 3D
          </button>
        </div>
        <img
          :src="resultUrl"
          alt="Rendered promo image"
          class="flex-1 min-h-0 object-contain"
        />
      </div>
    </aside>
  </main>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { storeToRefs } from "pinia";
import { computed, onMounted, ref, watch } from "vue";
import { type BlenderInfo, commands } from "../bindings.ts";
import FileSelect from "../components/FileSelect.vue";
import ProgressBar from "../components/ProgressBar.vue";
// NOT `import type`: the component is used in the template, which
// biome's useImportType can't see (rule disabled for .vue in biome.json)
import StlViewport from "../components/StlViewport.vue";
import { filesFromPaths } from "../composables/useFileSelect";
import type { SelectedFile } from "../composables/useFileSelect";
import { useRenderStatus } from "../composables/useRenderStatus";
import { useReleasesStore } from "../stores/releasesStore.ts";
import { useToastStore } from "../stores/toastStore.ts";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const {
  isRendering,
  percent,
  resultPath,
  elapsedSeconds,
  errorMessage,
  start,
  cancel,
} = useRenderStatus();

const viewport = ref<InstanceType<typeof StlViewport> | null>(null);
const parts = ref<SelectedFile[]>([]);
const partPaths = computed(() => parts.value.map((f) => f.path));

// The catalog hands STL parts over via the store ("Render promo" button)
const { renderParts } = storeToRefs(releasesStore);
watch(renderParts, async (paths) => {
  if (!paths.length) return;
  parts.value = await filesFromPaths(paths);
  renderParts.value = [];
});

const rotation = ref<[number, number, number]>([90, 0, 0]);
const view = ref({ azimuth: -15, elevation: 0.22, zoom: 1.15 });
const matchCamera = ref(true);
const resolution = ref(1600);
const samples = ref(96);
const look = ref<"rich" | "flat">("rich");
// sRGB of the locked linear resin color (0.80, 0.54, 0.35)
const colorHex = ref("#e7c2a0");
const outputPath = ref("");
const showResult = ref(false);

// A finished render takes over the viewport + toasts — previously it only
// appeared as a small thumbnail below the fold and looked like nothing
// happened
watch(resultPath, (path) => {
  if (!path) return;
  showResult.value = true;
  toastStore.addToast("Render complete", "success");
});

const sendToAddStl = () => {
  if (!resultPath.value) return;
  releasesStore.queueModelImage(resultPath.value);
  toastStore.addToast("Render queued as model image in Add STL", "success");
  releasesStore.setActiveTab("addStl");
};

const blenderInfo = ref<BlenderInfo | null>(null);
const blenderStatus = ref<"unknown" | "found" | "missing">("unknown");

onMounted(async () => {
  const result = await commands.detectBlender();
  if (result.status === "ok") {
    blenderInfo.value = result.data;
    blenderStatus.value = "found";
  } else {
    blenderStatus.value = "missing";
  }
});

const rotationLabel = computed(
  () =>
    `X ${rotation.value[0]}° · Y ${rotation.value[1]}° · Z ${rotation.value[2]}°`,
);

const srgbToLinear = (c: number) =>
  c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;

const colorLinear = computed<[number, number, number]>(() => {
  const hex = colorHex.value.replace("#", "");
  const r = Number.parseInt(hex.slice(0, 2), 16) / 255;
  const g = Number.parseInt(hex.slice(2, 4), 16) / 255;
  const b = Number.parseInt(hex.slice(4, 6), 16) / 255;
  return [srgbToLinear(r), srgbToLinear(g), srgbToLinear(b)];
});

const defaultOutputPath = computed(() =>
  parts.value.length
    ? `${parts.value[0].path.replace(/\.[^.]+$/, "")}.png`
    : "",
);

const resultUrl = computed(() =>
  resultPath.value
    ? `${convertFileSrc(resultPath.value)}?v=${elapsedSeconds.value}`
    : null,
);

const onRotation = (value: [number, number, number]) => {
  rotation.value = value;
};

const onView = (value: {
  azimuth: number;
  elevation: number;
  zoom: number;
}) => {
  view.value = value;
};

const onLoaded = () => {
  // stand freshly loaded models up by default, like the script's --rotate 90,0,0
  viewport.value?.setRotation(rotation.value);
};

const onViewportError = (message: string) => {
  toastStore.addToast(message, "error", 0);
};

const chooseOutput = async () => {
  const selected = await save({
    defaultPath: outputPath.value || defaultOutputPath.value,
    filters: [{ name: "PNG Image", extensions: ["png"] }],
  });
  if (selected) outputPath.value = selected;
};

const render = async () => {
  const roundedColor = colorLinear.value.map(
    (c) => Math.round(c * 1000) / 1000,
  ) as [number, number, number];

  const result = await start(partPaths.value, {
    rotate: rotation.value,
    color: roundedColor,
    azimuth: matchCamera.value ? view.value.azimuth : null,
    elevation: matchCamera.value ? view.value.elevation : null,
    zoom: matchCamera.value ? view.value.zoom : null,
    resolution: resolution.value,
    samples: samples.value,
    look: look.value,
    output_path: outputPath.value || null,
  });

  if (result.status === "error") {
    toastStore.reportError("Failed to start render", result.error);
  }
};
</script>
