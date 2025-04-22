<template>
  <label :for="id" class="floating-label mb-2">
    <span class="label"> {{ label }}</span>
    <input
        :id="id"
        :value="modelValue"
        class="input w-full"
        type="text"
        @input="$emit('update:modelValue', ($event.target as HTMLInputElement).value)"
        :placeholder="placeholder"
        :required="required"
        :list="options && options.length > 0 ? `${id}-datalist` : undefined"
     />
     <datalist v-if="options && options.length > 0" :id="`${id}-datalist`">
       <option v-for="option in options" :key="option" :value="option" />
     </datalist>
  </label>
</template>

<script setup lang="ts">
defineProps<{
  id: string;
  label?: string;
  placeholder?: string;
  modelValue?: string | null;
  required?: boolean;
  options?: string[];
}>();

defineEmits<{
  "update:modelValue": [value: string];
}>();
</script>
