<template>
  <details
    class="collapse collapse-arrow border border-base-content/10 bg-base-200/20 rounded-box"
  >
    <summary
      class="collapse-title min-h-0 py-2.5 px-3 flex items-center gap-2 cursor-pointer"
    >
      <span
        class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
        >ADVANCED — BLENDER LOOK RECIPE</span
      >
      <!-- Folded is the default state: without the badge, tweaks hidden in
           here would silently shape every render -->
      <span
        v-if="tweakCount"
        class="badge badge-primary badge-xs font-mono"
        title="Overrides applied to every render until reset"
        >{{ tweakCount }} tweak{{ tweakCount === 1 ? "" : "s" }}</span
      >
    </summary>
    <div class="collapse-content flex flex-col gap-3 px-3">
      <p class="text-[11px] text-base-content/50">
        Raw dials of the Blender recipe, for people who know their way around a
        light rig. They shape the final render only — the 3D preview does not
        simulate them.
      </p>
      <section
        v-for="group in LOOK_GROUPS"
        :key="group.title"
        class="flex flex-col gap-1.5"
        :class="{ 'opacity-40': isDormant(group) }"
      >
        <div class="flex items-baseline gap-2">
          <span
            class="font-mono font-semibold text-[9.5px] tracking-widest text-base-content/40"
            >{{ group.title.toUpperCase() }}</span
          >
          <span
            v-if="isDormant(group)"
            class="text-[9.5px] text-base-content/40"
            >applies when Look =
            {{ group.appliesTo === "resin" ? "Resin" : "Rich" }}</span
          >
        </div>
        <div
          v-for="knob in group.knobs"
          :key="knob.path"
          class="flex items-center gap-2"
          :title="knob.hint"
        >
          <span class="text-[11px] w-26 shrink-0 flex items-center gap-1">
            <span
              v-if="isModified(knob)"
              class="w-1.5 h-1.5 rounded-full bg-primary shrink-0"
            ></span>
            {{ knob.label }}
          </span>
          <template v-if="knob.kind === 'number'">
            <input
              type="range"
              class="range range-xs flex-1"
              :min="knob.min"
              :max="knob.max"
              :step="knob.step"
              :value="numberValue(knob)"
              @input="setNumber(knob, $event)"
            />
            <input
              type="number"
              class="input input-xs w-16 shrink-0"
              :step="knob.step"
              :value="numberValue(knob)"
              @change="setNumber(knob, $event)"
            />
          </template>
          <template v-else-if="knob.kind === 'color'">
            <label
              class="w-5.5 h-5.5 rounded-full cursor-pointer relative overflow-hidden border border-base-content/30 shrink-0"
            >
              <input
                type="color"
                class="absolute -top-1 -left-1 w-8 h-8 cursor-pointer"
                :value="colorValue(knob)"
                @input="setColor(knob, $event)"
              />
            </label>
            <span class="font-mono text-[10px] text-base-content/50 flex-1">{{
              colorValue(knob)
            }}</span>
          </template>
          <template v-else>
            <input
              v-for="axis in 3"
              :key="axis"
              type="number"
              class="input input-xs w-0 flex-1"
              :step="knob.step"
              :value="vecValue(knob)[axis - 1]"
              @change="setVecAxis(knob, axis - 1, $event)"
            />
          </template>
          <button
            type="button"
            class="btn btn-ghost btn-xs px-1 shrink-0"
            :class="{ invisible: !isModified(knob) }"
            title="Back to the locked look"
            @click="resetKnob(knob)"
          >
            ↺
          </button>
        </div>
      </section>
      <div class="flex gap-1.5 pt-2 border-t border-base-content/10">
        <button
          type="button"
          class="btn btn-xs btn-ghost"
          :disabled="!tweakCount"
          @click="emit('update:modelValue', {})"
        >
          Reset all
        </button>
      </div>
    </div>
  </details>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { hexToLinear, linearToHex } from "../utils/color";
import {
  isKnobDefault,
  LOOK_GROUPS,
  type LookGroup,
  type LookKnob,
  type LookOverrides,
  type LookValue,
  type Vec3,
} from "../utils/renderLookSchema";

const props = defineProps<{
  /** Diff from the locked look only — an empty record means "stock". */
  modelValue: LookOverrides;
  look: string;
}>();

const emit = defineEmits<{
  (e: "update:modelValue", value: LookOverrides): void;
}>();

const tweakCount = computed(() => Object.keys(props.modelValue).length);
const isModified = (knob: LookKnob) => knob.path in props.modelValue;
// Dormant, not hidden: overrides for the OTHER looks stay visible (and
// count in the badge) so switching looks never reveals surprise tweaks
const isDormant = (group: LookGroup) =>
  !!group.appliesTo && group.appliesTo !== props.look;

const numberValue = (knob: LookKnob) =>
  (props.modelValue[knob.path] as number) ?? (knob.default as number);
const vecValue = (knob: LookKnob) =>
  (props.modelValue[knob.path] as Vec3) ?? (knob.default as Vec3);
const colorValue = (knob: LookKnob) => linearToHex(vecValue(knob));

/** Setting a knob back onto its default REMOVES the override — the diff
 * record is what renders, persists, and exports. */
const setValue = (knob: LookKnob, value: LookValue) => {
  const next = { ...props.modelValue };
  if (isKnobDefault(knob, value)) delete next[knob.path];
  else next[knob.path] = value;
  emit("update:modelValue", next);
};

const parseInput = (knob: LookKnob, event: Event): number | null => {
  const value = Number.parseFloat((event.target as HTMLInputElement).value);
  if (Number.isNaN(value)) return null;
  return Math.min(
    knob.max ?? Number.POSITIVE_INFINITY,
    Math.max(knob.min ?? Number.NEGATIVE_INFINITY, value),
  );
};

const setNumber = (knob: LookKnob, event: Event) => {
  const value = parseInput(knob, event);
  if (value !== null) setValue(knob, value);
};

const setVecAxis = (knob: LookKnob, axis: number, event: Event) => {
  const value = parseInput(knob, event);
  if (value === null) return;
  const next = [...vecValue(knob)] as Vec3;
  next[axis] = value;
  setValue(knob, next);
};

const setColor = (knob: LookKnob, event: Event) => {
  setValue(knob, hexToLinear((event.target as HTMLInputElement).value));
};

const resetKnob = (knob: LookKnob) => {
  const next = { ...props.modelValue };
  delete next[knob.path];
  emit("update:modelValue", next);
};
</script>
