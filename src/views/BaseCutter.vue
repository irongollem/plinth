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
  GeneratedPieceKind,
  GeneratorPreset,
  LandscapeParams,
  MagnetSpec,
  NoiseLayer,
  Placement,
  PlinthParams,
  RipplesLayer,
  ScatterAsset,
  ScatterJob,
  ScatterParams,
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
import { useScatter } from "../composables/useScatter";
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
import {
  angularDelta,
  centroidOf,
  moveDelta,
  normalizeDeg,
  reindexSelection,
  rotateGroup,
} from "../utils/placementGroups";
import { popLast, pushBounded } from "../utils/placementUndo";

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { selectFiles } = useFileSelect();
const baseCut = useBaseCut();
const landscapeGen = useLandscapeGen();
const debrisScatter = useScatter();
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

// ---- Scatter state (docs/SCATTER.md "UI (BaseCutter view)"): decorates
// whatever landscape is active with generated debris via its own headless
// pass (scatter_landscape.py). Named `debrisScatter`/`debris*` throughout —
// NOT `scatter*` — because that name is already taken by the placement
// GENERATORS' random-cutter-scatter mode above (GENERATOR_MODES, scatterCount,
// runScatter): two unrelated features that both happen to be called
// "scatter" in their own domains.
const scatterAssets = ref<ScatterAsset[]>([]);

/** Undecorated-source bookkeeping (docs/SCATTER.md "re-scatter never
 * compounds"): the landscape path as it stood BEFORE its first scatter.
 * Scatter always runs FROM this path — never from the currently-displayed
 * decorated one — so repeated runs never scatter debris onto debris.
 * Updated only when the user picks/generates a genuinely NEW landscape
 * (chooseLandscape, landscapeGen.finished below); a scatter job's own
 * Finished handler deliberately leaves it alone. */
const scatterSourcePath = ref("");

/** True once a scatter has actually been applied to the active landscape —
 * gates the "Remove scatter" button and swaps the run button's label to
 * "Re-scatter". */
const hasScatterApplied = computed(
  () =>
    !!scatterSourcePath.value &&
    landscapePath.value !== scatterSourcePath.value,
);

type ScatterMixEntry = {
  kind: GeneratedPieceKind;
  enabled: boolean;
  weight: number;
};
/** Generated kinds only — get_scatter_assets() is called on mount but
 * currently returns [] until S4 curation lands (docs/SCATTER.md "Execution
 * phases"), so there is no asset row to seed here yet. */
const debrisPieces = reactive<ScatterMixEntry[]>([
  { kind: "pebble", enabled: true, weight: 1 },
  { kind: "rock", enabled: true, weight: 1 },
]);

type DebrisParams = {
  seed: number;
  density_per_dm2: number;
  scale_min: number;
  scale_max: number;
  scale_factor: number;
  sink_min: number;
  sink_max: number;
  align_to_surface: boolean;
  max_slope_deg: number;
  edge_margin_mm: number;
};
const debrisParams = reactive<DebrisParams>({
  seed: 1,
  density_per_dm2: 8,
  scale_min: 0.8,
  scale_max: 1.2,
  scale_factor: 1,
  sink_min: 0.5,
  sink_max: 2,
  align_to_surface: true,
  max_slope_deg: 45,
  edge_margin_mm: 3,
});

/** Preset mixes as chips (docs/SCATTER.md UI spec) — a local literal, the
 * cutter-library move: a new mix is a new row here, not a new pipeline.
 * Weights are relative (PieceChoice.weight), not fractions. */
type ScatterPreset = {
  id: string;
  label: string;
  density_per_dm2: number;
  weights: { pebble: number; rock: number };
};
const SCATTER_PRESETS: ScatterPreset[] = [
  {
    id: "light-pebbles",
    label: "Light pebbles",
    density_per_dm2: 4,
    weights: { pebble: 4, rock: 1 },
  },
  {
    id: "rocky-debris",
    label: "Rocky debris",
    density_per_dm2: 9,
    weights: { pebble: 1, rock: 3 },
  },
  {
    id: "dense-rubble",
    label: "Dense rubble",
    density_per_dm2: 18,
    weights: { pebble: 1, rock: 1 },
  },
];
const selectedScatterPresetId = ref<string | null>(null);

const selectScatterPreset = (preset: ScatterPreset) => {
  debrisParams.density_per_dm2 = preset.density_per_dm2;
  for (const piece of debrisPieces) {
    piece.enabled = true;
    piece.weight = preset.weights[piece.kind];
  }
  selectedScatterPresetId.value = preset.id;
};

/** Reroll to a fresh random seed — same convention as the landscape
 * generator's own 🎲 (rerollSeed above). */
const rerollDebrisSeed = () => {
  debrisParams.seed = Math.floor(Math.random() * 0xffffffff);
};

/** Why the scatter section is disabled — the palette's own
 * disabled-with-tooltip convention, never click-then-toast. */
const debrisScatterBlockedReason = computed(() => {
  if (locked.value || debrisScatter.isRunning.value) {
    return "Locked while a job is running";
  }
  if (!landscapeBounds.value) return "Load or generate a landscape first";
  if (!debrisPieces.some((p) => p.enabled && p.weight > 0)) {
    return "Enable at least one piece";
  }
  return "";
});
const canRunDebrisScatter = computed(() => !debrisScatterBlockedReason.value);

const removeScatterBlockedReason = computed(() => {
  if (locked.value || debrisScatter.isRunning.value) {
    return "Locked while a job is running";
  }
  if (!hasScatterApplied.value) return "No scatter to remove";
  return "";
});
const canRemoveScatter = computed(() => !removeScatterBlockedReason.value);

/** Derives a stable, OVERWRITTEN output path from the undecorated source —
 * source stem + "-scattered.stl" beside it. Stable (not unique-suffixed) on
 * purpose: a re-scatter is meant to replace the previous decorated STL, the
 * same "regenerated bake overwrites its own file" policy the landscape
 * generator's preset+seed filenames already rely on. */
const deriveScatterOutPath = (sourcePath: string): string => {
  const splitAt = Math.max(
    sourcePath.lastIndexOf("/"),
    sourcePath.lastIndexOf("\\"),
  );
  const dir = splitAt >= 0 ? sourcePath.slice(0, splitAt + 1) : "";
  const base = splitAt >= 0 ? sourcePath.slice(splitAt + 1) : sourcePath;
  const stem = base.replace(/\.stl$/i, "");
  return `${dir}${stem}-scattered.stl`;
};

const buildScatterParams = (): ScatterParams => ({
  seed: debrisParams.seed,
  density_per_dm2: debrisParams.density_per_dm2,
  scale: [debrisParams.scale_min, debrisParams.scale_max],
  scale_factor: debrisParams.scale_factor,
  sink_mm: [debrisParams.sink_min, debrisParams.sink_max],
  align_to_surface: debrisParams.align_to_surface,
  max_slope_deg: debrisParams.max_slope_deg,
  edge_margin_mm: debrisParams.edge_margin_mm,
  pieces: debrisPieces
    .filter((p) => p.enabled && p.weight > 0)
    .map((p) => ({ piece: { Generated: { kind: p.kind } }, weight: p.weight })),
});

/** Runs (or re-runs) scatter FROM the tracked source, never from the
 * currently-displayed decorated path — see scatterSourcePath's comment. */
const startDebrisScatter = async () => {
  if (!canRunDebrisScatter.value) return;
  const source = scatterSourcePath.value || landscapePath.value;
  const job: ScatterJob = {
    landscape_path: source,
    out_path: deriveScatterOutPath(source),
    params: buildScatterParams(),
  };
  const result = await debrisScatter.start(job);
  if (result.status === "error") {
    toastStore.reportError("Failed to start scatter", result.error);
  }
};

const cancelDebrisScatter = () => debrisScatter.cancel();

/** Restores the undecorated source into the viewport — the mirror of a
 * scatter's own Finished handler below, both routed through
 * setLandscapePath so placements/undo get the same terrain-swap treatment. */
const removeScatter = () => {
  if (!canRemoveScatter.value) return;
  setLandscapePath(scatterSourcePath.value);
};

const debrisScatterPercent = computed(() => {
  const p = debrisScatter.progress.value;
  if (!p || !p.total) return 0;
  return Math.min(100, Math.round((p.placed * 100) / p.total));
});
const debrisScatterStepLabel = computed(() => {
  const p = debrisScatter.progress.value;
  return p ? `Placing ${p.placed}/${p.total}…` : "Starting…";
});

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
  const [
    library,
    plinthDefaults,
    presets,
    settingsResult,
    rootsResult,
    assets,
  ] = await Promise.all([
    commands.getCutterLibrary(),
    commands.getPlinthDefaults(),
    commands.getLandscapePresets(),
    commands.getSettings(),
    commands.listCatalogRoots(),
    // S4 (docs/SCATTER.md "Execution phases") returns [] until curation
    // lands — called now anyway so the piece mix editor lights up
    // automatically the day it isn't, with no UI change needed.
    commands.getScatterAssets(),
  ]);
  cutterLibrary.value = library;
  scatterAssets.value = assets;
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

// ---- groups (docs/BASECUTTER.md phase 6 follow-up: "regiment cutters
// rotate/move as a group") ----
// VIEW STATE ONLY: a group is never sent to the backend and never touches
// the Placement wire type (BaseCutJob's placements are plain Placement[],
// unchanged) — it exists purely so this view can move/rotate a regiment as
// one formation. Membership is keyed by placement NAME rather than array
// index, because indices shift on every delete/splice while names don't:
// nextNames() above mints a name that's collision-free AND never reused
// while a placement carrying it still lives (it takes 1 + the highest
// existing numeric suffix, specifically so a still-live placement's name
// can never be handed to a new one — see its own comment), so a name is a
// stable, durable handle for the placement's whole lifetime.
type PlacementGroup = { id: string; label: string; names: string[] };
const groups = ref<PlacementGroup[]>([]);
let groupSeq = 0;

/** The group (if any) holding placement `name` — the one place group
 * membership is queried, so the viewport-move mapping, the rotate buttons,
 * the list UI, and co-selection all agree on what's grouped. */
const groupOfName = (name: string | null): PlacementGroup | undefined =>
  name ? groups.value.find((g) => g.names.includes(name)) : undefined;
const groupOfIndex = (index: number): PlacementGroup | undefined =>
  groupOfName(placements.value[index]?.name ?? null);

/** Live placement indices for a group's member names, in `placements`'
 * current order. Recomputed on demand rather than cached: deletes shift
 * indices, and a stale index cache would silently go wrong. */
const memberIndices = (group: PlacementGroup): number[] =>
  group.names
    .map((n) => placements.value.findIndex((p) => p.name === n))
    .filter((i) => i !== -1);

/** Drop `name` from whichever group holds it (a no-op if none does), and
 * dissolve that group once it falls to a single remaining member — a
 * "group" of one is just a placement, per the list UI's own rule. */
const removeMemberFromGroups = (name: string) => {
  groups.value = groups.value
    .map((g) =>
      g.names.includes(name)
        ? { ...g, names: g.names.filter((n) => n !== name) }
        : g,
    )
    .filter((g) => g.names.length > 1);
};

/** Pivot a group's whole formation by `deltaDeg` around ITS OWN centroid:
 * every member's position orbits the centroid (rotateGroup's bearing math)
 * and every member's own rotation_deg advances by the same delta — a real
 * unit block pivots as one rigid body, so position and facing move
 * together or the ranks slide out of alignment relative to each other.
 * Shared by the viewport's rotation handle/[ / ] keys (via
 * onUpdatePlacement), the row's per-member ↺/↻ (via rotatePlacement), and
 * the group header's own ↺/↻ (rotateGroupBy) — one implementation, so all
 * three read the delta identically. */
const applyGroupRotation = (group: PlacementGroup, deltaDeg: number) => {
  if (!deltaDeg) return;
  const indices = memberIndices(group);
  const members = indices.map((i) => placements.value[i]);
  const { x, y } = centroidOf(members);
  const rotated = rotateGroup(members, x, y, deltaDeg);
  indices.forEach((i, k) => {
    const p = placements.value[i];
    p.x_mm = rotated[k].x_mm;
    p.y_mm = rotated[k].y_mm;
    p.rotation_deg = rotated[k].rotation_deg;
  });
};

// ---- undo (bounded history of placement/group edits) ----
type PlacementSnapshot = {
  placements: Placement[];
  groups: PlacementGroup[];
  selectedIndex: number | null;
};
const MAX_UNDO_STEPS = 10;
const undoStack = ref<PlacementSnapshot[]>([]);

/** Snapshot BEFORE a discrete mutation (or once per drag/rotate GESTURE,
 * at its start — see `gestureInFlight` below) so undo can restore it.
 * cloneRaw's on the individual refs' `.value`, NOT on a fresh wrapper
 * object built from them: `placements.value`/`groups.value` are
 * themselves reactive Proxies (same trap selectPreset's own comment
 * documents for presets), and cloneRaw's toRaw() unwrap only works on a
 * value that IS a proxy — wrapping them in `{ placements: ..., groups:
 * ... }` first would hand structuredClone a plain object with proxies
 * still nested inside it, which throws DataCloneError exactly like a bare
 * structuredClone(preset) did before selectPreset was fixed. */
const pushUndoSnapshot = () => {
  const snapshot: PlacementSnapshot = {
    placements: cloneRaw(placements.value),
    groups: cloneRaw(groups.value),
    selectedIndex: selectedIndex.value,
  };
  undoStack.value = pushBounded(undoStack.value, snapshot, MAX_UNDO_STEPS);
};

const canUndo = computed(() => !locked.value && undoStack.value.length > 0);
/** Disabled-with-tooltip convention (see the palette/generator buttons) —
 * why Undo is greyed out, never a click-then-toast. */
const undoBlockedReason = computed(() => {
  if (locked.value) return "Locked while a job is running";
  if (!undoStack.value.length) return "Nothing to undo";
  return "";
});

/** Restores the whole snapshot — placements, group membership, and
 * selection together — never partially. Undo-only: no redo stack exists
 * (a forward stack could be added later without reshaping this one). */
const undo = () => {
  if (!canUndo.value) return;
  const { item, rest } = popLast(undoStack.value);
  if (!item) return;
  undoStack.value = rest;
  placements.value = item.placements;
  groups.value = item.groups;
  selectedIndex.value = item.selectedIndex;
};

/** True while a viewport drag/rotate GESTURE is in flight (between its
 * `gesture-start` and the next pointerup) — the viewport streams many
 * `update` patches per gesture (one per pointermove), and undo must
 * coalesce all of them into the ONE snapshot already pushed at
 * gesture-start, not push a fresh one per patch. Set/cleared by
 * onGestureStart/onGestureEnd below; onUpdatePlacement only pushes its own
 * snapshot when this is false (a standalone patch, e.g. a [ / ] keypress,
 * which has no gesture wrapping it). */
let gestureInFlight = false;
const onGestureStart = () => {
  gestureInFlight = true;
  pushUndoSnapshot();
};
const onGestureEnd = () => {
  gestureInFlight = false;
};

// Placement mutation is locked out while a job is running: the job already
// took a snapshot (jobPlacementNames below) and mid-job add/delete would
// desync indices between the live array and the in-flight cut list.
// Placement mutation is also locked while a landscape bake OR a scatter pass
// is in flight — either one may swap `landscapePath` out from under any
// in-progress edits (see setLandscapePath's clear-on-swap logic), and all
// three jobs share the one Blender process slot besides.
const locked = computed(
  () =>
    baseCut.isRunning.value ||
    landscapeGen.isRunning.value ||
    debrisScatter.isRunning.value,
);

const addPlacement = (cutter: Cutter) => {
  generatorCutterId.value = cutter.id;
  // The palette already disables itself while locked or landscape-less
  // (a greyed button beats a click-then-scold toast); the guard stays for
  // any future non-palette caller.
  if (locked.value || !landscapeBounds.value) return;
  pushUndoSnapshot();
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

/** Append a generated batch, minting all names in one placements scan.
 * Returns the minted names — placeRegiment groups them; runScatter ignores
 * the return (scatter placements always stay ungrouped). Callers push their
 * own undo snapshot BEFORE calling this (one user action = one snapshot,
 * regardless of how many placements the action produces). */
const pushGenerated = (
  cutterId: string,
  generated: GeneratedPlacement[],
): string[] => {
  if (!generated.length) return [];
  const names = nextNames(cutterId, generated.length);
  placements.value.push(...generated.map((g, i) => ({ ...g, name: names[i] })));
  selectedIndex.value = placements.value.length - 1;
  return names;
};

const placeRegiment = () => {
  if (
    !canPlaceRegiment.value ||
    !generatorCutter.value ||
    !landscapeBounds.value
  ) {
    return;
  }
  pushUndoSnapshot();
  const cutter = generatorCutter.value;
  const b = landscapeBounds.value;
  const generated = regimentPlacements(
    cutter,
    Math.min(regimentRows.value, MAX_REGIMENT_DIM),
    Math.min(regimentCols.value, MAX_REGIMENT_DIM),
    regimentGapMm.value,
    { x: b.centerX, y: b.centerY },
  );
  const names = pushGenerated(cutter.id, generated);
  // A regiment is a GROUP only once it has more than one member — a single
  // cutter placed via a 1x1 "regiment" behaves like any hand-placed base.
  // Scatter never groups (see pushGenerated's doc comment): random loose
  // scatter isn't a formation the way a ranked grid is.
  if (names.length > 1) {
    groupSeq++;
    groups.value.push({
      id: `group-${groupSeq}`,
      label: `regiment ${groupSeq} — ${cutterLabel(cutter.kind)}`,
      names,
    });
  }
};

const runScatter = () => {
  if (!canScatter.value || !generatorCutter.value || !landscapeBounds.value)
    return;
  pushUndoSnapshot();
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

/** Rotate a single member, EXCEPT when it's grouped: rotating a grouped
 * member is a group rotate (the FORMATION pivots), consistent with the
 * viewport's own rotation handle — a member never spins in place while its
 * squadmates stay put. Drives the per-member row buttons. */
const rotatePlacement = (index: number, deltaDeg: number) => {
  if (locked.value) return;
  const p = placements.value[index];
  if (!p) return;
  pushUndoSnapshot();
  const group = groupOfIndex(index);
  if (group) {
    applyGroupRotation(group, deltaDeg);
    return;
  }
  p.rotation_deg = normalizeDeg(p.rotation_deg + deltaDeg);
};

const deletePlacement = (index: number) => {
  if (locked.value) return;
  pushUndoSnapshot();
  const removedName = placements.value[index]?.name ?? null;
  placements.value.splice(index, 1);
  selectedIndex.value = reindexSelection(selectedIndex.value, [index]);
  // A member deleted this way just drops out of its group (not the whole
  // group) — deleting the GROUP itself is deleteGroup, below.
  if (removedName) removeMemberFromGroups(removedName);
};

/** Release a group's members back to plain, ungrouped placements — the
 * placements themselves are untouched. */
const ungroupGroup = (group: PlacementGroup) => {
  if (locked.value) return;
  pushUndoSnapshot();
  groups.value = groups.value.filter((g) => g.id !== group.id);
};

/** Delete every member of a group in one action. Splices high-to-low so
 * earlier indices stay valid mid-loop, then reindexes the selection against
 * the WHOLE removed set at once (reindexSelection, extended from
 * deletePlacement's single-index compensation to cover simultaneous
 * multi-member removal). */
const deleteGroup = (group: PlacementGroup) => {
  if (locked.value) return;
  pushUndoSnapshot();
  // .sort(), not .toSorted(): the tsconfig lib target predates ES2023.
  // Safe here regardless — memberIndices() already returns a fresh array
  // (map+filter), so there's no shared reference to mutate out from under.
  const indices = memberIndices(group).sort((a, b) => b - a);
  for (const i of indices) placements.value.splice(i, 1);
  selectedIndex.value = reindexSelection(selectedIndex.value, indices);
  groups.value = groups.value.filter((g) => g.id !== group.id);
};

/** The group header's own ↺/↻ (±15°, same step as the per-member row
 * buttons) — identical math to a grouped member's row rotate, just entered
 * directly from the group rather than via one of its members. */
const rotateGroupBy = (group: PlacementGroup, deltaDeg: number) => {
  if (locked.value) return;
  pushUndoSnapshot();
  applyGroupRotation(group, deltaDeg);
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
  pushUndoSnapshot();
  const count = selectedPlacement.value.magnet?.count ?? 1;
  selectedPlacement.value.magnet = {
    diameter_mm: spec.diameter_mm,
    height_mm: spec.height_mm,
    count,
  };
};

const clearMagnet = () => {
  if (!selectedPlacement.value) return;
  pushUndoSnapshot();
  selectedPlacement.value.magnet = null;
};

const setMagnetCount = (count: number) => {
  if (!selectedPlacement.value?.magnet) return;
  pushUndoSnapshot();
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
  pushUndoSnapshot();
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
  if (!p) return;

  // A patch arriving OUTSIDE a viewport gesture (e.g. a [ / ] keypress) is
  // its own discrete undo step; a patch that's part of an ongoing drag/
  // rotate gesture was already snapshotted once at gesture-start, so
  // pushing here too would coalesce into more than one undo step per drag.
  if (!gestureInFlight) pushUndoSnapshot();

  const group = groupOfIndex(index);

  if (group && patch.rotation_deg !== undefined) {
    // Group ROTATE (rotation handle or [ / ] keys): the FORMATION pivots —
    // every member's position orbits the group centroid by the delta AND
    // every member's own rotation_deg advances by it. The delta is the
    // SHORTEST signed angle from this member's pre-patch rotation to the
    // patched one (angularDelta, not raw subtraction — see its own comment
    // for why a step across the 0/360 seam would otherwise invert
    // direction), computed fresh against the pre-patch value every event so
    // a continuous handle drag can't double-apply.
    applyGroupRotation(group, angularDelta(p.rotation_deg, patch.rotation_deg));
    return;
  }

  if (group && (patch.x_mm !== undefined || patch.y_mm !== undefined)) {
    // Group MOVE: the same dx/dy lands on every OTHER member. Computed
    // against THIS member's PRE-patch position each event (old vs patch),
    // never accumulated across the drag — see moveDelta's own comment for
    // why an accumulator or a stale "drag start" reference would
    // double-apply the motion as the drag continues.
    const { dx, dy } = moveDelta(p, {
      x_mm: patch.x_mm ?? p.x_mm,
      y_mm: patch.y_mm ?? p.y_mm,
    });
    for (const i of memberIndices(group)) {
      if (i === index) continue;
      const member = placements.value[i];
      member.x_mm += dx;
      member.y_mm += dy;
    }
  }

  Object.assign(p, patch);
};
const onDeletePlacement = (index: number) => deletePlacement(index);
const onLandscapeLoaded = (bounds: LandscapeBounds) => {
  landscapeBounds.value = bounds;
};
const onViewportError = (message: string) => {
  toastStore.addToast(message, "error", 0);
};

/** Other members of the selected placement's group, if any — softens their
 * viewport outline to read as co-selected (docs task: "the other group
 * members' outlines should read as co-selected"). Empty when nothing's
 * selected or the selection is ungrouped. */
const coSelectedIndices = computed<number[]>(() => {
  if (selectedIndex.value === null) return [];
  const group = groupOfIndex(selectedIndex.value);
  if (!group) return [];
  return memberIndices(group).filter((i) => i !== selectedIndex.value);
});

/** Placements list rendering: singles render one row each; a group's
 * members render together under one collapsible header row. Relies on a
 * group's members staying CONTIGUOUS in `placements` — true in practice
 * because every mutation that adds placements (addPlacement, pushGenerated)
 * only ever appends to the end of the array, so a group's members (always
 * created together) are never interleaved with placements created after
 * them; deleting a member only ever removes from the run, never reorders
 * it. */
type PlacementRow =
  | { kind: "single"; index: number; p: Placement }
  | {
      kind: "group";
      group: PlacementGroup;
      members: { index: number; p: Placement }[];
    };
const placementRows = computed<PlacementRow[]>(() => {
  const rows: PlacementRow[] = [];
  const list = placements.value;
  let i = 0;
  while (i < list.length) {
    const p = list[i];
    const group = groupOfName(p.name);
    if (!group) {
      rows.push({ kind: "single", index: i, p });
      i++;
      continue;
    }
    const members: { index: number; p: Placement }[] = [];
    while (i < list.length && groupOfName(list[i].name) === group) {
      members.push({ index: i, p: list[i] });
      i++;
    }
    rows.push({ kind: "group", group, members });
  }
  return rows;
});

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
  // Every undo snapshot references placements on the OLD terrain — a scatter
  // or bake swap invalidates all of them, same reasoning as clearing
  // `placements` above, so the stack is dropped rather than left dangling.
  undoStack.value = [];
};

const chooseLandscape = async () => {
  const files = await selectFiles({
    accept: ".stl",
    multiple: false,
    title: "Choose landscape STL",
  });
  if (!files?.length) return;
  setLandscapePath(files[0].path);
  // A hand-picked file is by definition undecorated — it becomes the new
  // scatter source (docs/SCATTER.md: "If the user picks/generates a NEW
  // landscape, scatterSourcePath resets to it").
  scatterSourcePath.value = files[0].path;
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
    !landscapeGen.isRunning.value && // generation and cutting share Blender — never both at once
    !debrisScatter.isRunning.value,
);

/** BASE TOPPER mode (docs/BASECUTTER.md "Pinned interfaces" topper_mm): no
 * plinth at all — the plug is flat-trimmed and exported as a glue-on
 * terrain slab for hard plastic bases. Lives in step 4 (it changes what
 * gets built), not tucked into ADVANCED — PLINTH, whose contents don't
 * apply in this mode (see that fold's disabled fieldset below). */
const topperMode = ref(false);
/** Thickness sent as `topper_mm` when topperMode is on — base_cut.py clamps
 * to 1..3mm; 1.5 mirrors the script's own default. */
const topperMm = ref(1.5);

/** "Cut N bases" / "Cut N toppers" — the cut button's own label must say
 * which flow topperMode will actually run, not just the count. */
const cutButtonLabel = computed(() => {
  const n = placements.value.length;
  const noun = topperMode.value ? "topper" : "base";
  return `Cut ${n} ${noun}${n === 1 ? "" : "s"}`;
});

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
    topper_mm: topperMode.value ? topperMm.value : null,
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
    !baseCut.isRunning.value && // generation, scatter, and cutting share Blender — never more than one at once
    !landscapeGen.isRunning.value &&
    !debrisScatter.isRunning.value,
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
  // A freshly baked terrain is by definition undecorated — same reasoning
  // as chooseLandscape's own scatterSourcePath reset above.
  scatterSourcePath.value = finished.out_path;
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

// On a finished scatter, auto-load the decorated STL into the viewport —
// same swap path the generator/file-picker use, so stale placements+undo
// get cleared too. `scatterSourcePath` is deliberately left untouched: it
// still points at the undecorated terrain, so "re-scatter" and "remove
// scatter" both keep working off it (docs/SCATTER.md "re-scatter never
// compounds").
watch(debrisScatter.finished, (finished) => {
  if (!finished) return;
  setLandscapePath(finished.out_path, { force: true });
  toastStore.addToast(
    `Scattered ${finished.placed} piece${finished.placed === 1 ? "" : "s"}${finished.manifold ? "" : " — non-manifold"}`,
    finished.manifold ? "success" : "warning",
  );
});
watch(debrisScatter.failedMessage, (message) => {
  if (!message) return;
  toastStore.addToast(`Scatter failed: ${message}`, "error", 0);
});
watch(debrisScatter.cancelled, (isCancelled) => {
  if (!isCancelled) return;
  toastStore.addToast("Scatter cancelled", "info");
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

// ---- four-step accordion (side panel UX: "which step am I on") ----
// Free navigation, never a locked wizard — activeStep only ever changes via
// an explicit header click (selectStep) or an auto-advance nudge on a
// milestone TRANSITION (false -> true, never re-fired on every render while
// the milestone stays true). Session-only: no localStorage, resets to 1 on
// reload.
const activeStep = ref<1 | 2 | 3 | 4>(1);
type StepNumber = 1 | 2 | 3 | 4;
/** High-water mark of steps the user has explicitly opened via a header
 * click — NOT bumped by autoAdvance. Once the user has manually looked at a
 * later step, autoAdvance never re-opens an earlier (or equal) one out from
 * under them again this session — "never auto-move backward" plus "respect
 * a manual choice that's already ahead of the milestone". */
let highestManualStep: StepNumber = 1;
const selectStep = (step: StepNumber) => {
  activeStep.value = step;
  if (step > highestManualStep) highestManualStep = step;
};
/** Forward-only, and only below the user's own high-water mark — see
 * highestManualStep's comment. Called from milestone-transition watchers
 * below, never from a steady-state check (so it fires once per transition,
 * not on every subsequent render while the milestone stays true). */
const autoAdvance = (target: StepNumber) => {
  if (target <= activeStep.value || target <= highestManualStep) return;
  activeStep.value = target;
};

const step1Done = computed(() => landscapeBounds.value != null);
/** Scatter is OPTIONAL — this ✓ is a "you did scatter" marker, never a gate:
 * no later step's chip or content keys off it (step 3/4 completion is judged
 * purely by their own milestones), so skipping scatter entirely never reads
 * as "blocked". */
const step2Done = computed(() => hasScatterApplied.value);
const step3Done = computed(() => placements.value.length > 0);
/** Latches true the first time a job finishes with >=1 ok result and never
 * resets — unlike baseCut.finishedSummary (which reverts to null the moment
 * the NEXT job starts, see useBaseCut's resetState), this is a "this
 * session" milestone, not a "this job" one. */
const hasCutMilestone = ref(false);
const step4Done = computed(() => hasCutMilestone.value);

/** "no landscape yet" / basename + dims (cheap: landscapeBounds is already
 * the loaded extent, no re-measurement needed). The "· scattered" note
 * moved to step 2's own summary when scatter became its own step. */
const step1Summary = computed(() => {
  if (!landscapeBounds.value) return "no landscape yet";
  const b = landscapeBounds.value;
  const base = landscapePath.value.split(/[/\\]/).pop() || "landscape";
  const w = Math.round(b.maxX - b.minX);
  const d = Math.round(b.maxY - b.minY);
  return `${base} (${w}×${d}mm)`;
});

/** The seed the last APPLIED scatter actually ran with — captured when the
 * job finishes, not read live from debrisParams.seed, so rerolling the seed
 * field after a scatter doesn't lie about what's on the terrain. (Capturing
 * at finish is safe: the seed input is disabled while a scatter runs.) */
const lastScatterSeed = ref<number | null>(null);
watch(debrisScatter.finished, (finished) => {
  if (finished) lastScatterSeed.value = debrisParams.seed;
});
const step2Summary = computed(() => {
  if (!hasScatterApplied.value) return "not scattered";
  return lastScatterSeed.value !== null
    ? `scattered · seed ${lastScatterSeed.value}`
    : "scattered";
});

/** "N placements" (+ "· M magnets" counting placements with a magnet set, +
 * "· K grouped" counting groups) — always numeric, no separate empty-state
 * copy (unlike step 1/4): "0 placements" already reads fine here. */
const step3Summary = computed(() => {
  const n = placements.value.length;
  const magnets = placements.value.filter((p) => p.magnet != null).length;
  let summary = `${n} placement${n === 1 ? "" : "s"}`;
  if (magnets > 0) summary += ` · ${magnets} magnet${magnets === 1 ? "" : "s"}`;
  if (groups.value.length > 0) summary += ` · ${groups.value.length} grouped`;
  return summary;
});

/** The last completed job's tally — kept separately from
 * baseCut.finishedSummary (which the toast watcher above clears back to
 * null the moment a new job starts) so the step-4 summary doesn't blank out
 * mid-second-run. */
const lastCutSummary = ref<{ ok_count: number; total: number } | null>(null);
const step4Summary = computed(() => {
  if (!lastCutSummary.value) return "not cut yet";
  return `${lastCutSummary.value.ok_count}/${lastCutSummary.value.total} cut ok`;
});

// Auto-advance on milestone transitions (false -> true) only — landscape
// loads -> open step 2 (SCATTER, the optional decoration pass), NOT layout:
// jumping straight to LAYOUT hid the scatter section behind a header most
// users never clicked back to find. Deliberately NO auto-advance out of
// step 2 (a finished scatter stays put — the user may want to re-scatter;
// only a manual click moves on) and none on the first placement (stays on
// 3: placing a base shouldn't jump the user straight to the cut screen).
watch(landscapeBounds, (loaded, wasLoaded) => {
  if (loaded && !wasLoaded) autoAdvance(2);
});

// A second, independent watch on the same source the toast watcher above
// already observes (baseCut.finishedSummary) — separate concerns (accordion
// state vs. toast copy), not a replacement for it.
watch(baseCut.finishedSummary, (summary) => {
  if (!summary) return;
  lastCutSummary.value = summary;
  if (summary.ok_count > 0) {
    hasCutMilestone.value = true;
    autoAdvance(4);
  }
});
</script>

<template>
  <main class="relative flex h-full min-w-0">
    <section
      class="w-82.5 shrink-0 border-r border-base-content/10 overflow-y-auto p-4 flex flex-col gap-3.5"
    >
      <div class="flex items-baseline justify-between">
        <span class="font-bold text-[17px]">Base Cutter</span>
      </div>

      <div
        class="rounded-box border overflow-hidden shrink-0"
        :class="activeStep === 1 ? 'border-primary' : 'border-base-content/10'"
      >
        <button
          type="button"
          class="w-full flex items-center gap-2 p-3 text-left"
          @click="selectStep(1)"
        >
          <span
            class="flex items-center justify-center w-5 h-5 rounded-full text-[10px] font-mono shrink-0"
            :class="
              activeStep === 1
                ? 'bg-primary text-primary-content'
                : step1Done
                  ? 'bg-success/20 text-success'
                  : 'bg-base-content/10 text-base-content/50'
            "
            >1</span
          >
          <span class="flex-1 min-w-0 flex flex-col">
            <span
              class="font-mono font-semibold text-[10px] tracking-widest"
              :class="
                activeStep === 1 ? 'text-primary' : 'text-base-content/40'
              "
              >TERRAIN</span
            >
            <span class="text-[11px] text-base-content/50 truncate">{{
              step1Summary
            }}</span>
          </span>
          <span
            v-if="step1Done"
            class="text-success text-[13px] shrink-0"
            title="Landscape loaded"
            >✓</span
          >
        </button>
        <div
          v-show="activeStep === 1"
          class="flex flex-col gap-3.5 px-3 pb-3.5"
        >
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
              <span class="text-[11px] text-base-content/50 shrink-0"
                >Seed</span
              >
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
                    <Switch
                      v-model="genParams.layers.flow.enabled"
                      label="Flow"
                    />
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
        </div>
      </div>

      <div
        class="rounded-box border overflow-hidden shrink-0"
        :class="activeStep === 2 ? 'border-primary' : 'border-base-content/10'"
      >
        <button
          type="button"
          class="w-full flex items-center gap-2 p-3 text-left"
          @click="selectStep(2)"
        >
          <span
            class="flex items-center justify-center w-5 h-5 rounded-full text-[10px] font-mono shrink-0"
            :class="
              activeStep === 2
                ? 'bg-primary text-primary-content'
                : step2Done
                  ? 'bg-success/20 text-success'
                  : 'bg-base-content/10 text-base-content/50'
            "
            >2</span
          >
          <span class="flex-1 min-w-0 flex flex-col">
            <span class="flex items-baseline gap-1.5">
              <span
                class="font-mono font-semibold text-[10px] tracking-widest"
                :class="
                  activeStep === 2 ? 'text-primary' : 'text-base-content/40'
                "
                >SCATTER</span
              >
              <span class="font-mono text-[9px] text-base-content/30"
                >optional</span
              >
            </span>
            <span class="text-[11px] text-base-content/50 truncate">{{
              step2Summary
            }}</span>
          </span>
          <span
            v-if="step2Done"
            class="text-success text-[13px] shrink-0"
            title="Scatter applied"
            >✓</span
          >
        </button>
        <div
          v-show="activeStep === 2"
          class="flex flex-col gap-1.5 px-3 pb-3.5"
        >
          <div class="flex flex-wrap gap-1">
            <button
              v-for="preset in SCATTER_PRESETS"
              :key="preset.id"
              type="button"
              class="btn btn-xs"
              :class="
                preset.id === selectedScatterPresetId ? 'btn-primary' : ''
              "
              :disabled="!!debrisScatterBlockedReason"
              :title="debrisScatterBlockedReason || undefined"
              @click="selectScatterPreset(preset)"
            >
              {{ preset.label }}
            </button>
          </div>

          <div class="flex items-center gap-1.5">
            <span class="text-[11px] text-base-content/50 shrink-0"
              >Density /dm²</span
            >
            <input
              type="number"
              class="input input-xs flex-1 font-mono"
              min="0"
              step="0.5"
              :disabled="debrisScatter.isRunning.value"
              v-model.number="debrisParams.density_per_dm2"
            />
            <!-- Front-row, not an advanced knob: like the terrain's own
                 feature_scale, this is the "what scale am I basing for"
                 dial — 1 = the 28-32mm heroic anchor every piece size is
                 canonical at (docs/SCATTER.md "Scale anchor"); 15mm gaming
                 wants ~0.5, 54mm display work ~2. -->
            <span
              class="text-[11px] text-base-content/50 shrink-0"
              title="Whole-pass piece rescale — 1 = 28-32mm heroic"
              >Scale ×</span
            >
            <input
              type="number"
              class="input input-xs w-16 font-mono"
              min="0.1"
              step="0.05"
              :disabled="debrisScatter.isRunning.value"
              v-model.number="debrisParams.scale_factor"
            />
          </div>

          <div class="flex items-center gap-1.5">
            <span class="text-[11px] text-base-content/50 shrink-0">Seed</span>
            <input
              type="number"
              class="input input-xs flex-1 font-mono"
              :disabled="debrisScatter.isRunning.value"
              v-model.number="debrisParams.seed"
            />
            <button
              type="button"
              class="btn btn-xs"
              title="Reroll seed"
              :disabled="debrisScatter.isRunning.value"
              @click="rerollDebrisSeed"
            >
              🎲
            </button>
          </div>

          <div class="flex items-center gap-3">
            <button
              class="btn btn-secondary btn-sm grow"
              :disabled="!canRunDebrisScatter"
              :title="debrisScatterBlockedReason || undefined"
              @click="startDebrisScatter"
            >
              <template v-if="debrisScatter.isRunning.value">
                <span class="loading loading-spinner loading-xs"></span>
                <span>Scattering…</span>
              </template>
              <span v-else>{{
                hasScatterApplied ? "Re-scatter" : "Scatter"
              }}</span>
            </button>
            <button
              v-if="debrisScatter.isRunning.value"
              class="btn btn-error btn-sm"
              @click="cancelDebrisScatter"
            >
              Cancel
            </button>
          </div>
          <div
            v-if="debrisScatter.isRunning.value"
            class="flex items-center gap-3"
          >
            <ProgressBar :progress="debrisScatterPercent" />
            <span class="text-sm opacity-70">{{ debrisScatterStepLabel }}</span>
          </div>
          <button
            type="button"
            class="btn btn-ghost btn-xs self-start"
            :disabled="!canRemoveScatter"
            :title="removeScatterBlockedReason || undefined"
            @click="removeScatter"
          >
            Remove scatter
          </button>
          <div
            v-if="debrisScatter.failedMessage.value"
            class="alert alert-error text-xs whitespace-pre-wrap flex-col items-start"
          >
            <span>{{ debrisScatter.failedMessage.value }}</span>
            <pre
              v-if="debrisScatter.failedStdoutTail.value"
              class="font-mono text-[10px] opacity-70 whitespace-pre-wrap mt-1"
              >{{ debrisScatter.failedStdoutTail.value }}</pre
            >
          </div>

          <details
            class="collapse collapse-arrow border border-base-content/10 bg-base-200/20 rounded-box"
          >
            <summary
              class="collapse-title min-h-0 py-2.5 px-3 flex items-center gap-2 cursor-pointer"
            >
              <span
                class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
                >ADVANCED — SCATTER</span
              >
            </summary>
            <div class="collapse-content flex flex-col gap-2.5 px-3">
              <NumberInput
                id="scatter-scale-min"
                label="Scale min ×"
                :step="0.05"
                :min="0.1"
                v-model="debrisParams.scale_min"
              />
              <NumberInput
                id="scatter-scale-max"
                label="Scale max ×"
                :step="0.05"
                :min="0.1"
                v-model="debrisParams.scale_max"
              />
              <NumberInput
                id="scatter-sink-min"
                label="Sink min (mm)"
                :step="0.1"
                :min="0"
                v-model="debrisParams.sink_min"
              />
              <NumberInput
                id="scatter-sink-max"
                label="Sink max (mm)"
                :step="0.1"
                :min="0"
                v-model="debrisParams.sink_max"
              />
              <Switch
                v-model="debrisParams.align_to_surface"
                label="Align to surface"
              />
              <NumberInput
                id="scatter-max-slope"
                label="Max slope (deg)"
                :step="1"
                :min="0"
                :max="90"
                v-model="debrisParams.max_slope_deg"
              />
              <NumberInput
                id="scatter-edge-margin"
                label="Edge margin (mm)"
                :step="0.5"
                :min="0"
                v-model="debrisParams.edge_margin_mm"
              />

              <div
                class="flex flex-col gap-1.5 border-t border-base-content/10 pt-2"
              >
                <span
                  class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
                  >PIECE MIX — GENERATED</span
                >
                <div
                  v-for="piece in debrisPieces"
                  :key="piece.kind"
                  class="flex items-center gap-1.5"
                >
                  <input
                    type="checkbox"
                    class="checkbox checkbox-xs"
                    v-model="piece.enabled"
                  />
                  <span class="text-[11px] flex-1 capitalize">{{
                    piece.kind
                  }}</span>
                  <input
                    type="number"
                    class="input input-xs w-16 font-mono"
                    min="0"
                    step="0.1"
                    :disabled="!piece.enabled"
                    v-model.number="piece.weight"
                  />
                </div>
                <!-- get_scatter_assets() returns [] until S4 curation lands
                     (docs/SCATTER.md "Execution phases") — no assets group
                     renders until there's actually something in it, rather
                     than an always-empty list confusing the picker. -->
              </div>
            </div>
          </details>
        </div>
      </div>

      <div
        class="rounded-box border overflow-hidden shrink-0"
        :class="activeStep === 3 ? 'border-primary' : 'border-base-content/10'"
      >
        <button
          type="button"
          class="w-full flex items-center gap-2 p-3 text-left"
          @click="selectStep(3)"
        >
          <span
            class="flex items-center justify-center w-5 h-5 rounded-full text-[10px] font-mono shrink-0"
            :class="
              activeStep === 3
                ? 'bg-primary text-primary-content'
                : step3Done
                  ? 'bg-success/20 text-success'
                  : 'bg-base-content/10 text-base-content/50'
            "
            >3</span
          >
          <span class="flex-1 min-w-0 flex flex-col">
            <span
              class="font-mono font-semibold text-[10px] tracking-widest"
              :class="
                activeStep === 3 ? 'text-primary' : 'text-base-content/40'
              "
              >LAYOUT</span
            >
            <span class="text-[11px] text-base-content/50 truncate">{{
              step3Summary
            }}</span>
          </span>
          <span
            v-if="step3Done"
            class="text-success text-[13px] shrink-0"
            title="Placements added"
            >✓</span
          >
        </button>
        <div
          v-show="activeStep === 3"
          class="flex flex-col gap-3.5 px-3 pb-3.5"
        >
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
              :title="
                landscapeBounds ? '' : 'Load or generate a landscape first'
              "
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
            <p
              v-if="!landscapeBounds"
              class="text-[10.5px] text-base-content/40"
            >
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
                <option
                  v-for="c in cutterGroups.rounds"
                  :key="c.id"
                  :value="c.id"
                >
                  {{ c.label }}
                </option>
              </optgroup>
              <optgroup label="Ovals">
                <option
                  v-for="c in cutterGroups.ovals"
                  :key="c.id"
                  :value="c.id"
                >
                  {{ c.label }}
                </option>
              </optgroup>
              <optgroup label="Squares & rects">
                <option
                  v-for="c in cutterGroups.rects"
                  :key="c.id"
                  :value="c.id"
                >
                  {{ c.label }}
                </option>
              </optgroup>
            </select>

            <div
              v-if="generatorMode === 'regiment'"
              class="flex flex-col gap-1.5"
            >
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
                <span class="text-[10.5px] text-base-content/40 shrink-0"
                  >mm</span
                >
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
            <div class="flex items-center justify-between gap-2">
              <span
                class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
                >PLACEMENTS ({{ placements.length }})</span
              >
              <button
                type="button"
                class="btn btn-ghost btn-xs gap-1"
                :title="undoBlockedReason || 'Undo (Ctrl/Cmd+Z)'"
                :disabled="!canUndo"
                @click="undo"
              >
                ↶ Undo
              </button>
            </div>
            <ul
              v-if="placements.length"
              class="flex flex-col gap-1 max-h-48 overflow-y-auto"
            >
              <template
                v-for="row in placementRows"
                :key="row.kind === 'group' ? row.group.id : `p-${row.index}`"
              >
                <li
                  v-if="row.kind === 'single'"
                  class="flex items-center gap-1.5 px-2 py-1.5 rounded border cursor-pointer text-[12px]"
                  :class="
                    row.index === selectedIndex
                      ? 'bg-primary/10 border-primary'
                      : 'border-base-content/10 hover:border-base-content/30'
                  "
                  @click="selectedIndex = row.index"
                >
                  <span class="flex-1 truncate font-medium">{{
                    row.p.name
                  }}</span>
                  <span class="text-base-content/50 font-mono text-[10px]">{{
                    cutterLabel(row.p.cutter)
                  }}</span>
                  <span
                    class="font-mono text-[10px] text-base-content/40 w-9 text-right"
                    >{{ Math.round(row.p.rotation_deg) }}°</span
                  >
                  <span
                    v-if="row.p.magnet"
                    class="badge badge-xs badge-info"
                    title="Magnet pocket"
                    >{{
                      row.p.magnet.count > 1 ? `M×${row.p.magnet.count}` : "M"
                    }}</span
                  >
                  <button
                    type="button"
                    class="btn btn-ghost btn-xs px-1"
                    title="Rotate -15°"
                    :disabled="locked"
                    @click.stop="rotatePlacement(row.index, -15)"
                  >
                    ↺
                  </button>
                  <button
                    type="button"
                    class="btn btn-ghost btn-xs px-1"
                    title="Rotate +15°"
                    :disabled="locked"
                    @click.stop="rotatePlacement(row.index, 15)"
                  >
                    ↻
                  </button>
                  <button
                    type="button"
                    class="btn btn-ghost btn-xs px-1 text-error"
                    title="Delete placement"
                    :disabled="locked"
                    @click.stop="deletePlacement(row.index)"
                  >
                    ✕
                  </button>
                </li>

                <li
                  v-else
                  class="flex flex-col gap-1 rounded border border-base-content/10 px-2 py-1.5 text-[12px]"
                >
                  <div class="flex items-center gap-1.5">
                    <span class="flex-1 truncate font-semibold">{{
                      row.group.label
                    }}</span>
                    <span class="text-base-content/40 font-mono text-[10px]">{{
                      row.members.length
                    }}</span>
                    <button
                      type="button"
                      class="btn btn-ghost btn-xs px-1"
                      title="Rotate group -15°"
                      :disabled="locked"
                      @click.stop="rotateGroupBy(row.group, -15)"
                    >
                      ↺
                    </button>
                    <button
                      type="button"
                      class="btn btn-ghost btn-xs px-1"
                      title="Rotate group +15°"
                      :disabled="locked"
                      @click.stop="rotateGroupBy(row.group, 15)"
                    >
                      ↻
                    </button>
                    <button
                      type="button"
                      class="btn btn-ghost btn-xs px-1.5"
                      title="Ungroup — release members to single bases"
                      :disabled="locked"
                      @click.stop="ungroupGroup(row.group)"
                    >
                      ungroup
                    </button>
                    <button
                      type="button"
                      class="btn btn-ghost btn-xs px-1 text-error"
                      title="Delete group"
                      :disabled="locked"
                      @click.stop="deleteGroup(row.group)"
                    >
                      ✕
                    </button>
                  </div>
                  <ul
                    class="flex flex-col gap-1 pl-2 border-l border-base-content/10"
                  >
                    <li
                      v-for="m in row.members"
                      :key="m.index"
                      class="flex items-center gap-1.5 py-1 rounded cursor-pointer"
                      :class="
                        m.index === selectedIndex
                          ? 'bg-primary/10'
                          : 'hover:bg-base-content/5'
                      "
                      @click="selectedIndex = m.index"
                    >
                      <span class="flex-1 truncate font-medium">{{
                        m.p.name
                      }}</span>
                      <span
                        class="text-base-content/50 font-mono text-[10px]"
                        >{{ cutterLabel(m.p.cutter) }}</span
                      >
                      <span
                        class="font-mono text-[10px] text-base-content/40 w-9 text-right"
                        >{{ Math.round(m.p.rotation_deg) }}°</span
                      >
                      <span
                        v-if="m.p.magnet"
                        class="badge badge-xs badge-info"
                        title="Magnet pocket"
                        >{{
                          m.p.magnet.count > 1 ? `M×${m.p.magnet.count}` : "M"
                        }}</span
                      >
                      <button
                        type="button"
                        class="btn btn-ghost btn-xs px-1"
                        title="Rotate group -15° (this base is grouped)"
                        :disabled="locked"
                        @click.stop="rotatePlacement(m.index, -15)"
                      >
                        ↺
                      </button>
                      <button
                        type="button"
                        class="btn btn-ghost btn-xs px-1"
                        title="Rotate group +15° (this base is grouped)"
                        :disabled="locked"
                        @click.stop="rotatePlacement(m.index, 15)"
                      >
                        ↻
                      </button>
                      <button
                        type="button"
                        class="btn btn-ghost btn-xs px-1 text-error"
                        title="Remove from group"
                        :disabled="locked"
                        @click.stop="deletePlacement(m.index)"
                      >
                        ✕
                      </button>
                    </li>
                  </ul>
                </li>
              </template>
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
              <p v-if="topperMode" class="text-[10.5px] text-base-content/40">
                topper mode ignores magnets
              </p>
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
        </div>
      </div>

      <div
        class="rounded-box border overflow-hidden shrink-0"
        :class="activeStep === 4 ? 'border-primary' : 'border-base-content/10'"
      >
        <button
          type="button"
          class="w-full flex items-center gap-2 p-3 text-left"
          @click="selectStep(4)"
        >
          <span
            class="flex items-center justify-center w-5 h-5 rounded-full text-[10px] font-mono shrink-0"
            :class="
              activeStep === 4
                ? 'bg-primary text-primary-content'
                : step4Done
                  ? 'bg-success/20 text-success'
                  : 'bg-base-content/10 text-base-content/50'
            "
            >4</span
          >
          <span class="flex-1 min-w-0 flex flex-col">
            <span
              class="font-mono font-semibold text-[10px] tracking-widest"
              :class="
                activeStep === 4 ? 'text-primary' : 'text-base-content/40'
              "
              >CUT &amp; EXPORT</span
            >
            <span class="text-[11px] text-base-content/50 truncate">{{
              step4Summary
            }}</span>
          </span>
          <span
            v-if="step4Done"
            class="text-success text-[13px] shrink-0"
            title="Cut finished"
            >✓</span
          >
        </button>
        <div
          v-show="activeStep === 4"
          class="flex flex-col gap-3.5 px-3 pb-3.5"
        >
          <div class="flex flex-col gap-1.5">
            <Switch v-model="topperMode" label="Base topper only" />
            <p class="text-[10.5px] text-base-content/40 -mt-1.5 px-2">
              no plinth — a glue-on terrain slab for hard plastic bases
            </p>
            <NumberInput
              v-if="topperMode"
              id="topper-thickness"
              label="Thickness (mm)"
              :min="1"
              :max="3"
              :step="0.1"
              v-model="topperMm"
            />
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
            <fieldset
              class="collapse-content flex flex-col gap-2 px-3"
              :disabled="topperMode"
              :title="
                topperMode
                  ? 'plinth options don\'t apply to toppers'
                  : undefined
              "
            >
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
            </fieldset>
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
              <span v-else>{{ cutButtonLabel }}</span>
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
                class="flex items-center gap-2 flex-wrap"
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
                <template v-else>
                  <span v-if="!r.manifold" class="text-warning text-[11px]"
                    >non-manifold</span
                  >
                  <span
                    v-if="r.fused === false"
                    class="badge badge-xs badge-warning"
                    :title="`the plug didn't join the plinth — the STL holds ${r.shells} loose shell${r.shells === 1 ? '' : 's'}`"
                    >not fused ({{ r.shells }} shells)</span
                  >
                  <span
                    v-if="r.magnet_ignored"
                    class="text-[10px] text-base-content/40"
                    >magnet ignored</span
                  >
                  <span
                    v-if="r.topper_mm_clamped != null"
                    class="text-[10px] text-base-content/40"
                    >clamped to {{ r.topper_mm_clamped }} mm</span
                  >
                </template>
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
                    {{ rootLabel(root.root)
                    }}{{ root.primary ? " (primary)" : "" }}
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
        :co-selected="coSelectedIndices"
        :locked="locked"
        @select="onSelect"
        @update="onUpdatePlacement"
        @delete="onDeletePlacement"
        @loaded="onLandscapeLoaded"
        @error="onViewportError"
        @gesture-start="onGestureStart"
        @gesture-end="onGestureEnd"
        @undo="undo"
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
