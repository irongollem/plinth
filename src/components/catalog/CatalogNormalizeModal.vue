<template>
  <!-- Normalizer: review-first cleanup of the on-disk structure -->
  <ModalView :is-open="showNormalize" @close="showNormalize = false">
    <div
      class="w-170 max-w-[90vw] bg-base-100 rounded-box p-4 flex flex-col gap-3"
    >
      <div>
        <div class="font-bold text-[15px]">Clean up library</div>
        <p class="text-[11px] text-base-content/50 mt-0.5">
          Moves folders into
          <span class="font-mono"
            >designer / release / model / Supported·Unsupported</span
          >
          and writes each model's metadata beside its files. Nothing moves until
          you approve the list below.
          <template v-if="normalizeScope">
            Planning only for <b>{{ normalizeScope }}</b
            >.</template
          >
          <template v-else-if="designerFilter">
            Planning only for <b>{{ designerFilter }}</b> (the toolbar
            filter).</template
          >
        </p>
      </div>

      <div
        v-if="normalizePlanning"
        class="h-24 flex items-center justify-center gap-2 opacity-60 text-sm"
      >
        <span class="loading loading-spinner loading-sm"></span>
        Planning moves…
      </div>

      <template v-else-if="normalizePlanData">
        <div
          class="flex items-center gap-3 font-mono text-[10.5px] text-base-content/50"
        >
          <label
            v-if="normalizePlanData.groups.length > 1"
            class="flex items-center gap-1.5 cursor-pointer"
          >
            <input
              type="checkbox"
              class="checkbox checkbox-xs"
              :checked="allPlanChecked"
              @change="toggleAllPlan"
            />
            all
          </label>
          <span>
            {{ normalizePlanData.groups.length }} model{{
              normalizePlanData.groups.length === 1 ? "" : "s"
            }}
            to restructure ·
            {{ normalizePlanData.clean_groups }} already clean
            <template v-if="normalizePlanData.skipped.length">
              · {{ normalizePlanData.skipped.length }} skipped</template
            >
          </span>
          <button
            v-if="normalizePlanData.clean_names.length"
            type="button"
            class="link text-base-content/50 hover:text-primary"
            title="Re-write model.json for the clean models from the catalog — no files move. Use after a Plinth update improves what gets written."
            :disabled="refreshingSidecars"
            @click="refreshSidecars(normalizePlanData.clean_names)"
          >
            {{
              refreshingSidecars
                ? "refreshing…"
                : `refresh metadata for ${normalizePlanData.clean_names.length} clean`
            }}
          </button>
        </div>

        <div
          v-if="!normalizePlanData.groups.length"
          class="py-6 text-center text-sm opacity-50"
        >
          Every folder already matches its model metadata 🎉
        </div>

        <ul v-else class="flex flex-col gap-1 max-h-80 overflow-y-auto pr-1">
          <li
            v-for="group in normalizePlanData.groups"
            :key="group.group_name"
            class="border border-base-content/10 rounded-lg px-2.5 py-1.5"
          >
            <div class="flex items-center gap-2">
              <input
                type="checkbox"
                class="checkbox checkbox-xs"
                :checked="normalizeChecked.includes(group.group_name)"
                @change="toggleNormalizeGroup(group.group_name)"
              />
              <span class="font-medium text-[12.5px] truncate">{{
                group.group_name
              }}</span>
              <span
                class="font-mono text-[10px] text-base-content/40 truncate"
                >{{ group.designer }}</span
              >
              <span class="flex-1"></span>
              <button
                type="button"
                class="link font-mono text-[10px] text-base-content/50"
                @click="
                  expandedPlanGroup =
                    expandedPlanGroup === group.group_name
                      ? null
                      : group.group_name
                "
              >
                {{ group.ops.length }} move{{
                  group.ops.length === 1 ? "" : "s"
                }}
              </button>
            </div>
            <div
              class="font-mono text-[10px] text-base-content/40 truncate pl-6"
              :title="group.target_dir"
            >
              → {{ group.target_dir }}
            </div>
            <div
              v-for="note in group.notes"
              :key="note"
              class="text-[10px] text-warning pl-6"
            >
              ⚠ {{ note }}
            </div>
            <ul
              v-if="expandedPlanGroup === group.group_name"
              class="pl-6 pt-1 flex flex-col gap-0.5"
            >
              <li
                v-for="op in group.ops"
                :key="op.from + op.to"
                class="font-mono text-[9.5px] text-base-content/50 truncate"
                :title="`${op.from} → ${op.to}`"
              >
                {{ op.kind === "pose" ? "tag" : op.kind }}
                {{ opLabel(op.from, op.to) }}
              </li>
            </ul>
          </li>
        </ul>

        <div
          v-if="normalizePlanData.skipped.length"
          class="flex flex-col gap-0.5 max-h-24 overflow-y-auto"
        >
          <div
            v-for="skip in normalizePlanData.skipped"
            :key="skip.group_name"
            class="font-mono text-[10px] text-base-content/40 truncate"
            :title="skip.reason"
          >
            skipped {{ skip.group_name }} — {{ skip.reason }}
          </div>
        </div>
      </template>

      <div
        v-if="normalizeIssues.length"
        class="alert alert-warning text-[11px] py-2 max-h-32 overflow-y-auto whitespace-pre-wrap"
      >
        {{ normalizeIssues.join("\n") }}
      </div>

      <div class="flex items-center gap-2">
        <template v-if="normalizeBusy">
          <progress
            class="progress progress-primary flex-1"
            :value="normalizeDone"
            :max="normalizeTotal"
          ></progress>
          <span class="font-mono text-[10.5px] text-base-content/50">
            {{ normalizeDone }} / {{ normalizeTotal }}
          </span>
        </template>
        <template v-else>
          <span class="flex-1"></span>
          <button
            type="button"
            class="btn btn-sm"
            @click="showNormalize = false"
          >
            Close
          </button>
          <button
            v-if="normalizePlanData?.groups.length"
            type="button"
            class="btn btn-sm btn-primary"
            :disabled="!normalizeChecked.length"
            @click="applyNormalizePlan"
          >
            Clean up {{ normalizeChecked.length }} model{{
              normalizeChecked.length === 1 ? "" : "s"
            }}
          </button>
        </template>
      </div>
    </div>
  </ModalView>
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import ModalView from "../ModalView.vue";
import { useCatalogStore } from "../../stores/catalogStore";

const store = useCatalogStore();
const {
  showNormalize,
  normalizeScope,
  designerFilter,
  normalizePlanning,
  normalizePlanData,
  allPlanChecked,
  refreshingSidecars,
  normalizeChecked,
  expandedPlanGroup,
  normalizeIssues,
  normalizeBusy,
  normalizeDone,
  normalizeTotal,
} = storeToRefs(store);
const {
  toggleAllPlan,
  refreshSidecars,
  toggleNormalizeGroup,
  opLabel,
  applyNormalizePlan,
} = store;
</script>
