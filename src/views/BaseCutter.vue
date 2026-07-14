<script setup lang="ts">
/**
 * Base Cutter tool: place standard base cutters over a landscape sculpt and
 * cut them out with the headless-Blender job pipeline (basecutter::job).
 * See docs/BASECUTTER.md — this view owns all placement state; the
 * viewport (LandscapeViewport) is a dumb renderer + drag/select/rotate
 * input surface that emits update/select/delete events.
 */
import { computed, onMounted, reactive, ref, watch } from "vue";
import type {
  BaseCutJob,
  Cutter,
  CutterKind,
  MagnetSpec,
  Placement,
  PlinthParams,
} from "../bindings";
import { commands } from "../bindings";
import LandscapeViewport from "../components/LandscapeViewport.vue";
import ProgressBar from "../components/ProgressBar.vue";
import { useBaseCut } from "../composables/useBaseCut";
import { useBlenderProvision } from "../composables/useBlenderProvision";
import { selectDirectory, useFileSelect } from "../composables/useFileSelect";
import { useReleasesStore } from "../stores/releasesStore";
import { useToastStore } from "../stores/toastStore";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { selectFiles } = useFileSelect();
const baseCut = useBaseCut();
// The cut path hard-requires Blender >= 4.2 (wm.stl_import/export), same as
// Render.vue's gate — reuse that composable/verdict rather than inventing a
// second Blender-detection mechanism.
const { blenderInfo, verdict, renderBlocked, managedVersion, openDialog } =
  useBlenderProvision();

const landscapePath = ref("");
const landscapeBounds = ref<{ centerX: number; centerY: number } | null>(null);
const outDir = ref("");

const cutterLibrary = ref<Cutter[]>([]);

// Pre-load initial state only — overwritten by commands.getPlinthDefaults()
// on mount below. The Rust default is caliper-measured and test-pinned
// (basecutter::cutters::PlinthParams's Default impl, see docs/BASECUTTER.md),
// so it's the runtime source of truth; this literal just avoids a blank
// form flashing before the command resolves.
const plinth = reactive<PlinthParams>({
  height_mm: 3.7,
  taper_deg: 15.0,
  hollow: true,
  wall_mm: 1.2,
  top_mm: 1.2,
  magnet_clearance_mm: 0.15,
});

onMounted(async () => {
  const [library, plinthDefaults] = await Promise.all([
    commands.getCutterLibrary(),
    commands.getPlinthDefaults(),
  ]);
  cutterLibrary.value = library;
  Object.assign(plinth, plinthDefaults);
});

const rounds = computed(() =>
  cutterLibrary.value.filter((c) => c.kind.kind === "circle"),
);
const ovals = computed(() =>
  cutterLibrary.value.filter((c) => c.kind.kind === "ellipse"),
);
const rects = computed(() =>
  cutterLibrary.value.filter((c) => c.kind.kind === "rect"),
);

const placements = ref<Placement[]>([]);
const selectedIndex = ref<number | null>(null);
const selectedPlacement = computed(() =>
  selectedIndex.value !== null ? placements.value[selectedIndex.value] : null,
);

const MAGNET_PRESETS: { label: string; spec: MagnetSpec }[] = [
  { label: "5×1", spec: { diameter_mm: 5, height_mm: 1, count: 1 } },
  { label: "6×2", spec: { diameter_mm: 6, height_mm: 2, count: 1 } },
  { label: "10×2", spec: { diameter_mm: 10, height_mm: 2, count: 1 } },
];

/** Human label for a placement's cutter kind — display only, doesn't need
 * to byte-match the backend seed library's labels (JS number->string
 * already drops trailing zeros the way fmt_mm does in Rust). */
const cutterLabel = (kind: CutterKind): string => {
  switch (kind.kind) {
    case "circle":
      return `${kind.diameter_mm} mm round`;
    case "ellipse":
      return `${kind.major_mm}×${kind.minor_mm} mm oval`;
    case "rect":
      return kind.width_mm === kind.depth_mm
        ? `${kind.width_mm} mm square`
        : `${kind.width_mm}×${kind.depth_mm} mm rect`;
  }
};

/** "round32-1", "square25-2" — cutter id (dashes stripped) + 1-past the
 * highest numeric suffix currently in use for that slug. Deliberately not a
 * count of survivors: delete round32-2 out of {1,2,3} and a naive count
 * (now 2 survivors) would hand the next placement "round32-2" again — same
 * name as a still-live placement, so the job silently overwrites one
 * output STL with another. Taking 1 + max(existing suffixes) instead never
 * reuses a name that's still on the list. */
const nextName = (cutterId: string): string => {
  const slug = cutterId.replace(/-/g, "");
  const prefix = `${slug}-`;
  let maxSuffix = 0;
  for (const p of placements.value) {
    if (!p.name?.startsWith(prefix)) continue;
    const suffix = Number(p.name.slice(prefix.length));
    if (Number.isFinite(suffix)) maxSuffix = Math.max(maxSuffix, suffix);
  }
  return `${prefix}${maxSuffix + 1}`;
};

// Placement mutation is locked out while a job is running: the job already
// took a snapshot (jobPlacementNames below) and mid-job add/delete would
// desync indices between the live array and the in-flight cut list.
const locked = computed(() => baseCut.isRunning.value);

const addPlacement = (cutter: Cutter) => {
  if (locked.value) return;
  if (!landscapeBounds.value) {
    toastStore.addToast("Choose a landscape STL first", "info");
    return;
  }
  placements.value.push({
    cutter: cutter.kind,
    x_mm: landscapeBounds.value.centerX,
    y_mm: landscapeBounds.value.centerY,
    rotation_deg: 0,
    magnet: null,
    name: nextName(cutter.id),
  });
  selectedIndex.value = placements.value.length - 1;
};

const rotatePlacement = (index: number, deltaDeg: number) => {
  if (locked.value) return;
  const p = placements.value[index];
  if (!p) return;
  p.rotation_deg = (((p.rotation_deg + deltaDeg) % 360) + 360) % 360;
};

const deletePlacement = (index: number) => {
  if (locked.value) return;
  placements.value.splice(index, 1);
  if (selectedIndex.value === index) selectedIndex.value = null;
  else if (selectedIndex.value !== null && selectedIndex.value > index) {
    selectedIndex.value--;
  }
};

const isMagnetPreset = (spec: MagnetSpec) => {
  const m = selectedPlacement.value?.magnet;
  return (
    !!m && m.diameter_mm === spec.diameter_mm && m.height_mm === spec.height_mm
  );
};

const setMagnet = (spec: MagnetSpec | null) => {
  if (!selectedPlacement.value) return;
  selectedPlacement.value.magnet = spec;
};

/* ---- viewport wiring: the view owns placement state, the viewport is a
   dumb drag/select/rotate input surface ---- */
const onSelect = (index: number | null) => {
  selectedIndex.value = index;
};
const onUpdatePlacement = (index: number, patch: Partial<Placement>) => {
  if (locked.value) return;
  const p = placements.value[index];
  if (p) Object.assign(p, patch);
};
const onDeletePlacement = (index: number) => deletePlacement(index);
const onLandscapeLoaded = (bounds: { centerX: number; centerY: number }) => {
  landscapeBounds.value = bounds;
};
const onViewportError = (message: string) => {
  toastStore.addToast(message, "error", 0);
};

const chooseLandscape = async () => {
  const files = await selectFiles({
    accept: ".stl",
    multiple: false,
    title: "Choose landscape STL",
  });
  if (!files?.length) return;
  const newPath = files[0].path;
  if (newPath === landscapePath.value) return; // re-picking the same file
  if (placements.value.length) {
    placements.value = [];
    toastStore.addToast(
      "Placements cleared — coordinates belong to the previous landscape",
      "info",
    );
  }
  selectedIndex.value = null;
  landscapePath.value = newPath;
};

const chooseOutDir = async () => {
  const dir = await selectDirectory({ title: "Choose output folder" });
  if (dir) outDir.value = dir;
};

const canCut = computed(
  () =>
    !!landscapePath.value &&
    placements.value.length > 0 &&
    !!outDir.value &&
    !baseCut.isRunning.value,
);

// Names as they were when the job was submitted — progress/result labels
// resolve from this snapshot, never the live `placements` array, so a name
// stays stable even though editing is locked out anyway while running (see
// `locked`). Belt-and-suspenders against index drift, not just UI lockout.
const jobPlacementNames = ref<(string | null)[]>([]);

const startCut = async () => {
  if (!canCut.value) return;
  jobPlacementNames.value = placements.value.map((p) => p.name);
  const job: BaseCutJob = {
    landscape: landscapePath.value,
    placements: placements.value,
    plinth: { ...plinth },
    out_dir: outDir.value,
  };
  const result = await baseCut.start(job);
  if (result.status === "error") {
    toastStore.reportError("Failed to start base cut", result.error);
  }
};

const cancelCut = () => baseCut.cancel();

// Surface terminal states as toasts — the results list already shows the
// per-cut detail, this is just the headline. Watching the composable's own
// projections (instead of re-discriminating the raw status union here)
// keeps the "what counts as finished/failed/cancelled" logic in one place.
watch(baseCut.finishedSummary, (summary) => {
  if (!summary) return;
  const { ok_count, total } = summary;
  toastStore.addToast(
    `Cut ${ok_count}/${total} base${total === 1 ? "" : "s"}`,
    ok_count === total ? "success" : "warning",
  );
});
watch(baseCut.failedMessage, (message) => {
  if (!message) return;
  toastStore.addToast(`Base cut failed: ${message}`, "error", 0);
});
watch(baseCut.cancelled, (isCancelled) => {
  if (!isCancelled) return;
  toastStore.addToast("Base cut cancelled", "info");
});

const stepLabel = computed(() => {
  const status = baseCut.status.value;
  if (!status) return "";
  if ("Validating" in status) return "Validating landscape…";
  if ("Validated" in status) return "Validated — cutting…";
  if ("CutStarted" in status) {
    const name = jobPlacementNames.value[status.CutStarted.index];
    return `Cutting ${name ?? status.CutStarted.index + 1}…`;
  }
  if ("CutDone" in status || "CutFailed" in status) {
    return `${baseCut.results.value.length} / ${baseCut.total.value} done`;
  }
  return "Starting…";
});

const resultName = (index: number) =>
  jobPlacementNames.value[index] ?? `#${index + 1}`;
</script>

<template>
  <main class="relative flex h-full min-w-0">
    <section
      class="w-82.5 shrink-0 border-r border-base-content/10 overflow-y-auto p-4 flex flex-col gap-3.5"
    >
      <div class="flex items-baseline justify-between">
        <span class="font-bold text-[17px]">Base Cutter</span>
      </div>

      <div class="flex flex-col gap-1">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >LANDSCAPE</span
        >
        <div class="flex">
          <input
            type="text"
            readonly
            class="input input-sm flex-1 font-mono text-[11px]"
            :value="landscapePath || 'No landscape selected'"
            :title="landscapePath"
          />
          <button class="btn btn-sm" @click="chooseLandscape">
            Choose STL…
          </button>
        </div>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >CUTTER PALETTE — CLICK TO PLACE</span
        >
        <div class="flex flex-col gap-1.5">
          <div>
            <div class="text-[10.5px] text-base-content/50 mb-1">Rounds</div>
            <div class="flex flex-wrap gap-1">
              <button
                v-for="c in rounds"
                :key="c.id"
                type="button"
                class="btn btn-xs"
                :disabled="locked"
                @click="addPlacement(c)"
              >
                {{ c.label }}
              </button>
            </div>
          </div>
          <div>
            <div class="text-[10.5px] text-base-content/50 mb-1">Ovals</div>
            <div class="flex flex-wrap gap-1">
              <button
                v-for="c in ovals"
                :key="c.id"
                type="button"
                class="btn btn-xs"
                :disabled="locked"
                @click="addPlacement(c)"
              >
                {{ c.label }}
              </button>
            </div>
          </div>
          <div>
            <div class="text-[10.5px] text-base-content/50 mb-1">
              Squares / rects
            </div>
            <div class="flex flex-wrap gap-1">
              <button
                v-for="c in rects"
                :key="c.id"
                type="button"
                class="btn btn-xs"
                :disabled="locked"
                @click="addPlacement(c)"
              >
                {{ c.label }}
              </button>
            </div>
          </div>
        </div>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >PLACEMENTS ({{ placements.length }})</span
        >
        <ul
          v-if="placements.length"
          class="flex flex-col gap-1 max-h-48 overflow-y-auto"
        >
          <li
            v-for="(p, i) in placements"
            :key="i"
            class="flex items-center gap-1.5 px-2 py-1.5 rounded border cursor-pointer text-[12px]"
            :class="
              i === selectedIndex
                ? 'bg-primary/10 border-primary'
                : 'border-base-content/10 hover:border-base-content/30'
            "
            @click="selectedIndex = i"
          >
            <span class="flex-1 truncate font-medium">{{ p.name }}</span>
            <span class="text-base-content/50 font-mono text-[10px]">{{
              cutterLabel(p.cutter)
            }}</span>
            <span
              class="font-mono text-[10px] text-base-content/40 w-9 text-right"
              >{{ Math.round(p.rotation_deg) }}°</span
            >
            <span
              v-if="p.magnet"
              class="badge badge-xs badge-info"
              title="Magnet pocket"
              >M</span
            >
            <button
              type="button"
              class="btn btn-ghost btn-xs px-1"
              title="Rotate -15°"
              :disabled="locked"
              @click.stop="rotatePlacement(i, -15)"
            >
              ↺
            </button>
            <button
              type="button"
              class="btn btn-ghost btn-xs px-1"
              title="Rotate +15°"
              :disabled="locked"
              @click.stop="rotatePlacement(i, 15)"
            >
              ↻
            </button>
            <button
              type="button"
              class="btn btn-ghost btn-xs px-1 text-error"
              title="Delete placement"
              :disabled="locked"
              @click.stop="deletePlacement(i)"
            >
              ✕
            </button>
          </li>
        </ul>
        <p v-else class="text-[11px] text-base-content/40">
          Pick a landscape, then click a cutter above to place one at its
          center.
        </p>

        <div
          v-if="selectedPlacement"
          class="flex flex-col gap-1.5 border-t border-base-content/10 pt-2 mt-1"
        >
          <span
            class="font-mono text-[10px] tracking-widest text-base-content/40"
            >MAGNET — {{ selectedPlacement.name }}</span
          >
          <div class="flex flex-wrap gap-1.5">
            <button
              type="button"
              class="btn btn-xs"
              :class="!selectedPlacement.magnet ? 'btn-primary' : ''"
              @click="setMagnet(null)"
            >
              None
            </button>
            <button
              v-for="preset in MAGNET_PRESETS"
              :key="preset.label"
              type="button"
              class="btn btn-xs"
              :class="isMagnetPreset(preset.spec) ? 'btn-primary' : ''"
              @click="setMagnet(preset.spec)"
            >
              {{ preset.label }}
            </button>
          </div>
        </div>
      </div>

      <details
        class="collapse collapse-arrow border border-base-content/10 bg-base-200/20 rounded-box"
      >
        <summary
          class="collapse-title min-h-0 py-2.5 px-3 flex items-center gap-2 cursor-pointer"
        >
          <span
            class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
            >ADVANCED — PLINTH</span
          >
        </summary>
        <div class="collapse-content flex flex-col gap-2 px-3">
          <label class="flex items-center gap-2 text-[12px]">
            <span class="w-28 shrink-0">Height (mm)</span>
            <input
              type="number"
              step="0.1"
              class="input input-xs flex-1"
              v-model.number="plinth.height_mm"
            />
          </label>
          <label class="flex items-center gap-2 text-[12px]">
            <span class="w-28 shrink-0">Taper (°)</span>
            <input
              type="number"
              step="0.5"
              class="input input-xs flex-1"
              v-model.number="plinth.taper_deg"
            />
          </label>
          <label class="flex items-center gap-2 text-[12px] cursor-pointer">
            <input
              type="checkbox"
              class="checkbox checkbox-xs"
              v-model="plinth.hollow"
            />
            Hollow (open-bottom shell)
          </label>
          <label class="flex items-center gap-2 text-[12px]">
            <span class="w-28 shrink-0">Wall (mm)</span>
            <input
              type="number"
              step="0.1"
              class="input input-xs flex-1"
              v-model.number="plinth.wall_mm"
            />
          </label>
          <label class="flex items-center gap-2 text-[12px]">
            <span class="w-28 shrink-0">Top plate (mm)</span>
            <input
              type="number"
              step="0.1"
              class="input input-xs flex-1"
              v-model.number="plinth.top_mm"
            />
          </label>
          <label class="flex items-center gap-2 text-[12px]">
            <span class="w-28 shrink-0">Magnet clearance</span>
            <input
              type="number"
              step="0.05"
              class="input input-xs flex-1"
              v-model.number="plinth.magnet_clearance_mm"
            />
          </label>
        </div>
      </details>

      <div class="flex flex-col gap-1">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >OUTPUT FOLDER</span
        >
        <div class="flex">
          <input
            type="text"
            readonly
            class="input input-sm flex-1 font-mono text-[11px]"
            :value="outDir || 'No folder selected'"
            :title="outDir"
          />
          <button class="btn btn-sm" @click="chooseOutDir">Choose…</button>
        </div>
      </div>

      <div class="flex items-center gap-3">
        <button
          class="btn btn-primary grow"
          :disabled="!canCut"
          @click="startCut"
        >
          <template v-if="baseCut.isRunning.value">
            <span class="loading loading-spinner"></span>
            <span>Cutting…</span>
          </template>
          <span v-else
            >Cut {{ placements.length }} base{{
              placements.length === 1 ? "" : "s"
            }}</span
          >
        </button>
        <button
          v-if="baseCut.isRunning.value"
          class="btn btn-error"
          @click="cancelCut"
        >
          Cancel
        </button>
      </div>

      <div v-if="baseCut.isRunning.value" class="flex items-center gap-3">
        <ProgressBar :progress="baseCut.percent.value" />
        <span class="text-sm opacity-70">{{ stepLabel }}</span>
      </div>

      <div
        v-if="baseCut.validationWarning.value"
        class="alert alert-warning text-xs whitespace-pre-wrap"
      >
        {{ baseCut.validationWarning.value }}
      </div>

      <div
        v-if="baseCut.failedMessage.value"
        class="alert alert-error text-xs whitespace-pre-wrap flex-col items-start"
      >
        <span>{{ baseCut.failedMessage.value }}</span>
        <pre
          v-if="baseCut.failedStdoutTail.value"
          class="font-mono text-[10px] opacity-70 whitespace-pre-wrap mt-1"
          >{{ baseCut.failedStdoutTail.value }}</pre
        >
      </div>

      <div v-if="baseCut.results.value.length" class="flex flex-col gap-1">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >RESULTS</span
        >
        <ul class="flex flex-col gap-1 text-[12px]">
          <li
            v-for="r in baseCut.results.value"
            :key="r.index"
            class="flex items-center gap-2"
          >
            <span
              :class="
                r.ok && r.manifold
                  ? 'text-success'
                  : r.ok
                    ? 'text-warning'
                    : 'text-error'
              "
              >{{ r.ok ? (r.manifold ? "✓" : "⚠") : "✗" }}</span
            >
            <span class="flex-1 truncate">{{ resultName(r.index) }}</span>
            <span v-if="!r.ok" class="text-error text-[11px] truncate">{{
              r.reason
            }}</span>
            <span v-else-if="!r.manifold" class="text-warning text-[11px]"
              >non-manifold</span
            >
          </li>
        </ul>
      </div>
    </section>

    <aside class="flex-1 min-w-0 relative">
      <LandscapeViewport
        :landscape-path="landscapePath"
        :placements="placements"
        :plinth="plinth"
        :selected-index="selectedIndex"
        :locked="locked"
        @select="onSelect"
        @update="onUpdatePlacement"
        @delete="onDeletePlacement"
        @loaded="onLandscapeLoaded"
        @error="onViewportError"
      />
    </aside>

    <!-- Milk-glass: without a usable Blender the cut path can't run at all
         (wm.stl_import/export need >= 4.2), so the whole tab frosts over
         and says why — mirrors Render.vue's gate on the same verdict. -->
    <div
      v-if="renderBlocked"
      class="absolute inset-0 z-40 bg-base-100/50 backdrop-blur-md flex items-center justify-center"
    >
      <div
        class="bg-base-100 border border-base-content/10 rounded-xl shadow-xl w-105 max-w-[90vw] p-5 flex flex-col gap-3"
      >
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >BASE CUTTER</span
        >
        <span class="font-bold text-[15px]">{{
          verdict === "TooOld"
            ? "Your Blender is too old to cut bases"
            : "Base Cutter needs Blender"
        }}</span>
        <p class="text-[12.5px] text-base-content/70 leading-relaxed">
          <template v-if="verdict === 'TooOld'">
            Cutting drives Blender headlessly and needs
            <code>wm.stl_import</code>/<code>wm.stl_export</code>, which only
            exist from 4.2 — {{ blenderInfo?.version ?? "your install" }}
            predates that. Plinth can download its own Blender
            {{ managedVersion }} without touching yours.
          </template>
          <template v-else>
            Cutting drives Blender headlessly for STL import/export (4.2+ only)
            — no Blender, no cut. Plinth can download its own copy
            (~350&nbsp;MB), or you can point it at an existing install in
            Settings.
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
