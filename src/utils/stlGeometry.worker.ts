/**
 * STL decoding off the main thread. Parsing a multi-million-triangle mini
 * and mergeVertices() together take seconds of pure CPU — on the main
 * thread that freezes every click and animation in the app. The worker
 * receives raw file bytes and returns plain typed arrays (a BufferGeometry
 * isn't structured-cloneable); the viewport rebuilds geometries from them,
 * which is just wrapping the arrays — no copying, thanks to transfer.
 */
import { STLLoader } from "three/examples/jsm/loaders/STLLoader.js";
import { mergeVertices } from "three/examples/jsm/utils/BufferGeometryUtils.js";

export type StlPartPayload = {
  position: Float32Array;
  normal: Float32Array | null;
  index: Uint32Array | Uint16Array | null;
};

export type StlDecodeRequest = { id: number; buffers: ArrayBuffer[] };
export type StlDecodeResponse = {
  id: number;
  parts: StlPartPayload[];
  error?: string;
};

self.onmessage = (event: MessageEvent<StlDecodeRequest>) => {
  const { id, buffers } = event.data;
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
      return {
        position: geometry.getAttribute("position").array as Float32Array,
        normal: normal ? (normal.array as Float32Array) : null,
        index: index ? (index.array as Uint32Array | Uint16Array) : null,
      };
    });
    const transfer: Transferable[] = [];
    for (const part of parts) {
      transfer.push(part.position.buffer);
      if (part.normal) transfer.push(part.normal.buffer);
      if (part.index) transfer.push(part.index.buffer);
    }
    postMessage({ id, parts } satisfies StlDecodeResponse, { transfer });
  } catch (error) {
    postMessage({
      id,
      parts: [],
      error: String(error),
    } satisfies StlDecodeResponse);
  }
};
