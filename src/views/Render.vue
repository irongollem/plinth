<template>
  <main class="flex h-full min-w-0">
    <!-- Controls -->
    <section
      class="w-82.5 shrink-0 border-r border-base-content/10 overflow-y-auto p-4 flex flex-col gap-3.5"
    >
      <div class="flex items-baseline justify-between">
        <span class="font-bold text-[17px]">Render studio</span>
        <span
          class="font-mono text-[10px]"
          :class="
            blenderStatus === 'found' ? 'text-success' : 'text-base-content/40'
          "
          >{{
            blenderStatus === "found"
              ? `${blenderInfo?.version} ✓`
              : blenderStatus === "missing"
                ? "not found"
                : ""
          }}</span
        >
      </div>

      <div
        v-if="blenderStatus === 'missing'"
        class="alert alert-warning text-sm"
      >
        <span
          >Blender was not found. Install Blender 4.x+ or set its location in
          settings.</span
        >
        <button
          class="btn btn-xs"
          @click="releasesStore.setActiveTab('settings')"
        >
          Open Settings
        </button>
      </div>

      <div class="flex flex-col gap-1">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >MODEL PARTS — JOINED INTO ONE MINI</span
        >
        <FileSelect
          id="render-parts"
          label=""
          multiple
          accept=".stl"
          v-model="parts"
        />
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >ORIENTATION — DRAG, GIZMO RING, OR TYPE EXACT ANGLES</span
        >
        <div class="grid grid-cols-3 gap-2">
          <label
            v-for="(axis, index) in ['X', 'Y', 'Z'] as const"
            :key="axis"
            class="input input-xs flex items-center gap-1"
          >
            <span class="opacity-50">{{ axis }}°</span>
            <input
              type="number"
              step="1"
              class="w-full"
              :value="rotation[index]"
              @change="setRotationAxis(index, $event)"
            />
          </label>
        </div>
        <div class="flex flex-wrap gap-1.5">
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
          <button
            class="btn btn-xs btn-ghost"
            @click="viewport?.resetRotation()"
          >
            Reset
          </button>
        </div>
      </div>

      <div class="grid grid-cols-3 gap-2">
        <label class="input input-xs flex items-center gap-1">
          <span class="opacity-50">az°</span>
          <input
            type="number"
            step="1"
            class="w-full"
            :value="view.azimuth"
            @change="setViewField('azimuth', $event)"
          />
        </label>
        <label class="input input-xs flex items-center gap-1">
          <span class="opacity-50">elev</span>
          <input
            type="number"
            step="0.05"
            class="w-full"
            :value="view.elevation"
            @change="setViewField('elevation', $event)"
          />
        </label>
        <label class="input input-xs flex items-center gap-1">
          <span class="opacity-50">zoom</span>
          <input
            type="number"
            step="0.05"
            class="w-full"
            :value="view.zoom"
            @change="setViewField('zoom', $event)"
          />
        </label>
      </div>

      <div class="grid grid-cols-2 gap-2">
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

      <div class="grid grid-cols-2 gap-2">
        <div>
          <label class="label text-sm" for="render-look">Look</label>
          <select
            id="render-look"
            class="select select-sm w-full"
            v-model="look"
          >
            <option value="flat">Classic (locked look)</option>
            <option value="resin">Resin (glossy coat)</option>
            <option value="rich">Rich (experimental)</option>
          </select>
        </div>
        <div>
          <label class="label text-sm">Resin color</label>
          <div class="flex items-center gap-1.5 pt-0.5">
            <button
              v-for="swatch in resinSwatches"
              :key="swatch.hex"
              type="button"
              class="w-6 h-6 rounded-full cursor-pointer"
              :style="{
                background: swatch.hex,
                boxShadow:
                  colorHex === swatch.hex
                    ? '0 0 0 2px var(--color-base-100), 0 0 0 4px var(--color-primary)'
                    : '0 0 0 1px var(--color-base-content, #000)',
              }"
              :title="swatch.name"
              @click="colorHex = swatch.hex"
            ></button>
            <label
              class="w-6 h-6 rounded-full cursor-pointer relative overflow-hidden border border-base-content/30"
              title="Custom color"
            >
              <input
                type="color"
                class="absolute -top-1 -left-1 w-8 h-8 cursor-pointer"
                v-model="colorHex"
              />
            </label>
            <button
              v-if="!isPresetColor"
              type="button"
              class="btn btn-xs"
              title="Back to the locked resin color"
              @click="colorHex = DEFAULT_RESIN_HEX"
            >
              Reset
            </button>
          </div>
        </div>
      </div>

      <label
        class="label cursor-pointer gap-2 text-sm"
        title="For parts exported around different origins: seats the mini on the part named *base*. Leave off when parts already fit together."
      >
        <input
          type="checkbox"
          class="checkbox checkbox-sm"
          v-model="alignParts"
        />
        Align parts on base
      </label>

      <div class="flex items-center gap-2">
        <label class="label cursor-pointer gap-2 text-sm flex-1">
          <input
            type="checkbox"
            class="checkbox checkbox-sm"
            v-model="matchCamera"
          />
          Match preview camera
        </label>
        <button
          type="button"
          class="btn btn-xs btn-ghost"
          title="Back to the studio defaults (settings are otherwise remembered between sessions)"
          @click="resetRenderSettings"
        >
          Reset settings
        </button>
      </div>

      <!-- BRANDING — baked into the output PNG after Blender finishes -->
      <div class="flex flex-col gap-2 border-t border-base-content/10 pt-3">
        <div class="flex items-center gap-1.5">
          <span
            class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
            >BRANDING — BAKED INTO THE RENDER</span
          >
        </div>
        <div class="flex items-center gap-2">
          <input
            type="checkbox"
            class="checkbox checkbox-xs"
            v-model="branding.logoOn"
          />
          <span class="text-[12px] font-medium">Logo watermark</span>
          <span class="flex-1"></span>
          <div class="flex gap-1">
            <button
              v-for="pos in cornerPositions"
              :key="pos"
              type="button"
              class="font-mono text-[9px] px-1.5 py-0.5 rounded cursor-pointer border"
              :class="
                branding.logoPos === pos
                  ? 'bg-primary text-primary-content border-primary'
                  : 'text-base-content/60 border-base-content/15'
              "
              @click="branding.logoPos = pos"
            >
              {{ pos.toUpperCase() }}
            </button>
          </div>
        </div>
        <div v-if="branding.logoOn" class="flex items-center gap-2">
          <button type="button" class="btn btn-xs" @click="chooseLogo">
            Logo image…
          </button>
          <span
            class="flex-1 truncate font-mono text-[10.5px]"
            :class="branding.logoPath ? 'text-base-content/60' : 'text-warning'"
            :title="branding.logoPath"
          >
            {{ logoFileName || "none chosen — logo won't be baked" }}
          </span>
          <button
            v-if="branding.logoPath"
            type="button"
            class="btn btn-xs btn-ghost"
            @click="branding.logoPath = ''"
          >
            ✕
          </button>
        </div>
        <div class="flex items-center gap-2">
          <input
            type="checkbox"
            class="checkbox checkbox-xs"
            v-model="branding.textOn"
          />
          <span class="text-[12px] font-medium">Text overlay</span>
          <span class="flex-1"></span>
          <div class="flex gap-1 flex-wrap justify-end">
            <button
              v-for="pos in textPositions"
              :key="pos"
              type="button"
              class="font-mono text-[9px] px-1.5 py-0.5 rounded cursor-pointer border"
              :class="
                branding.textPos === pos
                  ? 'bg-primary text-primary-content border-primary'
                  : 'text-base-content/60 border-base-content/15'
              "
              @click="branding.textPos = pos"
            >
              {{ pos.toUpperCase() }}
            </button>
          </div>
        </div>
        <input
          v-model="branding.title"
          placeholder="Overlay title…"
          class="input input-sm w-full"
        />
        <input
          v-model="branding.credit"
          placeholder="Credit line…"
          class="input input-sm font-mono w-full"
        />
        <div class="flex gap-1.5">
          <button
            v-for="f in fontOptions"
            :key="f.value"
            type="button"
            class="flex-1 text-center text-[11px] py-1.5 rounded cursor-pointer border"
            :style="{ fontFamily: f.css }"
            :class="
              branding.font === f.value
                ? 'bg-primary text-primary-content border-primary'
                : 'text-base-content/60 border-base-content/15'
            "
            @click="branding.font = f.value"
          >
            {{ f.label }}
          </button>
        </div>
        <div class="flex items-center gap-2.5">
          <span
            class="font-mono font-semibold text-[9.5px] text-base-content/40"
            >SIZE</span
          >
          <input
            type="range"
            min="12"
            max="48"
            v-model.number="branding.size"
            class="range range-xs flex-1"
          />
          <span class="font-mono text-[11px] w-9 text-right"
            >{{ branding.size }}px</span
          >
        </div>
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
          class="btn btn-primary grow"
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
          <span v-if="elapsedSeconds" class="text-xs opacity-60"
            >({{ elapsedSeconds.toFixed(1) }}s)</span
          >
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
          v-if="releasesStore.releaseExists || releasesStore.renderDraftTarget"
          class="btn btn-sm btn-secondary w-full"
          @click="sendToAddStl"
        >
          Use as model image in release step 2
        </button>
      </div>
    </section>

    <!-- Viewport / result -->
    <aside class="flex-1 min-w-0 relative">
      <StlViewport
        ref="viewport"
        :parts="partPaths"
        :color="colorLinear"
        :align-parts="alignParts"
        @rotation="onRotation"
        @view="onView"
        @loaded="onLoaded"
        @error="onViewportError"
      />
      <!-- Branding overlay preview — the same spec the bake composites -->
      <div
        v-if="branding.logoOn && parts.length"
        class="absolute w-13 h-13 flex items-center justify-center pointer-events-none"
        :class="
          branding.logoPath
            ? ''
            : 'rounded-lg border-2 border-dashed border-base-content/40 font-mono text-[8px] tracking-[0.08em] text-base-content/40'
        "
        :style="logoPosStyle"
      >
        <img
          v-if="branding.logoPath"
          :src="convertFileSrc(branding.logoPath)"
          alt=""
          class="max-w-full max-h-full object-contain"
        />
        <template v-else>LOGO</template>
      </div>
      <div
        v-if="branding.textOn && parts.length"
        class="absolute pointer-events-none"
        :style="textPosStyle"
      >
        <div
          :style="{
            fontFamily: fontCss,
            fontSize: `${branding.size}px`,
            fontWeight: 700,
          }"
        >
          {{ branding.title || "Untitled" }}
        </div>
        <div
          class="font-mono text-[9.5px] tracking-[0.18em] text-base-content/60 mt-0.5"
        >
          {{ branding.credit }}
        </div>
      </div>

      <!-- Finished render takes over the viewport so it can't be missed -->
      <div
        v-if="showResult && resultUrl"
        class="absolute inset-0 bg-black rounded-box flex flex-col z-10"
      >
        <div class="flex items-center gap-2 p-2">
          <span class="text-sm font-semibold opacity-80">
            Render result
            <span v-if="elapsedSeconds" class="opacity-50"
              >— {{ elapsedSeconds.toFixed(1) }}s</span
            >
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
import { computed, onMounted, reactive, ref, watch } from "vue";
import { type BlenderInfo, commands } from "../bindings.ts";
import FileSelect from "../components/FileSelect.vue";
import ProgressBar from "../components/ProgressBar.vue";
// NOT `import type`: the component is used in the template, which
// biome's useImportType can't see (rule disabled for .vue in biome.json)
import StlViewport from "../components/StlViewport.vue";
import { filesFromPaths, useFileSelect } from "../composables/useFileSelect";
import type { SelectedFile } from "../composables/useFileSelect";
import { useRenderStatus } from "../composables/useRenderStatus";
import { drawOverlay } from "../utils/promoOverlay";
import { useReleasesStore } from "../stores/releasesStore.ts";
import { useToastStore } from "../stores/toastStore.ts";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { selectFiles } = useFileSelect();
const {
  isRendering,
  percent,
  resultPath,
  elapsedSeconds,
  errorMessage,
  start,
  cancel,
  reset,
} = useRenderStatus();

const viewport = ref<InstanceType<typeof StlViewport> | null>(null);
const parts = ref<SelectedFile[]>([]);
const partPaths = computed(() => parts.value.map((f) => f.path));

// The catalog hands STL parts over via the store ("Render promo" button, or
// per-model "open studio" links from the release stepper's Render step)
const { renderParts, renderPreviewTarget, renderPreviewVariantKey } =
  storeToRefs(releasesStore);
// The exact part set the catalog asked to render; if the user swaps files
// afterwards, the finished image is NOT that model's preview anymore
let previewTargetParts = "";
watch(
  renderParts,
  async (paths) => {
    if (!paths.length) return;
    parts.value = await filesFromPaths(paths);
    previewTargetParts = paths.join("\n");
    renderParts.value = [];
  },
  // immediate: this view mounts lazily on first visit, AFTER the catalog
  // has already written the handoff into the store — a mount-time watcher
  // alone would miss it and the studio would open with an empty part list
  { immediate: true },
);

const rotation = ref<[number, number, number]>([90, 0, 0]);
const view = ref({ azimuth: -15, elevation: 0.22, zoom: 1.15 });
const matchCamera = ref(true);
// Re-seat parts exported around different origins (see StlViewport's
// stackOnBase). Off by default: on correctly-exported multi-part minis
// the re-seat would WRONGLY collapse well-placed parts onto the base.
const alignParts = ref(false);
const resolution = ref(1600);
const samples = ref(96);
// "flat" (the handover's locked look) won the three-way comparison against
// the DTL reference; "resin" adds the physical-print sheen on top of it
const look = ref<"rich" | "flat" | "resin">("flat");
// sRGB of the default linear resin color (0.85, 0.65, 0.43) — pale warm
// cream, matched against formal DTL product renders
const DEFAULT_RESIN_HEX = "#edd3af";
const colorHex = ref(DEFAULT_RESIN_HEX);
const resinSwatches = [
  { name: "Warm cream (default)", hex: DEFAULT_RESIN_HEX },
  { name: "Neutral gray", hex: "#b9bcbe" },
  { name: "Sage green", hex: "#9aa78b" },
  { name: "Charcoal", hex: "#4a4c4e" },
];
const isPresetColor = computed(() =>
  resinSwatches.some((s) => s.hex === colorHex.value),
);
const outputPath = ref("");

const setRotationAxis = (index: number, event: Event) => {
  const value = Number.parseFloat((event.target as HTMLInputElement).value);
  if (Number.isNaN(value)) return;
  const next = [...rotation.value] as [number, number, number];
  next[index] = value;
  viewport.value?.setRotation(next);
};

const setViewField = (
  field: "azimuth" | "elevation" | "zoom",
  event: Event,
) => {
  const value = Number.parseFloat((event.target as HTMLInputElement).value);
  if (Number.isNaN(value)) return;
  viewport.value?.setView({ [field]: value });
};
const showResult = ref(false);
// What the user asked the render to be called, to detect auto-renames
let requestedOutputPath = "";

// A new subject on the stage clears the previous model's finished shot —
// this view is kept alive across tab switches, so without this the last
// mini's render greeted whoever came next from the catalog
watch(partPaths, (next, prev) => {
  if (next.join("\n") === prev.join("\n")) return;
  showResult.value = false;
  if (!isRendering.value) reset();
});

// A finished render takes over the viewport + toasts — previously it only
// appeared as a small thumbnail below the fold and looked like nothing
// happened
watch(resultPath, async (path) => {
  if (!path) return;
  // Bake FIRST: everything downstream (the takeover view, the catalog
  // preview, step 2 of the release) must see the branded file, not a
  // pristine render that silently changes underneath them a beat later
  await bakeBranding(path);
  showResult.value = true;
  if (requestedOutputPath && path !== requestedOutputPath) {
    const savedAs = path.split(/[/\\]/).pop();
    toastStore.addToast(
      `Existing file kept — render saved as ${savedAs}`,
      "info",
      8000,
    );
  } else {
    toastStore.addToast("Render complete", "success");
  }

  // Close the catalog loop: a render started from a catalog model becomes
  // that model's preview — as long as these are still the same parts
  if (
    renderPreviewTarget.value &&
    partPaths.value.join("\n") === previewTargetParts
  ) {
    const result = await commands.setModelPreview(
      renderPreviewTarget.value,
      path,
      renderPreviewVariantKey.value,
    );
    if (result.status === "ok") {
      toastStore.addToast("Catalog preview updated", "success");
    } else {
      toastStore.reportError("Failed to set catalog preview", result.error);
    }
    renderPreviewTarget.value = null;
    renderPreviewVariantKey.value = null;
  }
});

const sendToAddStl = () => {
  if (!resultPath.value) return;
  if (releasesStore.renderDraftTarget) {
    releasesStore.attachImageToModel(
      releasesStore.renderDraftTarget,
      resultPath.value,
    );
    releasesStore.renderDraftTarget = null;
    toastStore.addToast("Render attached to the model", "success");
    releasesStore.setReleaseStep(1);
    return;
  }
  releasesStore.queueModelImage(resultPath.value);
  toastStore.addToast("Render queued as a model image", "success");
  releasesStore.setReleaseStep(2);
};

/* ------------------------- branding overlay (UI-only stub) ------------------------- */
type CornerPos = "tl" | "tr" | "bl" | "br";
type TextPos = "tl" | "tc" | "tr" | "bl" | "bc" | "br";
const cornerPositions: CornerPos[] = ["tl", "tr", "bl", "br"];
const textPositions: TextPos[] = ["tl", "tc", "tr", "bl", "bc", "br"];
const fontOptions: { value: string; label: string; css: string }[] = [
  { value: "Archivo", label: "Grotesk", css: "'Archivo', sans-serif" },
  { value: "Bebas Neue", label: "Display", css: "'Bebas Neue', sans-serif" },
  {
    value: "Cormorant Garamond",
    label: "Serif",
    css: "'Cormorant Garamond', serif",
  },
  { value: "IBM Plex Mono", label: "Mono", css: "'IBM Plex Mono', monospace" },
];

const BRANDING_DEFAULTS = {
  logoOn: true,
  logoPos: "tr" as CornerPos,
  logoPath: "",
  textOn: true,
  textPos: "bl" as TextPos,
  title: "",
  credit: "",
  font: "Archivo",
  size: 20,
};
const branding = reactive({ ...BRANDING_DEFAULTS });

const logoFileName = computed(
  () => branding.logoPath.split(/[/\\]/).pop() ?? "",
);

const chooseLogo = async () => {
  const files = await selectFiles({
    multiple: false,
    accept: ".png,.jpg,.jpeg,.webp",
    title: "Choose logo image",
  });
  if (files?.length) branding.logoPath = files[0].path;
};

/* ------------------------- branding bake ------------------------- */
// Bumped after every bake: the result <img> URL must change or the
// webview happily keeps showing its cached, unbranded bytes
const bakeStamp = ref(0);

const loadImage = (dataUrl: string) =>
  new Promise<HTMLImageElement>((resolve, reject) => {
    const img = new Image();
    img.addEventListener("load", () => resolve(img), { once: true });
    img.addEventListener(
      "error",
      () => reject(new Error("Failed to decode image")),
      { once: true },
    );
    img.src = dataUrl;
  });

/** Composite the branding overlay onto the finished render, in place. */
const bakeBranding = async (path: string) => {
  const wantsLogo = branding.logoOn && !!branding.logoPath;
  const wantsText =
    branding.textOn && !!(branding.title.trim() || branding.credit.trim());
  if (!wantsLogo && !wantsText) {
    // Logo toggle on but no image chosen deserves a nudge, not silence
    if (branding.logoOn && !branding.logoPath && !wantsText) {
      toastStore.addToast(
        "Branding skipped — choose a logo image or add overlay text",
        "info",
      );
    }
    return;
  }
  try {
    // Data-URL detour via Rust: asset:-protocol images would taint the
    // canvas and make toBlob() throw a SecurityError
    const baseResult = await commands.readImageBase64(path);
    if (baseResult.status !== "ok") {
      toastStore.reportError("Branding bake failed", baseResult.error);
      return;
    }
    const baseImage = await loadImage(baseResult.data);

    let logoImage: HTMLImageElement | null = null;
    if (wantsLogo) {
      const logoResult = await commands.readImageBase64(branding.logoPath);
      if (logoResult.status === "ok") {
        logoImage = await loadImage(logoResult.data);
      } else {
        toastStore.reportError("Logo not baked", logoResult.error);
      }
    }

    const spec = {
      logoOn: wantsLogo && !!logoImage,
      logoPos: branding.logoPos,
      textOn: branding.textOn,
      textPos: branding.textPos,
      title: branding.title,
      credit: branding.credit,
      fontCss: fontCss.value,
      size: branding.size,
    };

    // Make sure the real fonts are in before drawing — otherwise the
    // canvas silently substitutes whatever is loaded at that instant
    await Promise.all([
      document.fonts.load(`700 32px ${fontCss.value}`),
      document.fonts.load("500 16px 'IBM Plex Mono', monospace"),
    ]).catch(() => {
      /* offline: canvas falls back exactly like the preview does */
    });

    const canvas = document.createElement("canvas");
    canvas.width = baseImage.naturalWidth;
    canvas.height = baseImage.naturalHeight;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.drawImage(baseImage, 0, 0);
    drawOverlay(ctx, canvas.width, canvas.height, spec, logoImage);

    const dataUrl: string = await new Promise((resolve, reject) => {
      canvas.toBlob((blob) => {
        if (!blob) {
          reject(new Error("Canvas export produced no image"));
          return;
        }
        const reader = new FileReader();
        reader.addEventListener(
          "load",
          () => resolve(reader.result as string),
          { once: true },
        );
        reader.addEventListener(
          "error",
          () => reject(new Error("Failed to read image data")),
          { once: true },
        );
        reader.readAsDataURL(blob);
      }, "image/png");
    });

    const writeResult = await commands.writePngBase64(path, dataUrl);
    if (writeResult.status !== "ok") {
      toastStore.reportError("Branding bake failed", writeResult.error);
      return;
    }
    bakeStamp.value++;
    toastStore.addToast("Branding baked into the render", "success");
  } catch (error) {
    // The unbranded render on disk is still intact — say so
    toastStore.reportError(
      "Branding bake failed — the plain render is untouched",
      error,
    );
  }
};

/* ------------------------- sticky settings ------------------------- */
// Studio knobs persist across restarts so a dialed-in look isn't lost.
// Rotation is deliberately NOT persisted: it belongs to the model on the
// stage, not to the studio.
const STICKY_KEY = "plinth.renderSettings";
const VIEW_DEFAULTS = { azimuth: -15, elevation: 0.22, zoom: 1.15 };

const persistRenderSettings = () => {
  localStorage.setItem(
    STICKY_KEY,
    JSON.stringify({
      view: view.value,
      matchCamera: matchCamera.value,
      alignParts: alignParts.value,
      resolution: resolution.value,
      samples: samples.value,
      look: look.value,
      colorHex: colorHex.value,
      branding: { ...branding },
    }),
  );
};

/** Restore persisted knobs, ignoring anything malformed or out of range. */
const loadRenderSettings = () => {
  try {
    const raw = localStorage.getItem(STICKY_KEY);
    if (!raw) return;
    const saved = JSON.parse(raw);
    if (typeof saved.matchCamera === "boolean")
      matchCamera.value = saved.matchCamera;
    if (typeof saved.alignParts === "boolean")
      alignParts.value = saved.alignParts;
    if ([512, 1024, 1600, 2048].includes(saved.resolution))
      resolution.value = saved.resolution;
    if ([32, 96, 256].includes(saved.samples)) samples.value = saved.samples;
    if (["rich", "flat", "resin"].includes(saved.look)) look.value = saved.look;
    if (typeof saved.colorHex === "string" && saved.colorHex.startsWith("#"))
      colorHex.value = saved.colorHex;
    if (
      saved.view &&
      [saved.view.azimuth, saved.view.elevation, saved.view.zoom].every(
        (v: unknown) => typeof v === "number",
      )
    ) {
      view.value = { ...saved.view };
      viewport.value?.setView(saved.view);
    }
    if (saved.branding && typeof saved.branding === "object") {
      for (const key of Object.keys(BRANDING_DEFAULTS) as Array<
        keyof typeof BRANDING_DEFAULTS
      >) {
        if (typeof saved.branding[key] === typeof BRANDING_DEFAULTS[key]) {
          // biome/oxlint: keyed assignment keeps both sides' types aligned
          (branding as Record<string, unknown>)[key] = saved.branding[key];
        }
      }
    }
  } catch {
    // a corrupt blob must never break the studio — fall back to defaults
    localStorage.removeItem(STICKY_KEY);
  }
};

const resetRenderSettings = () => {
  view.value = { ...VIEW_DEFAULTS };
  viewport.value?.setView(VIEW_DEFAULTS);
  matchCamera.value = true;
  alignParts.value = false;
  resolution.value = 1600;
  samples.value = 96;
  look.value = "flat";
  colorHex.value = DEFAULT_RESIN_HEX;
  Object.assign(branding, BRANDING_DEFAULTS);
  localStorage.removeItem(STICKY_KEY);
  toastStore.addToast("Render settings reset to defaults", "success");
};

watch(
  [
    view,
    matchCamera,
    alignParts,
    resolution,
    samples,
    look,
    colorHex,
    branding,
  ],
  persistRenderSettings,
  { deep: true },
);

const cornerStyle: Record<CornerPos, Record<string, string>> = {
  tl: { top: "18px", left: "18px" },
  tr: { top: "18px", right: "18px" },
  bl: { bottom: "46px", left: "20px" },
  br: { bottom: "46px", right: "20px" },
};
const textStyle: Record<TextPos, Record<string, string>> = {
  tl: { top: "18px", left: "18px" },
  tc: { top: "18px", left: "0", right: "0", textAlign: "center" },
  tr: { top: "18px", right: "18px", textAlign: "right" },
  bl: { bottom: "46px", left: "20px" },
  bc: { bottom: "46px", left: "0", right: "0", textAlign: "center" },
  br: { bottom: "46px", right: "20px", textAlign: "right" },
};
const logoPosStyle = computed(() => cornerStyle[branding.logoPos]);
const textPosStyle = computed(() => textStyle[branding.textPos]);
const fontCss = computed(
  () =>
    fontOptions.find((f) => f.value === branding.font)?.css ??
    "'Archivo', sans-serif",
);

const blenderInfo = ref<BlenderInfo | null>(null);
const blenderStatus = ref<"unknown" | "found" | "missing">("unknown");

onMounted(async () => {
  loadRenderSettings();
  const result = await commands.detectBlender();
  if (result.status === "ok") {
    blenderInfo.value = result.data;
    blenderStatus.value = "found";
  } else {
    blenderStatus.value = "missing";
  }
});

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
    ? `${convertFileSrc(resultPath.value)}?v=${elapsedSeconds.value}-${bakeStamp.value}`
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
  requestedOutputPath = outputPath.value || defaultOutputPath.value;

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
    // The OS save dialog already asked about replacing an explicit choice;
    // default outputs never overwrite — the backend uniquifies with -N
    overwrite: !!outputPath.value,
    align_parts: alignParts.value,
  });

  if (result.status === "error") {
    toastStore.reportError("Failed to start render", result.error);
  }
};
</script>
