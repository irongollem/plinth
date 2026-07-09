<template>
  <div v-if="isScanning" class="text-xs opacity-70 flex items-center gap-2">
    <span class="loading loading-spinner loading-xs"></span>
    <span>
      Indexing... {{ scanProgress?.files_indexed ?? 0 }} files
      <span class="opacity-50">{{ scanProgress?.current_dir }}</span>
    </span>
  </div>
  <!-- Bulk pack progress lives at page level: the job may span models the
       drawer never opened -->
  <div v-if="isPacking" class="text-xs opacity-70 flex items-center gap-2">
    <span class="loading loading-spinner loading-xs"></span>
    <span>
      {{ packJobLabel }}…
      <template v-if="packProgress">
        {{ packProgress.model_index }}/{{ packProgress.total_models }} ·
        {{ packProgress.phase }} · {{ packProgress.percent }}%
        <span class="opacity-50">{{ packProgress.current_model }}</span>
      </template>
    </span>
    <button type="button" class="btn btn-xs btn-ghost" @click="cancelPack">
      cancel
    </button>
  </div>
  <!-- Batch render progress: one Blender process working the whole list -->
  <div
    v-if="isBatchRendering"
    class="text-xs opacity-70 flex items-center gap-2"
  >
    <span class="loading loading-spinner loading-xs"></span>
    <span>
      Rendering previews…
      <template v-if="batchProgress">
        {{ batchProgress.model_index }}/{{ batchProgress.total_models }} ·
        {{ batchProgress.percent }}%
        <span class="opacity-50">{{ batchProgress.current_model }}</span>
      </template>
    </span>
    <button type="button" class="btn btn-xs btn-ghost" @click="cancelBatch">
      cancel
    </button>
  </div>
  <div v-if="scanError" class="alert alert-error text-xs py-2">
    {{ scanError }}
  </div>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useCatalogStore } from "../../stores/catalogStore";

const store = useCatalogStore();
const {
  isScanning,
  scanProgress,
  isPacking,
  packJobLabel,
  packProgress,
  isBatchRendering,
  batchProgress,
  scanError,
} = storeToRefs(store);
const { cancelPack, cancelBatch } = store;
</script>
