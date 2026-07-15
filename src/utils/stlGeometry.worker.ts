/**
 * STL decoding off the main thread. Parsing a multi-million-triangle mini
 * and mergeVertices() together take seconds of pure CPU — on the main
 * thread that freezes every click and animation in the app. The worker
 * receives raw file bytes and returns plain typed arrays (a BufferGeometry
 * isn't structured-cloneable); the viewport rebuilds geometries from them,
 * which is just wrapping the arrays — no copying, thanks to transfer.
 */
import { Color } from "three";
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import { mergeVertices } from "three/examples/jsm/utils/BufferGeometryUtils.js";
import { componentVertexColors, type Rgb } from "./meshComponents.ts";

export type StlPartPayload = {
  position: Float32Array;
  normal: Float32Array | null;
  index: Uint32Array | Uint16Array | null;
  /** Per-vertex RGB when `splitComponents` was requested and the mesh is
   * multi-shell — terrain grey, scatter pieces accent (see meshComponents).
   * null otherwise; the viewport renders plain grey then. */
  color: Float32Array | null;
};

export type StlDecodeRequest = {
  id: number;
  buffers: ArrayBuffer[];
  /** Tint disjoint components apart (terrain vs. scatter debris). Opt-in so
   * StlViewport's mini previews stay single-coloured. */
  splitComponents?: boolean;
};
export type StlDecodeResponse = {
  id: number;
  parts: StlPartPayload[];
  error?: string;
};

// Vertex-colour albedo (three interprets a color attribute as linear
// working space; Color(hex) converts the sRGB hex for us). Terrain matches
// the plain-grey material; the piece accent is a warm tan that reads
// clearly against it in the fixed dark viewport, both themes.
const TERRAIN_RGB = ((c) => [c.r, c.g, c.b] as Rgb)(new Color(0x8a8f86));
const PIECE_RGB = ((c) => [c.r, c.g, c.b] as Rgb)(new Color(0xd8965a));
// A scattered generated plate is ~60k terrain tris plus pieces; a
// multi-million-tri designer sculpt is single-shell and gains nothing, so
// bail well before the union-find would cost anything noticeable.
const MAX_SPLIT_TRIANGLES = 2_000_000;

self.addEventListener("message", (event: MessageEvent<StlDecodeRequest>) => {
  const { id, buffers, splitComponents } = event.data;
  try {
    const loader = new STLLoader();
    const parts: StlPartPayload[] = buffers.map((buffer) => {
      let geometry = loader.parse(buffer);
      try {
        // STL is a triangle soup; merge + recompute normals ~= Blender
        // shade_smooth (keep in sync with the render look)
        geometry = mergeVertices(geometry, 1e-4);
        geometry.computeVertexNormals();
      } catch {
        // fall back to flat shading from the file's own normals
      }
      const normal = geometry.getAttribute("normal");
      const index = geometry.getIndex();
      const position = geometry.getAttribute("position").array as Float32Array;
      // Only meaningful on a successfully-welded (indexed) mesh — see
      // componentVertexColors' own null cases.
      const color = splitComponents
        ? componentVertexColors(
            position,
            index ? (index.array as Uint32Array | Uint16Array) : null,
            TERRAIN_RGB,
            PIECE_RGB,
            MAX_SPLIT_TRIANGLES,
          )
        : null;
      return {
        position,
        normal: normal ? (normal.array as Float32Array) : null,
        index: index ? (index.array as Uint32Array | Uint16Array) : null,
        color,
      };
    });
    const transfer: Transferable[] = [];
    for (const part of parts) {
      transfer.push(part.position.buffer);
      if (part.normal) transfer.push(part.normal.buffer);
      if (part.index) transfer.push(part.index.buffer);
      if (part.color) transfer.push(part.color.buffer);
    }
    postMessage({ id, parts } satisfies StlDecodeResponse, { transfer });
  } catch (error) {
    postMessage({
      id,
      parts: [],
      error: String(error),
    } satisfies StlDecodeResponse);
  }
});
