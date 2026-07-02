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
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import { mergeVertices } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps<{
  parts: string[];
  /** Linear RGB resin color, matching the Blender script */
  color?: [number, number, number];
}>();

const emit = defineEmits<{
  rotation: [value: [number, number, number]];
  view: [value: { azimuth: number; elevation: number; zoom: number }];
  loaded: [];
  error: [message: string];
}>();

const container = ref<HTMLDivElement | null>(null);
const isLoading = ref(false);

const DEFAULT_COLOR: [number, number, number] = [0.8, 0.54, 0.35];
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

  updateCamera();
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

const updateCamera = () => {
  if (!camera || !pivot) return;
  const target = new THREE.Vector3(0, 0, 0.7);
  if (meshGroup) {
    const box = new THREE.Box3().setFromObject(pivot);
    if (!box.isEmpty()) box.getCenter(target);
  }
  const radius = Math.sqrt(3); // bounding sphere of the 2-unit normalized model
  const distance =
    (radius / Math.tan((camera.fov * Math.PI) / 360)) * view.zoom;
  const az = (view.azimuth * Math.PI) / 180;
  const direction = new THREE.Vector3(
    Math.sin(az),
    -Math.cos(az),
    view.elevation,
  ).normalize();
  camera.position.copy(target).addScaledVector(direction, distance);
  camera.lookAt(target);
  requestRender();
};

const floorModel = () => {
  if (!pivot) return;
  pivot.position.set(0, 0, 0);
  pivot.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(pivot);
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

const afterRotationChange = () => {
  floorModel();
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
  afterRotationChange();
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
  afterRotationChange();
};

const resetRotation = () => setRotation([0, 0, 0]);

defineExpose({ setRotation, rotateWorld, resetRotation });

// ---- pointer interaction ----
let dragButton: number | null = null;
let lastX = 0;
let lastY = 0;

const onPointerDown = (e: PointerEvent) => {
  dragButton = e.button;
  lastX = e.clientX;
  lastY = e.clientY;
  (e.target as HTMLElement).setPointerCapture(e.pointerId);
};

const onPointerMove = (e: PointerEvent) => {
  if (dragButton === null || !pivot || !camera) return;
  const dx = e.clientX - lastX;
  const dy = e.clientY - lastY;
  lastX = e.clientX;
  lastY = e.clientY;

  if (dragButton === 0) {
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
  dragButton = null;
  (e.target as HTMLElement).releasePointerCapture(e.pointerId);
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

const loadParts = async () => {
  if (!pivot || !material) return;
  const token = ++loadToken;
  disposeModel();
  if (!props.parts.length) {
    requestRender();
    return;
  }

  isLoading.value = true;
  try {
    const loader = new STLLoader();
    // Read all parts concurrently; Promise.all preserves part order
    const byteArrays = await Promise.all(
      props.parts.map((path) => readFile(path)),
    );
    if (token !== loadToken) return; // superseded by a newer selection

    const geometries: THREE.BufferGeometry[] = [];
    for (const bytes of byteArrays) {
      const buffer = bytes.buffer.slice(
        bytes.byteOffset,
        bytes.byteOffset + bytes.byteLength,
      ) as ArrayBuffer;
      let geometry: THREE.BufferGeometry = loader.parse(buffer);
      try {
        // STL is a triangle soup; merge + recompute normals ~= Blender shade_smooth
        geometry = mergeVertices(geometry, 1e-4);
        geometry.computeVertexNormals();
      } catch {
        // fall back to flat shading from the file's own normals
      }
      geometries.push(geometry);
    }

    // Join parts in their native coordinates (same as the Blender join),
    // then center and normalize the whole to 2 units like normalize() does.
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
    afterRotationChange();
    emit("loaded");
  } catch (error) {
    emit("error", `Failed to load STL: ${error}`);
  } finally {
    if (token === loadToken) isLoading.value = false;
  }
};

watch(() => props.parts, loadParts, { deep: true });
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
  resizeObserver?.disconnect();
  disposeModel();
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
  <div class="relative w-full h-full min-h-64">
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
      class="absolute inset-0 flex items-center justify-center bg-black/50 rounded-box"
    >
      <span class="loading loading-spinner loading-lg"></span>
    </div>
    <div
      v-if="!parts.length"
      class="absolute inset-0 flex items-center justify-center text-base-content/40 pointer-events-none"
    >
      Select STL files to preview
    </div>
    <div
      v-else
      class="absolute bottom-2 left-2 text-xs text-base-content/40 pointer-events-none"
    >
      drag: rotate model · right-drag: orbit view · wheel: zoom
    </div>
  </div>
</template>
