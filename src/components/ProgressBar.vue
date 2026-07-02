<script setup lang="ts">
defineProps<{
  progress: number;
}>();
</script>

<template>
  <div
    class="radial-progress"
    :style="`--value:${progress};`"
    :aria-valuenow="progress"
    role="progressbar"
  >
    {{ progress }}%
  </div>
</template>
