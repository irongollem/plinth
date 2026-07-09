<template>
  <div class="flex flex-wrap gap-1.5 items-center">
    <span class="font-mono text-[11px] text-base-content/40 shrink-0">
      {{ total.toLocaleString() }} result{{ total === 1 ? "" : "s" }}
    </span>
    <span
      v-if="visibleTags.length"
      class="w-px h-3.5 bg-base-content/15 shrink-0"
    ></span>
    <button
      v-for="tag in visibleTags"
      :key="tag.tag"
      type="button"
      class="font-mono text-[11px] rounded-full px-2.5 py-1 border cursor-pointer"
      :class="
        selectedTags.includes(tag.tag)
          ? 'bg-primary text-primary-content border-primary'
          : 'text-base-content/60 border-base-content/15'
      "
      @click="toggleTag(tag.tag)"
    >
      {{ tag.tag }} {{ tag.count }}
    </button>
  </div>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useCatalogStore } from "../../stores/catalogStore";

const store = useCatalogStore();
const { total, visibleTags, selectedTags } = storeToRefs(store);
const { toggleTag } = store;
</script>
