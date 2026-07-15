import { describe, expect, it } from "vitest";
import { popLast, pushBounded } from "./placementUndo";

describe("pushBounded", () => {
  it("appends below the cap", () => {
    expect(pushBounded([1, 2], 3, 10)).toEqual([1, 2, 3]);
  });

  it("drops the OLDEST entry once length exceeds max", () => {
    const stack = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    expect(pushBounded(stack, 11, 10)).toEqual([
      2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
    ]);
  });

  it("caps at exactly 10 (the Base Cutter undo depth)", () => {
    let stack: number[] = [];
    for (let i = 1; i <= 25; i++) stack = pushBounded(stack, i, 10);
    expect(stack).toHaveLength(10);
    expect(stack).toEqual([16, 17, 18, 19, 20, 21, 22, 23, 24, 25]);
  });

  it("never mutates the input stack", () => {
    const stack = [1, 2, 3];
    const out = pushBounded(stack, 4, 10);
    expect(stack).toEqual([1, 2, 3]);
    expect(out).not.toBe(stack);
  });
});

describe("popLast", () => {
  it("returns undefined and the same (empty) stack when empty", () => {
    const { item, rest } = popLast([]);
    expect(item).toBeUndefined();
    expect(rest).toEqual([]);
  });

  it("pops the most recently pushed entry", () => {
    const { item, rest } = popLast([1, 2, 3]);
    expect(item).toBe(3);
    expect(rest).toEqual([1, 2]);
  });

  it("never mutates the input stack", () => {
    const stack = [1, 2, 3];
    popLast(stack);
    expect(stack).toEqual([1, 2, 3]);
  });

  it("round-trips with pushBounded (pop what was just pushed)", () => {
    const pushed = pushBounded([1, 2], 3, 10);
    const { item, rest } = popLast(pushed);
    expect(item).toBe(3);
    expect(rest).toEqual([1, 2]);
  });
});
