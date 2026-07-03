<template>
  <main class="bg-base-100 text-base-content flex flex-col h-full p-4 gap-3">
    <!-- Toolbar -->
    <div class="flex flex-wrap items-center gap-2">
      <label class="input input-sm flex-1 min-w-48 items-center gap-2">
        <svg
          width="13"
          height="13"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          class="opacity-40"
        >
          <circle cx="11" cy="11" r="7"></circle>
          <path d="M21 21l-4.3-4.3"></path>
        </svg>
        <input
          type="search"
          class="grow font-mono"
          placeholder="query models, tags…"
          v-model="query"
        />
      </label>
      <div
        class="flex bg-base-200 border border-base-content/10 rounded-lg p-0.5"
      >
        <button
          type="button"
          class="font-semibold text-[11px] px-2.5 py-1 rounded-md cursor-pointer"
          :class="
            viewMode === 'list'
              ? 'bg-primary text-primary-content'
              : 'text-base-content/60'
          "
          @click="viewMode = 'list'"
        >
          List
        </button>
        <button
          type="button"
          class="font-semibold text-[11px] px-2.5 py-1 rounded-md cursor-pointer"
          :class="
            viewMode === 'grid'
              ? 'bg-primary text-primary-content'
              : 'text-base-content/60'
          "
          @click="viewMode = 'grid'"
        >
          Grid
        </button>
      </div>
      <div class="join">
        <input
          type="text"
          readonly
          class="input input-sm join-item w-56 font-mono"
          :value="catalogRoot"
          placeholder="Choose a folder to index..."
        />
        <button type="button" class="btn btn-sm join-item" @click="chooseRoot">
          Folder
        </button>
        <button
          v-if="!isScanning"
          type="button"
          class="btn btn-sm btn-primary join-item"
          :disabled="!catalogRoot"
          @click="scan"
        >
          Scan
        </button>
        <button
          v-else
          type="button"
          class="btn btn-sm btn-error join-item"
          @click="cancelScan"
        >
          Cancel
        </button>
      </div>
      <span class="flex-1"></span>
      <span class="font-mono text-[11px] text-base-content/40">
        {{ total.toLocaleString() }} result{{ total === 1 ? "" : "s" }}
      </span>
    </div>

    <div v-if="isScanning" class="text-xs opacity-70 flex items-center gap-2">
      <span class="loading loading-spinner loading-xs"></span>
      <span>
        Indexing... {{ scanProgress?.files_indexed ?? 0 }} files
        <span class="opacity-50">{{ scanProgress?.current_dir }}</span>
      </span>
    </div>
    <div v-if="scanError" class="alert alert-error text-xs py-2">
      {{ scanError }}
    </div>

    <!-- Tag filter chips -->
    <div v-if="visibleTags.length" class="flex flex-wrap gap-1.5 items-center">
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

    <!-- Batch move action bar (rows are checkable in list mode) -->
    <div
      v-if="checkedDirs.length"
      class="flex items-center gap-2 bg-base-200 border border-base-content/10 rounded-lg px-3 py-1.5 text-xs"
    >
      <span class="font-mono text-base-content/60">
        {{ checkedDirs.length }} model{{ checkedDirs.length === 1 ? "" : "s" }}
        selected
      </span>
      <button type="button" class="btn btn-xs btn-primary" @click="moveChecked">
        Move to folder…
      </button>
      <button
        type="button"
        class="btn btn-xs btn-ghost"
        @click="checkedDirs = []"
      >
        clear
      </button>
    </div>

    <!-- Content -->
    <div class="flex flex-1 gap-3 min-h-0">
      <section class="flex-1 overflow-y-auto min-h-0">
        <div
          v-if="!entries.length && !isScanning"
          class="h-full flex items-center justify-center opacity-40 text-sm"
        >
          {{
            stats?.total_models
              ? "No models match your search"
              : "No catalog yet — choose a folder and hit Scan"
          }}
        </div>

        <!-- LIST MODE -->
        <template v-if="viewMode === 'list'">
          <div
            v-if="entries.length"
            class="flex items-center gap-3 font-mono text-[9.5px] tracking-[0.12em] text-base-content/40 border-b border-base-content/10 pb-1.5 pr-3 sticky top-0 bg-base-100"
          >
            <span class="w-4"></span>
            <span class="w-10"></span>
            <span class="flex-1">MODEL</span>
            <span class="w-[140px]">DESIGNER</span>
            <span class="w-[160px]">TAGS</span>
            <span class="w-[60px] text-right">SIZE</span>
          </div>
          <!-- div, not button: the row hosts a nested checkbox and
               interactive elements can't nest -->
          <div
            v-for="entry in entries"
            :key="entry.dir_path"
            role="button"
            class="flex items-center gap-3 w-full text-left border-b border-base-content/5 py-1.5 pr-3 pl-2.5 cursor-pointer"
            :class="
              entry.dir_path === selected?.dir_path
                ? 'bg-primary/10 border-l-2 border-l-primary'
                : 'border-l-2 border-l-transparent'
            "
            @click="selectEntry(entry)"
          >
            <input
              type="checkbox"
              class="checkbox checkbox-xs w-4 shrink-0"
              :checked="checkedDirs.includes(entry.dir_path)"
              @click.stop
              @change="toggleChecked(entry.dir_path)"
            />
            <div
              class="w-10 h-10 shrink-0 rounded-md bg-base-300 overflow-hidden flex items-center justify-center text-base-content/30"
            >
              <img
                v-if="entry.preview_path"
                :src="convertFileSrc(entry.preview_path)"
                class="w-full h-full object-cover"
                alt=""
              />
              <span v-else class="text-lg">🗿</span>
            </div>
            <span class="flex-1 font-medium text-[13px] truncate">{{
              entry.name
            }}</span>
            <span class="w-[140px] text-[12px] text-base-content/60 truncate">{{
              entry.designer
            }}</span>
            <span
              class="w-[160px] font-mono text-[10.5px] text-base-content/50 truncate"
              >{{ entry.tags.join(", ") }}</span
            >
            <span
              class="w-[60px] text-right font-mono text-[11px] text-base-content/50"
              >{{ formatFileSize(entry.total_size_bytes) }}</span
            >
          </div>
        </template>

        <!-- GRID MODE -->
        <div
          v-else
          class="grid gap-3"
          style="grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr))"
        >
          <CatalogCard
            v-for="entry in entries"
            :key="entry.dir_path"
            :entry="entry"
            :selected="entry.dir_path === selected?.dir_path"
            @select="selectEntry"
          />
        </div>

        <div v-if="entries.length < total" class="flex justify-center py-4">
          <button type="button" class="btn btn-sm" @click="loadMore">
            Load more ({{ entries.length }} / {{ total }})
          </button>
        </div>
      </section>

      <!-- Detail drawer -->
      <aside v-if="selected" class="w-[312px] shrink-0 overflow-y-auto">
        <div
          class="aspect-[4/3] rounded-box bg-base-300 border border-base-content/10 flex items-center justify-center text-base-content/30 overflow-hidden"
        >
          <img
            v-if="selected.preview_path"
            :src="convertFileSrc(selected.preview_path)"
            :alt="selected.name"
            class="w-full h-full object-cover"
          />
          <span v-else class="text-5xl">🗿</span>
        </div>
        <div class="py-3.5 flex flex-col gap-2.5">
          <div>
            <h2 class="font-bold text-[16px] leading-tight">
              {{ selected.name }}
            </h2>
            <p
              v-if="selected.designer || selected.release_name"
              class="font-mono text-[11px] text-base-content/50 mt-0.5"
            >
              {{
                [selected.designer, selected.release_name]
                  .filter(Boolean)
                  .join(" · ")
              }}
            </p>
          </div>

          <div class="flex flex-wrap gap-1.5">
            <span
              v-for="tag in selected.tags"
              :key="tag"
              class="font-mono text-[10px] text-base-content/60 border border-base-content/15 rounded-full px-2.5 py-0.5 flex items-center gap-1"
            >
              {{ tag }}
              <button
                type="button"
                class="opacity-50 hover:opacity-100"
                @click="removeTag(tag)"
              >
                ✕
              </button>
            </span>
            <form class="join" @submit.prevent="addTag">
              <input
                v-model="newTag"
                type="text"
                class="input input-xs join-item w-24"
                placeholder="+ tag"
              />
            </form>
          </div>

          <!-- Model details (pose/scale/supports/release date) -->
          <div>
            <div
              class="font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40 mb-1.5"
            >
              DETAILS
            </div>
            <div class="grid grid-cols-2 gap-1.5">
              <label class="flex flex-col gap-0.5">
                <span class="font-mono text-[9px] text-base-content/40"
                  >POSE / VARIANT</span
                >
                <input
                  v-model="metaDraft.pose"
                  type="text"
                  class="input input-xs font-mono"
                  placeholder="e.g. A"
                />
              </label>
              <label class="flex flex-col gap-0.5">
                <span class="font-mono text-[9px] text-base-content/40"
                  >SCALE</span
                >
                <input
                  v-model="metaDraft.scale"
                  type="text"
                  class="input input-xs font-mono"
                  placeholder="e.g. 32mm"
                />
              </label>
              <label class="flex flex-col gap-0.5">
                <span class="font-mono text-[9px] text-base-content/40"
                  >SUPPORTS</span
                >
                <select
                  v-model="metaDraft.support_status"
                  class="select select-xs font-mono"
                >
                  <option value="">unknown</option>
                  <option value="supported">supported</option>
                  <option value="unsupported">unsupported</option>
                  <option value="both">both</option>
                </select>
              </label>
              <label class="flex flex-col gap-0.5">
                <span class="font-mono text-[9px] text-base-content/40"
                  >RELEASED</span
                >
                <input
                  v-model="metaDraft.release_date"
                  type="text"
                  class="input input-xs font-mono"
                  placeholder="YYYY-MM"
                />
              </label>
            </div>
            <button
              v-if="metaDirty"
              type="button"
              class="btn btn-xs btn-primary w-full mt-1.5"
              @click="saveMetadata"
            >
              Save details
            </button>
          </div>

          <div class="flex gap-1.5">
            <button
              type="button"
              class="flex-1 text-center font-semibold text-[11px] tracking-[0.05em] bg-primary text-primary-content rounded-md py-2 cursor-pointer"
              @click="printModel"
            >
              PRINT
            </button>
            <button
              type="button"
              class="flex-1 text-center font-semibold text-[11px] tracking-[0.05em] border border-base-content/15 rounded-md py-2 cursor-pointer disabled:opacity-40"
              :disabled="!stlPaths.length"
              @click="show3d = true"
            >
              3D
            </button>
            <button
              type="button"
              class="flex-1 text-center font-semibold text-[11px] tracking-[0.05em] border border-base-content/15 rounded-md py-2 cursor-pointer disabled:opacity-40"
              :disabled="!stlPaths.length"
              @click="releasesStore.requestRender(stlPaths)"
            >
              RENDER
            </button>
          </div>

          <button
            type="button"
            class="font-semibold text-[11px] tracking-[0.03em] text-center border border-dashed rounded-md py-2 cursor-pointer"
            :class="
              releasesStore.releaseExists
                ? 'border-base-content/25 text-primary'
                : 'border-base-content/15 text-base-content/40'
            "
            @click="addToDraftRelease"
          >
            + Add to draft release
          </button>

          <div>
            <div
              class="font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40 mb-1.5"
            >
              FILES · {{ formatFileSize(selected.total_size_bytes) }}
            </div>
            <div
              v-for="file in files"
              :key="file.path"
              class="flex justify-between font-mono text-[11px] text-base-content/60 py-0.5"
            >
              <span class="truncate" :title="file.path">{{
                file.file_name
              }}</span>
              <span class="opacity-60 shrink-0">{{
                formatFileSize(file.size_bytes)
              }}</span>
            </div>
          </div>
        </div>
      </aside>
    </div>

    <!-- Footer: stats + duplicates -->
    <div
      class="flex flex-wrap items-center gap-4 font-mono text-[10.5px] text-base-content/40 border-t border-base-content/10 pt-2"
    >
      <template v-if="stats">
        <span
          @click="toggleDups"
          :class="dupGroups.length ? 'text-primary cursor-pointer' : ''"
        >
          <template v-if="dupGroups.length"
            >{{ dupGroups.length }} duplicate groups ·
            {{ formatFileSize(wastedBytes) }} reclaimable</template
          >
          <template v-else
            >{{ stats.total_models }} models · {{ stats.total_files }} files ·
            {{ formatFileSize(stats.total_size_bytes) }}</template
          >
        </span>
      </template>
      <span class="flex-1"></span>
      <span v-if="lastScanLabel">scanned {{ lastScanLabel }}</span>
      <button
        v-if="!isFindingDuplicates"
        type="button"
        class="border border-base-content/15 rounded-full px-2.5 py-0.5 text-base-content/60 cursor-pointer disabled:opacity-40"
        :disabled="!stats?.total_files"
        @click="startDuplicateScan"
      >
        rescan duplicates
      </button>
      <span v-else class="flex items-center gap-2">
        <span class="loading loading-spinner loading-xs"></span>
        hashing {{ dupProgress?.processed ?? 0 }}/{{
          dupProgress?.total ?? "?"
        }}
        <button type="button" class="link" @click="cancelDuplicateScan">
          cancel
        </button>
      </span>
    </div>

    <!-- Duplicates panel -->
    <div
      v-if="showDups && dupGroups.length"
      class="max-h-48 overflow-y-auto bg-base-200 border border-base-content/10 rounded-box p-3 text-xs space-y-2"
    >
      <div
        class="font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40 pb-1"
      >
        DUPLICATE GROUPS — PICK THE COPY TO KEEP, RECLAIM THE REST
      </div>
      <div v-for="group in dupGroups" :key="group.hash">
        <div class="flex items-center gap-2">
          <span class="font-semibold">
            {{ group.paths.length }}× {{ formatFileSize(group.size_bytes) }}
          </span>
          <span class="flex-1"></span>
          <button
            type="button"
            class="btn btn-xs btn-outline btn-error"
            :disabled="reclaimBusy"
            @click="reclaimGroup(group)"
          >
            reclaim
            {{ formatFileSize(group.size_bytes * (group.paths.length - 1)) }}
          </button>
        </div>
        <ul class="opacity-70">
          <li
            v-for="path in group.paths"
            :key="path"
            class="flex items-center justify-between gap-2"
          >
            <label class="flex items-center gap-1.5 truncate cursor-pointer">
              <input
                type="radio"
                class="radio radio-xs"
                :name="`keep-${group.hash}`"
                :checked="keepFor(group) === path"
                @change="keepChoice[group.hash] = path"
              />
              <span class="truncate" :title="path">{{ path }}</span>
            </label>
            <button type="button" class="link shrink-0" @click="reveal(path)">
              reveal
            </button>
          </li>
        </ul>
      </div>
    </div>

    <!-- 3D preview modal -->
    <ModalView :is-open="show3d" @close="show3d = false">
      <div class="w-[70vw] h-[70vh] bg-base-300 rounded-box">
        <StlViewport v-if="show3d" :parts="stlPaths" />
      </div>
    </ModalView>
  </main>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { computed, onMounted, ref, watch } from "vue";
import {
  type CatalogEntry,
  type CatalogFile,
  type CatalogStats,
  type DuplicateGroup,
  type TagCount,
  commands,
} from "../bindings";
import CatalogCard from "../components/CatalogCard.vue";
import ModalView from "../components/ModalView.vue";
import StlViewport from "../components/StlViewport.vue";
import { useCatalogJobs } from "../composables/useCatalogJobs";
import { useFileSelect } from "../composables/useFileSelect";
import { useReleasesStore } from "../stores/releasesStore";
import { useToastStore } from "../stores/toastStore";
import { formatFileSize } from "../utils/format";

const PAGE_SIZE = 60;

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { selectDirectory } = useFileSelect();
const {
  isScanning,
  scanProgress,
  scanError,
  scanCompletedCount,
  startScan,
  cancelScan,
  isFindingDuplicates,
  dupProgress,
  dupCompletedCount,
  startDuplicateScan,
  cancelDuplicateScan,
} = useCatalogJobs();

const catalogRoot = ref("");
const query = ref("");
const viewMode = ref<"list" | "grid">("grid");
const selectedTags = ref<string[]>([]);
const allTags = ref<TagCount[]>([]);
const entries = ref<CatalogEntry[]>([]);
const total = ref(0);
const stats = ref<CatalogStats | null>(null);
const selected = ref<CatalogEntry | null>(null);
const files = ref<CatalogFile[]>([]);
const newTag = ref("");
const dupGroups = ref<DuplicateGroup[]>([]);
const showDups = ref(false);
const show3d = ref(false);
// per-group hash -> path the user wants to keep (defaults to the first)
const keepChoice = ref<Record<string, string>>({});
const reclaimBusy = ref(false);
// dir_paths ticked for a batch move
const checkedDirs = ref<string[]>([]);
const metaDraft = ref({
  pose: "",
  scale: "",
  support_status: "",
  release_date: "",
});

const visibleTags = computed(() => {
  const top = allTags.value.slice(0, 12);
  // keep selected tags visible even when they fall outside the top list
  for (const tag of selectedTags.value) {
    if (!top.some((t) => t.tag === tag)) {
      const known = allTags.value.find((t) => t.tag === tag);
      top.push(known ?? { tag, count: 0 });
    }
  }
  return top;
});

const stlPaths = computed(() =>
  files.value.filter((f) => f.extension === "stl").map((f) => f.path),
);

const wastedBytes = computed(() =>
  dupGroups.value.reduce(
    (sum, g) => sum + g.size_bytes * (g.paths.length - 1),
    0,
  ),
);

const lastScanLabel = computed(() => {
  if (!stats.value?.last_scan_epoch) return null;
  return new Date(stats.value.last_scan_epoch * 1000).toLocaleString();
});

const runSearch = async (append = false) => {
  const offset = append ? entries.value.length : 0;
  const result = await commands.searchCatalog(
    query.value,
    selectedTags.value,
    PAGE_SIZE,
    offset,
  );
  if (result.status === "ok") {
    entries.value = append
      ? [...entries.value, ...result.data.entries]
      : result.data.entries;
    total.value = result.data.total;
  } else {
    toastStore.reportError("Search failed", result.error);
  }
};

const loadMore = () => runSearch(true);

let searchTimeout: number | null = null;
watch([query, selectedTags], () => {
  if (searchTimeout) clearTimeout(searchTimeout);
  searchTimeout = setTimeout(() => runSearch(), 250) as unknown as number;
});

const refreshMeta = async () => {
  const [tagsResult, statsResult, dupResult] = await Promise.all([
    commands.getCatalogTags(),
    commands.getCatalogStats(),
    commands.getDuplicateGroups(),
  ]);
  if (tagsResult.status === "ok") allTags.value = tagsResult.data;
  if (statsResult.status === "ok") stats.value = statsResult.data;
  if (dupResult.status === "ok") dupGroups.value = dupResult.data;
};

const toggleTag = (tag: string) => {
  selectedTags.value = selectedTags.value.includes(tag)
    ? selectedTags.value.filter((t) => t !== tag)
    : [...selectedTags.value, tag];
};

const toggleDups = () => {
  if (dupGroups.value.length) showDups.value = !showDups.value;
};

const selectEntry = async (entry: CatalogEntry) => {
  selected.value = entry;
  files.value = [];
  const result = await commands.getCatalogModelFiles(entry.dir_path);
  if (result.status === "ok") files.value = result.data;
};

const addTag = async () => {
  if (!selected.value || !newTag.value.trim()) return;
  const result = await commands.addCatalogTag(
    selected.value.dir_path,
    newTag.value,
  );
  if (result.status === "ok") {
    newTag.value = "";
    await refreshSelected();
    await refreshMeta();
  } else {
    toastStore.reportError("Failed to add tag", result.error);
  }
};

const removeTag = async (tag: string) => {
  if (!selected.value) return;
  const result = await commands.removeCatalogTag(selected.value.dir_path, tag);
  if (result.status === "ok") {
    await refreshSelected();
    await refreshMeta();
  } else {
    toastStore.reportError("Failed to remove tag", result.error);
  }
};

/** Re-fetch the selected entry so tag edits show up immediately. */
const refreshSelected = async () => {
  if (!selected.value) return;
  const dirPath = selected.value.dir_path;
  await runSearch();
  const updated = entries.value.find((e) => e.dir_path === dirPath);
  if (updated) selected.value = updated;
};

const printModel = async () => {
  if (!selected.value) return;
  // Reveal the first model file so the folder opens with it selected,
  // ready to drag into a slicer (v2: hand the file to the slicer directly)
  const target = files.value[0]?.path ?? selected.value.dir_path;
  try {
    await revealItemInDir(target);
  } catch (error) {
    toastStore.reportError("Failed to open folder", error);
  }
};

const reveal = async (path: string) => {
  try {
    await revealItemInDir(path);
  } catch (error) {
    toastStore.reportError("Failed to reveal file", error);
  }
};

const keepFor = (group: DuplicateGroup) =>
  keepChoice.value[group.hash] ?? group.paths[0];

const reclaimGroup = async (group: DuplicateGroup) => {
  const keep = keepFor(group);
  const doomed = group.paths.filter((path) => path !== keep);
  const confirmed = await confirm(
    `Delete ${doomed.length} duplicate file${doomed.length === 1 ? "" : "s"} and keep:\n${keep}`,
    { title: "Reclaim duplicates", kind: "warning" },
  );
  if (!confirmed) return;
  reclaimBusy.value = true;
  try {
    const result = await commands.deleteDuplicateFiles(doomed);
    if (result.status === "ok") {
      const { succeeded, errors } = result.data;
      if (succeeded) {
        toastStore.addToast(
          `Reclaimed ${succeeded} duplicate file${succeeded === 1 ? "" : "s"}`,
          "success",
        );
      }
      for (const error of errors) toastStore.addToast(error, "error");
      // the backend pruned the index, so groups/stats/sizes are already fresh
      await Promise.all([runSearch(), refreshMeta()]);
    } else {
      toastStore.reportError("Failed to delete duplicates", result.error);
    }
  } finally {
    reclaimBusy.value = false;
  }
};

const toggleChecked = (dirPath: string) => {
  checkedDirs.value = checkedDirs.value.includes(dirPath)
    ? checkedDirs.value.filter((d) => d !== dirPath)
    : [...checkedDirs.value, dirPath];
};

const moveChecked = async () => {
  const dest = await selectDirectory({ title: "Move selected models into…" });
  if (!dest) return;
  const sep = dest.includes("\\") ? "\\" : "/";
  const operations = checkedDirs.value
    .map((from) => ({
      from,
      to: `${dest}${sep}${from.split(/[\\/]/).pop()}`,
    }))
    .filter((op) => op.from !== op.to);
  if (!operations.length) {
    toastStore.addToast("Those models are already in that folder", "warning");
    return;
  }
  const confirmed = await confirm(
    `Move ${operations.length} model folder${operations.length === 1 ? "" : "s"} into:\n${dest}`,
    { title: "Reorganize models", kind: "warning" },
  );
  if (!confirmed) return;
  const result = await commands.batchMoveModels(operations);
  if (result.status === "ok") {
    const { succeeded, errors } = result.data;
    if (succeeded) {
      toastStore.addToast(
        `Moved ${succeeded} model${succeeded === 1 ? "" : "s"}`,
        "success",
      );
    }
    for (const error of errors) toastStore.addToast(error, "error");
    checkedDirs.value = [];
    // the selected entry's dir_path may have just changed
    selected.value = null;
    files.value = [];
    await Promise.all([runSearch(), refreshMeta()]);
  } else {
    toastStore.reportError("Failed to move models", result.error);
  }
};

watch(selected, (entry) => {
  metaDraft.value = {
    pose: entry?.pose ?? "",
    scale: entry?.scale ?? "",
    support_status: entry?.support_status ?? "",
    release_date: entry?.release_date ?? "",
  };
});

const metaDirty = computed(() => {
  const entry = selected.value;
  if (!entry) return false;
  const draft = metaDraft.value;
  return (
    draft.pose !== (entry.pose ?? "") ||
    draft.scale !== (entry.scale ?? "") ||
    draft.support_status !== (entry.support_status ?? "") ||
    draft.release_date !== (entry.release_date ?? "")
  );
});

const saveMetadata = async () => {
  if (!selected.value) return;
  const orNull = (value: string) => value.trim() || null;
  const draft = metaDraft.value;
  const result = await commands.updateModelMetadata(
    selected.value.dir_path,
    orNull(draft.pose),
    orNull(draft.scale),
    orNull(draft.support_status),
    orNull(draft.release_date),
  );
  if (result.status === "ok") {
    toastStore.addToast("Details saved", "success");
    await refreshSelected();
  } else {
    toastStore.reportError("Failed to save details", result.error);
  }
};

/**
 * Composes two existing, already-tested commands (getCatalogModelFiles +
 * addModel) — this is exactly the path AddSTL/step 2 uses, so a catalog
 * model becomes a real release model with no new backend code.
 */
const addToDraftRelease = async () => {
  if (!selected.value) return;
  if (!releasesStore.releaseExists || !releasesStore.releaseDir) {
    toastStore.addToast(
      "Create a release first, then add models to it from the catalog.",
      "error",
    );
    releasesStore.setReleaseStep(1);
    return;
  }
  const entry = selected.value;
  const fileResult = await commands.getCatalogModelFiles(entry.dir_path);
  if (fileResult.status !== "ok") {
    toastStore.reportError("Failed to read model files", fileResult.error);
    return;
  }
  const result = await commands.addModel(
    {
      id: null,
      name: entry.name,
      description: entry.description,
      tags: entry.tags,
      images: [],
      model_files: [],
      group: null,
    },
    releasesStore.releaseDir,
    fileResult.data.map((f) => f.path),
    entry.preview_path ? [entry.preview_path] : [],
  );
  if (result.status === "ok") {
    releasesStore.addModel(...result.data);
    toastStore.addToast(
      `Added "${entry.name}" to the draft release`,
      "success",
    );
  } else {
    toastStore.reportError("Failed to add model to release", result.error);
  }
};

const chooseRoot = async () => {
  const dir = await selectDirectory({ title: "Choose catalog folder" });
  if (!dir) return;
  catalogRoot.value = dir;
  const current = await commands.getSettings();
  if (current.status === "ok") {
    await commands.setSettings({ ...current.data, catalog_root: dir });
  }
};

const scan = async () => {
  if (!catalogRoot.value) return;
  const result = await startScan(catalogRoot.value);
  if (result.status === "error") {
    toastStore.reportError("Failed to start scan", result.error);
  }
};

watch(scanCompletedCount, async () => {
  toastStore.addToast("Catalog scan complete", "success");
  await Promise.all([runSearch(), refreshMeta()]);
});

watch(dupCompletedCount, async () => {
  const dupResult = await commands.getDuplicateGroups();
  if (dupResult.status === "ok") {
    dupGroups.value = dupResult.data;
    showDups.value = dupGroups.value.length > 0;
    toastStore.addToast(
      dupGroups.value.length
        ? `Found ${dupGroups.value.length} duplicate groups`
        : "No duplicates found",
      dupGroups.value.length ? "warning" : "success",
    );
  }
});

onMounted(async () => {
  const settings = await commands.getSettings();
  if (settings.status === "ok" && settings.data.catalog_root) {
    catalogRoot.value = settings.data.catalog_root;
  }
  await Promise.all([runSearch(), refreshMeta()]);
});
</script>
