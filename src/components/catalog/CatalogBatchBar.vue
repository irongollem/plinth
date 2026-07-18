<template>
  <!-- Batch move action bar (cards and rows are checkable) -->
  <div
    v-if="checkedGroups.length"
    class="flex items-center gap-2 bg-base-200 border border-base-content/10 rounded-lg px-3 py-1.5 text-xs"
  >
    <span class="font-mono text-base-content/60">
      {{ checkedGroups.length }} model{{
        checkedGroups.length === 1 ? "" : "s"
      }}
      selected
    </span>
    <template v-if="!combining">
      <button type="button" class="btn btn-xs btn-primary" @click="moveChecked">
        Move to folder…
      </button>
      <button
        v-if="checkedGroups.length >= 2"
        type="button"
        class="btn btn-xs"
        @click="startCombine"
      >
        Combine into one…
      </button>
      <button
        type="button"
        class="btn btn-xs"
        :disabled="isPacking"
        title="Compress the selected models into pack archives"
        @click="bulkPack(checkedGroups)"
      >
        Pack…
      </button>
      <button
        type="button"
        class="btn btn-xs"
        :disabled="isBatchRendering"
        title="Render the selected models' missing previews in one Blender launch"
        @click="openBatchRender(checkedGroups)"
      >
        Render previews…
      </button>
      <button
        type="button"
        class="btn btn-xs btn-error btn-outline"
        title="Delete the selected models — you confirm first"
        @click="openDeleteModal(checkedGroups)"
      >
        Delete…
      </button>
      <button
        type="button"
        class="btn btn-xs btn-ghost"
        @click="clearSelection"
      >
        clear
      </button>
    </template>
    <form
      v-else
      class="flex items-center gap-1.5"
      @submit.prevent="combineChecked"
    >
      <input
        v-model="combineName"
        type="text"
        class="input input-xs font-mono w-48"
        placeholder="combined model name"
      />
      <button type="submit" class="btn btn-xs btn-primary">
        combine {{ checkedGroups.length }}
      </button>
      <button
        type="button"
        class="btn btn-xs btn-ghost"
        @click="combining = false"
      >
        cancel
      </button>
    </form>
  </div>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useCatalogStore } from "../../stores/catalogStore";

const store = useCatalogStore();
const { checkedGroups, combining, combineName, isPacking, isBatchRendering } =
  storeToRefs(store);
const {
  moveChecked,
  startCombine,
  bulkPack,
  openBatchRender,
  openDeleteModal,
  clearSelection,
  combineChecked,
} = store;
</script>
