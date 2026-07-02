<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { computed } from "vue";
import type { CatalogEntry } from "../bindings";
import { formatFileSize } from "../utils/format";

const props = defineProps<{
  entry: CatalogEntry;
  selected?: boolean;
}>();

defineEmits<{
  select: [entry: CatalogEntry];
}>();

const previewUrl = computed(() =>
  props.entry.preview_path ? convertFileSrc(props.entry.preview_path) : null,
);
</script>

<template>
  <button
    type="button"
    class="card bg-base-100 border border-gray-600 text-left hover:border-primary transition-colors overflow-hidden"
    :class="{ 'border-primary ring-1 ring-primary': selected }"
    @click="$emit('select', entry)"
  >
    <figure class="aspect-square bg-black/40">
      <img
        v-if="previewUrl"
        :src="previewUrl"
        :alt="entry.name"
        loading="lazy"
        class="object-cover w-full h-full"
      />
      <div
        v-else
        class="w-full h-full flex items-center justify-center text-4xl opacity-30"
      >
        🗿
      </div>
    </figure>
    <div class="card-body p-3 gap-1">
      <h3 class="font-semibold text-sm truncate" :title="entry.name">
        {{ entry.name }}
      </h3>
      <p v-if="entry.designer" class="text-xs opacity-60 truncate">
        {{ entry.designer }}
      </p>
      <p class="text-xs opacity-40">
        {{ entry.file_count }} file{{ entry.file_count === 1 ? "" : "s" }} ·
        {{ formatFileSize(entry.total_size_bytes) }}
      </p>
      <div v-if="entry.tags.length" class="flex flex-wrap gap-1 mt-1">
        <span
          v-for="tag in entry.tags.slice(0, 4)"
          :key="tag"
          class="badge badge-ghost badge-xs"
        >
          {{ tag }}
        </span>
        <span v-if="entry.tags.length > 4" class="badge badge-ghost badge-xs">
          +{{ entry.tags.length - 4 }}
        </span>
      </div>
    </div>
  </button>
</template>
