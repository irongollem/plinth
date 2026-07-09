<template>
  <section class="flex-1 overflow-y-auto min-h-0">
    <div
      v-if="!groups.length && !isScanning"
      class="h-full flex items-center justify-center opacity-40 text-sm"
    >
      {{
        stats?.total_models
          ? "No models match your search"
          : "No catalog yet — choose a folder and hit Scan"
      }}
    </div>

    <!-- LIST MODE (one row per logical model) -->
    <template v-if="viewMode === 'list'">
      <div
        v-if="groups.length"
        class="flex items-center gap-3 font-mono text-[9.5px] tracking-[0.12em] text-base-content/40 border-b border-base-content/10 pb-1.5 pr-3 sticky top-0 bg-base-100"
      >
        <span class="w-4"></span>
        <span class="w-10"></span>
        <span class="flex-1">MODEL</span>
        <span class="w-35">DESIGNER</span>
        <span class="w-40">VARIANTS</span>
        <span class="w-15 text-right">SIZE</span>
      </div>
      <template v-for="section in sections" :key="section.key">
        <div
          v-if="section.designer !== null"
          class="flex items-baseline gap-2 pt-3 pb-1"
        >
          <span class="font-bold text-[13px]">{{ section.designer }}</span>
          <span class="font-mono text-[10px] text-base-content/40">
            {{ sectionModelCount(section) }} model{{
              sectionModelCount(section) === 1 ? "" : "s"
            }}
          </span>
        </div>
        <template v-for="bucket in section.releases" :key="bucket.key">
          <div
            v-if="bucket.label !== null"
            class="flex items-baseline gap-2 py-1 pl-0.5"
          >
            <span
              class="font-mono font-semibold text-[10px] tracking-widest uppercase text-base-content/50"
              >{{ bucket.label }}</span
            >
            <span
              v-if="bucket.date"
              class="font-mono text-[9.5px] text-base-content/35"
              >{{ bucket.date }}</span
            >
          </div>
          <!-- div, not button: the row hosts a nested checkbox and
           interactive elements can't nest -->
          <div
            v-for="group in bucket.groups"
            :key="group.group_name"
            role="button"
            class="flex items-center gap-3 w-full text-left border-b border-base-content/5 py-1.5 pr-3 pl-2.5 cursor-pointer"
            :class="
              group.group_name === selectedGroup?.group_name
                ? 'bg-primary/10 border-l-2 border-l-primary'
                : 'border-l-2 border-l-transparent'
            "
            @click="selectGroup(group)"
          >
            <input
              type="checkbox"
              class="checkbox checkbox-xs w-4 shrink-0"
              :checked="checkedGroups.includes(group.group_name)"
              @click.stop
              @change="toggleCheckedGroup(group.group_name)"
            />
            <div
              class="w-10 h-10 shrink-0 rounded-md bg-base-300 overflow-hidden flex items-center justify-center text-base-content/30"
            >
              <img
                v-if="group.preview_path"
                :src="convertFileSrc(group.preview_path)"
                class="w-full h-full object-cover"
                alt=""
              />
              <span v-else class="text-lg">🗿</span>
            </div>
            <span class="flex-1 font-medium text-[13px] truncate">{{
              group.group_name
            }}</span>
            <span class="w-35 text-[12px] text-base-content/60 truncate">{{
              group.designer
            }}</span>
            <span
              class="w-40 font-mono text-[10.5px] text-base-content/50 truncate"
              >{{ groupSummary(group) }}</span
            >
            <span
              class="w-15 text-right font-mono text-[11px] text-base-content/50"
              >{{ formatFileSize(group.total_size_bytes) }}</span
            >
          </div>
        </template>
      </template>
    </template>

    <!-- GRID MODE (sections stack; each release bucket is its own grid) -->
    <div v-else class="flex flex-col gap-1.5">
      <template v-for="section in sections" :key="section.key">
        <div
          v-if="section.designer !== null"
          class="flex items-baseline gap-2 pt-2"
        >
          <span class="font-bold text-[13px]">{{ section.designer }}</span>
          <span class="font-mono text-[10px] text-base-content/40">
            {{ sectionModelCount(section) }} model{{
              sectionModelCount(section) === 1 ? "" : "s"
            }}
          </span>
        </div>
        <template v-for="bucket in section.releases" :key="bucket.key">
          <div
            v-if="bucket.label !== null"
            class="flex items-baseline gap-2 pl-0.5"
          >
            <span
              class="font-mono font-semibold text-[10px] tracking-widest uppercase text-base-content/50"
              >{{ bucket.label }}</span
            >
            <span
              v-if="bucket.date"
              class="font-mono text-[9.5px] text-base-content/35"
              >{{ bucket.date }}</span
            >
          </div>
          <div
            class="grid gap-3 mb-1.5"
            style="grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr))"
          >
            <CatalogCard
              v-for="group in bucket.groups"
              :key="group.group_name"
              :group="group"
              :selected="group.group_name === selectedGroup?.group_name"
              :checked="checkedGroups.includes(group.group_name)"
              @select="selectGroup"
              @toggle-check="toggleCheckedGroup($event.group_name)"
            />
          </div>
        </template>
      </template>
    </div>

    <div v-if="groups.length < total" class="flex justify-center py-4">
      <button type="button" class="btn btn-sm" @click="loadMore">
        Load more ({{ groups.length }} / {{ total }})
      </button>
    </div>
  </section>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { storeToRefs } from "pinia";
import CatalogCard from "../CatalogCard.vue";
import { useCatalogStore } from "../../stores/catalogStore";
import { formatFileSize } from "../../utils/format";

const store = useCatalogStore();
const {
  groups,
  isScanning,
  stats,
  viewMode,
  sections,
  selectedGroup,
  checkedGroups,
  total,
} = storeToRefs(store);
const {
  sectionModelCount,
  selectGroup,
  groupSummary,
  toggleCheckedGroup,
  loadMore,
} = store;
</script>
