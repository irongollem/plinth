<template>
  <label :for="id" class="floating-label mb-2">
    <span class="label">{{ label }}</span>
    <input
      :id="id"
      :value="modelValue"
      class="input w-full"
      type="number"
      @input="handleInput"
      @change="handleCommit"
      :placeholder="placeholder"
      :required="required"
      :min="min"
      :max="max"
      :step="step"
    />
  </label>
</template>

<script setup lang="ts">
const props = defineProps<{
  id: string;
  label?: string;
  placeholder?: string;
  modelValue?: number | null;
  required?: boolean;
  min?: number;
  max?: number;
  step?: number;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: number | null];
}>();

/** While typing, pass the raw parsed value through UNCLAMPED: clamping per
 * keystroke makes intermediate states impossible to type — with min=10,
 * entering "200" starts with "2", which would snap to 10 before the "00"
 * ever lands. The range is enforced on commit instead. */
const handleInput = (event: Event) => {
  const value = (event.target as HTMLInputElement).value;
  if (value === "") {
    emit("update:modelValue", null);
    return;
  }
  const parsed = Number.parseFloat(value);
  if (!Number.isNaN(parsed)) emit("update:modelValue", parsed);
};

/** Commit (blur / Enter): clamp into [min, max] and reflect it back. */
const handleCommit = (event: Event) => {
  const input = event.target as HTMLInputElement;
  if (input.value === "") return;
  let parsed = Number.parseFloat(input.value);
  if (Number.isNaN(parsed)) return;
  if (props.min !== undefined) parsed = Math.max(props.min, parsed);
  if (props.max !== undefined) parsed = Math.min(props.max, parsed);
  if (String(parsed) !== input.value) input.value = String(parsed);
  emit("update:modelValue", parsed);
};
</script>
