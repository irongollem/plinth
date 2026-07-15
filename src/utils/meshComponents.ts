/**
 * Per-vertex colouring of a welded mesh by connected component — how the
 * placement viewport tints scatter debris apart from the terrain it sits
 * on. Scattered landscapes are multi-shell by construction (base_cut.py's
 * separate_into_shells, commit 918442b): the terrain and every loose
 * scatter piece are distinct connected components in one STL, sharing no
 * vertices. After the decode worker welds vertices (mergeVertices), a
 * union-find over the triangle index recovers those components; the one
 * with the largest bounding-box diagonal is the terrain (the same
 * largest-extent invariant base_cut.py uses to pick which shell to cut),
 * and everything else gets the accent colour.
 *
 * Pure and three.js-free so it's unit-testable: colours come in as plain
 * RGB triples, geometry as plain typed arrays.
 */
export type Rgb = [number, number, number];

/**
 * Returns a Float32Array of per-vertex RGB (terrain grey, pieces accent),
 * or null when tinting doesn't apply: a single component (a plain bake or
 * designer sculpt — nothing to distinguish), a mesh over the triangle
 * budget (a multi-million-tri sculpt isn't worth the union-find, and has
 * no scatter to highlight anyway), or a non-indexed mesh (only a failed
 * weld leaves geometry unindexed — without shared vertices every triangle
 * reads as its own component, which is meaningless).
 */
export const componentVertexColors = (
  position: Float32Array | number[],
  index: Uint32Array | Uint16Array | number[] | null,
  terrain: Rgb,
  piece: Rgb,
  maxTriangles: number,
): Float32Array | null => {
  if (!index) return null;
  const triangles = index.length / 3;
  if (triangles > maxTriangles) return null;

  const vertexCount = position.length / 3;
  const parent = new Uint32Array(vertexCount);
  for (let i = 0; i < vertexCount; i++) parent[i] = i;

  const find = (x: number): number => {
    let r = x;
    while (parent[r] !== r) {
      parent[r] = parent[parent[r]]; // path halving
      r = parent[r];
    }
    return r;
  };
  const union = (a: number, b: number) => {
    const ra = find(a);
    const rb = find(b);
    if (ra !== rb) parent[ra] = rb;
  };

  // Connectivity must be by POSITION, not by the caller's index. The decode
  // worker's mergeVertices hashes normals too, and STL carries flat
  // per-face normals — so coincident vertices at every face boundary stay
  // UNMERGED, and a union-find over that index shatters one smooth surface
  // into hundreds of components (the terrain then loses "largest" to a
  // stray flat patch, and most of it gets mis-tinted). Welding coincident
  // positions first re-fuses those splits; distinct shells never share a
  // position, so they stay apart. Quantized to 1e-4 mm — the normal-split
  // duplicates are exactly-equal positions, so they collide identically.
  const posRep = new Map<string, number>();
  for (let v = 0; v < vertexCount; v++) {
    const key = `${Math.round(position[v * 3] * 1e4)},${Math.round(
      position[v * 3 + 1] * 1e4,
    )},${Math.round(position[v * 3 + 2] * 1e4)}`;
    const rep = posRep.get(key);
    if (rep === undefined) posRep.set(key, v);
    else union(v, rep);
  }

  for (let t = 0; t < index.length; t += 3) {
    union(index[t], index[t + 1]);
    union(index[t + 1], index[t + 2]);
  }

  // Bounding box per root, in one pass over the vertices.
  type Box = {
    minX: number;
    maxX: number;
    minY: number;
    maxY: number;
    minZ: number;
    maxZ: number;
  };
  const boxes = new Map<number, Box>();
  for (let v = 0; v < vertexCount; v++) {
    const root = find(v);
    const x = position[v * 3];
    const y = position[v * 3 + 1];
    const z = position[v * 3 + 2];
    const b = boxes.get(root);
    if (b) {
      if (x < b.minX) b.minX = x;
      if (x > b.maxX) b.maxX = x;
      if (y < b.minY) b.minY = y;
      if (y > b.maxY) b.maxY = y;
      if (z < b.minZ) b.minZ = z;
      if (z > b.maxZ) b.maxZ = z;
    } else {
      boxes.set(root, { minX: x, maxX: x, minY: y, maxY: y, minZ: z, maxZ: z });
    }
  }

  if (boxes.size < 2) return null; // single shell — nothing to tint apart

  let terrainRoot = -1;
  let bestDiagonal = -1;
  for (const [root, b] of boxes) {
    const d =
      (b.maxX - b.minX) ** 2 + (b.maxY - b.minY) ** 2 + (b.maxZ - b.minZ) ** 2;
    if (d > bestDiagonal) {
      bestDiagonal = d;
      terrainRoot = root;
    }
  }

  const colors = new Float32Array(vertexCount * 3);
  for (let v = 0; v < vertexCount; v++) {
    const [r, g, b] = find(v) === terrainRoot ? terrain : piece;
    colors[v * 3] = r;
    colors[v * 3 + 1] = g;
    colors[v * 3 + 2] = b;
  }
  return colors;
};
