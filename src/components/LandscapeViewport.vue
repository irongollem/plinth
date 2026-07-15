<script setup lang="ts">
/**
 * Top-down placement viewport for Base Cutter. A map view, not a 3D
 * preview: orthographic camera looking straight down -Z (+Y up on screen),
 * no tumbling. The landscape STL is decoded through the same worker
 * StlViewport uses (stlGeometry.worker.ts, via the shared useStlDecode
 * composable) so a multi-million-triangle sculpt never freezes the UI, but
 * — unlike StlViewport — the mesh is kept in its native mm coordinates:
 * placements' x_mm/y_mm are the landscape's own STL coordinates, the same
 * frame base_cut.py will cut in.
 *
 * Placements are overlay outlines only (line loops just above the
 * landscape's max Z): a NOMINAL footprint (where the base stands) and a
 * smaller derived CUT footprint (nominal shrunk by the plinth taper inset,
 * via utils/cutFootprint — the same module a Rust top_face_of twin test
 * pins), per docs/BASECUTTER.md "The plinth". This component owns no
 * placement state — it raycasts drags into x/y and emits `update`, the view
 * owns the array.
 */
import { readFile } from "@tauri-apps/plugin-fs";
import * as THREE from "three";
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { CutterKind, Placement, PlinthParams } from "../bindings";
import {
  SUPERSEDED,
  toTransferableBuffer,
  useStlDecode,
} from "../composables/useStlDecode";
import { insetShrink, shrinkKind } from "../utils/cutFootprint";
import { footprintDims } from "../utils/cutterKinds";

const props = defineProps<{
  landscapePath: string;
  /** Bump to force a reload at an unchanged path — a regenerated bake
   * overwrites its own file (preset + seed = stable filename), so the
   * path alone can't signal "new terrain". */
  reloadToken?: number;
  placements: Placement[];
  plinth: PlinthParams;
  selectedIndex: number | null;
  /** Suppresses drag/rotate/delete while a cut job is running — the
   * submitted job already snapshotted names/positions, mid-job edits would
   * just desync the live view from what's actually being cut. */
  locked: boolean;
}>();

const emit = defineEmits<{
  select: [index: number | null];
  update: [index: number, patch: Partial<Placement>];
  delete: [index: number];
  /** Landscape (re)loaded — its XY bounds, for "add at landscape center". */
  loaded: [
    bounds: {
      centerX: number;
      centerY: number;
      minX: number;
      maxX: number;
      minY: number;
      maxY: number;
    },
  ];
  error: [message: string];
}>();

const container = ref<HTMLDivElement | null>(null);
const isLoading = ref(false);

const CIRCLE_SEGMENTS = 96; // matches base_cut.py's CIRCLE_SEGMENTS
const ROTATE_STEP_DEG = 5;
const ROTATE_STEP_FAST_DEG = 15;

let renderer: THREE.WebGLRenderer | null = null;
let scene: THREE.Scene | null = null;
let camera: THREE.OrthographicCamera | null = null;
let landscapeGroup: THREE.Group | null = null;
let landscapeMesh: THREE.Mesh | null = null;
let landscapeMaterial: THREE.MeshStandardMaterial | null = null;
let overlayGroup: THREE.Group | null = null;
/** Overlay groups, one per placement, index-aligned with `props.placements`
 * (see `syncOverlays`) — kept around across rebuilds so a drag only ever
 * touches position/rotation instead of tearing down and re-creating
 * geometry every frame. */
let overlayGroups: THREE.Group[] = [];
let resizeObserver: ResizeObserver | null = null;
let loadToken = 0;

/* ---- worker-side STL decoding (shared with StlViewport via useStlDecode) ---- */
const stlDecode = useStlDecode();

// ---- camera framing (ortho top-down: left/right/top/bottom + zoom) ----
let baseHalfWidth = 100;
let baseHalfHeight = 100;
let camX = 0;
let camY = 0;
let camZoom = 1;
let landscapeMaxZ = 0;

const setupScene = (el: HTMLDivElement) => {
  renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(el.clientWidth, el.clientHeight);
  el.appendChild(renderer.domElement);

  scene = new THREE.Scene();
  scene.background = new THREE.Color(0x1a1a1a);

  camera = new THREE.OrthographicCamera(-100, 100, 100, -100, 0.01, 1e6);
  camera.up.set(0, 1, 0);

  const key = new THREE.DirectionalLight(0xffffff, 2.2);
  key.position.set(0.4, -0.3, 1);
  scene.add(key);
  const fill = new THREE.DirectionalLight(0xbfd4ff, 0.5);
  fill.position.set(-0.5, 0.4, 0.6);
  scene.add(fill);
  scene.add(new THREE.AmbientLight(0xffffff, 0.35));

  landscapeGroup = new THREE.Group();
  scene.add(landscapeGroup);

  overlayGroup = new THREE.Group();
  scene.add(overlayGroup);

  landscapeMaterial = new THREE.MeshStandardMaterial({
    color: 0x8a8f86,
    roughness: 0.85,
    metalness: 0,
  });

  applyCamera();
};

let renderQueued = false;
const requestRender = () => {
  if (renderQueued || !renderer) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (renderer && scene && camera) renderer.render(scene, camera);
  });
};

const applyCamera = () => {
  if (!camera || !container.value) return;
  const { clientWidth, clientHeight } = container.value;
  const aspect = clientWidth && clientHeight ? clientWidth / clientHeight : 1;
  // Fit the landscape bbox (with margin already baked into base half-extents)
  // inside the viewport regardless of aspect ratio.
  let halfW = baseHalfWidth;
  let halfH = baseHalfHeight;
  const boxAspect = halfW / halfH || 1;
  if (boxAspect > aspect) {
    halfH = halfW / aspect;
  } else {
    halfW = halfH * aspect;
  }
  camera.left = -halfW;
  camera.right = halfW;
  camera.top = halfH;
  camera.bottom = -halfH;
  camera.zoom = camZoom;
  // The peek pose: pitch (tilt) around the view center's screen-horizontal
  // axis + spin around the vertical Z axis through the same center — a
  // transient orbit; cuts are always vertical, so the resting pose stays a
  // true top-down map. At tilt=spin=0 this reduces exactly to the original
  // straight-down pose. Interactions stay correct while peeking because
  // pointerWorld raycasts against the maxZ plane rather than assuming a
  // vertical view ray.
  const dist = Math.max(halfW, halfH, 10) * 4 + 100;
  const radT = (tiltDeg * Math.PI) / 180;
  const zAxis = new THREE.Vector3(0, 0, 1);
  const radS = (spinDeg * Math.PI) / 180;
  const offset = new THREE.Vector3(
    0,
    -Math.sin(radT) * dist,
    Math.cos(radT) * dist,
  ).applyAxisAngle(zAxis, radS);
  const up = new THREE.Vector3(
    0,
    Math.cos(radT),
    Math.sin(radT),
  ).applyAxisAngle(zAxis, radS);
  camera.position.set(
    camX + offset.x,
    camY + offset.y,
    landscapeMaxZ + offset.z,
  );
  camera.up.copy(up);
  camera.lookAt(camX, camY, landscapeMaxZ);
  camera.updateProjectionMatrix();
  requestRender();
};

// ---- peek (right-drag: dy pitches, dx spins; snaps back on release) ----
const TILT_MAX_DEG = 60;
let tiltDeg = 0;
let spinDeg = 0;
let tiltAnimation: number | null = null;

const cancelTiltSnapBack = () => {
  if (tiltAnimation !== null) {
    cancelAnimationFrame(tiltAnimation);
    tiltAnimation = null;
  }
};

const snapTiltBack = () => {
  cancelTiltSnapBack();
  const fromTilt = tiltDeg;
  const fromSpin = spinDeg;
  if (Math.abs(fromTilt) <= 0.01 && Math.abs(fromSpin) <= 0.01) {
    tiltDeg = 0;
    spinDeg = 0;
    applyCamera();
    return;
  }
  const started = performance.now();
  const durationMs = 260;
  const step = (now: number) => {
    const t = Math.min(1, (now - started) / durationMs);
    const eased = 1 - (1 - t) ** 3; // ease-out cubic
    tiltDeg = fromTilt * (1 - eased);
    spinDeg = fromSpin * (1 - eased);
    applyCamera();
    tiltAnimation = t < 1 ? requestAnimationFrame(step) : null;
  };
  tiltAnimation = requestAnimationFrame(step);
};

const frameToLandscape = (
  minX: number,
  maxX: number,
  minY: number,
  maxY: number,
) => {
  const margin = 1.15;
  const sizeX = Math.max(maxX - minX, 1);
  const sizeY = Math.max(maxY - minY, 1);
  baseHalfWidth = (sizeX / 2) * margin;
  baseHalfHeight = (sizeY / 2) * margin;
  camX = (minX + maxX) / 2;
  camY = (minY + maxY) / 2;
  camZoom = 1;
  applyCamera();
};

// ---- landscape loading ----
const disposeLandscape = () => {
  if (!landscapeGroup) return;
  if (landscapeMesh) {
    landscapeGroup.remove(landscapeMesh);
    landscapeMesh.geometry.dispose();
    landscapeMesh = null;
  }
};

const loadLandscape = async () => {
  const token = ++loadToken;
  disposeLandscape();
  if (!props.landscapePath) {
    requestRender();
    return;
  }
  isLoading.value = true;
  try {
    const bytes = await readFile(props.landscapePath);
    if (token !== loadToken) return;
    const buffer = toTransferableBuffer(bytes);
    const [part] = await stlDecode.decodeInWorker(token, [buffer]);
    if (token !== loadToken || !part) return;

    const geometry = new THREE.BufferGeometry();
    geometry.setAttribute(
      "position",
      new THREE.BufferAttribute(part.position, 3),
    );
    if (part.normal) {
      geometry.setAttribute(
        "normal",
        new THREE.BufferAttribute(part.normal, 3),
      );
    } else {
      geometry.computeVertexNormals();
    }
    if (part.index) geometry.setIndex(new THREE.BufferAttribute(part.index, 1));
    geometry.computeBoundingBox();

    if (!landscapeMaterial || !landscapeGroup) return;
    landscapeMesh = new THREE.Mesh(geometry, landscapeMaterial);
    landscapeGroup.add(landscapeMesh);

    const box = geometry.boundingBox as THREE.Box3;
    landscapeMaxZ = box.max.z;
    frameToLandscape(box.min.x, box.max.x, box.min.y, box.max.y);
    syncOverlays();
    requestRender();
    emit("loaded", {
      centerX: (box.min.x + box.max.x) / 2,
      centerY: (box.min.y + box.max.y) / 2,
      minX: box.min.x,
      maxX: box.max.x,
      minY: box.min.y,
      maxY: box.max.y,
    });
  } catch (error) {
    if (!(error instanceof Error && error.message === SUPERSEDED)) {
      emit("error", `Failed to load landscape: ${error}`);
    }
  } finally {
    if (token === loadToken) isLoading.value = false;
  }
};

watch(() => [props.landscapePath, props.reloadToken], loadLandscape);

/** Local-space (unrotated, uncentered) polygon points for a cutter kind. */
const footprintPoints = (kind: CutterKind): [number, number][] => {
  if (kind.kind === "rect") {
    const hw = kind.width_mm / 2;
    const hd = kind.depth_mm / 2;
    return [
      [-hw, -hd],
      [hw, -hd],
      [hw, hd],
      [-hw, hd],
    ];
  }
  const rx = kind.kind === "circle" ? kind.diameter_mm / 2 : kind.major_mm / 2;
  const ry = kind.kind === "circle" ? kind.diameter_mm / 2 : kind.minor_mm / 2;
  const pts: [number, number][] = [];
  for (let i = 0; i < CIRCLE_SEGMENTS; i++) {
    const a = (i / CIRCLE_SEGMENTS) * Math.PI * 2;
    pts.push([Math.cos(a) * rx, Math.sin(a) * ry]);
  }
  return pts;
};

const OUTER_COLOR = 0x9ad1ff;
const OUTER_SELECTED_COLOR = 0xffcc55;
const INNER_COLOR = 0x5a8fc0;
const INNER_SELECTED_COLOR = 0xcf9a3a;

const makeLoop = (
  points: [number, number][],
  color: number,
  dashed: boolean,
) => {
  const positions = new Float32Array(points.length * 3);
  points.forEach(([x, y], i) => {
    positions[i * 3] = x;
    positions[i * 3 + 1] = y;
    positions[i * 3 + 2] = 0;
  });
  const geometry = new THREE.BufferGeometry();
  geometry.setAttribute("position", new THREE.BufferAttribute(positions, 3));
  const material = dashed
    ? new THREE.LineDashedMaterial({ color, dashSize: 1.5, gapSize: 1 })
    : new THREE.LineBasicMaterial({ color });
  const loop = new THREE.LineLoop(geometry, material);
  if (dashed) loop.computeLineDistances();
  return loop;
};

/** Two-level walk (Group -> LineLoop): dispose a single overlay group's
 * outer/inner line loops without removing the group itself. */
const disposeOverlayGroup = (group: THREE.Group) => {
  for (const child of group.children) {
    if (child instanceof THREE.LineLoop) {
      child.geometry.dispose();
      (child.material as THREE.Material).dispose();
    }
  }
};

/** Full teardown: dispose every overlay group's geometry/material, remove
 * them from the scene, and forget them. Used on unmount and whenever the
 * placement list drops to zero. */
const disposeOverlays = () => {
  if (!overlayGroup) return;
  for (const group of overlayGroups) {
    overlayGroup.remove(group);
    disposeOverlayGroup(group);
  }
  overlayGroups = [];
};

const overlayZ = () =>
  landscapeMaxZ + Math.max(0.5, (baseHalfWidth + baseHalfHeight) * 0.001);

/** Stable stringify of a cutter kind — cheap identity check for "did this
 * placement's shape change" without a deep-equal. */
const kindKeyOf = (kind: CutterKind): string => JSON.stringify(kind);
/** Only the plinth fields that feed insetShrink affect the cut (inner)
 * outline, so only they need to invalidate cached overlay geometry. */
const plinthKeyOf = (plinth: PlinthParams): string =>
  `${plinth.height_mm}:${plinth.taper_deg}`;

/** Local +X half-extent of a footprint — where the rotation handle sits
 * (along the shape's own major axis, so it rotates with the placement). */
const plusXExtent = (kind: CutterKind): number => footprintDims(kind).width / 2;

const HANDLE_STEM_MM = 3;
const HANDLE_RADIUS_MM = 1.4;

/** Distance from the placement center to the rotation handle's center. */
const handleDist = (kind: CutterKind): number =>
  plusXExtent(kind) + HANDLE_STEM_MM + HANDLE_RADIUS_MM;

const buildOverlayLoops = (
  group: THREE.Group,
  placement: Placement,
  shrink: number,
  selected: boolean,
) => {
  const outer = makeLoop(
    footprintPoints(placement.cutter),
    selected ? OUTER_SELECTED_COLOR : OUTER_COLOR,
    false,
  );
  const inner = makeLoop(
    footprintPoints(shrinkKind(placement.cutter, shrink)),
    selected ? INNER_SELECTED_COLOR : INNER_COLOR,
    true,
  );
  // Rotation handle: a stem out of the footprint's local +X plus a small
  // circle to grab — lives in the group, so the placement's own rotation
  // carries it. Only shown on the selected placement (and pointless for
  // circles, whose rotation is invisible).
  const xEdge = plusXExtent(placement.cutter);
  const stem = makeLoop(
    [
      [xEdge, 0],
      [xEdge + HANDLE_STEM_MM, 0],
    ],
    OUTER_SELECTED_COLOR,
    false,
  );
  const knob = makeLoop(
    Array.from({ length: 24 }, (_, i): [number, number] => {
      const a = (i / 24) * Math.PI * 2;
      return [
        xEdge +
          HANDLE_STEM_MM +
          HANDLE_RADIUS_MM +
          Math.cos(a) * HANDLE_RADIUS_MM,
        Math.sin(a) * HANDLE_RADIUS_MM,
      ];
    }),
    OUTER_SELECTED_COLOR,
    false,
  );
  stem.visible = knob.visible =
    selected && placement.cutter.kind !== "circle" && !props.locked;
  group.add(outer, inner, stem, knob);
};

/** Show the rotation handle only on the selected, rotatable placement. */
const syncHandleVisibility = () => {
  overlayGroups.forEach((group, index) => {
    const [, , stem, knob] = group.children;
    const p = props.placements[index];
    const show =
      index === props.selectedIndex &&
      p !== undefined &&
      p.cutter.kind !== "circle" &&
      !props.locked;
    if (stem) stem.visible = show;
    if (knob) knob.visible = show;
  });
};

/**
 * Reconciles the overlay scene graph with `props.placements` instead of
 * tearing everything down every time (the old rebuildOverlays did a full
 * dispose+recreate on every position change, i.e. every frame of a drag).
 * Index-aligned with the placement array: a group whose stored kindKey and
 * plinthKey haven't changed just gets its transform updated (cheap, the
 * drag-time hot path); a mismatch (different cutter, or a plinth taper
 * edit) regenerates that group's geometry; extra trailing groups are
 * disposed when the list shrinks.
 */
const syncOverlays = () => {
  if (!overlayGroup) return;
  const placements = props.placements;

  if (placements.length === 0) {
    disposeOverlays();
    requestRender();
    return;
  }

  while (overlayGroups.length > placements.length) {
    const group = overlayGroups.pop();
    if (!group) continue;
    overlayGroup.remove(group);
    disposeOverlayGroup(group);
  }

  const shrink = insetShrink(props.plinth);
  const z = overlayZ();
  const plinthKey = plinthKeyOf(props.plinth);

  placements.forEach((placement, index) => {
    const selected = index === props.selectedIndex;
    const kindKey = kindKeyOf(placement.cutter);
    let group = overlayGroups[index];

    if (!group) {
      group = new THREE.Group();
      buildOverlayLoops(group, placement, shrink, selected);
      overlayGroups[index] = group;
      overlayGroup?.add(group);
    } else if (
      group.userData.kindKey !== kindKey ||
      group.userData.plinthKey !== plinthKey
    ) {
      disposeOverlayGroup(group);
      while (group.children.length) group.remove(group.children[0]);
      buildOverlayLoops(group, placement, shrink, selected);
    }

    group.userData.index = index;
    group.userData.kindKey = kindKey;
    group.userData.plinthKey = plinthKey;
    group.position.set(placement.x_mm, placement.y_mm, z);
    group.rotation.z = (placement.rotation_deg * Math.PI) / 180;
  });

  // Transform-only updates skip buildOverlayLoops, so after deletes shift
  // indices the handle could linger on the wrong placement.
  syncHandleVisibility();
  requestRender();
};

/** Selection changed: recolor the outer/inner loops in place, no geometry
 * work (see syncOverlays' doc comment). */
const updateOverlayColors = () => {
  overlayGroups.forEach((group, index) => {
    const selected = index === props.selectedIndex;
    const [outer, inner] = group.children as THREE.LineLoop[];
    if (outer) {
      (outer.material as THREE.LineBasicMaterial).color.setHex(
        selected ? OUTER_SELECTED_COLOR : OUTER_COLOR,
      );
    }
    if (inner) {
      (inner.material as THREE.LineDashedMaterial).color.setHex(
        selected ? INNER_SELECTED_COLOR : INNER_COLOR,
      );
    }
  });
  syncHandleVisibility();
  requestRender();
};

watch(() => props.placements, syncOverlays, { deep: true });
watch(() => props.plinth, syncOverlays, { deep: true });
watch(() => props.selectedIndex, updateOverlayColors);
watch(() => props.locked, updateOverlayColors);

// ---- pointer interaction ----
const raycaster = new THREE.Raycaster();
const groundPlane = new THREE.Plane(new THREE.Vector3(0, 0, 1), 0);

const pointerNdc = (e: PointerEvent) => {
  const el = container.value;
  if (!el) return new THREE.Vector2();
  const rect = el.getBoundingClientRect();
  return new THREE.Vector2(
    ((e.clientX - rect.left) / rect.width) * 2 - 1,
    -((e.clientY - rect.top) / rect.height) * 2 + 1,
  );
};

/** World XY under the pointer, via a vertical ray through the ortho camera
 * intersected with the landscape's own horizontal plane — same X/Y at any
 * Z since the camera looks straight down. */
const pointerWorld = (e: PointerEvent): THREE.Vector2 | null => {
  if (!camera) return null;
  raycaster.setFromCamera(pointerNdc(e), camera);
  groundPlane.constant = -landscapeMaxZ;
  const hit = new THREE.Vector3();
  if (!raycaster.ray.intersectPlane(groundPlane, hit)) return null;
  return new THREE.Vector2(hit.x, hit.y);
};

/** Hit-test placements (topmost/last first) against their NOMINAL footprint
 * (the bigger, easier-to-grab outline). */
const hitTestPlacement = (world: THREE.Vector2): number | null => {
  for (let i = props.placements.length - 1; i >= 0; i--) {
    const p = props.placements[i];
    const dx = world.x - p.x_mm;
    const dy = world.y - p.y_mm;
    const rad = (-p.rotation_deg * Math.PI) / 180;
    const lx = dx * Math.cos(rad) - dy * Math.sin(rad);
    const ly = dx * Math.sin(rad) + dy * Math.cos(rad);
    const kind = p.cutter;
    if (kind.kind === "rect") {
      if (
        Math.abs(lx) <= kind.width_mm / 2 &&
        Math.abs(ly) <= kind.depth_mm / 2
      ) {
        return i;
      }
    } else {
      const rx =
        kind.kind === "circle" ? kind.diameter_mm / 2 : kind.major_mm / 2;
      const ry =
        kind.kind === "circle" ? kind.diameter_mm / 2 : kind.minor_mm / 2;
      if ((lx / rx) ** 2 + (ly / ry) ** 2 <= 1) return i;
    }
  }
  return null;
};

let dragButton: number | null = null;
let isPanning = false;
let isTilting = false;
let rotateIndex: number | null = null;
let dragIndex: number | null = null;
let dragOffset = new THREE.Vector2();
let lastX = 0;
let lastY = 0;

const onPointerDown = (e: PointerEvent) => {
  container.value?.focus();
  dragButton = e.button;
  lastX = e.clientX;
  lastY = e.clientY;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);

  if (e.button === 1) {
    isPanning = true;
    dragIndex = null;
    return;
  }
  if (e.button === 2) {
    // Right-drag = tilt peek (mirrors StlViewport's right-drag-orbits
    // convention); it snaps back on release in onPointerUp.
    cancelTiltSnapBack();
    isTilting = true;
    dragIndex = null;
    return;
  }
  isPanning = false;

  if (e.button === 0) {
    const world = pointerWorld(e);

    // The rotation handle of the selected placement wins over footprint
    // hit-testing — it sits outside the outline, so there's no overlap in
    // practice, but a neighboring placement underneath must not steal it.
    if (world && props.selectedIndex !== null && !props.locked) {
      const p = props.placements[props.selectedIndex];
      if (p && p.cutter.kind !== "circle") {
        const rad = (p.rotation_deg * Math.PI) / 180;
        const d = handleDist(p.cutter);
        const hx = p.x_mm + Math.cos(rad) * d;
        const hy = p.y_mm + Math.sin(rad) * d;
        // Grab tolerance: the knob itself plus a few screen pixels so it
        // stays grabbable when zoomed out.
        const worldPerPixel = camera
          ? (camera.right - camera.left) /
            camera.zoom /
            (container.value?.clientWidth || 1)
          : 0.5;
        const tol = Math.max(HANDLE_RADIUS_MM * 1.8, 6 * worldPerPixel);
        if (Math.hypot(world.x - hx, world.y - hy) <= tol) {
          rotateIndex = props.selectedIndex;
          dragIndex = null;
          return;
        }
      }
    }

    const hit = world ? hitTestPlacement(world) : null;
    if (hit !== null && world) {
      const p = props.placements[hit];
      // Selecting stays allowed while locked (viewing which cut is which);
      // only starting a drag is suppressed.
      if (!props.locked) {
        dragIndex = hit;
        dragOffset.set(world.x - p.x_mm, world.y - p.y_mm);
      }
      if (hit !== props.selectedIndex) emit("select", hit);
    } else {
      dragIndex = null;
      if (props.selectedIndex !== null) emit("select", null);
    }
  }
};

const onPointerMove = (e: PointerEvent) => {
  if (dragButton === null) return;
  const dx = e.clientX - lastX;
  const dy = e.clientY - lastY;
  lastX = e.clientX;
  lastY = e.clientY;

  if (isTilting) {
    tiltDeg = Math.min(TILT_MAX_DEG, Math.max(0, tiltDeg + dy * 0.4));
    spinDeg = (((spinDeg + dx * 0.4) % 360) + 360) % 360;
    // Snap back the short way around: keep spin in (-180, 180].
    if (spinDeg > 180) spinDeg -= 360;
    applyCamera();
    return;
  }

  if (isPanning) {
    if (!camera || !container.value) return;
    const { clientWidth, clientHeight } = container.value;
    const worldPerPixelX =
      (camera.right - camera.left) / camera.zoom / (clientWidth || 1);
    const worldPerPixelY =
      (camera.top - camera.bottom) / camera.zoom / (clientHeight || 1);
    camX -= dx * worldPerPixelX;
    camY += dy * worldPerPixelY;
    applyCamera();
    return;
  }

  if (rotateIndex !== null) {
    const world = pointerWorld(e);
    const p = props.placements[rotateIndex];
    if (!world || !p) return;
    // The handle rides local +X, so the pointer's bearing from the center
    // IS the rotation. Shift snaps to 15° for rank-and-flank neatness.
    let deg = (Math.atan2(world.y - p.y_mm, world.x - p.x_mm) * 180) / Math.PI;
    if (e.shiftKey) deg = Math.round(deg / 15) * 15;
    emit("update", rotateIndex, { rotation_deg: Math.round(deg * 10) / 10 });
    return;
  }

  if (dragIndex !== null) {
    const world = pointerWorld(e);
    if (!world) return;
    emit("update", dragIndex, {
      x_mm: world.x - dragOffset.x,
      y_mm: world.y - dragOffset.y,
    });
  }
};

const onPointerUp = (e: PointerEvent) => {
  dragButton = null;
  isPanning = false;
  dragIndex = null;
  rotateIndex = null;
  if (isTilting) {
    isTilting = false;
    snapTiltBack();
  }
  (e.target as HTMLElement).releasePointerCapture(e.pointerId);
};

const onWheel = (e: WheelEvent) => {
  e.preventDefault();
  camZoom = Math.min(20, Math.max(0.1, camZoom * (1 - e.deltaY * 0.001)));
  applyCamera();
};

const onKeydown = (e: KeyboardEvent) => {
  if (props.locked) return;
  if (props.selectedIndex === null) return;
  const p = props.placements[props.selectedIndex];
  if (!p) return;
  if (e.key === "[" || e.key === "]") {
    e.preventDefault();
    const step = e.shiftKey ? ROTATE_STEP_FAST_DEG : ROTATE_STEP_DEG;
    const delta = e.key === "[" ? -step : step;
    const next = (((p.rotation_deg + delta) % 360) + 360) % 360;
    emit("update", props.selectedIndex, { rotation_deg: next });
  } else if (e.key === "Delete" || e.key === "Backspace") {
    e.preventDefault();
    emit("delete", props.selectedIndex);
  }
};

onMounted(() => {
  if (!container.value) return;
  setupScene(container.value);
  resizeObserver = new ResizeObserver(() => {
    if (!renderer || !container.value) return;
    const { clientWidth, clientHeight } = container.value;
    if (!clientWidth || !clientHeight) return;
    renderer.setSize(clientWidth, clientHeight);
    applyCamera();
  });
  resizeObserver.observe(container.value);
  loadLandscape();
});

onBeforeUnmount(() => {
  loadToken++;
  cancelTiltSnapBack();
  stlDecode.dispose();
  resizeObserver?.disconnect();
  disposeLandscape();
  disposeOverlays();
  landscapeMaterial?.dispose();
  renderer?.dispose();
  renderer?.domElement.remove();
  renderer = null;
  scene = null;
  camera = null;
  landscapeGroup = null;
  overlayGroup = null;
});
</script>

<template>
  <div class="relative w-full h-full min-h-64">
    <div
      ref="container"
      tabindex="0"
      class="w-full h-full rounded-box overflow-hidden outline-none cursor-grab active:cursor-grabbing"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @wheel="onWheel"
      @keydown="onKeydown"
      @contextmenu.prevent
    ></div>
    <div
      v-if="isLoading"
      class="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/50 rounded-box"
    >
      <span class="loading loading-spinner loading-lg"></span>
      <span class="text-sm opacity-70">Loading landscape…</span>
    </div>
    <div
      v-if="!landscapePath"
      class="absolute inset-0 flex items-center justify-center text-base-content/40 pointer-events-none"
    >
      Select a landscape STL to place cutters
    </div>
    <div
      v-else
      class="absolute bottom-2 left-2 text-xs text-base-content/40 pointer-events-none"
    >
      drag: move · handle: rotate (shift snaps 15°) · [ / ]: rotate · delete:
      remove · middle-drag: pan · right-drag: tilt/rotate peek · wheel: zoom
    </div>
  </div>
</template>
