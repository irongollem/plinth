<template>
  <main class="bg-gray-800 text-gray-100 flex flex-col h-full rounded-b-lg p-4 gap-3">
    <!-- Toolbar -->
    <div class="flex flex-wrap items-center gap-2">
      <input
        type="search"
        class="input input-sm flex-1 min-w-48"
        placeholder="Search models, tags, descriptions..."
        v-model="query"
      />
      <div class="join">
        <input
          type="text"
          readonly
          class="input input-sm join-item w-56"
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
    </div>

    <div v-if="isScanning" class="text-xs opacity-70 flex items-center gap-2">
      <span class="loading loading-spinner loading-xs"></span>
      <span>
        Indexing... {{ scanProgress?.files_indexed ?? 0 }} files
        <span class="opacity-50">{{ scanProgress?.current_dir }}</span>
      </span>
    </div>
    <div v-if="scanError" class="alert alert-error text-xs py-2">{{ scanError }}</div>

    <!-- Tag filter chips -->
    <div v-if="visibleTags.length" class="flex flex-wrap gap-1 items-center">
      <button
        v-for="tag in visibleTags"
        :key="tag.tag"
        type="button"
        class="badge cursor-pointer"
        :class="selectedTags.includes(tag.tag) ? 'badge-primary' : 'badge-outline'"
        @click="toggleTag(tag.tag)"
      >
        {{ tag.tag }} <span class="opacity-50 ml-1">{{ tag.count }}</span>
      </button>
    </div>

    <!-- Content -->
    <div class="flex flex-1 gap-3 min-h-0">
      <section class="flex-1 overflow-y-auto min-h-0">
        <div v-if="!entries.length && !isScanning" class="h-full flex items-center justify-center opacity-40 text-sm">
          {{ stats?.total_models ? "No models match your search" : "No catalog yet — choose a folder and hit Scan" }}
        </div>
        <div class="grid gap-3" style="grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr))">
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
      <aside
        v-if="selected"
        class="w-96 shrink-0 overflow-y-auto bg-base-100 border border-gray-600 rounded-box p-4 space-y-3"
      >
        <div class="flex items-start justify-between gap-2">
          <div>
            <h2 class="font-bold text-lg leading-tight">{{ selected.name }}</h2>
            <p v-if="selected.designer || selected.release_name" class="text-xs opacity-60">
              {{ [selected.designer, selected.release_name].filter(Boolean).join(" · ") }}
            </p>
          </div>
          <button type="button" class="btn btn-ghost btn-xs" @click="selected = null">✕</button>
        </div>

        <img
          v-if="selected.preview_path"
          :src="convertFileSrc(selected.preview_path)"
          :alt="selected.name"
          class="rounded-box w-full"
        />
        <p v-if="selected.description" class="text-sm opacity-80">{{ selected.description }}</p>

        <div class="flex flex-wrap gap-1">
          <span v-for="tag in selected.tags" :key="tag" class="badge badge-outline gap-1">
            {{ tag }}
            <button type="button" class="opacity-50 hover:opacity-100" @click="removeTag(tag)">✕</button>
          </span>
          <form class="join" @submit.prevent="addTag">
            <input
              v-model="newTag"
              type="text"
              class="input input-xs join-item w-24"
              placeholder="add tag"
            />
            <button type="submit" class="btn btn-xs join-item">+</button>
          </form>
        </div>

        <div class="flex flex-wrap gap-2">
          <button type="button" class="btn btn-primary btn-sm" @click="printModel">
            🖨️ Print
          </button>
          <button
            type="button"
            class="btn btn-sm"
            :disabled="!stlPaths.length"
            @click="show3d = true"
          >
            3D view
          </button>
          <button
            type="button"
            class="btn btn-sm"
            :disabled="!stlPaths.length"
            @click="releasesStore.requestRender(stlPaths)"
          >
            Render promo
          </button>
        </div>

        <div>
          <h3 class="font-semibold text-sm mb-1">
            Files ({{ formatFileSize(selected.total_size_bytes) }})
          </h3>
          <ul class="text-xs space-y-1">
            <li v-for="file in files" :key="file.path" class="flex justify-between gap-2">
              <span class="truncate" :title="file.path">{{ file.file_name }}</span>
              <span class="opacity-50 shrink-0">{{ formatFileSize(file.size_bytes) }}</span>
            </li>
          </ul>
        </div>
      </aside>
    </div>

    <!-- Footer: stats + duplicates -->
    <div class="flex flex-wrap items-center gap-3 text-xs opacity-80 border-t border-gray-700 pt-2">
      <template v-if="stats">
        <span><b>{{ stats.total_models }}</b> models</span>
        <span><b>{{ stats.total_files }}</b> files</span>
        <span><b>{{ formatFileSize(stats.total_size_bytes) }}</b> on disk</span>
        <span v-for="ext in stats.extensions.slice(0, 4)" :key="ext.extension" class="opacity-60">
          .{{ ext.extension }} {{ formatFileSize(ext.total_size_bytes) }}
        </span>
        <span v-if="lastScanLabel" class="opacity-50">scanned {{ lastScanLabel }}</span>
      </template>
      <span class="flex-1"></span>
      <button
        v-if="!isFindingDuplicates"
        type="button"
        class="btn btn-xs"
        :disabled="!stats?.total_files"
        @click="startDuplicateScan"
      >
        Find duplicates
      </button>
      <span v-else class="flex items-center gap-2">
        <span class="loading loading-spinner loading-xs"></span>
        hashing {{ dupProgress?.processed ?? 0 }}/{{ dupProgress?.total ?? "?" }}
        <button type="button" class="btn btn-xs btn-error" @click="cancelDuplicateScan">✕</button>
      </span>
      <button
        v-if="dupGroups.length"
        type="button"
        class="btn btn-xs btn-warning"
        @click="showDups = !showDups"
      >
        {{ dupGroups.length }} duplicate groups ({{ formatFileSize(wastedBytes) }} wasted)
      </button>
    </div>

    <!-- Duplicates panel -->
    <div v-if="showDups && dupGroups.length" class="max-h-48 overflow-y-auto bg-base-100 border border-gray-600 rounded-box p-3 text-xs space-y-2">
      <div v-for="group in dupGroups" :key="group.hash">
        <div class="font-semibold">
          {{ group.paths.length }}× {{ formatFileSize(group.size_bytes) }}
        </div>
        <ul class="opacity-70">
          <li v-for="path in group.paths" :key="path" class="flex justify-between gap-2">
            <span class="truncate" :title="path">{{ path }}</span>
            <button type="button" class="link shrink-0" @click="reveal(path)">reveal</button>
          </li>
        </ul>
      </div>
    </div>

    <!-- 3D preview modal -->
    <ModalView :is-open="show3d" @close="show3d = false">
      <div class="w-[70vw] h-[70vh] bg-gray-900 rounded-box">
        <StlViewport v-if="show3d" :parts="stlPaths" />
      </div>
    </ModalView>
  </main>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
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
