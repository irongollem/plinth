<template>
  <!-- Drag handle to widen the drawer at the list's expense — long file
       paths and the details grid want more room than a fixed width -->
  <div
    v-if="selectedGroup"
    class="w-1.5 shrink-0 cursor-col-resize rounded-full bg-base-content/5 hover:bg-primary/40 transition-colors"
    title="Drag to resize"
    @mousedown.prevent="startDrawerResize"
  ></div>

  <!-- Detail drawer: keyed on the GROUP so switching cards swaps the
       content in place — keying on the loaded entry unmounted the whole
       drawer during the members fetch and the layout flashed -->
  <aside
    v-if="selectedGroup"
    :style="{ width: `${drawerWidth}px` }"
    class="shrink-0 overflow-y-auto"
  >
    <div
      v-if="!selected"
      class="h-40 flex items-center justify-center opacity-40"
    >
      <span class="loading loading-spinner loading-sm"></span>
    </div>
    <template v-else>
      <!-- Picture area: preview image, or the 3D viewport inline when
         toggled (no more full-screen overlay) -->
      <div
        class="relative aspect-4/3 rounded-box bg-base-300 border border-base-content/10 flex items-center justify-center text-base-content/30 overflow-hidden"
      >
        <StlViewport
          v-if="show3d && stlPaths.length"
          :parts="stlPaths"
          compact
        />
        <div
          v-else-if="viewer3dBusy"
          class="absolute inset-0 flex items-center justify-center gap-2 bg-base-300/70 font-mono text-[10.5px]"
        >
          <span class="loading loading-spinner loading-xs"></span>
          unpacking…
        </div>
        <img
          v-else-if="drawerPreview"
          :src="convertFileSrc(drawerPreview)"
          :alt="selected.name"
          class="w-full h-full object-cover cursor-zoom-in"
          title="Click to view large"
          @click="showImageModal = true"
        />
        <span v-else class="text-5xl">🗿</span>
        <button
          v-if="!show3d"
          type="button"
          class="absolute bottom-1.5 right-1.5 btn btn-xs bg-base-100/70"
          @click="pickPreviewImage"
        >
          set image…
        </button>
        <button
          v-if="!show3d && drawerPreview"
          type="button"
          class="absolute bottom-1.5 left-1.5 btn btn-xs bg-base-100/70"
          title="Use this variant's image as the card image in the catalog"
          @click="useAsCardImage"
        >
          ★ card image
        </button>
        <button
          v-if="show3d"
          type="button"
          class="absolute top-1.5 right-1.5 btn btn-xs bg-base-100/70"
          title="Open large viewer"
          @click="show3dModal = true"
        >
          ⤢
        </button>
      </div>

      <!-- Primary actions ride right under the preview they act on.
           On a packed member the needed files are extracted from the
           archive just-in-time (ensure_model_files) — the archive stays
           authoritative and cleanup takes the copies back. -->
      <div class="flex gap-1.5 mt-2">
        <button
          type="button"
          class="flex-1 text-center font-semibold text-[11px] tracking-wider bg-primary text-primary-content rounded-md py-2 cursor-pointer disabled:opacity-40"
          :title="
            selected.packed
              ? 'Files extract from the archive automatically'
              : undefined
          "
          @click="printModel"
        >
          PRINT
        </button>
        <button
          type="button"
          class="flex-1 text-center font-semibold text-[11px] tracking-wider border rounded-md py-2 cursor-pointer disabled:opacity-40"
          :class="
            show3d ? 'border-primary text-primary' : 'border-base-content/15'
          "
          :disabled="!stlPaths.length || viewer3dBusy"
          :title="
            selected.packed
              ? 'Files extract from the archive automatically'
              : undefined
          "
          @click="toggle3d"
        >
          3D
        </button>
        <button
          type="button"
          class="flex-1 text-center font-semibold text-[11px] tracking-wider border border-base-content/15 rounded-md py-2 cursor-pointer disabled:opacity-40"
          :disabled="!stlPaths.length"
          :title="
            selected.packed
              ? 'Files extract from the archive automatically'
              : undefined
          "
          @click="renderSelected"
        >
          RENDER
        </button>
      </div>

      <!-- Compressed-at-rest state: a running job, the packed banner,
           or the offer to pack -->
      <div
        v-if="isPacking"
        class="mt-2 border border-base-content/15 rounded-md px-2 py-1.5 flex items-center gap-2 font-mono text-[10.5px]"
      >
        <span class="loading loading-spinner loading-xs shrink-0"></span>
        <span class="flex-1 truncate">
          {{ packJobLabel }}
          <template v-if="packProgress">
            · {{ packProgress.model_index }}/{{ packProgress.total_models }} ·
            {{ packProgress.phase }} · {{ packProgress.percent }}%
          </template>
        </span>
        <button type="button" class="btn btn-xs btn-ghost" @click="cancelPack">
          cancel
        </button>
      </div>
      <div
        v-else-if="selected.packed || packedDirs.length"
        class="mt-2 border border-primary/30 bg-primary/5 rounded-md px-2 py-1.5 flex items-center gap-2 font-mono text-[10.5px]"
      >
        <span title="Compressed at rest">📦</span>
        <span class="flex-1 truncate">
          {{
            packableDirs.length
              ? `${packedDirs.length} of ${packedDirs.length + packableDirs.length} folders packed`
              : "Packed — files live in a compressed archive"
          }}
        </span>
        <button
          v-if="packableDirs.length"
          type="button"
          class="btn btn-xs btn-ghost"
          title="Compress the remaining folders too"
          @click="packSelectedGroup"
        >
          pack rest
        </button>
        <button type="button" class="btn btn-xs" @click="unpackSelectedGroup">
          Unpack
        </button>
      </div>
      <button
        v-else-if="packableDirs.length"
        type="button"
        class="mt-2 w-full font-mono text-[10.5px] text-base-content/40 hover:text-base-content/70 border border-dashed border-base-content/15 rounded-md py-1 cursor-pointer"
        title="Compress this model's files into pack archives to save disk space — it stays in the catalog and unpacks on demand"
        @click="packSelectedGroup"
      >
        📦 pack — save disk space
      </button>
      <div class="py-3.5 flex flex-col gap-2.5">
        <div>
          <!-- Group title: the logical model; rename applies to the whole
             group and survives rescans -->
          <div class="flex items-start gap-1.5">
            <h2
              v-if="!renamingGroup"
              class="font-bold text-[16px] leading-tight flex-1"
            >
              {{ selectedGroup?.group_name ?? selected.name }}
            </h2>
            <form
              v-else
              class="flex-1 flex gap-1"
              @submit.prevent="renameGroup"
            >
              <input
                v-model="groupNameDraft"
                type="text"
                class="input input-xs font-mono flex-1"
                placeholder="empty = folder name"
              />
              <button type="submit" class="btn btn-xs btn-primary">save</button>
            </form>
            <button
              v-if="!renamingGroup"
              type="button"
              class="text-xs opacity-40 hover:opacity-100 cursor-pointer"
              title="Rename this model (all variants move with it; naming it like another model merges them)"
              @click="startRenameGroup"
            >
              ✎
            </button>
          </div>
          <div
            v-if="!renamingGroup && groupSources.length > 1"
            class="flex flex-wrap gap-x-3 mt-0.5"
          >
            <button
              type="button"
              class="font-mono text-[10px] text-primary/70 hover:text-primary cursor-pointer"
              :title="`Combined from: ${groupSources.join(', ')} — click to split them apart again`"
              @click="splitGroup"
            >
              combined from {{ groupSources.length }} models · split
            </button>
            <button
              v-if="
                selected.source_group.toLowerCase() !==
                selectedGroup?.group_name.toLowerCase()
              "
              type="button"
              class="font-mono text-[10px] text-error/70 hover:text-error cursor-pointer"
              :title="`Pull “${selected.source_group}” back out of this model — the rest stays combined`"
              @click="detachSelectedSource"
            >
              remove “{{ selected.source_group }}”
            </button>
          </div>
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
          <button
            type="button"
            class="block max-w-full font-mono text-[10px] text-base-content/40 truncate mt-0.5 cursor-pointer hover:text-base-content/70"
            :title="`${selected.dir_path} — click to reveal`"
            @click="reveal(selected.dir_path)"
          >
            {{ displayPath }}
          </button>
          <!-- Machine facts from the render pipeline: true printed size -->
          <p
            v-if="measuredLabel"
            class="font-mono text-[10px] text-base-content/40 mt-0.5"
            title="Measured from the geometry when this model was rendered"
          >
            📐 {{ measuredLabel }}
          </p>
        </div>

        <!-- Variant navigation: supported/unsupported tabs, poses within -->
        <div
          v-if="supportTabs.length > 1"
          class="flex bg-base-200 border border-base-content/10 rounded-lg p-0.5"
        >
          <button
            v-for="tab in supportTabs"
            :key="tab"
            type="button"
            class="flex-1 font-semibold text-[11px] px-2 py-1 rounded-md cursor-pointer"
            :class="
              activeSupport === tab
                ? 'bg-primary text-primary-content'
                : 'text-base-content/60'
            "
            @click="setSupportTab(tab)"
          >
            {{ tabLabel(tab) }}
          </button>
        </div>
        <!-- variant tier: shown only when a build has more than one -->
        <div v-if="variantsInTab.length > 1" class="flex flex-wrap gap-1.5">
          <button
            v-for="variant in variantsInTab"
            :key="variant"
            type="button"
            class="font-mono text-[11px] rounded-md px-2.5 py-1 border cursor-pointer"
            :class="
              activeVariant === variant
                ? 'bg-primary text-primary-content border-primary'
                : 'text-base-content/60 border-base-content/15'
            "
            @click="setVariant(variant)"
          >
            {{ variantLabel(variant) }}
          </button>
        </div>
        <!-- pose tier: the members within the active (support, variant) -->
        <div v-if="tabMembers.length > 1" class="flex flex-wrap gap-1.5">
          <button
            v-for="member in tabMembers"
            :key="memberKey(member)"
            type="button"
            class="font-mono text-[11px] rounded-full px-2.5 py-1 border cursor-pointer"
            :class="
              memberKey(member) === memberKey(selected)
                ? 'bg-primary text-primary-content border-primary'
                : 'text-base-content/60 border-base-content/15'
            "
            @click="selectEntry(member)"
          >
            {{ member.pose || member.name }}
          </button>
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
            <label class="flex flex-col gap-0.5 col-span-2">
              <span class="font-mono text-[9px] text-base-content/40"
                >NAME</span
              >
              <input
                v-model="metaDraft.name"
                type="text"
                class="input input-xs font-mono"
                placeholder="model name"
              />
            </label>
            <label class="flex flex-col gap-0.5">
              <span class="font-mono text-[9px] text-base-content/40"
                >DESIGNER</span
              >
              <input
                v-model="metaDraft.designer"
                type="text"
                class="input input-xs font-mono"
                placeholder="studio / brand"
              />
            </label>
            <label class="flex flex-col gap-0.5">
              <span class="font-mono text-[9px] text-base-content/40"
                >SCULPTOR</span
              >
              <input
                v-model="metaDraft.sculptor"
                type="text"
                class="input input-xs font-mono"
                placeholder="artist (if known)"
              />
            </label>
            <label class="flex flex-col gap-0.5">
              <span class="font-mono text-[9px] text-base-content/40"
                >VARIANT</span
              >
              <input
                v-model="metaDraft.variant"
                type="text"
                class="input input-xs font-mono"
                placeholder="e.g. sword, mounted"
              />
            </label>
            <label class="flex flex-col gap-0.5">
              <span class="font-mono text-[9px] text-base-content/40"
                >POSE</span
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
            <label class="flex flex-col gap-0.5">
              <span
                class="font-mono text-[9px] text-base-content/40"
                title="Round/oval base in millimetres — just numbers: 25, or 60x35 for an oval"
                >BASE ROUND MM</span
              >
              <input
                v-model="metaDraft.base_round_mm"
                type="text"
                class="input input-xs font-mono"
                placeholder="25 or 60x35"
              />
            </label>
            <label class="flex flex-col gap-0.5">
              <span
                class="font-mono text-[9px] text-base-content/40"
                title="Square/rectangular base in millimetres — just numbers: 25, or 50x25 for a rectangle"
                >BASE SQUARE MM</span
              >
              <input
                v-model="metaDraft.base_square_mm"
                type="text"
                class="input input-xs font-mono"
                placeholder="25 or 50x25"
              />
            </label>
            <label class="flex flex-col gap-0.5 col-span-2">
              <span class="font-mono text-[9px] text-base-content/40"
                >RELEASE</span
              >
              <input
                v-model="metaDraft.release_name"
                type="text"
                class="input input-xs font-mono"
                placeholder="e.g. Order of the Unicorn"
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

        <button
          v-if="structureClean === true"
          type="button"
          class="font-semibold text-[11px] tracking-[0.03em] text-center rounded-md py-2 text-success flex items-center justify-center gap-1.5 cursor-pointer disabled:opacity-60"
          title="Folders already match the canonical layout — click to re-write this model's metadata file from the catalog (repairs a stale/incomplete model.json)"
          :disabled="refreshingSidecars"
          @click="refreshSidecars([selectedGroup?.group_name ?? ''])"
        >
          {{
            refreshingSidecars
              ? "refreshing metadata…"
              : "✓ folder structure OK"
          }}
        </button>
        <button
          v-else
          type="button"
          class="font-semibold text-[11px] tracking-[0.03em] text-center border border-dashed rounded-md py-2 cursor-pointer disabled:opacity-40 disabled:cursor-default"
          :class="
            structureClean === false
              ? 'border-warning/40 text-warning'
              : 'border-base-content/15 text-base-content/40'
          "
          :disabled="structureClean === null"
          title="Restructure only this model's folders to the canonical layout — you review the moves first"
          @click="openNormalize(selectedGroup?.group_name)"
        >
          {{
            structureClean === null
              ? "checking folder structure…"
              : "⚠ fix folder structure…"
          }}
        </button>

        <button
          type="button"
          class="font-semibold text-[11px] tracking-[0.03em] text-center border border-dashed rounded-md py-2 cursor-pointer"
          :class="
            releasesStore.modelCount
              ? 'border-base-content/25 text-primary'
              : 'border-base-content/15 text-base-content/40'
          "
          @click="addToDraftRelease"
        >
          + Add to release
        </button>

        <button
          v-if="hasAutoSplit"
          type="button"
          class="font-semibold text-[11px] tracking-[0.03em] text-center border border-dashed border-base-content/15 text-base-content/40 hover:border-error/40 hover:text-error rounded-md py-2 cursor-pointer disabled:opacity-40 disabled:cursor-default"
          title="The auto-detected variant/pose split got it wrong — clear every variant and pose tag on this model and dump all files into one box to re-file by hand. Nothing on disk moves."
          :disabled="isFlattening"
          @click="flattenGroup"
        >
          {{ isFlattening ? "resetting…" : "⇋ dump into one box" }}
        </button>

        <div>
          <div
            class="flex items-center gap-2 font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40 mb-1.5"
          >
            <span>FILES · {{ formatFileSize(selected.total_size_bytes) }}</span>
            <span class="flex-1"></span>
            <span class="normal-case tracking-normal font-normal opacity-70">
              tick files to file them under a pose
            </span>
          </div>

          <!-- Assignment bar: always visible on multi-file members.
               Splits a dump folder into pose members without moving
               anything. "match" ticks every file whose name carries
               the typed facets — two spear types x five poses is ten
               type-match-file rounds, not a hundred checkbox clicks. -->
          <div
            v-if="files.length > 1 || checkedFiles.length"
            class="flex items-center gap-1.5 bg-base-200 border border-base-content/10 rounded-lg px-2 py-1.5 mb-1.5"
          >
            <span class="font-mono text-[10px] text-base-content/60 shrink-0">
              {{ checkedFiles.length }} sel
            </span>
            <form
              class="flex items-center gap-1.5 flex-1 min-w-0"
              @submit.prevent="assignChecked"
            >
              <input
                v-model="variantAssignDraft"
                type="text"
                class="input input-xs font-mono flex-1 min-w-0"
                placeholder="variant e.g. sword"
              />
              <input
                v-model="poseAssignDraft"
                type="text"
                class="input input-xs font-mono w-20 shrink-0"
                placeholder="pose"
              />
              <button
                type="button"
                class="btn btn-xs"
                title="Tick every file whose name contains the variant and pose typed above"
                :disabled="
                  !variantAssignDraft.trim() && !poseAssignDraft.trim()
                "
                @click="selectMatchingFiles"
              >
                match
              </button>
              <button
                type="submit"
                class="btn btn-xs btn-primary"
                :disabled="
                  !checkedFiles.length ||
                  (!variantAssignDraft.trim() && !poseAssignDraft.trim())
                "
              >
                file
              </button>
            </form>
            <button
              type="button"
              class="btn btn-xs btn-ghost"
              :disabled="!checkedFiles.length"
              @click="clearChecked"
            >
              unfile
            </button>
          </div>

          <label
            v-for="file in files"
            :key="file.path"
            class="flex items-center gap-2 font-mono text-[11px] text-base-content/60 py-0.5 cursor-pointer"
          >
            <input
              type="checkbox"
              class="checkbox checkbox-xs shrink-0"
              :checked="checkedFiles.includes(file.path)"
              @change="toggleCheckedFile(file.path)"
            />
            <span class="truncate flex-1" :title="file.path">{{
              file.file_name
            }}</span>
            <span
              v-if="fileVariantMap[file.path]"
              class="shrink-0 text-primary"
              title="assigned pose"
            >
              ▸ {{ fileVariantMap[file.path] }}
            </span>
            <span
              v-if="file.packed"
              class="shrink-0"
              title="in the pack archive — extracted on unpack"
            >
              📦
            </span>
            <span class="opacity-60 shrink-0">{{
              formatFileSize(file.size_bytes)
            }}</span>
          </label>
        </div>
      </div>
    </template>
  </aside>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { storeToRefs } from "pinia";
import StlViewport from "../StlViewport.vue";
import { useCatalogStore } from "../../stores/catalogStore";
import { useReleasesStore } from "../../stores/releasesStore";
import { formatFileSize } from "../../utils/format";

const store = useCatalogStore();
const releasesStore = useReleasesStore();
const {
  selectedGroup,
  drawerWidth,
  selected,
  show3d,
  stlPaths,
  viewer3dBusy,
  drawerPreview,
  showImageModal,
  show3dModal,
  isPacking,
  packJobLabel,
  packProgress,
  packedDirs,
  packableDirs,
  renamingGroup,
  groupNameDraft,
  groupSources,
  displayPath,
  measuredLabel,
  supportTabs,
  activeSupport,
  variantsInTab,
  activeVariant,
  tabMembers,
  newTag,
  metaDraft,
  metaDirty,
  structureClean,
  refreshingSidecars,
  hasAutoSplit,
  isFlattening,
  files,
  checkedFiles,
  variantAssignDraft,
  poseAssignDraft,
  fileVariantMap,
} = storeToRefs(store);
const {
  startDrawerResize,
  pickPreviewImage,
  useAsCardImage,
  printModel,
  toggle3d,
  renderSelected,
  cancelPack,
  packSelectedGroup,
  unpackSelectedGroup,
  renameGroup,
  startRenameGroup,
  splitGroup,
  detachSelectedSource,
  reveal,
  setSupportTab,
  tabLabel,
  setVariant,
  variantLabel,
  memberKey,
  selectEntry,
  addTag,
  removeTag,
  saveMetadata,
  refreshSidecars,
  openNormalize,
  addToDraftRelease,
  flattenGroup,
  selectMatchingFiles,
  assignChecked,
  clearChecked,
  toggleCheckedFile,
} = store;
</script>
