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
  BouldersLayer,
  CamberLayer,
  CatalogRootSummary,
  Cutter,
  CutterKind,
  FlowLayer,
  GeneratorPreset,
  LandscapeParams,
  MagnetSpec,
  NoiseLayer,
  Placement,
  PlinthParams,
  RipplesLayer,
  StonesLayer,
} from "../bindings";
import { commands } from "../bindings";
import LandscapeViewport from "../components/LandscapeViewport.vue";
import NumberInput from "../components/NumberInput.vue";
import ProgressBar from "../components/ProgressBar.vue";
import Switch from "../components/Switch.vue";
import { useBaseCut } from "../composables/useBaseCut";
import { useBlenderProvision } from "../composables/useBlenderProvision";
import { selectDirectory, useFileSelect } from "../composables/useFileSelect";
import { useLandscapeGen } from "../composables/useLandscapeGen";
import { useReleasesStore } from "../stores/releasesStore";
import { useToastStore } from "../stores/toastStore";
import { cloneRaw } from "../utils/cloneRaw";
import { groupCutters } from "../utils/cutterKinds";
import { MAX_MAGNET_COUNT, suggestMagnet } from "../utils/magnetSuggest";
import {
  type GeneratedPlacement,
  mulberry32,
  regimentExtent,
  regimentPlacements,
  scatterPlacements,
} from "../utils/placementGenerators";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { selectFiles } = useFileSelect();
const baseCut = useBaseCut();
const landscapeGen = useLandscapeGen();
// The cut path hard-requires Blender >= 4.2 (wm.stl_import/export), same as
// Render.vue's gate — reuse that composable/verdict rather than inventing a
// second Blender-detection mechanism.
const { blenderInfo, verdict, renderBlocked, managedVersion, openDialog } =
  useBlenderProvision();

const landscapePath = ref("");
/** Bumped whenever the landscape must reload even at an unchanged path —
 * a regenerated bake overwrites its own file. */
const landscapeReloadToken = ref(0);
/** The landscape's full XY extent, as emitted by LandscapeViewport's
 * `loaded` event — centerX/centerY feed "place at landscape center"
 * (addPlacement, regiment default center), min/max feed the generators
 * (regimentExtent's fit check, scatterPlacements' bounds). */
type LandscapeBounds = {
  centerX: number;
  centerY: number;
  minX: number;
  maxX: number;
  minY: number;
  maxY: number;
};
const landscapeBounds = ref<LandscapeBounds | null>(null);

/** Where cut STLs land — remembered across sessions (localStorage, like the
 * theme: it only feeds the job payload, no backend round-trip needed). */
const OUT_DIR_STORAGE_KEY = "plinth-basecutter-out-dir";
const outDir = ref(localStorage.getItem(OUT_DIR_STORAGE_KEY) ?? "");
watch(outDir, (dir) => localStorage.setItem(OUT_DIR_STORAGE_KEY, dir));

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

// Generator state (docs/BASECUTTER.md "The landscape generator (phase 6)").
// `genParams` is a plain literal, not yet a real preset, until onMounted's
// commands.getLandscapePresets() resolves and selectPreset() overwrites it —
// same "avoid a blank form flash" reasoning as `plinth` above.
//
// The bindings' LandscapeParams/*Layer types mark every layer field
// optional (specta reflecting Rust's #[serde(default)], which is about
// lenient DEserialization) even though Rust always SERIALIZES every field —
// a preset from get_landscape_presets() or a hand-built literal here is
// therefore always fully populated at runtime. GenParams asserts that in
// the type system too, so the template's v-model bindings (Switch requires
// a definite `boolean`, not `boolean | undefined`) don't need per-field
// `?? false` scattered everywhere.
type GenLayers = {
  noise: Required<NoiseLayer>;
  ripples: Required<RipplesLayer>;
  stones: Required<StonesLayer>;
  boulders: Required<BouldersLayer>;
  flow: Required<FlowLayer>;
  camber: Required<CamberLayer>;
};
type GenParams = Omit<LandscapeParams, "layers"> & { layers: GenLayers };

const landscapePresets = ref<GeneratorPreset[]>([]);
const selectedPresetId = ref<string | null>(null);
const genParams = reactive<GenParams>({
  seed: 1,
  width_mm: 120,
  depth_mm: 80,
  resolution_mm: 0.75,
  feature_scale: 1.0,
  carrier_mm: 2.0,
  relief_mm: 6.0,
  layers: {
    noise: {
      enabled: false,
      scale: 0.05,
      octaves: 4,
      ridged: false,
      amount: 1.0,
    },
    ripples: {
      enabled: false,
      wavelength_mm: 8.0,
      direction_deg: 0.0,
      amount: 1.0,
      waviness: 0.3,
    },
    stones: {
      enabled: false,
      cell_mm: 4.0,
      gap_mm: 0.5,
      dome: 0.6,
      jitter: 0.15,
      cluster: 0.0,
      rough: 0.0,
      amount: 1.0,
    },
    boulders: {
      enabled: false,
      count: 6,
      min_mm: 8.0,
      max_mm: 20.0,
      amount: 1.0,
    },
    flow: {
      enabled: false,
      channel_width_mm: 10.0,
      meander_scale: 0.3,
      bank_height: 1.0,
      amount: 1.0,
    },
    camber: { enabled: false, amount: 1.0 },
  },
});

/** Load a preset's params into the editable `genParams` (a deep copy — the
 * preset table itself must never be mutated by editing). See GenParams'
 * comment for why the cast is safe: the wire payload is always fully
 * populated even though the generated type marks layer fields optional. */
const selectPreset = (preset: GeneratorPreset) => {
  // cloneRaw, NOT bare structuredClone: presets clicked in the template
  // are reactive Proxies (they live in a ref), and structuredClone throws
  // DataCloneError on Proxies — the params copy silently died in Vue's
  // error handler while the chip highlighted, so every bake kept the
  // first-loaded preset's terrain. The id is set only after the copy
  // succeeds so the highlight can never desync from the actual params.
  Object.assign(genParams, cloneRaw(preset.params) as GenParams);
  selectedPresetId.value = preset.id;
};

/** Reroll to a fresh random seed, keeping the rest of the params (preset or
 * hand-tweaked) as they are — a new roll of the same style, not a reset. */
const rerollSeed = () => {
  genParams.seed = Math.floor(Math.random() * 0xffffffff);
};

/** The user's magnet inventory (app settings — docs/BASECUTTER.md "Hollow,
 * with magnet mounts"): what the per-placement magnet panel offers as
 * chips and what suggestMagnet() picks from. Loaded once on mount, same
 * as the cutter library and plinth defaults; Settings.vue is the only
 * place that edits it. */
const magnetInventory = ref<MagnetSpec[]>([]);

/** Configured catalog folders (docs/BASECUTTER.md phase 5,
 * "export-into-catalog") — feeds the "Add to catalog…" root picker below
 * the results list. Loaded once on mount like the rest of this block;
 * list_catalog_roots already resolves which one (if any) is primary, so no
 * separate settings read is needed. */
const catalogRoots = ref<CatalogRootSummary[]>([]);
/** Selected destination folder — defaulted to catalog_primary_root (via its
 * `primary` flag on the loaded list) once roots resolve below. */
const exportRoot = ref("");
/** Group-name field, prefilled from the landscape's own file name the first
 * time a job finishes with a keeper (see the finishedSummary watcher) —
 * never overwritten after that, so a user edit survives a later job. */
const exportGroupName = ref("");
const exportBusy = ref(false);

onMounted(async () => {
  const [library, plinthDefaults, presets, settingsResult, rootsResult] =
    await Promise.all([
      commands.getCutterLibrary(),
      commands.getPlinthDefaults(),
      commands.getLandscapePresets(),
      commands.getSettings(),
      commands.listCatalogRoots(),
    ]);
  cutterLibrary.value = library;
  Object.assign(plinth, plinthDefaults);
  landscapePresets.value = presets;
  if (presets.length) selectPreset(presets[0]);
  // Failed loads must SAY so: an empty inventory/roots list otherwise
  // renders the same "add some in Settings" hints a genuinely empty
  // config gets, gaslighting a user whose data simply failed to load.
  if (settingsResult.status === "ok") {
    magnetInventory.value = settingsResult.data.magnet_inventory ?? [];
  } else {
    toastStore.reportError(
      "Failed to load settings — magnet inventory unavailable",
      settingsResult.error,
    );
  }
  if (rootsResult.status === "ok") {
    catalogRoots.value = rootsResult.data;
    exportRoot.value =
      rootsResult.data.find((r) => r.primary)?.root ??
      rootsResult.data[0]?.root ??
      "";
  } else {
    toastStore.reportError(
      "Failed to load catalog folders — export to catalog unavailable",
      rootsResult.error,
    );
  }
});

// The palette shows one shape family at a time: all 24 cutters as
// always-visible chips read as a wall of buttons that dwarfed the rest of
// the panel. A family tab + dimension-only chips keeps click-to-place one
// click (two when switching family) while fitting in a couple of rows.
const cutterGroups = computed(() => groupCutters(cutterLibrary.value));
const PALETTE_FAMILIES = [
  { key: "rounds", label: "Rounds" },
  { key: "ovals", label: "Ovals" },
  { key: "rects", label: "Squares & rects" },
] as const;
const paletteFamily = ref<(typeof PALETTE_FAMILIES)[number]["key"]>("rounds");
const paletteCutters = computed(() => cutterGroups.value[paletteFamily.value]);

/** The generators' cutter, held by ID and edited two ways: the dedicated
 * select in the GENERATORS block picks WITHOUT placing (selecting a size
 * for bulk generation must not drop a stray base as a side effect), while
 * a palette chip click ALSO records itself here — the click already places
 * deliberately, so inheriting it as the generator default costs nothing. */
const generatorCutterId = ref("");
const generatorCutter = computed(
  () =>
    cutterLibrary.value.find((c) => c.id === generatorCutterId.value) ?? null,
);

/** Dimension-only chip label — the active family tab already says
 * round/oval/rect, so repeating "mm round" on every chip is pure noise.
 * The full label rides along as the chip's title tooltip. */
const sizeLabel = (kind: CutterKind): string => {
  switch (kind.kind) {
    case "circle":
      return `${kind.diameter_mm}`;
    case "ellipse":
      return `${kind.major_mm}×${kind.minor_mm}`;
    case "rect":
      return kind.width_mm === kind.depth_mm
        ? `${kind.width_mm}`
        : `${kind.width_mm}×${kind.depth_mm}`;
  }
};

const placements = ref<Placement[]>([]);
const selectedIndex = ref<number | null>(null);
const selectedPlacement = computed(() =>
  selectedIndex.value !== null ? placements.value[selectedIndex.value] : null,
);

/** One chip per inventory magnet — size only (diameter x height); count is
 * a separate per-placement control (see the COUNT button group below), not
 * part of the chip identity, so picking a different size never resets an
 * already-chosen count. */
const magnetChips = computed(() =>
  magnetInventory.value.map((spec) => ({
    label: `${spec.diameter_mm}×${spec.height_mm}`,
    spec,
  })),
);

/** Human label for a placement's cutter kind — sizeLabel's dimensions plus
 * a unit and shape noun, so the numeric formatting lives in exactly one
 * place. Display only: doesn't need to byte-match the backend seed
 * library's labels (JS number->string already drops trailing zeros the way
 * fmt_mm does in Rust). */
const cutterLabel = (kind: CutterKind): string => {
  const noun =
    kind.kind === "circle"
      ? "round"
      : kind.kind === "ellipse"
        ? "oval"
        : kind.width_mm === kind.depth_mm
          ? "square"
          : "rect";
  return `${sizeLabel(kind)} mm ${noun}`;
};

/** "round32-1", "square25-2" — cutter id (dashes stripped) + 1-past the
 * highest numeric suffix currently in use for that slug. Deliberately not a
 * count of survivors: delete round32-2 out of {1,2,3} and a naive count
 * (now 2 survivors) would hand the next placement "round32-2" again — same
 * name as a still-live placement, so the job silently overwrites one
 * output STL with another. Taking 1 + max(existing suffixes) instead never
 * reuses a name that's still on the list. */
const nextNames = (cutterId: string, count: number): string[] => {
  const slug = cutterId.replace(/-/g, "");
  const prefix = `${slug}-`;
  let maxSuffix = 0;
  for (const p of placements.value) {
    if (!p.name?.startsWith(prefix)) continue;
    const suffix = Number(p.name.slice(prefix.length));
    if (Number.isFinite(suffix)) maxSuffix = Math.max(maxSuffix, suffix);
  }
  // One scan mints the whole batch — the generators name dozens at a time,
  // and re-scanning a growing list per name is quadratic for no benefit.
  return Array.from(
    { length: count },
    (_, i) => `${prefix}${maxSuffix + 1 + i}`,
  );
};
const nextName = (cutterId: string): string => nextNames(cutterId, 1)[0];

// Placement mutation is locked out while a job is running: the job already
// took a snapshot (jobPlacementNames below) and mid-job add/delete would
// desync indices between the live array and the in-flight cut list.
// Placement mutation is also locked while a landscape bake is in flight —
// the bake may swap `landscapePath` out from under any in-progress edits
// (see setLandscapePath's clear-on-swap logic).
const locked = computed(
  () => baseCut.isRunning.value || landscapeGen.isRunning.value,
);

const addPlacement = (cutter: Cutter) => {
  generatorCutterId.value = cutter.id;
  // The palette already disables itself while locked or landscape-less
  // (a greyed button beats a click-then-scold toast); the guard stays for
  // any future non-palette caller.
  if (locked.value || !landscapeBounds.value) return;
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

// ---- generators (docs/BASECUTTER.md phase 6): regiment (grid) and
// scatter (random, non-overlapping) placement of the palette's currently
// selected cutter. Both append to `placements` (never wipe it) and name
// each new placement through the same `nextName` the palette uses, so
// generated and hand-placed bases share one naming sequence.
const GENERATOR_MODES = [
  { key: "regiment", label: "Regiment" },
  { key: "scatter", label: "Scatter" },
] as const;
const generatorMode = ref<(typeof GENERATOR_MODES)[number]["key"]>("regiment");
const regimentRows = ref(2);
const regimentCols = ref(5);
const regimentGapMm = ref(0);
const scatterCount = ref(10);
// Hard ceilings, mirrored as `max` on the inputs and clamped again at run
// time: the scatter's rejection sampling is O(count x attempts x
// obstacles) SYNCHRONOUS on the main thread, so an unbounded count typed
// into the field could freeze the webview for seconds. 20x20 regiments and
// 200 scattered bases are already far past any real tabletop batch.
const MAX_REGIMENT_DIM = 20;
const MAX_SCATTER_COUNT = 200;

/** Why the generator buttons are disabled — the palette's own tooltip
 * convention (a greyed button with a `title`, never a click-then-toast). */
const generatorBlockedReason = computed(() => {
  if (locked.value) return "Locked while a job is running";
  if (!landscapeBounds.value) return "Load or generate a landscape first";
  if (!generatorCutter.value) return "Pick a cutter size first";
  return "";
});

/** What placeRegiment will actually place (clamped) — the button label
 * must promise the clamped number, not the raw typed one. */
const regimentPlannedCount = computed(
  () =>
    Math.min(Math.max(regimentRows.value, 0), MAX_REGIMENT_DIM) *
    Math.min(Math.max(regimentCols.value, 0), MAX_REGIMENT_DIM),
);

const canPlaceRegiment = computed(
  () =>
    !generatorBlockedReason.value &&
    regimentRows.value > 0 &&
    regimentCols.value > 0,
);
const canScatter = computed(
  () => !generatorBlockedReason.value && scatterCount.value > 0,
);

/** Warns (inline text, not a toast — see docs/BASECUTTER.md phase 6 and
 * the palette's own "disabled beats click-then-scold" convention) when the
 * regiment as configured would spill outside the landscape. It still gets
 * placed: cuts outside the sculpt simply fail per-cut with a reason,
 * that's the pipeline's job, not this preview's. */
const regimentOutOfBounds = computed(() => {
  if (
    !generatorCutter.value ||
    !landscapeBounds.value ||
    regimentRows.value <= 0 ||
    regimentCols.value <= 0
  ) {
    return false;
  }
  const b = landscapeBounds.value;
  const ext = regimentExtent(
    generatorCutter.value,
    Math.min(regimentRows.value, MAX_REGIMENT_DIM),
    Math.min(regimentCols.value, MAX_REGIMENT_DIM),
    regimentGapMm.value,
    { x: b.centerX, y: b.centerY },
  );
  return (
    ext.minX < b.minX ||
    ext.maxX > b.maxX ||
    ext.minY < b.minY ||
    ext.maxY > b.maxY
  );
});

/** Append a generated batch, minting all names in one placements scan. */
const pushGenerated = (cutterId: string, generated: GeneratedPlacement[]) => {
  if (!generated.length) return;
  const names = nextNames(cutterId, generated.length);
  placements.value.push(...generated.map((g, i) => ({ ...g, name: names[i] })));
  selectedIndex.value = placements.value.length - 1;
};

const placeRegiment = () => {
  if (
    !canPlaceRegiment.value ||
    !generatorCutter.value ||
    !landscapeBounds.value
  ) {
    return;
  }
  const cutter = generatorCutter.value;
  const b = landscapeBounds.value;
  const generated = regimentPlacements(
    cutter,
    Math.min(regimentRows.value, MAX_REGIMENT_DIM),
    Math.min(regimentCols.value, MAX_REGIMENT_DIM),
    regimentGapMm.value,
    { x: b.centerX, y: b.centerY },
  );
  pushGenerated(cutter.id, generated);
};

const runScatter = () => {
  if (!canScatter.value || !generatorCutter.value || !landscapeBounds.value)
    return;
  const cutter = generatorCutter.value;
  const requested = Math.min(scatterCount.value, MAX_SCATTER_COUNT);
  const generated = scatterPlacements(
    cutter,
    requested,
    landscapeBounds.value,
    placements.value,
    mulberry32(Date.now() >>> 0),
  );
  pushGenerated(cutter.id, generated);
  toastStore.addToast(
    `Scattered ${generated.length} of ${requested} — ${
      generated.length < requested
        ? "ran out of room without overlapping"
        : "placed"
    }`,
    generated.length === requested ? "success" : "warning",
  );
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

const isMagnetSize = (spec: MagnetSpec) => {
  const m = selectedPlacement.value?.magnet;
  return (
    !!m && m.diameter_mm === spec.diameter_mm && m.height_mm === spec.height_mm
  );
};

/** Picking a size chip sets diameter/height and keeps whatever count is
 * already on the placement (defaulting to 1 the first time a magnet is
 * added) — count is edited independently via the button group below. */
const setMagnetSize = (spec: MagnetSpec) => {
  if (!selectedPlacement.value) return;
  const count = selectedPlacement.value.magnet?.count ?? 1;
  selectedPlacement.value.magnet = {
    diameter_mm: spec.diameter_mm,
    height_mm: spec.height_mm,
    count,
  };
};

const clearMagnet = () => {
  if (!selectedPlacement.value) return;
  selectedPlacement.value.magnet = null;
};

const setMagnetCount = (count: number) => {
  if (!selectedPlacement.value?.magnet) return;
  selectedPlacement.value.magnet.count = count;
};

/** The suggestion rule (docs/BASECUTTER.md: "the tool suggests the largest
 * inventory magnet whose boss fits the base's top face") for the selected
 * placement — badged on the matching chip, never auto-applied. */
const suggestedMagnet = computed(() => {
  if (!selectedPlacement.value) return null;
  return suggestMagnet(
    selectedPlacement.value.cutter,
    plinth,
    magnetInventory.value,
  );
});

const isSuggestedMagnet = (spec: MagnetSpec) => {
  const s = suggestedMagnet.value;
  return (
    !!s &&
    s.spec.diameter_mm === spec.diameter_mm &&
    s.spec.height_mm === spec.height_mm
  );
};

/** Applies both the suggested size AND count in one action — the only way
 * a suggestion ever changes a placement, since picking a size chip alone
 * deliberately preserves the existing count instead. */
const applySuggestedMagnet = () => {
  if (!selectedPlacement.value || !suggestedMagnet.value) return;
  // count comes from the suggestion, NOT the inventory spec — inventory
  // rows always carry count 1, so spreading the spec alone would apply a
  // "suggested ×2" as a single magnet (and leave the button visible).
  selectedPlacement.value.magnet = {
    ...suggestedMagnet.value.spec,
    count: suggestedMagnet.value.count,
  };
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
const onLandscapeLoaded = (bounds: LandscapeBounds) => {
  landscapeBounds.value = bounds;
};
const onViewportError = (message: string) => {
  toastStore.addToast(message, "error", 0);
};

/** Swap in a different landscape STL — from the file picker OR a freshly
 * generated one (below). Existing placements' coordinates belong to the
 * PREVIOUS landscape, so they're cleared rather than silently reinterpreted
 * against the new one. */
const setLandscapePath = (newPath: string, options?: { force?: boolean }) => {
  // Re-picking the same FILE is a no-op — but a fresh bake OVERWRITES its
  // file (preset+seed = stable filename), so generation passes force: the
  // path may be identical while the terrain underneath is brand new. That
  // was the "I generated 200x200 and still saw 120x80" bug: the viewport
  // only reloads when something it watches changes.
  if (newPath === landscapePath.value && !options?.force) return;
  if (placements.value.length) {
    placements.value = [];
    toastStore.addToast(
      "Placements cleared — coordinates belong to the previous landscape",
      "info",
    );
  }
  selectedIndex.value = null;
  landscapePath.value = newPath;
  landscapeReloadToken.value++;
};

const chooseLandscape = async () => {
  const files = await selectFiles({
    accept: ".stl",
    multiple: false,
    title: "Choose landscape STL",
  });
  if (!files?.length) return;
  setLandscapePath(files[0].path);
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
    !baseCut.isRunning.value &&
    !landscapeGen.isRunning.value, // generation and cutting share Blender — never both at once
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

const canGenerate = computed(
  () =>
    genParams.width_mm > 0 &&
    genParams.depth_mm > 0 &&
    genParams.relief_mm >= 0 &&
    !baseCut.isRunning.value && // generation and cutting share Blender — never both at once
    !landscapeGen.isRunning.value,
);

const startGenerate = async () => {
  if (!canGenerate.value) return;
  const result = await landscapeGen.start(genParams, selectedPresetId.value);
  if (result.status === "error") {
    toastStore.reportError(
      "Failed to start landscape generation",
      result.error,
    );
  }
};

const cancelGenerate = () => landscapeGen.cancel();

// On a finished bake, auto-load the fresh STL into the viewport — the same
// swap path the file picker uses, so stale placements get cleared too.
watch(landscapeGen.finished, (finished) => {
  if (!finished) return;
  setLandscapePath(finished.out_path, { force: true });
  const [w, d, h] = finished.dims_mm;
  toastStore.addToast(
    `Generated landscape (${w}×${d}×${h}mm)${finished.manifold ? "" : " — non-manifold"}`,
    finished.manifold ? "success" : "warning",
  );
});
watch(landscapeGen.failedMessage, (message) => {
  if (!message) return;
  toastStore.addToast(`Landscape generation failed: ${message}`, "error", 0);
});
watch(landscapeGen.cancelled, (isCancelled) => {
  if (!isCancelled) return;
  toastStore.addToast("Landscape generation cancelled", "info");
});

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
  // Seed the "Add to catalog" group name from the landscape's own file name
  // the first time there's something to export — never overrides a name
  // the user already typed (this fires again on every finish, including a
  // second job in the same session).
  if (ok_count > 0 && !exportGroupName.value.trim()) {
    const base = landscapePath.value.split(/[/\\]/).pop() ?? "";
    const stem = base
      .replace(/\.stl$/i, "")
      .replace(/[_-]+/g, " ")
      .trim();
    exportGroupName.value = stem || "Cut bases";
  }
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

/* ---- export into the catalog (docs/BASECUTTER.md phase 5) ----
 * Cut output stays local/catalog-bound — this only ever copies into a
 * configured catalog folder, never into the release/share pipeline
 * (Releases.vue / file::commands::create_release): licensing covers
 * personal printing, not redistribution. */

const successfulOutPaths = computed(() =>
  baseCut.results.value
    .filter((r) => r.ok && r.out_path)
    .map((r) => r.out_path as string),
);
const hasSuccessfulResults = computed(
  () => !baseCut.isRunning.value && successfulOutPaths.value.length > 0,
);

/** Folder basename for the picker's option label — the full path is the
 * title tooltip, same "short label, full path on hover" convention as the
 * landscape/output-folder path fields above. */
const rootLabel = (root: string) => root.split(/[/\\]/).pop() || root;

const exportBlockedReason = computed(() => {
  if (!catalogRoots.value.length) return "Add a catalog folder in Settings";
  if (!exportRoot.value) return "Choose a catalog folder";
  if (!exportGroupName.value.trim()) return "Enter a group name";
  if (exportBusy.value) return "Adding to catalog…";
  return "";
});
const canExportToCatalog = computed(() => !exportBlockedReason.value);

const exportToCatalog = async () => {
  if (!canExportToCatalog.value) return;
  exportBusy.value = true;
  try {
    const result = await commands.exportCutsToCatalog(
      successfulOutPaths.value,
      exportRoot.value,
      exportGroupName.value.trim(),
    );
    if (result.status === "error") {
      toastStore.reportError("Failed to add bases to catalog", result.error);
      return;
    }
    const destDir = result.data;
    // Kick a rescan of the destination root so the new bases show up
    // without a manual rescan — a failure here doesn't undo the copy, it
    // just means the catalog view is stale until the next scan.
    const scanResult = await commands.startCatalogScan(exportRoot.value);
    if (scanResult.status === "error") {
      toastStore.reportError(
        `Added to catalog at ${destDir}, but the rescan didn't start`,
        scanResult.error,
      );
      return;
    }
    toastStore.addToast(
      `Added ${successfulOutPaths.value.length} base${
        successfulOutPaths.value.length === 1 ? "" : "s"
      } to catalog — ${destDir}`,
      "success",
    );
  } finally {
    exportBusy.value = false;
  }
};
</script>

<template>
  <main class="relative flex h-full min-w-0">
    <section
      class="w-82.5 shrink-0 border-r border-base-content/10 overflow-y-auto p-4 flex flex-col gap-3.5"
    >
      <div class="flex items-baseline justify-between">
        <span class="font-bold text-[17px]">Base Cutter</span>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >GENERATE LANDSCAPE</span
        >
        <div class="flex flex-wrap gap-1">
          <button
            v-for="preset in landscapePresets"
            :key="preset.id"
            type="button"
            class="btn btn-xs"
            :class="preset.id === selectedPresetId ? 'btn-primary' : ''"
            :disabled="landscapeGen.isRunning.value"
            @click="selectPreset(preset)"
          >
            {{ preset.label }}
          </button>
        </div>
        <div class="flex items-center gap-1.5">
          <span class="text-[11px] text-base-content/50 shrink-0">Seed</span>
          <input
            type="number"
            class="input input-xs flex-1 font-mono"
            :disabled="landscapeGen.isRunning.value"
            v-model.number="genParams.seed"
          />
          <button
            type="button"
            class="btn btn-xs"
            title="Reroll seed"
            :disabled="landscapeGen.isRunning.value"
            @click="rerollSeed"
          >
            🎲
          </button>
        </div>

        <details
          class="collapse collapse-arrow border border-base-content/10 bg-base-200/20 rounded-box"
        >
          <summary
            class="collapse-title min-h-0 py-2.5 px-3 flex items-center gap-2 cursor-pointer"
          >
            <span
              class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
              >ADVANCED — TERRAIN LAYERS</span
            >
          </summary>
          <div class="collapse-content flex flex-col gap-2.5 px-3">
            <NumberInput
              id="gen-width"
              label="Width (mm)"
              :step="1"
              :min="10"
              v-model="genParams.width_mm"
            />
            <NumberInput
              id="gen-depth"
              label="Depth (mm)"
              :step="1"
              :min="10"
              v-model="genParams.depth_mm"
            />
            <!-- 0.1mm is resin-grade; the script coarsens automatically if
                 the step would blow the vertex budget on a big plate and
                 reports the effective value back. -->
            <NumberInput
              id="gen-resolution"
              label="Resolution (mm, finest 0.1)"
              :step="0.05"
              :min="0.1"
              v-model="genParams.resolution_mm"
            />
            <!-- Zooms the terrain itself (stone/dune/boulder sizes), not
                 the mesh density — that's Resolution above. -->
            <NumberInput
              id="gen-feature-scale"
              label="Feature scale ×"
              :step="0.1"
              :min="0.25"
              :max="4"
              v-model="genParams.feature_scale"
            />
            <NumberInput
              id="gen-carrier"
              label="Carrier (mm)"
              :step="0.1"
              :min="0"
              v-model="genParams.carrier_mm"
            />
            <NumberInput
              id="gen-relief"
              label="Relief (mm)"
              :step="0.1"
              :min="0"
              v-model="genParams.relief_mm"
            />

            <div
              class="flex flex-col gap-2 border-t border-base-content/10 pt-2"
            >
              <div class="flex flex-col gap-1">
                <Switch
                  v-model="genParams.layers.noise.enabled"
                  label="Noise"
                />
                <template v-if="genParams.layers.noise.enabled">
                  <NumberInput
                    id="gen-noise-scale"
                    label="Scale"
                    :step="0.01"
                    :min="0"
                    v-model="genParams.layers.noise.scale"
                  />
                  <NumberInput
                    id="gen-noise-octaves"
                    label="Octaves"
                    :step="1"
                    :min="1"
                    :max="8"
                    v-model="genParams.layers.noise.octaves"
                  />
                  <Switch
                    v-model="genParams.layers.noise.ridged"
                    label="Ridged (sharp crests)"
                  />
                  <NumberInput
                    id="gen-noise-amount"
                    label="Amount"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.noise.amount"
                  />
                </template>
              </div>

              <div class="flex flex-col gap-1">
                <Switch
                  v-model="genParams.layers.ripples.enabled"
                  label="Ripples"
                />
                <template v-if="genParams.layers.ripples.enabled">
                  <NumberInput
                    id="gen-ripples-wavelength"
                    label="Wavelength (mm)"
                    :step="0.5"
                    :min="0.1"
                    v-model="genParams.layers.ripples.wavelength_mm"
                  />
                  <NumberInput
                    id="gen-ripples-direction"
                    label="Direction (deg)"
                    :step="5"
                    v-model="genParams.layers.ripples.direction_deg"
                  />
                  <NumberInput
                    id="gen-ripples-waviness"
                    label="Waviness"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.ripples.waviness"
                  />
                  <NumberInput
                    id="gen-ripples-amount"
                    label="Amount"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.ripples.amount"
                  />
                </template>
              </div>

              <div class="flex flex-col gap-1">
                <Switch
                  v-model="genParams.layers.stones.enabled"
                  label="Stones"
                />
                <template v-if="genParams.layers.stones.enabled">
                  <NumberInput
                    id="gen-stones-cell"
                    label="Cell size (mm)"
                    :step="0.5"
                    :min="1"
                    v-model="genParams.layers.stones.cell_mm"
                  />
                  <NumberInput
                    id="gen-stones-gap"
                    label="Gap / mortar (mm)"
                    :step="0.1"
                    :min="0"
                    v-model="genParams.layers.stones.gap_mm"
                  />
                  <NumberInput
                    id="gen-stones-dome"
                    label="Dome (0-1)"
                    :step="0.05"
                    :min="0"
                    :max="1"
                    v-model="genParams.layers.stones.dome"
                  />
                  <NumberInput
                    id="gen-stones-jitter"
                    label="Height jitter"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.stones.jitter"
                  />
                  <NumberInput
                    id="gen-stones-cluster"
                    label="Cluster (0-1, lava crust)"
                    :step="0.05"
                    :min="0"
                    :max="1"
                    v-model="genParams.layers.stones.cluster"
                  />
                  <NumberInput
                    id="gen-stones-rough"
                    label="Edge roughness (0-1)"
                    :step="0.05"
                    :min="0"
                    :max="1"
                    v-model="genParams.layers.stones.rough"
                  />
                  <NumberInput
                    id="gen-stones-amount"
                    label="Amount"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.stones.amount"
                  />
                </template>
              </div>

              <div class="flex flex-col gap-1">
                <Switch
                  v-model="genParams.layers.boulders.enabled"
                  label="Boulders"
                />
                <template v-if="genParams.layers.boulders.enabled">
                  <NumberInput
                    id="gen-boulders-count"
                    label="Count"
                    :step="1"
                    :min="0"
                    v-model="genParams.layers.boulders.count"
                  />
                  <NumberInput
                    id="gen-boulders-min"
                    label="Min diameter (mm)"
                    :step="1"
                    :min="1"
                    v-model="genParams.layers.boulders.min_mm"
                  />
                  <NumberInput
                    id="gen-boulders-max"
                    label="Max diameter (mm)"
                    :step="1"
                    :min="1"
                    v-model="genParams.layers.boulders.max_mm"
                  />
                  <NumberInput
                    id="gen-boulders-amount"
                    label="Amount"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.boulders.amount"
                  />
                </template>
              </div>

              <div class="flex flex-col gap-1">
                <Switch v-model="genParams.layers.flow.enabled" label="Flow" />
                <template v-if="genParams.layers.flow.enabled">
                  <NumberInput
                    id="gen-flow-width"
                    label="Channel width (mm)"
                    :step="0.5"
                    :min="0.5"
                    v-model="genParams.layers.flow.channel_width_mm"
                  />
                  <NumberInput
                    id="gen-flow-meander"
                    label="Meander scale"
                    :step="0.05"
                    :min="0.01"
                    v-model="genParams.layers.flow.meander_scale"
                  />
                  <NumberInput
                    id="gen-flow-bank"
                    label="Bank height"
                    :step="0.1"
                    :min="0.05"
                    v-model="genParams.layers.flow.bank_height"
                  />
                  <NumberInput
                    id="gen-flow-amount"
                    label="Amount"
                    :step="0.05"
                    :min="0"
                    v-model="genParams.layers.flow.amount"
                  />
                </template>
              </div>

              <div class="flex flex-col gap-1">
                <Switch
                  v-model="genParams.layers.camber.enabled"
                  label="Camber"
                />
                <NumberInput
                  v-if="genParams.layers.camber.enabled"
                  id="gen-camber-amount"
                  label="Amount"
                  :step="0.05"
                  :min="0"
                  v-model="genParams.layers.camber.amount"
                />
              </div>
            </div>
          </div>
        </details>

        <div class="flex items-center gap-3">
          <button
            class="btn btn-secondary btn-sm grow"
            :disabled="!canGenerate"
            @click="startGenerate"
          >
            <template v-if="landscapeGen.isRunning.value">
              <span class="loading loading-spinner loading-xs"></span>
              <span>Generating…</span>
            </template>
            <span v-else>Generate landscape</span>
          </button>
          <button
            v-if="landscapeGen.isRunning.value"
            class="btn btn-error btn-sm"
            @click="cancelGenerate"
          >
            Cancel
          </button>
        </div>
        <div
          v-if="landscapeGen.failedMessage.value"
          class="alert alert-error text-xs whitespace-pre-wrap flex-col items-start"
        >
          <span>{{ landscapeGen.failedMessage.value }}</span>
          <pre
            v-if="landscapeGen.failedStdoutTail.value"
            class="font-mono text-[10px] opacity-70 whitespace-pre-wrap mt-1"
            >{{ landscapeGen.failedStdoutTail.value }}</pre
          >
        </div>
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
        <div class="flex gap-1">
          <button
            v-for="f in PALETTE_FAMILIES"
            :key="f.key"
            type="button"
            class="btn btn-xs"
            :class="paletteFamily === f.key ? 'btn-primary' : 'btn-ghost'"
            @click="paletteFamily = f.key"
          >
            {{ f.label }}
          </button>
        </div>
        <div
          class="flex flex-wrap gap-1"
          :title="landscapeBounds ? '' : 'Load or generate a landscape first'"
        >
          <button
            v-for="c in paletteCutters"
            :key="c.id"
            type="button"
            class="btn btn-xs font-mono"
            :class="generatorCutterId === c.id ? 'btn-primary' : ''"
            :disabled="locked || !landscapeBounds"
            :title="c.label"
            @click="addPlacement(c)"
          >
            {{ sizeLabel(c.kind) }}
          </button>
        </div>
        <p v-if="!landscapeBounds" class="text-[10.5px] text-base-content/40">
          Load or generate a landscape to start placing.
        </p>
      </div>

      <div class="flex flex-col gap-1.5">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >GENERATORS</span
        >
        <div class="flex gap-1">
          <button
            v-for="m in GENERATOR_MODES"
            :key="m.key"
            type="button"
            class="btn btn-xs"
            :class="generatorMode === m.key ? 'btn-primary' : 'btn-ghost'"
            @click="generatorMode = m.key"
          >
            {{ m.label }}
          </button>
        </div>
        <!-- Own picker, NOT just "last palette click": selecting a size for
             bulk generation through the palette would place a stray base as
             a side effect. A palette click still pre-fills this. -->
        <select
          class="select select-xs font-mono"
          v-model="generatorCutterId"
          :disabled="locked"
        >
          <option value="" disabled>Pick a cutter…</option>
          <optgroup label="Rounds">
            <option v-for="c in cutterGroups.rounds" :key="c.id" :value="c.id">
              {{ c.label }}
            </option>
          </optgroup>
          <optgroup label="Ovals">
            <option v-for="c in cutterGroups.ovals" :key="c.id" :value="c.id">
              {{ c.label }}
            </option>
          </optgroup>
          <optgroup label="Squares & rects">
            <option v-for="c in cutterGroups.rects" :key="c.id" :value="c.id">
              {{ c.label }}
            </option>
          </optgroup>
        </select>

        <div v-if="generatorMode === 'regiment'" class="flex flex-col gap-1.5">
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] text-base-content/50 shrink-0 w-10"
              >Rows</span
            >
            <input
              type="number"
              min="1"
              :max="MAX_REGIMENT_DIM"
              step="1"
              class="input input-xs flex-1 font-mono"
              v-model.number="regimentRows"
            />
            <span class="text-[11px] text-base-content/50 shrink-0 w-10"
              >Cols</span
            >
            <input
              type="number"
              min="1"
              :max="MAX_REGIMENT_DIM"
              step="1"
              class="input input-xs flex-1 font-mono"
              v-model.number="regimentCols"
            />
          </div>
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] text-base-content/50 shrink-0 w-10"
              >Gap</span
            >
            <input
              type="number"
              min="0"
              step="0.5"
              class="input input-xs flex-1 font-mono"
              v-model.number="regimentGapMm"
            />
            <span class="text-[10.5px] text-base-content/40 shrink-0">mm</span>
          </div>
          <button
            type="button"
            class="btn btn-xs btn-secondary"
            :disabled="!canPlaceRegiment"
            :title="generatorBlockedReason"
            @click="placeRegiment"
          >
            Place regiment ({{ regimentPlannedCount }})
          </button>
          <p v-if="regimentOutOfBounds" class="text-[10.5px] text-warning">
            regiment extends past the landscape
          </p>
        </div>

        <div v-else class="flex flex-col gap-1.5">
          <div class="flex items-center gap-1.5">
            <span class="text-[11px] text-base-content/50 shrink-0 w-10"
              >Count</span
            >
            <input
              type="number"
              min="1"
              :max="MAX_SCATTER_COUNT"
              step="1"
              class="input input-xs flex-1 font-mono"
              v-model.number="scatterCount"
            />
          </div>
          <button
            type="button"
            class="btn btn-xs btn-secondary"
            :disabled="!canScatter"
            :title="generatorBlockedReason"
            @click="runScatter"
          >
            Scatter
          </button>
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
              >{{ p.magnet.count > 1 ? `M×${p.magnet.count}` : "M" }}</span
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
          Click a cutter above to place one at the landscape center.
        </p>

        <div
          v-if="selectedPlacement"
          class="flex flex-col gap-1.5 border-t border-base-content/10 pt-2 mt-1"
        >
          <span
            class="font-mono text-[10px] tracking-widest text-base-content/40"
            >MAGNET — {{ selectedPlacement.name }}</span
          >
          <div class="flex flex-wrap gap-1.5 items-center">
            <button
              type="button"
              class="btn btn-xs"
              :class="!selectedPlacement.magnet ? 'btn-primary' : ''"
              @click="clearMagnet"
            >
              None
            </button>
            <button
              v-for="chip in magnetChips"
              :key="chip.label"
              type="button"
              class="btn btn-xs gap-1"
              :class="isMagnetSize(chip.spec) ? 'btn-primary' : ''"
              @click="setMagnetSize(chip.spec)"
            >
              {{ chip.label }}
              <span
                v-if="isSuggestedMagnet(chip.spec)"
                class="badge badge-xs badge-accent"
                >suggested{{
                  suggestedMagnet && suggestedMagnet.count > 1
                    ? ` ×${suggestedMagnet.count}`
                    : ""
                }}</span
              >
            </button>
            <button
              v-if="
                suggestedMagnet &&
                (!selectedPlacement.magnet ||
                  !isMagnetSize(suggestedMagnet.spec) ||
                  selectedPlacement.magnet.count !== suggestedMagnet.count)
              "
              type="button"
              class="btn btn-xs btn-outline btn-accent"
              @click="applySuggestedMagnet"
            >
              Use suggested
            </button>
            <span
              v-if="!magnetChips.length"
              class="text-[10.5px] text-base-content/40"
            >
              No magnets in your inventory — add some in Settings.
            </span>
          </div>
          <div
            v-if="selectedPlacement.magnet"
            class="flex items-center gap-1.5"
          >
            <span class="text-[10.5px] text-base-content/50">Count</span>
            <div class="flex gap-1">
              <button
                v-for="n in MAX_MAGNET_COUNT"
                :key="n"
                type="button"
                class="btn btn-xs"
                :class="
                  selectedPlacement.magnet.count === n ? 'btn-primary' : ''
                "
                @click="setMagnetCount(n)"
              >
                {{ n }}
              </button>
            </div>
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

        <div
          v-if="hasSuccessfulResults"
          class="flex flex-col gap-1.5 border-t border-base-content/10 pt-2 mt-1"
        >
          <span
            class="font-mono text-[10px] tracking-widest text-base-content/40"
            >ADD TO CATALOG</span
          >
          <template v-if="catalogRoots.length">
            <select
              class="select select-xs w-full font-mono"
              v-model="exportRoot"
              :disabled="exportBusy"
            >
              <option
                v-for="root in catalogRoots"
                :key="root.root"
                :value="root.root"
                :title="root.root"
              >
                {{ rootLabel(root.root) }}{{ root.primary ? " (primary)" : "" }}
              </option>
            </select>
            <input
              type="text"
              class="input input-xs w-full"
              placeholder="Group name"
              :disabled="exportBusy"
              v-model="exportGroupName"
            />
          </template>
          <p v-else class="text-[10.5px] text-base-content/40">
            No catalog folder configured — add one in Settings.
          </p>
          <button
            type="button"
            class="btn btn-xs btn-secondary"
            :disabled="!canExportToCatalog"
            :title="exportBlockedReason"
            @click="exportToCatalog"
          >
            <template v-if="exportBusy">
              <span class="loading loading-spinner loading-xs"></span>
              <span>Adding…</span>
            </template>
            <span v-else
              >Add {{ successfulOutPaths.length }} base{{
                successfulOutPaths.length === 1 ? "" : "s"
              }}
              to catalog</span
            >
          </button>
        </div>
      </div>
    </section>

    <aside class="flex-1 min-w-0 relative">
      <LandscapeViewport
        :landscape-path="landscapePath"
        :reload-token="landscapeReloadToken"
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
