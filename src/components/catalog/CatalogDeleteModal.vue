<template>
  <!-- Delete confirmation: spells out exactly what goes before anything
       moves. Disk deletion defaults ON and goes to the OS trash, so even
       a confirmed mistake is recoverable. -->
  <ModalView :is-open="showDeleteModal" @close="showDeleteModal = false">
    <div
      class="w-120 max-w-[85vw] bg-base-100 rounded-box p-4 flex flex-col gap-3"
    >
      <div>
        <div class="font-bold text-[15px]">
          Delete
          {{
            deleteTargetNames.length === 1
              ? `“${deleteTargetNames[0]}”`
              : `${deleteTargetNames.length} models`
          }}
        </div>
        <p class="text-[11px] text-base-content/50 mt-0.5">
          <template v-if="deleteSummary">
            {{ deleteSummary.dir_count }} folder{{
              deleteSummary.dir_count === 1 ? "" : "s"
            }}
            · {{ deleteSummary.file_count }} file{{
              deleteSummary.file_count === 1 ? "" : "s"
            }}
            · {{ formatFileSize(deleteSummary.total_bytes) }}
          </template>
          <template v-else>
            <span class="loading loading-spinner loading-xs"></span>
            counting files…
          </template>
        </p>
      </div>

      <ul
        v-if="deleteTargetNames.length > 1"
        class="flex flex-col gap-0.5 max-h-32 overflow-y-auto text-[12px]"
      >
        <li v-for="name in deleteTargetNames" :key="name" class="truncate">
          • {{ name }}
        </li>
      </ul>

      <ul
        class="flex flex-col gap-0.5 max-h-40 overflow-y-auto font-mono text-[10.5px] text-base-content/50"
      >
        <li
          v-for="dir in deleteTargetDirs"
          :key="dir"
          class="truncate"
          :title="dir"
        >
          {{ dir }}
        </li>
      </ul>

      <label class="flex items-start gap-2 text-[11px] cursor-pointer">
        <input
          v-model="deleteAlsoFromDisk"
          type="checkbox"
          class="checkbox checkbox-xs mt-0.5"
        />
        <span>
          Also delete the files from disk
          <span class="block text-base-content/50">
            Folders are moved to the {{ trashName }} — recoverable until you
            empty it.
          </span>
        </span>
      </label>
      <p
        v-if="!deleteAlsoFromDisk"
        class="text-[11px] text-warning border border-warning/30 bg-warning/5 rounded-md px-2 py-1.5"
      >
        The files stay on disk, so these models will come back the next time
        their folder is scanned.
      </p>

      <div class="flex items-center gap-2">
        <span class="flex-1"></span>
        <button
          type="button"
          class="btn btn-sm"
          :disabled="deleteBusy"
          @click="showDeleteModal = false"
        >
          Cancel
        </button>
        <button
          type="button"
          class="btn btn-sm btn-error"
          :disabled="deleteBusy"
          @click="confirmDelete"
        >
          <span
            v-if="deleteBusy"
            class="loading loading-spinner loading-xs"
          ></span>
          {{ deleteAlsoFromDisk ? "Delete" : "Remove from catalog" }}
        </button>
      </div>
    </div>
  </ModalView>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import { computed } from "vue";
import { useOS } from "../../composables/useOS";
import { useCatalogStore } from "../../stores/catalogStore";
import { formatFileSize } from "../../utils/format";
import ModalView from "../ModalView.vue";

const store = useCatalogStore();
const {
  showDeleteModal,
  deleteBusy,
  deleteAlsoFromDisk,
  deleteTargetNames,
  deleteTargetDirs,
  deleteSummary,
} = storeToRefs(store);
const { confirmDelete } = store;

const { osType } = useOS();
const trashName = computed(() =>
  osType.value === "windows" ? "Recycle Bin" : "Trash",
);
</script>
