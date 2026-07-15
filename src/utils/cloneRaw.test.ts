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

  // The regression this guards: BaseCutter.vue's addPlacement/placeRegiment/
  // runScatter each read a "kind" object out of a reactive `cutterLibrary`
  // ref (via a v-for chip or a `.find()`'d computed) and store it as a
  // NESTED field inside a freshly built placement object, e.g.
  // `{ cutter: cutter.kind, x_mm, y_mm, ... }`. The outer object is a plain
  // literal, but `cutter.kind` is itself a Proxy (Vue wraps nested object
  // reads lazily) — and `toRaw()` only unwraps the OUTERMOST Proxy of
  // whatever you hand it, not Proxies buried inside a plain object's own
  // fields. Push that literal into a `ref<T[]>` array and the nested Proxy
  // rides along raw: a later `cloneRaw(theArray.value)` (e.g. undo's
  // pushUndoSnapshot) still throws DataCloneError, because
  // `toRaw(theArray.value)` unwraps the array's own Proxy but leaves each
  // element's nested `cutter` field exactly as poisoned as before.
  // The fix is to cloneRaw the nested value AT THE POINT OF STORAGE, before
  // it ever reaches the container — this test proves that pattern actually
  // neutralizes the poison, not just that cloneRaw works on a flat value.
  it("unwrapping a nested Proxy at the point of storage keeps the container cloneable later", () => {
    const library = ref([{ id: "round28_5", kind: { diameter_mm: 28.5 } }]);
    const nestedProxy = library.value[0].kind; // a Proxy, same as `cutter.kind`

    const container = ref<{ cutter: { diameter_mm: number } }[]>([]);

    // The broken pattern: store the Proxy verbatim. Poisons the container
    // for every later cloneRaw — exactly what made the magnet buttons (and
    // every other undo-snapshotted placement edit) throw and no-op after
    // the first placement.
    container.value.push({ cutter: nestedProxy });
    expect(() => cloneRaw(container.value)).toThrow();
    container.value.splice(0); // reset

    // The fix: cloneRaw the nested field before it's stored.
    container.value.push({ cutter: cloneRaw(nestedProxy) });
    expect(() => cloneRaw(container.value)).not.toThrow();
    expect(cloneRaw(container.value)).toEqual([
      { cutter: { diameter_mm: 28.5 } },
    ]);
  });
});
