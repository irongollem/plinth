<script setup lang="ts">
/**
 * Interactive STL preview / positioning viewport.
 *
 * Mirrors the Blender render_mini.py scene so what you see is what renders:
 * Z-up world, ~60mm lens, warm key/fill/rim lighting, model normalized to
 * 2 units and floored on the grid. Left-drag tumbles the MODEL (not the
 * camera); the resulting orientation is emitted as Blender euler XYZ degrees
 * (three.js 'ZYX' order == Blender 'XYZ'), ready for --rotate.
 * Right-drag orbits the camera, wheel zooms; both are emitted so the final
 * render can match the previewed framing.
 */
import { readFile } from "@tauri-apps/plugin-fs";
import * as THREE from "three";
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import type {
  StlDecodeResponse,
  StlPartPayload,
} from "../utils/stlGeometry.worker.ts";

const props = defineProps<{
  parts: string[];
  /** Linear RGB resin color, matching the Blender script */
  color?: [number, number, number];
  /** Small-embed mode (catalog drawer): hides the help text and crop guide */
  compact?: boolean;
  /** Re-seat parts on the part named *base* (mirrors --align-parts) */
  alignParts?: boolean;
}>();

const emit = defineEmits<{
  rotation: [value: [number, number, number]];
  view: [value: { azimuth: number; elevation: number; zoom: number }];
  loaded: [];
  error: [message: string];
}>();

const container = ref<HTMLDivElement | null>(null);
const isLoading = ref(false);

// Keep in sync with LOOK.base_color in render_mini.py
const DEFAULT_COLOR: [number, number, number] = [0.85, 0.65, 0.43];
// 60mm lens on a 36mm sensor, square frame
const CAMERA_FOV = (2 * Math.atan(18 / 60) * 180) / Math.PI;

let renderer: THREE.WebGLRenderer | null = null;
let scene: THREE.Scene | null = null;
let camera: THREE.PerspectiveCamera | null = null;
let pivot: THREE.Group | null = null;
let meshGroup: THREE.Group | null = null;
let material: THREE.MeshStandardMaterial | null = null;
let resizeObserver: ResizeObserver | null = null;
let loadToken = 0;

/* ---- worker-side STL decoding ----
   Parsing + mergeVertices run in a Web Worker so million-triangle minis
   never freeze the UI. One worker per viewport; a superseded decode is
   aborted by terminating the worker (the only way to stop CPU-bound JS),
   which rejects its pending promise and a fresh worker takes over. */
let decodeWorker: Worker | null = null;
let pendingDecode: {
  id: number;
  resolve: (parts: StlPartPayload[]) => void;
  reject: (error: Error) => void;
} | null = null;

const SUPERSEDED = "superseded";

const spawnWorker = () => {
  const worker = new Worker(
    new URL("../utils/stlGeometry.worker.ts", import.meta.url),
    { type: "module" },
  );
  worker.onmessage = (event: MessageEvent<StlDecodeResponse>) => {
    if (!pendingDecode || event.data.id !== pendingDecode.id) return;
    const { resolve, reject } = pendingDecode;
    pendingDecode = null;
    if (event.data.error) reject(new Error(event.data.error));
    else resolve(event.data.parts);
  };
  return worker;
};

const abortDecode = () => {
  if (!pendingDecode) return;
  decodeWorker?.terminate();
  decodeWorker = null;
  pendingDecode.reject(new Error(SUPERSEDED));
  pendingDecode = null;
};

const decodeInWorker = (id: number, buffers: ArrayBuffer[]) => {
  abortDecode();
  decodeWorker ??= spawnWorker();
  return new Promise<StlPartPayload[]>((resolve, reject) => {
    pendingDecode = { id, resolve, reject };
    // buffers transfer, not copy — the worker owns them from here
    decodeWorker?.postMessage({ id, buffers }, buffers);
  });
};

// Camera parametrization identical to render_mini.py
const view = {
  azimuth: -15,
  elevation: 0.22,
  zoom: 1.15,
};

const setupScene = (el: HTMLDivElement) => {
  renderer = new THREE.WebGLRenderer({ antialias: true });
  renderer.setPixelRatio(window.devicePixelRatio);
  renderer.setSize(el.clientWidth, el.clientHeight);
  el.appendChild(renderer.domElement);

  scene = new THREE.Scene();
  scene.background = new THREE.Color(0x000000);

  camera = new THREE.PerspectiveCamera(
    CAMERA_FOV,
    el.clientWidth / el.clientHeight || 1,
    0.01,
    100,
  );
  camera.up.set(0, 0, 1);

  // Warm key / fill / rim, positions from the LOOK recipe
  const addLight = (
    color: number,
    intensity: number,
    position: [number, number, number],
  ) => {
    const light = new THREE.DirectionalLight(color, intensity);
    light.position.set(...position);
    light.target.position.set(0, 0, 0.6);
    scene?.add(light);
    scene?.add(light.target);
  };
  addLight(0xffd18c, 2.4, [4, -4, 6]); // key
  addLight(0xffc78c, 0.3, [-5, -2, 3]); // fill (low on purpose: deep shadows)
  addLight(0xffcc99, 1.2, [0, 5, 5]); // rim
  scene.add(new THREE.AmbientLight(0xffffff, 0.06));

  const grid = new THREE.GridHelper(4, 16, 0x444444, 0x222222);
  grid.rotation.x = Math.PI / 2; // XZ -> XY plane (Z-up floor)
  scene.add(grid);

  pivot = new THREE.Group();
  scene.add(pivot);

  material = new THREE.MeshStandardMaterial({
    color: new THREE.Color().setRGB(...(props.color ?? DEFAULT_COLOR)),
    roughness: 0.52,
    metalness: 0,
  });

  buildGizmo();
  updateCamera();
};

// ---- rotation gizmo: draggable rings for constrained axis rotation ----
const GIZMO_RADIUS = 1.45;
const RING_OPACITY = 0.35;
const RING_OPACITY_HOVER = 0.9;
const AXIS_VECTORS: Record<"x" | "y" | "z", THREE.Vector3> = {
  x: new THREE.Vector3(1, 0, 0),
  y: new THREE.Vector3(0, 1, 0),
  z: new THREE.Vector3(0, 0, 1),
};

let gizmo: THREE.Group | null = null;
/** Invisible fat tori — what the raycaster actually hits. */
let gizmoRings: THREE.Mesh[] = [];
/** Thin visible tori, keyed by axis, for hover/drag feedback. */
let visibleRings: Partial<Record<"x" | "y" | "z", THREE.Mesh>> = {};
let hoveredRing: THREE.Mesh | null = null;
let ringDrag: {
  axis: THREE.Vector3;
  u: THREE.Vector3;
  w: THREE.Vector3;
  center: THREE.Vector3;
  startAngle: number;
  startQuat: THREE.Quaternion;
} | null = null;
const raycaster = new THREE.Raycaster();

const buildGizmo = () => {
  if (!scene) return;
  gizmo = new THREE.Group();
  const ring = (
    axis: "x" | "y" | "z",
    color: number,
    orient: (m: THREE.Mesh) => void,
  ) => {
    const mesh = new THREE.Mesh(
      new THREE.TorusGeometry(GIZMO_RADIUS, 0.025, 10, 128),
      new THREE.MeshBasicMaterial({
        color,
        transparent: true,
        opacity: RING_OPACITY,
      }),
    );
    orient(mesh);
    mesh.userData.axis = axis;
    gizmo?.add(mesh);
    visibleRings[axis] = mesh;

    // What the raycaster hits: an invisible torus 6x fatter than the
    // visible one. The drawn ring is ~2px thick — as an actual hit target
    // it demands pixel-perfect aim, and in small viewports (the catalog
    // drawer) grabbing it is pure luck. opacity 0 instead of visible=false
    // because three.js skips invisible objects during raycasting.
    const hit = new THREE.Mesh(
      new THREE.TorusGeometry(GIZMO_RADIUS, 0.15, 6, 48),
      new THREE.MeshBasicMaterial({
        transparent: true,
        opacity: 0,
        depthWrite: false,
      }),
    );
    orient(hit);
    hit.userData.axis = axis;
    gizmo?.add(hit);
    gizmoRings.push(hit);
  };
  // A torus lies in the XY plane (normal +Z); orient the others to match
  ring("z", 0x5588f0, () => {});
  ring("x", 0xf06060, (m) => {
    m.rotation.y = Math.PI / 2;
  });
  ring("y", 0x58c060, (m) => {
    m.rotation.x = Math.PI / 2;
  });
  gizmo.visible = false;
  scene.add(gizmo);
};

const updateGizmo = () => {
  if (!gizmo || !pivot) return;
  gizmo.visible = !!meshGroup;
  if (worldBox) {
    worldBox.getCenter(gizmo.position);
    // The rings ride along with the model (local-axis gizmo, like
    // Blender's): turning one visibly tilts the other two
    gizmo.quaternion.copy(pivot.quaternion);
  }
};

const pointerNdc = (e: PointerEvent) => {
  const el = container.value;
  if (!el) return new THREE.Vector2();
  const rect = el.getBoundingClientRect();
  return new THREE.Vector2(
    ((e.clientX - rect.left) / rect.width) * 2 - 1,
    -((e.clientY - rect.top) / rect.height) * 2 + 1,
  );
};

/** Angle of the pointer around `axis` in the ring's plane, or null when the
 *  view ray runs (near-)parallel to that plane. */
const ringAngleAt = (
  e: PointerEvent,
  axis: THREE.Vector3,
  center: THREE.Vector3,
  u: THREE.Vector3,
  w: THREE.Vector3,
): number | null => {
  if (!camera) return null;
  raycaster.setFromCamera(pointerNdc(e), camera);
  const plane = new THREE.Plane().setFromNormalAndCoplanarPoint(axis, center);
  const hit = new THREE.Vector3();
  if (!raycaster.ray.intersectPlane(plane, hit)) return null;
  const v = hit.sub(center);
  return Math.atan2(w.dot(v), u.dot(v));
};

const pickRing = (e: PointerEvent): THREE.Mesh | null => {
  if (!camera || !gizmo?.visible) return null;
  raycaster.setFromCamera(pointerNdc(e), camera);
  const hits = raycaster.intersectObjects(gizmoRings, false);
  return (hits[0]?.object as THREE.Mesh) ?? null;
};

const beginRingDrag = (e: PointerEvent, mesh: THREE.Mesh): boolean => {
  if (!pivot || !gizmo) return false;
  // The grabbed ring's axis in WORLD space: rings are model-attached, so
  // the model's current orientation carries the axis with it
  const axis = AXIS_VECTORS[mesh.userData.axis as "x" | "y" | "z"]
    .clone()
    .applyQuaternion(pivot.quaternion)
    .normalize();
  // In-plane basis: u ⟂ axis, w = axis × u closes the right-handed frame
  const helper =
    Math.abs(axis.z) < 0.9
      ? new THREE.Vector3(0, 0, 1)
      : new THREE.Vector3(1, 0, 0);
  const u = new THREE.Vector3().crossVectors(helper, axis).normalize();
  const w = new THREE.Vector3().crossVectors(axis, u).normalize();
  const center = gizmo.position.clone();
  const startAngle = ringAngleAt(e, axis, center, u, w);
  if (startAngle === null) return false;
  ringDrag = {
    axis,
    u,
    w,
    center,
    startAngle,
    startQuat: pivot.quaternion.clone(),
  };
  return true;
};

// Render on demand instead of a 60fps loop: the scene is static between
// interactions, and a desktop app shouldn't burn GPU redrawing an
// identical frame. Every mutation path below calls requestRender().
let renderQueued = false;
const requestRender = () => {
  if (renderQueued || !renderer) return;
  renderQueued = true;
  requestAnimationFrame(() => {
    renderQueued = false;
    if (renderer && scene && camera) renderer.render(scene, camera);
  });
};

// World-space model bounds, cached after every rotation change (precise
// vertex bounds on release/load, fast approximate mid-drag). Camera and
// gizmo updates reuse the cache: recomputing per wheel tick was both slow
// and — when the fast approximate box was used — INFLATED under rotation,
// making the preview roomier than the render at the same zoom.
let worldBox: THREE.Box3 | null = null;

const refreshBounds = (precise: boolean) => {
  if (!pivot || !meshGroup) {
    worldBox = null;
    return;
  }
  const box = new THREE.Box3().setFromObject(pivot, precise);
  worldBox = box.isEmpty() ? null : box;
};

const updateCamera = () => {
  if (!camera || !pivot) return;
  const target = new THREE.Vector3(0, 0, 0.7);
  const az = (view.azimuth * Math.PI) / 180;
  // target -> camera direction, identical parametrization to render_mini.py
  const direction = new THREE.Vector3(
    Math.sin(az),
    -Math.cos(az),
    view.elevation,
  ).normalize();
  const half = Math.tan((camera.fov * Math.PI) / 360);

  let distance = (Math.sqrt(3) / half) * view.zoom; // empty-scene fallback
  if (worldBox) {
    worldBox.getCenter(target);
    // Exact fit, same algorithm as render_mini.py camera(): the distance at
    // which all 8 bbox corners sit inside the SQUARE render frame. Cannot
    // clip and is shape-independent — keep the two implementations in sync.
    const forward = direction.clone().negate();
    const right = new THREE.Vector3()
      .crossVectors(forward, new THREE.Vector3(0, 0, 1))
      .normalize();
    const up = new THREE.Vector3().crossVectors(right, forward);
    let needed = 0;
    const corner = new THREE.Vector3();
    for (const x of [worldBox.min.x, worldBox.max.x]) {
      for (const y of [worldBox.min.y, worldBox.max.y]) {
        for (const z of [worldBox.min.z, worldBox.max.z]) {
          corner.set(x, y, z).sub(target);
          needed = Math.max(
            needed,
            corner.dot(direction) +
              Math.max(Math.abs(corner.dot(right)), Math.abs(corner.dot(up))) /
                half,
          );
        }
      }
    }
    distance = needed * view.zoom;
  }

  camera.position.copy(target).addScaledVector(direction, distance);
  camera.lookAt(target);
  requestRender();
};

const floorModel = (precise = false) => {
  if (!pivot) return;
  pivot.position.set(0, 0, 0);
  pivot.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(pivot, precise);
  if (!box.isEmpty()) {
    pivot.position.z = -box.min.z;
  }
};

const emitRotation = () => {
  if (!pivot) return;
  // three.js 'ZYX' intrinsic == Blender euler 'XYZ' — same (x, y, z) angles
  const euler = new THREE.Euler().setFromQuaternion(pivot.quaternion, "ZYX");
  emit("rotation", [
    Math.round(THREE.MathUtils.radToDeg(euler.x) * 100) / 100,
    Math.round(THREE.MathUtils.radToDeg(euler.y) * 100) / 100,
    Math.round(THREE.MathUtils.radToDeg(euler.z) * 100) / 100,
  ]);
};

const emitView = () => {
  emit("view", {
    azimuth: Math.round(view.azimuth * 10) / 10,
    elevation: Math.round(view.elevation * 100) / 100,
    zoom: Math.round(view.zoom * 100) / 100,
  });
};

// `precise` iterates real vertices for exact bounds (matching Blender's
// framing exactly) — too slow for every drag frame on million-vertex
// minis, so drags use fast approximate boxes and snap precise on release.
const afterRotationChange = (precise = false) => {
  floorModel(precise);
  refreshBounds(precise);
  updateGizmo();
  updateCamera();
  emitRotation();
};

/** Apply a Blender euler XYZ rotation (degrees), e.g. a stored catalog value. */
const setRotation = (degrees: [number, number, number]) => {
  if (!pivot) return;
  const euler = new THREE.Euler(
    THREE.MathUtils.degToRad(degrees[0]),
    THREE.MathUtils.degToRad(degrees[1]),
    THREE.MathUtils.degToRad(degrees[2]),
    "ZYX",
  );
  pivot.quaternion.setFromEuler(euler);
  afterRotationChange(true);
};

/** Rotate around a world axis by degrees (snap buttons). */
const rotateWorld = (axis: "x" | "y" | "z", degrees: number) => {
  if (!pivot) return;
  const axes = {
    x: new THREE.Vector3(1, 0, 0),
    y: new THREE.Vector3(0, 1, 0),
    z: new THREE.Vector3(0, 0, 1),
  };
  const q = new THREE.Quaternion().setFromAxisAngle(
    axes[axis],
    THREE.MathUtils.degToRad(degrees),
  );
  pivot.quaternion.premultiply(q);
  afterRotationChange(true);
};

const resetRotation = () => setRotation([0, 0, 0]);

/** Set the camera parameters (azimuth°, elevation factor, zoom) directly. */
const setView = (next: {
  azimuth?: number;
  elevation?: number;
  zoom?: number;
}) => {
  if (next.azimuth !== undefined) view.azimuth = next.azimuth;
  if (next.elevation !== undefined) view.elevation = next.elevation;
  if (next.zoom !== undefined)
    view.zoom = Math.min(3, Math.max(0.5, next.zoom));
  updateCamera();
  emitView();
};

defineExpose({ setRotation, rotateWorld, resetRotation, setView });

// ---- pointer interaction ----
let dragButton: number | null = null;
let lastX = 0;
let lastY = 0;

const onPointerDown = (e: PointerEvent) => {
  dragButton = e.button;
  lastX = e.clientX;
  lastY = e.clientY;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);

  // Grabbing a gizmo ring wins over free tumble
  if (e.button === 0) {
    const ring = pickRing(e);
    if (ring && beginRingDrag(e, ring)) return;
  }
  ringDrag = null;
};

const onPointerMove = (e: PointerEvent) => {
  if (!pivot || !camera) return;

  // Hover feedback when not dragging — the raycast hits the invisible fat
  // ring; the highlight goes on its visible twin of the same axis
  if (dragButton === null) {
    const ring = pickRing(e);
    if (ring !== hoveredRing) {
      const axis = ring?.userData.axis;
      for (const [ringAxis, mesh] of Object.entries(visibleRings)) {
        (mesh.material as THREE.MeshBasicMaterial).opacity =
          ringAxis === axis ? RING_OPACITY_HOVER : RING_OPACITY;
      }
      hoveredRing = ring;
      if (container.value) {
        container.value.style.cursor = ring ? "pointer" : "";
      }
      requestRender();
    }
    return;
  }

  const dx = e.clientX - lastX;
  const dy = e.clientY - lastY;
  lastX = e.clientX;
  lastY = e.clientY;

  if (dragButton === 0 && ringDrag) {
    // Constrained rotation: angle around the grabbed ring's axis, applied
    // to the quaternion captured at drag start (no incremental drift)
    const angle = ringAngleAt(
      e,
      ringDrag.axis,
      ringDrag.center,
      ringDrag.u,
      ringDrag.w,
    );
    if (angle === null) return;
    const q = new THREE.Quaternion().setFromAxisAngle(
      ringDrag.axis,
      angle - ringDrag.startAngle,
    );
    pivot.quaternion.copy(ringDrag.startQuat).premultiply(q);
    afterRotationChange();
  } else if (dragButton === 0) {
    // Tumble the model in world space: horizontal = spin around Z (up),
    // vertical = tilt around the camera's right axis.
    const speed = 0.008;
    const spin = new THREE.Quaternion().setFromAxisAngle(
      new THREE.Vector3(0, 0, 1),
      dx * speed,
    );
    const right = new THREE.Vector3(1, 0, 0).applyQuaternion(camera.quaternion);
    const tilt = new THREE.Quaternion().setFromAxisAngle(right, dy * speed);
    pivot.quaternion.premultiply(spin).premultiply(tilt);
    afterRotationChange();
  } else {
    // Orbit the camera (view only, but emitted so the render can match)
    view.azimuth = (view.azimuth + dx * 0.3) % 360;
    view.elevation = Math.min(1.5, Math.max(-0.1, view.elevation + dy * 0.005));
    updateCamera();
    emitView();
  }
};

const onPointerUp = (e: PointerEvent) => {
  const wasRotating = dragButton === 0;
  dragButton = null;
  ringDrag = null;
  (e.target as HTMLElement).releasePointerCapture(e.pointerId);
  // Snap floor + framing to exact vertex bounds now that the drag is over
  if (wasRotating) afterRotationChange(true);
};

const onWheel = (e: WheelEvent) => {
  e.preventDefault();
  view.zoom = Math.min(3, Math.max(0.5, view.zoom * (1 + e.deltaY * 0.001)));
  updateCamera();
  emitView();
};

// ---- model loading ----
const disposeModel = () => {
  if (!meshGroup || !pivot) return;
  pivot.remove(meshGroup);
  meshGroup.traverse((child) => {
    if (child instanceof THREE.Mesh) child.geometry.dispose();
  });
  meshGroup = null;
};

/**
 * Re-seat parts exported around different origins ("stack on base").
 *
 * STL carries no shared origin, so when a creator re-exports one part the
 * files drift apart and the join floats the mini through its base. The
 * part named *base* is the ground truth: its THINNEST bbox axis is the
 * model's up axis whatever orientation it was exported in (bases are
 * flat), every other part gets centered over it and seated on its top.
 * Must mirror stack_on_base in render_mini.py so preview == render.
 */
const stackOnBase = (geometries: THREE.BufferGeometry[], paths: string[]) => {
  if (geometries.length < 2) return;
  const baseIndex = paths.findIndex((p) =>
    /base/i.test(p.split(/[\\/]/).pop() ?? ""),
  );
  if (baseIndex === -1) return;
  const boxes = geometries.map((g) => {
    g.computeBoundingBox();
    return g.boundingBox as THREE.Box3;
  });
  const base = boxes[baseIndex];
  const size = base.getSize(new THREE.Vector3());
  const dims: Array<"x" | "y" | "z"> = ["x", "y", "z"];
  const up =
    dims[[size.x, size.y, size.z].indexOf(Math.min(size.x, size.y, size.z))];
  const across = dims.filter((d) => d !== up);
  const baseCenter = base.getCenter(new THREE.Vector3());
  const baseTop = base.max[up];
  geometries.forEach((g, i) => {
    if (i === baseIndex) return;
    const box = boxes[i];
    const center = box.getCenter(new THREE.Vector3());
    const offset = new THREE.Vector3();
    for (const d of across) offset[d] = baseCenter[d] - center[d];
    offset[up] = baseTop - box.min[up];
    g.translate(offset.x, offset.y, offset.z);
  });
};

const loadParts = async () => {
  if (!pivot || !material) return;
  const token = ++loadToken;
  disposeModel();
  if (!props.parts.length) {
    refreshBounds(false);
    updateGizmo();
    requestRender();
    return;
  }

  isLoading.value = true;
  try {
    // Read all parts concurrently; Promise.all preserves part order
    const byteArrays = await Promise.all(
      props.parts.map((path) => readFile(path)),
    );
    if (token !== loadToken) return; // superseded by a newer selection

    // Parse + mergeVertices in the worker: seconds of CPU on big minis,
    // and the UI stays fully interactive while it runs
    const buffers = byteArrays.map(
      (bytes) =>
        bytes.buffer.slice(
          bytes.byteOffset,
          bytes.byteOffset + bytes.byteLength,
        ) as ArrayBuffer,
    );
    const payloads = await decodeInWorker(token, buffers);
    if (token !== loadToken) return;

    const geometries: THREE.BufferGeometry[] = payloads.map((part) => {
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
      }
      if (part.index) {
        geometry.setIndex(new THREE.BufferAttribute(part.index, 1));
      }
      return geometry;
    });

    // Join parts in their native coordinates (same as the Blender join),
    // then center and normalize the whole to 2 units like normalize() does.
    if (props.alignParts) stackOnBase(geometries, props.parts);
    const group = new THREE.Group();
    for (const geometry of geometries) {
      group.add(new THREE.Mesh(geometry, material));
    }
    const box = new THREE.Box3().setFromObject(group);
    const center = box.getCenter(new THREE.Vector3());
    const size = box.getSize(new THREE.Vector3());
    const scale = 2 / (Math.max(size.x, size.y, size.z) || 1);
    group.position.copy(center).multiplyScalar(-scale);
    group.scale.setScalar(scale);

    meshGroup = group;
    pivot.add(group);
    afterRotationChange(true);
    emit("loaded");
  } catch (error) {
    // an aborted decode isn't a failure — a newer selection took over
    if (!(error instanceof Error && error.message === SUPERSEDED)) {
      emit("error", `Failed to load STL: ${error}`);
    }
  } finally {
    if (token === loadToken) isLoading.value = false;
  }
};

watch(() => props.parts, loadParts, { deep: true });
watch(() => props.alignParts, loadParts);
watch(
  () => props.color,
  (color) => {
    material?.color.setRGB(...(color ?? DEFAULT_COLOR));
    requestRender();
  },
  { deep: true },
);

onMounted(() => {
  if (!container.value) return;
  setupScene(container.value);
  resizeObserver = new ResizeObserver(() => {
    if (!renderer || !camera || !container.value) return;
    const { clientWidth, clientHeight } = container.value;
    if (!clientWidth || !clientHeight) return;
    renderer.setSize(clientWidth, clientHeight);
    camera.aspect = clientWidth / clientHeight;
    camera.updateProjectionMatrix();
    requestRender();
  });
  resizeObserver.observe(container.value);
  loadParts();
});

onBeforeUnmount(() => {
  loadToken++;
  abortDecode();
  decodeWorker?.terminate();
  decodeWorker = null;
  resizeObserver?.disconnect();
  disposeModel();
  for (const mesh of [...gizmoRings, ...Object.values(visibleRings)]) {
    mesh.geometry.dispose();
    (mesh.material as THREE.MeshBasicMaterial).dispose();
  }
  gizmoRings = [];
  visibleRings = {};
  gizmo = null;
  material?.dispose();
  renderer?.dispose();
  renderer?.domElement.remove();
  renderer = null;
  scene = null;
  camera = null;
  pivot = null;
});
</script>

<template>
  <div class="relative w-full h-full" :class="compact ? '' : 'min-h-64'">
    <div
      ref="container"
      class="w-full h-full rounded-box overflow-hidden cursor-grab active:cursor-grabbing"
      @pointerdown="onPointerDown"
      @pointermove="onPointerMove"
      @pointerup="onPointerUp"
      @pointercancel="onPointerUp"
      @wheel="onWheel"
      @contextmenu.prevent
    ></div>
    <div
      v-if="isLoading"
      class="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-black/50 rounded-box"
    >
      <span class="loading loading-spinner loading-lg"></span>
      <span class="text-sm opacity-70">Loading model…</span>
    </div>
    <div
      v-if="!parts.length"
      class="absolute inset-0 flex items-center justify-center text-base-content/40 pointer-events-none"
    >
      Select STL files to preview
    </div>
    <template v-else-if="!compact">
      <!-- The render is square; this guide marks the actual capture area
           inside the rectangular viewport -->
      <div
        class="absolute inset-0 flex items-center justify-center pointer-events-none"
      >
        <div
          class="h-full max-w-full aspect-square border-x border-dashed border-white/15"
        ></div>
      </div>
      <div
        class="absolute bottom-2 left-2 text-xs text-base-content/40 pointer-events-none"
      >
        drag ring: rotate on axis · drag: free rotate · right-drag: orbit ·
        wheel: zoom · dashed lines: render crop
      </div>
    </template>
  </div>
</template>
