<template>
  <!-- Batch render confirm: what one Blender launch is about to sweep -->
  <ModalView :is-open="showBatchRender" @close="showBatchRender = false">
    <div
      class="w-120 max-w-[85vw] bg-base-100 rounded-box p-4 flex flex-col gap-3"
    >
      <div>
        <div class="font-bold text-[15px]">Render previews</div>
        <p class="text-[11px] text-base-content/50 mt-0.5">
          One Blender launch renders every selected model in sequence — finished
          previews stay even if you cancel midway. Stored rotations are reused;
          everything else stands up at the default orientation.
        </p>
      </div>
      <div
        v-if="batchLoading"
        class="h-16 flex items-center justify-center opacity-50"
      >
        <span class="loading loading-spinner loading-sm"></span>
      </div>
      <template v-else>
        <div class="font-mono text-[11.5px] flex flex-col gap-1">
          <span>
            {{ batchMissing.length }} model{{
              batchMissing.length === 1 ? "" : "s"
            }}
            without a preview (of {{ batchCandidates.length }} in scope)
          </span>
          <span v-if="batchPackedSkipped.length" class="text-base-content/50">
            📦 {{ batchPackedSkipped.length }} skipped — packed (unpack first,
            or render them individually from the drawer)
          </span>
          <label
            v-if="batchExisting.length"
            class="flex items-center gap-2 cursor-pointer"
          >
            <input
              v-model="batchRerenderExisting"
              type="checkbox"
              class="checkbox checkbox-xs"
            />
            <span>
              Re-render the {{ batchExisting.length }} existing preview{{
                batchExisting.length === 1 ? "" : "s"
              }}
              too
            </span>
          </label>
        </div>
        <div class="flex items-center gap-2">
          <span class="flex-1"></span>
          <button
            type="button"
            class="btn btn-sm"
            @click="showBatchRender = false"
          >
            Cancel
          </button>
          <button
            type="button"
            class="btn btn-sm btn-primary"
            :disabled="
              !batchMissing.length &&
              !(batchRerenderExisting && batchExisting.length)
            "
            @click="startBatchRender"
          >
            Render
            {{
              batchMissing.length +
              (batchRerenderExisting ? batchExisting.length : 0)
            }}
          </button>
        </div>
      </template>
    </div>
  </ModalView>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import ModalView from "../ModalView.vue";
import { useCatalogStore } from "../../stores/catalogStore";

const store = useCatalogStore();
const {
  showBatchRender,
  batchLoading,
  batchMissing,
  batchCandidates,
  batchPackedSkipped,
  batchExisting,
  batchRerenderExisting,
} = storeToRefs(store);
const { startBatchRender } = store;
</script>
