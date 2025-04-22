<template>
  <label :for="id" class="floating-label mb-2">
    <span class="label">{{ label }}</span>
    <input
      :id="id"
      :value="modelValue"
      class="input w-full"
      type="number"
      @input="handleInput"
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

const handleInput = (event: Event) => {
  const inputElement = event.target as HTMLInputElement;
  const value = inputElement.value;

  if (value === "") {
    emit("update:modelValue", null);
  } else {
    const numValue = Number.parseFloat(value);

    if (props.min !== undefined && numValue < props.min) {
      emit("update:modelValue", props.min);
    } else if (props.max !== undefined && numValue > props.max) {
      emit("update:modelValue", props.max);
    } else {
      emit("update:modelValue", numValue);
    }
  }
};
</script>
