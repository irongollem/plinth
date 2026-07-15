import { describe, expect, it } from "vitest";
import { componentVertexColors, type Rgb } from "./meshComponents";

const TERRAIN: Rgb = [0.1, 0.1, 0.1];
const PIECE: Rgb = [0.9, 0.5, 0.2];

/** The colours come back as Float32Array, so compare per-channel with
 * float32 tolerance rather than exact equality against JS doubles. */
const expectRgb = (colors: Float32Array, vertex: number, rgb: Rgb) => {
  expect(colors[vertex * 3]).toBeCloseTo(rgb[0], 5);
  expect(colors[vertex * 3 + 1]).toBeCloseTo(rgb[1], 5);
  expect(colors[vertex * 3 + 2]).toBeCloseTo(rgb[2], 5);
};

/** A tetrahedron of the given size, offset to (ox,oy,oz). Returns its 4
 * vertex positions (flat) and 4 triangle faces (indices local to base). */
const tetra = (
  size: number,
  ox: number,
  oy: number,
  oz: number,
  base: number,
) => {
  const pos = [
    ox,
    oy,
    oz,
    ox + size,
    oy,
    oz,
    ox,
    oy + size,
    oz,
    ox,
    oy,
    oz + size,
  ];
  const idx = [
    base,
    base + 1,
    base + 2,
    base,
    base + 1,
    base + 3,
    base,
    base + 2,
    base + 3,
    base + 1,
    base + 2,
    base + 3,
  ];
  return { pos, idx };
};

describe("componentVertexColors", () => {
  it("splits two disjoint components and tints the smaller one", () => {
    // A big tetra (the 'terrain') and a small one far away (a 'piece').
    const big = tetra(10, 0, 0, 0, 0);
    const small = tetra(1, 50, 50, 50, 4);
    const position = [...big.pos, ...small.pos];
    const index = [...big.idx, ...small.idx];

    const colors = componentVertexColors(
      new Float32Array(position),
      new Uint32Array(index),
      TERRAIN,
      PIECE,
      1000,
    );
    expect(colors).not.toBeNull();
    // First 4 verts (big) = terrain, next 4 (small) = piece.
    expectRgb(colors!, 0, TERRAIN);
    expectRgb(colors!, 4, PIECE);
  });

  it("largest bbox diagonal wins regardless of vertex order", () => {
    // Small tetra FIRST, big one second — terrain must still be the big one.
    const small = tetra(1, 0, 0, 0, 0);
    const big = tetra(20, 30, 0, 0, 4);
    const colors = componentVertexColors(
      new Float32Array([...small.pos, ...big.pos]),
      new Uint32Array([...small.idx, ...big.idx]),
      TERRAIN,
      PIECE,
      1000,
    );
    expectRgb(colors!, 0, PIECE); // small = piece
    expectRgb(colors!, 4, TERRAIN); // big = terrain
  });

  it("returns null for a single component (nothing to tint apart)", () => {
    const one = tetra(10, 0, 0, 0, 0);
    expect(
      componentVertexColors(
        new Float32Array(one.pos),
        new Uint32Array(one.idx),
        TERRAIN,
        PIECE,
        1000,
      ),
    ).toBeNull();
  });

  it("returns null over the triangle budget", () => {
    const big = tetra(10, 0, 0, 0, 0);
    const small = tetra(1, 50, 0, 0, 4);
    // 8 triangles, budget 4 → skip.
    expect(
      componentVertexColors(
        new Float32Array([...big.pos, ...small.pos]),
        new Uint32Array([...big.idx, ...small.idx]),
        TERRAIN,
        PIECE,
        4,
      ),
    ).toBeNull();
  });

  it("returns null for a non-indexed mesh", () => {
    const one = tetra(10, 0, 0, 0, 0);
    expect(
      componentVertexColors(
        new Float32Array(one.pos),
        null,
        TERRAIN,
        PIECE,
        1000,
      ),
    ).toBeNull();
  });
});
