<template>
  <main class="relative flex h-full min-w-0">
    <!-- Controls -->
    <section
      class="w-82.5 shrink-0 border-r border-base-content/10 overflow-y-auto p-4 flex flex-col gap-3.5"
    >
      <div class="flex items-baseline justify-between">
        <span class="font-bold text-[17px]">Render studio</span>
        <span
          class="font-mono text-[10px]"
          :class="
            renderBlocked
              ? 'text-error'
              : blenderInfo
                ? 'text-success'
                : 'text-base-content/40'
          "
          >{{
            blenderInfo
              ? `${blenderInfo.version} ${renderBlocked ? "✗" : "✓"}`
              : verdict === "Missing"
                ? "not found"
                : ""
          }}</span
        >
      </div>

      <div
        v-if="verdict === 'Outdated' && !outdatedHintDismissed"
        class="alert alert-info text-sm"
      >
        <span
          >Previews are tuned for Blender {{ managedVersion }} — yours renders a
          slightly different look.</span
        >
        <button class="btn btn-xs" @click="openDialog">Download</button>
        <button
          class="btn btn-xs btn-ghost"
          @click="outdatedHintDismissed = true"
        >
          ✕
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
            <option value="rich">Rich (soft studio)</option>
            <option value="marmoset">Marmoset (high contrast)</option>
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

      <label
        class="label gap-2 text-sm"
        :class="scaleRefConfigured ? 'cursor-pointer' : 'opacity-50'"
        :title="
          scaleRefConfigured
            ? 'Renders your reference figure in grey beside the model, at true relative size'
            : 'Pick a scale figure STL in Settings first'
        "
      >
        <input
          type="checkbox"
          class="checkbox checkbox-sm"
          :disabled="!scaleRefConfigured"
          v-model="scaleReference"
        />
        Scale figure
      </label>

      <div class="flex flex-col gap-1">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          title="A tapered wargaming plinth under the mini — the hobby's own scale reference"
          >STAND ON BASE</span
        >
        <select
          class="select select-sm select-bordered w-full"
          v-model="baseCutterId"
        >
          <option value="">None</option>
          <optgroup v-if="cutterGroups.rounds.length" label="Rounds">
            <option v-for="c in cutterGroups.rounds" :key="c.id" :value="c.id">
              {{ c.label }}
            </option>
          </optgroup>
          <optgroup v-if="cutterGroups.ovals.length" label="Ovals">
            <option v-for="c in cutterGroups.ovals" :key="c.id" :value="c.id">
              {{ c.label }}
            </option>
          </optgroup>
          <optgroup v-if="cutterGroups.rects.length" label="Squares & rects">
            <option v-for="c in cutterGroups.rects" :key="c.id" :value="c.id">
              {{ c.label }}
            </option>
          </optgroup>
        </select>
      </div>

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

      <RenderAdvanced
        v-model="advanced"
        :look="look"
        @export="exportLook"
        @import="importLook"
      />

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
          :disabled="!parts.length || isRendering || renderBlocked"
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
          class="text-base-content/80 mt-0.5"
          :style="{
            fontFamily: fontCss,
            fontSize: `${branding.size * 0.55}px`,
            fontWeight: 400,
            letterSpacing: '0.04em',
          }"
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

    <!-- Milk-glass: without a usable Blender nothing in this studio can
         run, so the whole tab frosts over and says why instead of
         scattering disabled controls that look broken -->
    <div
      v-if="renderBlocked"
      class="absolute inset-0 z-40 bg-base-100/50 backdrop-blur-md flex items-center justify-center"
    >
      <div
        class="bg-base-100 border border-base-content/10 rounded-xl shadow-xl w-105 max-w-[90vw] p-5 flex flex-col gap-3"
      >
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >RENDER ENGINE</span
        >
        <span class="font-bold text-[15px]">{{
          verdict === "TooOld"
            ? "Your Blender is too old to render"
            : "Rendering needs Blender"
        }}</span>
        <p class="text-[12.5px] text-base-content/70 leading-relaxed">
          <template v-if="verdict === 'TooOld'">
            Promo renders drive Blender headlessly, and
            {{ blenderInfo?.version ?? "your install" }} predates the 4.2
            minimum. Plinth can download its own Blender
            {{ managedVersion }} without touching yours.
          </template>
          <template v-else>
            Promo renders drive Blender headlessly — no Blender, no image.
            Plinth can download its own copy (~350&nbsp;MB), or you can point it
            at an existing install in Settings.
          </template>
        </p>
        <div class="flex justify-end gap-2">
          <button
            class="btn btn-sm"
            @click="releasesStore.setActiveTab('settings')"
          >
            Open Settings
          </button>
          <button class="btn btn-sm btn-primary" @click="openDialog">
            Download Blender {{ managedVersion }}
          </button>
        </div>
      </div>
    </div>
  </main>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import { storeToRefs } from "pinia";
import { computed, onActivated, onMounted, reactive, ref, watch } from "vue";
import { commands } from "../bindings.ts";
import type { Cutter, CutterKind } from "../bindings.ts";
import FileSelect from "../components/FileSelect.vue";
import ProgressBar from "../components/ProgressBar.vue";
import RenderAdvanced from "../components/RenderAdvanced.vue";
// NOT `import type`: the component is used in the template, which
// biome's useImportType can't see (rule disabled for .vue in biome.json)
import StlViewport from "../components/StlViewport.vue";
import { useBlenderProvision } from "../composables/useBlenderProvision";
import { groupCutters } from "../utils/cutterKinds";
import { filesFromPaths, useFileSelect } from "../composables/useFileSelect";
import type { SelectedFile } from "../composables/useFileSelect";
import { useRenderStatus } from "../composables/useRenderStatus";
import { hexToLinear } from "../utils/color";
import { drawOverlay } from "../utils/promoOverlay";
import {
  type LookOverrides,
  overridesToNested,
  sanitizeOverrides,
} from "../utils/renderLookSchema";
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
    const nextTarget = paths.join("\n");
    parts.value = await filesFromPaths(paths);
    previewTargetParts = nextTarget;
    renderParts.value = [];
  },
  // immediate: this view mounts lazily on first visit, AFTER the catalog
  // has already written the handoff into the store — a mount-time watcher
  // alone would miss it and the studio would open with an empty part list
  { immediate: true },
);

const rotation = ref<[number, number, number]>([90, 0, 0]);
// The orientation the in-flight render was started with — what the
// completion handler persists to the catalog (the render IS the chosen
// orientation; batch re-renders reuse it).
let renderedRotation: [number, number, number] = [90, 0, 0];
const view = ref({ azimuth: -15, elevation: 0.22, zoom: 1.15 });
const matchCamera = ref(true);
// Re-seat parts exported around different origins (see StlViewport's
// stackOnBase). Off by default: on correctly-exported multi-part minis
// the re-seat would WRONGLY collapse well-placed parts onto the base.
const alignParts = ref(false);
// Scale figure ("banana for scale"): the toggle only says "include it" —
// which STL and how tall live in Settings, so the checkbox greys out until
// a figure is configured there.
const scaleReference = ref(false);
const scaleRefConfigured = ref(false);
const refreshScaleRefConfigured = async () => {
  const result = await commands.getSettings();
  scaleRefConfigured.value =
    result.status === "ok" && !!result.data.scale_reference_path?.trim();
  if (!scaleRefConfigured.value) scaleReference.value = false;
};
// Stand on base: the hobby's OWN scale reference — a standard tapered
// plinth under the mini. Library comes from the same get_cutter_library
// command the Base Cutter tool uses (docs/BASECUTTER.md: "tool-agnostic").
// "" = None; the id (not the CutterKind) is what the <select> can v-model.
const cutterLibrary = ref<Cutter[]>([]);
const baseCutterId = ref("");
const loadCutterLibrary = async () => {
  cutterLibrary.value = await commands.getCutterLibrary();
};
const cutterGroups = computed(() => groupCutters(cutterLibrary.value));
const selectedBaseKind = computed<CutterKind | null>(
  () =>
    cutterLibrary.value.find((c) => c.id === baseCutterId.value)?.kind ?? null,
);
const resolution = ref(1600);
const samples = ref(96);
// "flat" (the handover's locked look) won the three-way comparison against
// the DTL reference; "resin" adds the physical-print sheen on top of it
const look = ref<"rich" | "flat" | "resin" | "marmoset">("flat");
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
// Advanced look overrides: only the diff from the locked recipe, keyed by
// LOOK dot-path ("key.energy"). Empty record = stock look.
const advanced = ref<LookOverrides>({});
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
    // The render IS the chosen orientation — persist it so batch re-renders
    // and future studio sessions start from it instead of the 90,0,0 guess.
    // (Rotation used to be deliberately session-only; a render aimed at the
    // catalog is the moment it becomes a fact about the model.)
    const rotationResult = await commands.setModelRotation(
      renderPreviewTarget.value,
      renderedRotation,
    );
    if (rotationResult.status === "error") {
      // non-fatal: the preview landed, only the orientation memo failed
      toastStore.reportError("Failed to store rotation", rotationResult.error);
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
  {
    value: "Archivo",
    label: "Archivo",
    css: "'Archivo', sans-serif",
  },
  {
    value: "Bebas Neue",
    label: "Bebas Neue",
    css: "'Bebas Neue', sans-serif",
  },
  {
    value: "Cormorant Garamond",
    label: "Cormorant",
    css: "'Cormorant Garamond', serif",
  },
  {
    value: "IBM Plex Mono",
    label: "IBM Plex Mono",
    css: "'IBM Plex Mono', monospace",
  },
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

watch(partPaths, (paths, previous) => {
  if (!previous?.length || paths.join("\n") === previous.join("\n")) return;
  // Overlay copy belongs to the subject, unlike reusable studio choices
  // such as font, position and size. Never carry one model's title into the
  // next model loaded while the studio remains mounted.
  branding.title = "";
  branding.credit = "";
});

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
      document.fonts.load(`400 16px ${fontCss.value}`),
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
// Valid values for restore/import — must match the <select> options above
const RESOLUTION_OPTIONS = new Set([512, 1024, 1600, 2048]);
const SAMPLES_OPTIONS = new Set([32, 96, 256]);
const LOOK_OPTIONS = new Set(["rich", "flat", "resin", "marmoset"]);

const persistRenderSettings = () => {
  // Copy is model-specific. Persist only reusable branding presentation;
  // otherwise the next model silently inherits the previous one's title.
  const { title: _title, credit: _credit, ...brandingPreferences } = branding;
  localStorage.setItem(
    STICKY_KEY,
    JSON.stringify({
      view: view.value,
      matchCamera: matchCamera.value,
      alignParts: alignParts.value,
      scaleReference: scaleReference.value,
      resolution: resolution.value,
      samples: samples.value,
      look: look.value,
      colorHex: colorHex.value,
      advanced: advanced.value,
      branding: brandingPreferences,
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
    if (typeof saved.scaleReference === "boolean")
      scaleReference.value = saved.scaleReference;
    if (RESOLUTION_OPTIONS.has(saved.resolution))
      resolution.value = saved.resolution;
    if (SAMPLES_OPTIONS.has(saved.samples)) samples.value = saved.samples;
    if (LOOK_OPTIONS.has(saved.look)) look.value = saved.look;
    if (typeof saved.colorHex === "string" && saved.colorHex.startsWith("#"))
      colorHex.value = saved.colorHex;
    // Schema-validated: knobs that vanish in an update (or hand-edited
    // garbage) drop out silently instead of riding along to Blender
    if (saved.advanced)
      advanced.value = sanitizeOverrides(saved.advanced).overrides;
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
        // Older builds persisted model copy. Deliberately do not restore it.
        if (key === "title" || key === "credit") continue;
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

/* ------------------------- shareable look files ------------------------- */
// A look file carries everything that shapes the rendered PIXELS — look,
// resin color, camera, quality, and the advanced overrides. Branding stays
// out on purpose: logo paths are machine-local, and title/credit belong to
// a model, not to a look.
const LOOK_FILE_KIND = "plinth-look";
const LOOK_FILE_VERSION = 1;

const exportLook = async () => {
  const path = await save({
    defaultPath: "render-look.json",
    filters: [{ name: "Plinth look", extensions: ["json"] }],
  });
  if (!path) return;
  const payload = {
    kind: LOOK_FILE_KIND,
    version: LOOK_FILE_VERSION,
    look: look.value,
    colorHex: colorHex.value,
    view: { ...view.value },
    resolution: resolution.value,
    samples: samples.value,
    overrides: advanced.value,
  };
  const result = await commands.writeLookJson(
    path,
    JSON.stringify(payload, null, 2),
  );
  if (result.status === "ok") {
    toastStore.addToast("Look exported — share the .json freely", "success");
  } else {
    toastStore.reportError("Failed to export look", result.error);
  }
};

const importLook = async () => {
  const selected = await open({
    multiple: false,
    filters: [{ name: "Plinth look", extensions: ["json"] }],
    title: "Import look file",
  });
  if (typeof selected !== "string") return;
  const result = await commands.readLookJson(selected);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to import look", result.error);
    return;
  }
  let raw: unknown;
  try {
    raw = JSON.parse(result.data);
  } catch {
    toastStore.addToast("That file is not valid JSON", "error");
    return;
  }
  const file = (raw ?? {}) as Record<string, unknown>;
  if (file.kind !== LOOK_FILE_KIND) {
    toastStore.addToast("That file is not a Plinth look", "error");
    return;
  }
  if (typeof file.version === "number" && file.version > LOOK_FILE_VERSION) {
    toastStore.addToast(
      "This look was made by a newer version of the app — update to import it",
      "error",
    );
    return;
  }

  // Same defensive posture as loadRenderSettings: apply what validates,
  // skip the rest, and SAY how much was skipped — a silently half-applied
  // look would render differently than on the machine that shared it
  const ignored: string[] = [];
  const applyIf = (present: boolean, valid: boolean, name: string) => {
    if (present && !valid) ignored.push(name);
    return present && valid;
  };

  if (
    applyIf(
      file.look !== undefined,
      typeof file.look === "string" && LOOK_OPTIONS.has(file.look),
      "look",
    )
  ) {
    look.value = file.look as "rich" | "flat" | "resin" | "marmoset";
  }
  if (
    applyIf(
      file.colorHex !== undefined,
      typeof file.colorHex === "string" &&
        /^#[0-9a-f]{6}$/i.test(file.colorHex),
      "resin color",
    )
  ) {
    colorHex.value = file.colorHex as string;
  }
  const v = (file.view ?? {}) as Record<string, unknown>;
  if (
    applyIf(
      file.view !== undefined,
      [v.azimuth, v.elevation, v.zoom].every(
        (n) => typeof n === "number" && Number.isFinite(n),
      ),
      "camera",
    )
  ) {
    view.value = {
      azimuth: v.azimuth as number,
      elevation: v.elevation as number,
      zoom: v.zoom as number,
    };
    viewport.value?.setView(view.value);
  }
  if (
    applyIf(
      file.resolution !== undefined,
      RESOLUTION_OPTIONS.has(file.resolution as number),
      "resolution",
    )
  ) {
    resolution.value = file.resolution as number;
  }
  if (
    applyIf(
      file.samples !== undefined,
      SAMPLES_OPTIONS.has(file.samples as number),
      "quality",
    )
  ) {
    samples.value = file.samples as number;
  }
  const { overrides, dropped } = sanitizeOverrides(file.overrides);
  advanced.value = overrides;
  ignored.push(...dropped);

  if (ignored.length) {
    toastStore.addToast(
      `Look imported — ignored ${ignored.length} unknown or invalid setting${
        ignored.length === 1 ? "" : "s"
      } (${ignored.join(", ")})`,
      "info",
      8000,
    );
  } else {
    toastStore.addToast("Look imported", "success");
  }
};

const resetRenderSettings = () => {
  view.value = { ...VIEW_DEFAULTS };
  viewport.value?.setView(VIEW_DEFAULTS);
  viewport.value?.resetPan();
  matchCamera.value = true;
  alignParts.value = false;
  scaleReference.value = false;
  baseCutterId.value = "";
  resolution.value = 1600;
  samples.value = 96;
  look.value = "flat";
  colorHex.value = DEFAULT_RESIN_HEX;
  advanced.value = {};
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
    advanced,
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

// Shared with the setup dialog and Settings — the app probes once per
// launch, and an install landing mid-session flips this badge live even
// though KeepAlive never remounts this view.
const { blenderInfo, verdict, renderBlocked, managedVersion, openDialog } =
  useBlenderProvision();
// Session-only: an Outdated Blender still renders, the hint shouldn't nag
const outdatedHintDismissed = ref(false);

onMounted(() => {
  loadRenderSettings();
  refreshScaleRefConfigured();
  loadCutterLibrary();
});
// keep-alive'd view: the user may configure a figure in Settings and come back
onActivated(() => {
  refreshScaleRefConfigured();
});

const colorLinear = computed<[number, number, number]>(() =>
  hexToLinear(colorHex.value),
);

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
  // Snapshot THIS job's orientation: the viewport stays live during the
  // render, and the auto-save on completion must record what was rendered,
  // not whatever the user is fiddling with by then
  renderedRotation = [...rotation.value];

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
    look_config: Object.keys(advanced.value).length
      ? JSON.stringify(overridesToNested(advanced.value))
      : null,
    scale_reference: scaleReference.value,
    base: selectedBaseKind.value,
  });

  if (result.status === "error") {
    toastStore.reportError("Failed to start render", result.error);
  }
};
</script>
