<template>
  <!-- Duplicates panel: only groups with something left to gain — a merged
       group is done (stored once, every name works) and leaves the list -->
  <div
    v-if="showDups && reclaimableGroups.length"
    class="max-h-48 overflow-y-auto bg-base-200 border border-base-content/10 rounded-box p-3 text-xs space-y-2"
  >
    <div class="flex items-center gap-2 pb-1">
      <span
        class="font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40"
      >
        DUPLICATE GROUPS — MERGE TO SHARE ONE COPY, OR DELETE THE EXTRAS
      </span>
      <span class="flex-1"></span>
      <span v-if="linkSupport === false" class="text-base-content/50">
        this drive can't merge files — you can still delete copies
      </span>
      <button
        v-else-if="reclaimableGroups.length > 1"
        type="button"
        class="btn btn-xs btn-primary"
        :disabled="reclaimBusy || linkSupport === null"
        @click="mergeAllGroups"
      >
        merge all — free {{ formatFileSize(wastedBytes) }}
      </button>
    </div>
    <div v-for="group in reclaimableGroups" :key="group.hash">
      <div class="flex items-center gap-2">
        <span class="font-semibold">
          {{ group.paths.length }}× {{ formatFileSize(group.size_bytes) }}
        </span>
        <span class="flex-1"></span>
        <span
          v-if="!actionableOthers(group).length"
          class="text-base-content/40"
          title="Every extra copy lives inside a pack archive — unpack the model to merge or delete"
        >
          📦 packed — unpack to act
        </span>
        <button
          v-if="linkSupport !== false && actionableOthers(group).length"
          type="button"
          class="btn btn-xs btn-primary"
          :disabled="reclaimBusy || linkSupport === null"
          title="Keep every file where it is, but store the bytes once — all variants keep working"
          @click="mergeGroup(group)"
        >
          merge — free {{ formatFileSize(reclaimableBytes(group)) }}
        </button>
        <button
          v-if="actionableOthers(group).length"
          type="button"
          class="btn btn-xs btn-outline btn-error"
          :disabled="reclaimBusy"
          title="Remove the copies from disk — only the kept file remains"
          @click="reclaimGroup(group)"
        >
          delete copies
        </button>
      </div>
      <ul class="opacity-70">
        <li
          v-for="path in group.paths"
          :key="path"
          class="flex items-center justify-between gap-2"
        >
          <label
            class="flex items-center gap-1.5 truncate"
            :class="
              packedIn(group).includes(path) ? 'opacity-60' : 'cursor-pointer'
            "
          >
            <input
              type="radio"
              class="radio radio-xs"
              :name="`keep-${group.hash}`"
              :checked="keepFor(group) === path"
              :disabled="packedIn(group).includes(path)"
              @change="keepChoice[group.hash] = path"
            />
            <span class="truncate" :title="path">{{ path }}</span>
            <span
              v-if="packedIn(group).includes(path)"
              class="shrink-0"
              title="Inside a pack archive — unpack the model to merge or delete this copy"
            >
              📦
            </span>
          </label>
          <!-- a packed path has no file to reveal; show its folder -->
          <button
            type="button"
            class="link shrink-0"
            @click="revealDupPath(group, path)"
          >
            reveal
          </button>
        </li>
      </ul>
    </div>
  </div>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { useCatalogStore } from "../../stores/catalogStore";
import { formatFileSize } from "../../utils/format";

const store = useCatalogStore();
const {
  showDups,
  reclaimableGroups,
  linkSupport,
  reclaimBusy,
  wastedBytes,
  keepChoice,
} = storeToRefs(store);
const {
  mergeAllGroups,
  actionableOthers,
  mergeGroup,
  reclaimableBytes,
  reclaimGroup,
  packedIn,
  keepFor,
  revealDupPath,
} = store;
</script>
