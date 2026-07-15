import { reactive, ref } from "vue";
import { describe, expect, it } from "vitest";
import { cloneRaw } from "./cloneRaw";

describe("cloneRaw", () => {
  // The regression this guards: structuredClone works on a plain object and
  // throws DataCloneError on the SAME object once it's read back out of a
  // ref/reactive (a Proxy). Code that "worked in onMounted" (raw command
  // result) silently broke on every template interaction (proxied).
  it("clones a value read out of a ref (a reactive Proxy)", () => {
    const presets = ref([
      {
        id: "sandy",
        params: { seed: 2, layers: { ripples: { enabled: true } } },
      },
    ]);
    const proxied = presets.value[0].params;

    expect(() => structuredClone(proxied)).toThrow(); // the original bug
    const clone = cloneRaw(proxied);
    expect(clone).toEqual({ seed: 2, layers: { ripples: { enabled: true } } });

    // A real copy, not a reference — editing the clone can't touch the preset.
    clone.layers.ripples.enabled = false;
    expect(presets.value[0].params.layers.ripples.enabled).toBe(true);
  });

  it("clones reactive() objects and plain objects alike", () => {
    const r = reactive({ a: { b: 1 } });
    expect(cloneRaw(r)).toEqual({ a: { b: 1 } });
    expect(cloneRaw({ a: { b: 2 } })).toEqual({ a: { b: 2 } });
  });
});
