<template>
  <!-- Print file picker: tick exactly what goes to the slicer -->
  <ModalView :is-open="showPrintModal" @close="showPrintModal = false">
    <div
      class="w-120 max-w-[85vw] bg-base-100 rounded-box p-4 flex flex-col gap-3"
    >
      <div>
        <div class="font-bold text-[15px]">Print — {{ selected?.name }}</div>
        <p class="text-[11px] text-base-content/50 mt-0.5">
          Ticked files open in your slicer. Pre-sliced scenes carry supports and
          plate layout, so they're picked over raw geometry by default.
        </p>
      </div>
      <ul class="flex flex-col gap-0.5 max-h-72 overflow-y-auto">
        <li v-for="file in printCandidates" :key="file.path">
          <label
            class="flex items-center gap-2 cursor-pointer py-1 px-1.5 rounded hover:bg-base-200"
          >
            <input
              type="checkbox"
              class="checkbox checkbox-xs"
              :checked="printSelection.includes(file.path)"
              @change="togglePrintFile(file.path)"
            />
            <span
              class="flex-1 truncate font-mono text-[11.5px]"
              :title="file.path"
              >{{ file.file_name }}</span
            >
            <span
              v-if="SLICED_EXTS.includes(file.extension)"
              class="badge badge-xs badge-primary badge-outline"
              >pre-sliced</span
            >
            <span
              class="font-mono text-[10px] text-base-content/40 w-14 text-right"
              >{{ formatFileSize(file.size_bytes) }}</span
            >
          </label>
        </li>
      </ul>
      <!-- "print straight from the bundle": packed files are extracted
           just for this print and taken back afterwards -->
      <label
        v-if="printSelectionPacked"
        class="flex items-center gap-2 text-[11px] cursor-pointer"
      >
        <input
          v-model="packCleanupAfter"
          type="checkbox"
          class="checkbox checkbox-xs"
          @change="persistCleanupAfter"
        />
        <span>
          Clean up extracted files after sending
          <span class="text-base-content/50">
            — this model is packed; the slicer gets temporary copies
          </span>
        </span>
      </label>
      <div class="flex items-center gap-2">
        <button
          type="button"
          class="btn btn-sm btn-ghost"
          @click="revealFromPrintModal"
        >
          Reveal folder
        </button>
        <span class="flex-1"></span>
        <button
          type="button"
          class="btn btn-sm"
          @click="showPrintModal = false"
        >
          Cancel
        </button>
        <button
          type="button"
          class="btn btn-sm btn-primary"
          :disabled="!printSelection.length || printBusy"
          @click="sendToSlicer"
        >
          <span
            v-if="printBusy"
            class="loading loading-spinner loading-xs"
          ></span>
          Send {{ printSelection.length }} to slicer
        </button>
      </div>
    </div>
  </ModalView>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import ModalView from "../ModalView.vue";
import { useCatalogStore } from "../../stores/catalogStore";
import { formatFileSize } from "../../utils/format";

const store = useCatalogStore();
const {
  showPrintModal,
  selected,
  printCandidates,
  printSelection,
  SLICED_EXTS,
  printSelectionPacked,
  packCleanupAfter,
  printBusy,
} = storeToRefs(store);
const {
  togglePrintFile,
  persistCleanupAfter,
  revealFromPrintModal,
  sendToSlicer,
} = store;
</script>
