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
      <select
        v-model="designerFilter"
        class="select select-sm w-44 font-mono text-[11px]"
        title="Show only this designer's models"
      >
        <option value="">All designers</option>
        <option v-for="d in designers" :key="d.designer" :value="d.designer">
          {{ d.designer }} ({{ d.model_count }})
        </option>
      </select>
      <select
        v-model="groupMode"
        class="select select-sm w-48 font-mono text-[11px]"
        title="How the catalog is ordered"
      >
        <option value="none">Sort: model A–Z</option>
        <option value="designer">Group: designer › release</option>
        <option value="designer-date">Group: designer › newest</option>
      </select>
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
      <button
        type="button"
        class="btn btn-sm"
        :disabled="!catalogRoot || isScanning"
        title="Restructure folders on disk to match the curated catalog — you review every move first"
        @click="openNormalize()"
      >
        Clean up…
      </button>
      <button
        type="button"
        class="btn btn-sm"
        :disabled="!catalogRoot || isScanning || isPacking"
        title="Compress models into pack archives to save disk space — scoped to the designer filter when one is set. Safe to cancel; re-running resumes."
        @click="bulkPack()"
      >
        Pack…
      </button>
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
    <!-- Bulk pack progress lives at page level: the job may span models the
         drawer never opened -->
    <div v-if="isPacking" class="text-xs opacity-70 flex items-center gap-2">
      <span class="loading loading-spinner loading-xs"></span>
      <span>
        {{ packJobLabel }}…
        <template v-if="packProgress">
          {{ packProgress.model_index }}/{{ packProgress.total_models }} ·
          {{ packProgress.phase }} · {{ packProgress.percent }}%
          <span class="opacity-50">{{ packProgress.current_model }}</span>
        </template>
      </span>
      <button type="button" class="btn btn-xs btn-ghost" @click="cancelPack">
        cancel
      </button>
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

    <!-- Batch move action bar (cards and rows are checkable) -->
    <div
      v-if="checkedGroups.length"
      class="flex items-center gap-2 bg-base-200 border border-base-content/10 rounded-lg px-3 py-1.5 text-xs"
    >
      <span class="font-mono text-base-content/60">
        {{ checkedGroups.length }} model{{
          checkedGroups.length === 1 ? "" : "s"
        }}
        selected
      </span>
      <template v-if="!combining">
        <button
          type="button"
          class="btn btn-xs btn-primary"
          @click="moveChecked"
        >
          Move to folder…
        </button>
        <button
          v-if="checkedGroups.length >= 2"
          type="button"
          class="btn btn-xs"
          @click="startCombine"
        >
          Combine into one…
        </button>
        <button
          type="button"
          class="btn btn-xs"
          :disabled="isPacking"
          title="Compress the selected models into pack archives"
          @click="bulkPack(checkedGroups)"
        >
          Pack…
        </button>
        <button
          type="button"
          class="btn btn-xs btn-ghost"
          @click="clearSelection"
        >
          clear
        </button>
      </template>
      <form
        v-else
        class="flex items-center gap-1.5"
        @submit.prevent="combineChecked"
      >
        <input
          v-model="combineName"
          type="text"
          class="input input-xs font-mono w-48"
          placeholder="combined model name"
        />
        <button type="submit" class="btn btn-xs btn-primary">
          combine {{ checkedGroups.length }}
        </button>
        <button
          type="button"
          class="btn btn-xs btn-ghost"
          @click="combining = false"
        >
          cancel
        </button>
      </form>
    </div>

    <!-- Content -->
    <div class="flex flex-1 gap-3 min-h-0">
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
                style="
                  grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr));
                "
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
                show3d
                  ? 'border-primary text-primary'
                  : 'border-base-content/15'
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
                · {{ packProgress.model_index }}/{{
                  packProgress.total_models
                }}
                · {{ packProgress.phase }} · {{ packProgress.percent }}%
              </template>
            </span>
            <button
              type="button"
              class="btn btn-xs btn-ghost"
              @click="cancelPack"
            >
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
            <button
              type="button"
              class="btn btn-xs"
              @click="unpackSelectedGroup"
            >
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
                  <button type="submit" class="btn btn-xs btn-primary">
                    save
                  </button>
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

            <div>
              <div
                class="flex items-center gap-2 font-mono font-semibold text-[9.5px] tracking-[0.12em] text-base-content/40 mb-1.5"
              >
                <span
                  >FILES · {{ formatFileSize(selected.total_size_bytes) }}</span
                >
                <span class="flex-1"></span>
                <span
                  class="normal-case tracking-normal font-normal opacity-70"
                >
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
                <span
                  class="font-mono text-[10px] text-base-content/60 shrink-0"
                >
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
    </div>

    <!-- Footer: stats + duplicates -->
    <div
      class="flex flex-wrap items-center gap-4 font-mono text-[10.5px] text-base-content/40 border-t border-base-content/10 pt-2"
    >
      <template v-if="stats">
        <span
          @click="toggleDups"
          :class="reclaimableGroups.length ? 'text-primary cursor-pointer' : ''"
        >
          <template v-if="reclaimableGroups.length"
            >{{ reclaimableGroups.length }} duplicate groups ·
            {{ formatFileSize(wastedBytes) }} reclaimable</template
          >
          <template v-else-if="dupGroups.length"
            >{{ dupGroups.length }} groups shared · stored once</template
          >
          <template v-else
            >{{ stats.total_models }} models · {{ stats.total_files }} files ·
            {{ formatFileSize(stats.total_size_bytes) }}</template
          >
        </span>
        <span
          v-if="stats.packed_models"
          title="Models compressed at rest: what their files would occupy loose vs what the archives take"
        >
          📦 {{ stats.packed_models }} packed ·
          {{
            formatFileSize(
              (stats.packed_logical_bytes ?? 0) -
                (stats.packed_archive_bytes ?? 0),
            )
          }}
          saved
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

    <!-- Large 3D viewer, opened from the drawer's ⤢ button -->
    <ModalView :is-open="show3dModal" @close="show3dModal = false">
      <div class="w-[70vw] h-[70vh] bg-base-300 rounded-box">
        <StlViewport v-if="show3dModal" :parts="stlPaths" />
      </div>
    </ModalView>

    <!-- Image lightbox, opened by clicking the drawer preview -->
    <ModalView :is-open="showImageModal" @close="showImageModal = false">
      <img
        v-if="drawerPreview"
        :src="convertFileSrc(drawerPreview)"
        alt=""
        class="max-w-[85vw] max-h-[85vh] object-contain rounded-box cursor-zoom-out"
        @click="showImageModal = false"
      />
    </ModalView>

    <!-- Print file picker: tick exactly what goes to the slicer -->
    <ModalView :is-open="showPrintModal" @close="showPrintModal = false">
      <div
        class="w-120 max-w-[85vw] bg-base-100 rounded-box p-4 flex flex-col gap-3"
      >
        <div>
          <div class="font-bold text-[15px]">Print — {{ selected?.name }}</div>
          <p class="text-[11px] text-base-content/50 mt-0.5">
            Ticked files open in your slicer. Pre-sliced scenes carry supports
            and plate layout, so they're picked over raw geometry by default.
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
            and writes each model's metadata beside its files. Nothing moves
            until you approve the list below.
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
            Everything already matches the canonical layout 🎉
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
              Move {{ normalizeChecked.length }} model{{
                normalizeChecked.length === 1 ? "" : "s"
              }}
            </button>
          </template>
        </div>
      </div>
    </ModalView>
  </main>
</template>

<script setup lang="ts">
import { convertFileSrc } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { computed, onActivated, onMounted, ref, watch } from "vue";
import {
  type CatalogEntry,
  type CatalogFile,
  type CatalogGroup,
  type CatalogStats,
  type DesignerCount,
  type DuplicateGroup,
  type NormalizePlan,
  type TagCount,
  commands,
} from "../bindings";
import CatalogCard from "../components/CatalogCard.vue";
import ModalView from "../components/ModalView.vue";
import StlViewport from "../components/StlViewport.vue";
import { useCatalogJobs } from "../composables/useCatalogJobs";
import { useFileSelect } from "../composables/useFileSelect";
import { usePackStatus } from "../composables/usePackStatus";
import { useReleasesStore } from "../stores/releasesStore";
import { useToastStore } from "../stores/toastStore";
import { formatFileSize } from "../utils/format";

const PAGE_SIZE = 60;
const orNull = (value: string) => value.trim() || null;
// Base sizes are canonical dimension strings: "25" for regular bases,
// "60x35" for ovals/rectangles — bare numbers, unit implied. Junk (units,
// words, zeros) parses to null rather than storing garbage. Mirrors the
// Rust boundary's canonical_mm.
const mmOrNull = (value: string) => {
  const parts = value.trim().toLowerCase().replace(/×/g, "x").split("x");
  const nums = parts.map((p: string) => Number.parseInt(p.trim(), 10));
  if (nums.some((n: number) => !Number.isFinite(n) || n <= 0)) return null;
  if (nums.length === 1) return String(nums[0]);
  if (nums.length === 2) return `${nums[0]}x${nums[1]}`;
  return null;
};

const toastStore = useToastStore();
const releasesStore = useReleasesStore();
const { selectDirectory, selectFiles } = useFileSelect();
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
const {
  isPacking,
  packProgress,
  packError,
  packSummary,
  packCancelled,
  lastAction,
  packFinishedCount,
  startPack,
  startUnpack,
  cancelPack,
} = usePackStatus();

// One label for banners/progress: extraction rides the same event stream
// as pack/unpack jobs and must not read as "Packing…" during a print
const packJobLabel = computed(() => {
  if (lastAction.value === "unpack") return "Unpacking";
  if (lastAction.value === "extract") return "Extracting";
  return "Packing";
});

const catalogRoot = ref("");
const query = ref("");
const viewMode = ref<"list" | "grid">("grid");
const selectedTags = ref<string[]>([]);
const allTags = ref<TagCount[]>([]);

/* Ordering/grouping: flat A–Z by model name, or grouped designer › release
   with releases alphabetical or newest-first. The backend sorts (grouping
   must hold across pages); the view only draws headers where the designer
   or release changes between consecutive rows. */
type GroupMode = "none" | "designer" | "designer-date";
const SORT_FOR_MODE: Record<GroupMode, string> = {
  none: "name",
  designer: "designer",
  "designer-date": "designer_date",
};
const storedGroupMode = localStorage.getItem("catalogGroupMode");
const groupMode = ref<GroupMode>(
  storedGroupMode === "designer" || storedGroupMode === "designer-date"
    ? storedGroupMode
    : "none",
);
watch(groupMode, (mode) => localStorage.setItem("catalogGroupMode", mode));
// exact-match facet on top of the fuzzy text search; "" = all designers
const designerFilter = ref("");
const designers = ref<DesignerCount[]>([]);
// the browsable units: one group per logical model
const groups = ref<CatalogGroup[]>([]);
const total = ref(0);
const stats = ref<CatalogStats | null>(null);
// drill-down state: group -> its variant entries -> the active one
const selectedGroup = ref<CatalogGroup | null>(null);
const members = ref<CatalogEntry[]>([]);
const activeSupport = ref("");
// second navigation tier: within a support build, which variant is shown
const activeVariant = ref("");
const selected = ref<CatalogEntry | null>(null);
const files = ref<CatalogFile[]>([]);
const newTag = ref("");
const dupGroups = ref<DuplicateGroup[]>([]);
const showDups = ref(false);
const show3d = ref(false);
// per-group hash -> path the user wants to keep (defaults to the first)
const keepChoice = ref<Record<string, string>>({});
const reclaimBusy = ref(false);
// group names ticked for a batch move or combine
const checkedGroups = ref<string[]>([]);
const combining = ref(false);
const combineName = ref("");
const renamingGroup = ref(false);
const groupNameDraft = ref("");
const show3dModal = ref(false);
const showImageModal = ref(false);
const metaDraft = ref({
  name: "",
  variant: "",
  pose: "",
  scale: "",
  support_status: "",
  release_date: "",
  designer: "",
  sculptor: "",
  release_name: "",
  base_round_mm: "",
  base_square_mm: "",
});

// A synthesized member's variant_key is `dir\u{1f}variant\u{1f}pose`; keep the
// format in one place. Empty variant AND pose is the residual/unassigned pool.
const KEY_SEP = "\u{1f}";
const variantKeyFor = (dir: string, variant: string, pose: string) =>
  `${dir}${KEY_SEP}${variant}${KEY_SEP}${pose}`;

/* Resizable detail drawer — width persists so it survives navigation. */
const DRAWER_MIN = 300;
const DRAWER_MAX = 720;
const drawerWidth = ref(
  Math.min(
    DRAWER_MAX,
    Math.max(
      DRAWER_MIN,
      Number(localStorage.getItem("catalogDrawerWidth")) || 340,
    ),
  ),
);
const startDrawerResize = (event: MouseEvent) => {
  const startX = event.clientX;
  const startWidth = drawerWidth.value;
  const onMove = (moveEvent: MouseEvent) => {
    // the drawer sits on the right, so dragging left widens it
    const delta = startX - moveEvent.clientX;
    drawerWidth.value = Math.min(
      DRAWER_MAX,
      Math.max(DRAWER_MIN, startWidth + delta),
    );
  };
  const onUp = () => {
    window.removeEventListener("mousemove", onMove);
    window.removeEventListener("mouseup", onUp);
    localStorage.setItem("catalogDrawerWidth", String(drawerWidth.value));
  };
  window.addEventListener("mousemove", onMove);
  window.addEventListener("mouseup", onUp);
};

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

// Merged (hardlinked) copies cost the disk nothing, so reclaimable space
// counts distinct physical copies — a fully shared group contributes 0.
// Packed copies aren't loose bytes either (they occupy compressed archive
// space and can't be merged/deleted), so they don't count as reclaimable.
const reclaimableBytes = (g: DuplicateGroup) => {
  const looseCopies = g.distinct_copies - (g.packed_paths?.length ?? 0);
  return g.size_bytes * Math.max(0, looseCopies - 1);
};

const wastedBytes = computed(() =>
  dupGroups.value.reduce((sum, g) => sum + reclaimableBytes(g), 0),
);

// Groups whose names still occupy more than one copy; fully shared groups
// stay visible in the panel (as "shared") but out of the headline count
const reclaimableGroups = computed(() =>
  dupGroups.value.filter((g) => g.distinct_copies > 1),
);

const lastScanLabel = computed(() => {
  if (!stats.value?.last_scan_epoch) return null;
  return new Date(stats.value.last_scan_epoch * 1000).toLocaleString();
});

const runSearch = async (append = false) => {
  const offset = append ? groups.value.length : 0;
  const result = await commands.searchCatalogGroups(
    query.value,
    selectedTags.value,
    designerFilter.value || null,
    SORT_FOR_MODE[groupMode.value],
    PAGE_SIZE,
    offset,
  );
  if (result.status === "ok") {
    groups.value = append
      ? [...groups.value, ...result.data.groups]
      : result.data.groups;
    total.value = result.data.total;
    // keep the drawer header's aggregates fresh (poses/sizes may change)
    if (selectedGroup.value) {
      const current = selectedGroup.value.group_name.toLowerCase();
      const fresh = groups.value.find(
        (g) => g.group_name.toLowerCase() === current,
      );
      if (fresh) selectedGroup.value = fresh;
    }
  } else {
    toastStore.reportError("Search failed", result.error);
  }
};

const loadMore = () => runSearch(true);

let searchTimeout: number | null = null;
watch([query, selectedTags, designerFilter, groupMode], () => {
  if (searchTimeout) clearTimeout(searchTimeout);
  searchTimeout = setTimeout(() => runSearch(), 250) as unknown as number;
});

/* ---- designer › release sections, derived from the backend's order ---- */
type ReleaseBucket = {
  key: string;
  label: string | null; // null = no release header (flat mode)
  date: string | null;
  groups: CatalogGroup[];
};
type DesignerSection = {
  key: string;
  designer: string | null; // null = no designer header (flat mode)
  releases: ReleaseBucket[];
};

// One pass over the loaded page(s): a new header opens whenever the
// designer or release changes between consecutive rows — safe because the
// backend sorts by exactly (designer, release, name). Flat mode is the
// same structure with a single headerless section, so the template never
// branches on the mode.
const sections = computed<DesignerSection[]>(() => {
  if (groupMode.value === "none") {
    return [
      {
        key: "all",
        designer: null,
        releases: [
          { key: "all", label: null, date: null, groups: groups.value },
        ],
      },
    ];
  }
  const out: DesignerSection[] = [];
  for (const group of groups.value) {
    const designer = group.designer?.trim() || "Unknown designer";
    let section = out[out.length - 1];
    // compare case-insensitively, matching the backend's NOCASE ordering
    if (section?.designer?.toLowerCase() !== designer.toLowerCase()) {
      section = { key: designer.toLowerCase(), designer, releases: [] };
      out.push(section);
    }
    const label = group.release_name?.trim() || "No release";
    let bucket = section.releases[section.releases.length - 1];
    if (bucket?.label?.toLowerCase() !== label.toLowerCase()) {
      bucket = {
        key: `${section.key}\u{1f}${label.toLowerCase()}`,
        label,
        date: group.release_date,
        groups: [],
      };
      section.releases.push(bucket);
    }
    bucket.groups.push(group);
  }
  return out;
});

const sectionModelCount = (section: DesignerSection) =>
  section.releases.reduce((count, bucket) => count + bucket.groups.length, 0);

const refreshMeta = async () => {
  const [tagsResult, statsResult, dupResult, designerResult] =
    await Promise.all([
      commands.getCatalogTags(),
      commands.getCatalogStats(),
      commands.getDuplicateGroups(),
      commands.getCatalogDesigners(),
    ]);
  if (tagsResult.status === "ok") allTags.value = tagsResult.data;
  if (statsResult.status === "ok") stats.value = statsResult.data;
  if (dupResult.status === "ok") dupGroups.value = dupResult.data;
  if (designerResult.status === "ok") designers.value = designerResult.data;
};

const toggleTag = (tag: string) => {
  selectedTags.value = selectedTags.value.includes(tag)
    ? selectedTags.value.filter((t) => t !== tag)
    : [...selectedTags.value, tag];
};

const toggleDups = () => {
  if (reclaimableGroups.value.length) showDups.value = !showDups.value;
};

// A folder split into poses yields several members sharing one dir_path;
// their variant_key disambiguates. Fall back to dir_path for whole-folder
// members (variant_key null).
const memberKey = (entry: CatalogEntry) => entry.variant_key ?? entry.dir_path;

const basename = (path: string) => path.split(/[\\/]/).pop() ?? path;

// 3D preview is opt-in PER MEMBER. Leaving it latched meant every pose or
// model click immediately parsed multi-million-triangle STLs on the main
// thread — browsing became a chain of UI freezes with no way to turn the
// viewer off mid-load. Selection changes drop back to the image; the user
// re-opens 3D deliberately. (Moving the parse into a Worker is tracked in
// the todolist; this removes the accidental triggers.)
watch(selected, (next, prev) => {
  const nextKey = next ? memberKey(next) : null;
  const prevKey = prev ? memberKey(prev) : null;
  if (nextKey !== prevKey) close3d();
});

/* On a packed member the viewer's STLs are extracted first (the viewport
   readFile()s real paths); closing the viewer takes the copies back. */
const viewer3dBusy = ref(false);
const viewerExtracted = ref<string[]>([]);

const toggle3d = async () => {
  if (show3d.value) {
    close3d();
    return;
  }
  if (selected.value?.packed) {
    // extraction takes real time on a big model — if the user has moved to
    // another member meanwhile, opening the viewer would show the NEW
    // selection with the OLD selection's files attributed to it
    const key = memberKey(selected.value);
    viewer3dBusy.value = true;
    const extracted = await ensureLoose(stlPaths.value);
    viewer3dBusy.value = false;
    if (extracted === null) return;
    if (!selected.value || memberKey(selected.value) !== key) {
      if (extracted.length) cleanupEphemeralSafe(extracted);
      return;
    }
    viewerExtracted.value = extracted;
  }
  show3d.value = true;
};

const close3d = () => {
  show3d.value = false;
  show3dModal.value = false;
  if (viewerExtracted.value.length && packCleanupAfter.value) {
    // the viewport holds the geometry in memory; the files can go
    cleanupEphemeralSafe(viewerExtracted.value);
  }
  viewerExtracted.value = [];
};

const selectEntry = async (entry: CatalogEntry) => {
  selected.value = entry;
  files.value = [];
  // A synthesized pose member carries a variant_key; pass it so we list
  // only that pose's files. Whole-folder members send null (all files).
  const [fileResult, variantResult] = await Promise.all([
    commands.getCatalogModelFiles(entry.dir_path, entry.variant_key),
    commands.getFileVariants(entry.dir_path),
  ]);
  if (fileResult.status === "ok") files.value = fileResult.data;
  if (variantResult.status === "ok") {
    const map: Record<string, string> = {};
    for (const v of variantResult.data) {
      const label = [v.variant, v.pose].filter(Boolean).join(" · ");
      if (label) map[v.path] = label;
    }
    fileVariantMap.value = map;
  }
};

/* ---- assign files in a dump folder to variant/pose buckets ---- */
// checked file paths in the drawer's file list, and the facets to file them under
const checkedFiles = ref<string[]>([]);
const variantAssignDraft = ref("");
const poseAssignDraft = ref("");
// path -> "variant · pose" label, so already-sorted files show a badge
const fileVariantMap = ref<Record<string, string>>({});

const toggleCheckedFile = (path: string) => {
  checkedFiles.value = checkedFiles.value.includes(path)
    ? checkedFiles.value.filter((p) => p !== path)
    : [...checkedFiles.value, path];
};

/* "match": tick every file whose name carries the typed facets, so bulk
   filing is type -> match -> file instead of a hundred checkbox clicks.
   Underscores and spaces count as the same separator; the pose token
   demands word boundaries so pose "a" doesn't match every file with an
   'a' in it. */
const normalizeForMatch = (value: string) =>
  value.toLowerCase().replace(/[_\s]+/g, " ");
const escapeRegExp = (value: string) =>
  value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

const selectMatchingFiles = () => {
  const variant = normalizeForMatch(variantAssignDraft.value.trim());
  const pose = normalizeForMatch(poseAssignDraft.value.trim());
  if (!variant && !pose) return;
  const poseRe = pose
    ? new RegExp(`(^|[^a-z0-9])${escapeRegExp(pose)}([^a-z0-9]|$)`)
    : null;
  const matches = files.value
    .filter((file) => {
      const name = normalizeForMatch(file.file_name);
      return (
        (!variant || name.includes(variant)) && (!poseRe || poseRe.test(name))
      );
    })
    .map((file) => file.path);
  checkedFiles.value = matches;
  if (!matches.length) {
    toastStore.addToast("No file names match those facets", "info");
  }
};

/** Reload the open group's members and select a sensible one — used after a
 *  split changes the member set. Prefers `preferKey` when it still exists. */
const reloadMembers = async (preferKey?: string) => {
  const group = selectedGroup.value;
  if (!group) return;
  const result = await commands.getCatalogGroupMembers(group.group_name);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to reload variants", result.error);
    return;
  }
  members.value = result.data;
  const firstTab = supportTabs.value[0] ?? "";
  const next =
    (preferKey
      ? members.value.find((m) => memberKey(m) === preferKey)
      : undefined) ??
    members.value.find((m) => (m.support_status ?? "") === firstTab) ??
    members.value[0];
  // move the support + variant tiers to wherever we landed, so it's visible
  activeSupport.value = next?.support_status ?? firstTab;
  activeVariant.value = resolveVariant(next?.variant ?? "");
  if (next) await selectEntry(next);
};

const assignChecked = async () => {
  const dir = selected.value?.dir_path;
  const variant = variantAssignDraft.value.trim();
  const pose = poseAssignDraft.value.trim();
  // need at least one facet to file under, and files to file
  if (!dir || (!variant && !pose) || !checkedFiles.value.length) return;
  const count = checkedFiles.value.length;
  const result = await commands.assignFilesToPose(
    checkedFiles.value,
    variant || null,
    pose || null,
    null,
  );
  if (result.status !== "ok") {
    toastStore.reportError("Failed to assign files", result.error);
    return;
  }
  const label = [variant, pose].filter(Boolean).join(" · ");
  toastStore.addToast(
    `Filed ${count} file${count === 1 ? "" : "s"} under “${label}”`,
    "success",
  );
  checkedFiles.value = [];
  // the variant sticks for the next round — filing five poses of one
  // spear type means retyping only the pose letter
  poseAssignDraft.value = "";
  // Stay on the unassigned pool so the remaining files are still in front of
  // you to keep filing. When the last file is filed the pool is gone and
  // reloadMembers falls back to a real member.
  await Promise.all([runSearch(), reloadMembers(variantKeyFor(dir, "", ""))]);
};

const clearChecked = async () => {
  if (!checkedFiles.value.length) return;
  const result = await commands.clearFilePose(checkedFiles.value);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to clear assignment", result.error);
    return;
  }
  // 0 = the selection was never filed to a pose — a success toast here
  // would claim an effect that didn't happen (files that LIVE in another
  // folder can't be unfiled out of this model; that's split or move)
  if (result.data === 0) {
    toastStore.addToast(
      "Nothing to unfile — these files aren't assigned to a pose",
      "info",
    );
    return;
  }
  toastStore.addToast(
    `Unfiled ${result.data} file assignment${result.data === 1 ? "" : "s"}`,
    "success",
  );
  checkedFiles.value = [];
  await Promise.all([runSearch(), reloadMembers()]);
};

// Support statuses present among the members, stable order; "" = untagged
const supportTabs = computed(() => {
  const seen = new Set(members.value.map((m) => m.support_status ?? ""));
  const ordered = ["supported", "unsupported"].filter((s) => seen.has(s));
  for (const status of seen) {
    if (!ordered.includes(status)) ordered.push(status);
  }
  return ordered;
});

const tabLabel = (tab: string) => (tab === "" ? "other" : tab);

// members in the active support build (used to derive the variant tier)
const supportMembers = computed(() =>
  members.value.filter((m) => (m.support_status ?? "") === activeSupport.value),
);

// distinct variants within the active support build, in the backend's
// bucket order; "" = no variant. Only shown when there's more than one.
// Case-insensitive on purpose: new writes are Title Cased by convention,
// but legacy members may still carry "sword" beside "Sword" until their
// metadata is re-saved — one chip, not two.
const variantsInTab = computed(() => {
  const seen: string[] = [];
  for (const member of supportMembers.value) {
    const variant = member.variant ?? "";
    if (!seen.some((v) => v.toLowerCase() === variant.toLowerCase())) {
      seen.push(variant);
    }
  }
  return seen;
});
const variantLabel = (variant: string) => variant || "base";

// the pose members within the active (support, variant) bucket
const tabMembers = computed(() =>
  supportMembers.value.filter(
    (m) =>
      (m.variant ?? "").toLowerCase() === activeVariant.value.toLowerCase(),
  ),
);

// pick a variant present in the active support build, preferring `prefer`
// (case-insensitively — the chip's spelling wins over the member's)
const resolveVariant = (prefer: string) =>
  variantsInTab.value.find((v) => v.toLowerCase() === prefer.toLowerCase()) ??
  variantsInTab.value[0] ??
  "";

const setSupportTab = (tab: string) => {
  // keep the pose/variant when hopping between builds — you're looking at the
  // same mini, just the other build of it
  const currentPose = selected.value?.pose ?? null;
  const currentVariant = selected.value?.variant ?? "";
  activeSupport.value = tab;
  activeVariant.value = resolveVariant(currentVariant);
  const next =
    (currentPose
      ? tabMembers.value.find((m) => m.pose === currentPose)
      : undefined) ?? tabMembers.value[0];
  if (next) selectEntry(next);
};

const setVariant = (variant: string) => {
  const currentPose = selected.value?.pose ?? null;
  activeVariant.value = variant;
  const next =
    (currentPose
      ? tabMembers.value.find((m) => m.pose === currentPose)
      : undefined) ?? tabMembers.value[0];
  if (next) selectEntry(next);
};

// The scanner-level groups behind the selected card; more than one means
// it was combined and offers "split" in the drawer
const groupSources = ref<string[]>([]);

const selectGroup = async (group: CatalogGroup) => {
  selectedGroup.value = group;
  renamingGroup.value = false;
  members.value = [];
  selected.value = null;
  files.value = [];
  groupSources.value = [];
  commands.getCatalogGroupSources(group.group_name).then((sources) => {
    if (sources.status === "ok") groupSources.value = sources.data;
  });
  checkStructure(group.group_name);
  const result = await commands.getCatalogGroupMembers(group.group_name);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to load model variants", result.error);
    return;
  }
  members.value = result.data;
  const firstTab = supportTabs.value[0] ?? "";
  activeSupport.value = firstTab;
  const first =
    members.value.find((m) => (m.support_status ?? "") === firstTab) ??
    members.value[0];
  // through resolveVariant so the active chip carries the CHIP's spelling
  activeVariant.value = resolveVariant(first?.variant ?? "");
  if (first) await selectEntry(first);
};

const groupSummary = (group: CatalogGroup) => {
  const parts: string[] = [];
  if (group.pose_count > 1) parts.push(`${group.pose_count} poses`);
  if (group.support_statuses.length)
    parts.push(group.support_statuses.join(" / "));
  return parts.join(" · ");
};

const startRenameGroup = () => {
  groupNameDraft.value = selectedGroup.value?.group_name ?? "";
  renamingGroup.value = true;
};

// The selected variant's image becomes the group card's face — stored as
// WHICH member, so a re-render of that member updates the card too
const useAsCardImage = async () => {
  const group = selectedGroup.value;
  const entry = selected.value;
  if (!group || !entry) return;
  const result = await commands.setGroupCover(
    group.group_name,
    entry.dir_path,
    entry.variant_key ?? null,
  );
  if (result.status !== "ok") {
    toastStore.reportError("Failed to set card image", result.error);
    return;
  }
  toastStore.addToast("Card image updated", "success");
  await runSearch();
};

// Surgical combine-undo: pull ONE mis-combined model back out of this card
// (one checkbox too many happens); the rest of the combination stays
const detachSelectedSource = async () => {
  const group = selectedGroup.value;
  const source = selected.value?.source_group;
  if (!group || !source) return;
  const confirmed = await confirm(
    `Remove "${source}" from "${group.group_name}"?\n\nIt comes back as its own model; nothing on disk moves.`,
    { title: "Remove from model", kind: "warning" },
  );
  if (!confirmed) return;
  const result = await commands.detachCatalogGroupSource(
    group.group_name,
    source,
  );
  if (result.status !== "ok") {
    toastStore.reportError("Failed to remove from model", result.error);
    return;
  }
  toastStore.addToast(`"${source}" is its own model again`, "success");
  await Promise.all([runSearch(), refreshMeta()]);
  // the card still exists (other sources remain) — reload it in place
  await selectGroup(group);
};

// Undo for combine (and for a rename collision that merged two models):
// clearing the name overrides brings every source group back as its own
// card, named after its folder again. Nothing on disk moves.
const splitGroup = async () => {
  const group = selectedGroup.value;
  if (!group || groupSources.value.length < 2) return;
  const confirmed = await confirm(
    `Split "${group.group_name}" back into ${groupSources.value.length} separate models?\n\n${groupSources.value.join("\n")}`,
    { title: "Split model", kind: "warning" },
  );
  if (!confirmed) return;
  const result = await commands.renameCatalogGroup(group.group_name, "");
  if (result.status !== "ok") {
    toastStore.reportError("Failed to split model", result.error);
    return;
  }
  toastStore.addToast(
    `Split into ${groupSources.value.length} models`,
    "success",
  );
  selectedGroup.value = null;
  selected.value = null;
  members.value = [];
  groupSources.value = [];
  await Promise.all([runSearch(), refreshMeta()]);
};

const renameGroup = async () => {
  const group = selectedGroup.value;
  renamingGroup.value = false;
  if (!group) return;
  const newName = groupNameDraft.value.trim();
  if (newName === group.group_name) return;
  const result = await commands.renameCatalogGroup(group.group_name, newName);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to rename model", result.error);
    return;
  }
  toastStore.addToast(
    newName ? `Renamed to "${newName}"` : "Name reset to the folder name",
    "success",
  );
  await Promise.all([runSearch(), refreshMeta()]);
  const found = newName
    ? groups.value.find(
        (g) => g.group_name.toLowerCase() === newName.toLowerCase(),
      )
    : undefined;
  if (found) {
    await selectGroup(found);
  } else {
    selectedGroup.value = null;
    selected.value = null;
    members.value = [];
  }
};

// Tags apply to the whole group: a tag describes the mini, so tagging the
// supported and unsupported builds separately was busywork that drifted
const addTag = async () => {
  const group = selectedGroup.value;
  if (!group || !selected.value || !newTag.value.trim()) return;
  const result = await commands.addGroupTag(group.group_name, newTag.value);
  if (result.status === "ok") {
    newTag.value = "";
    await refreshSelected();
    await refreshMeta();
  } else {
    toastStore.reportError("Failed to add tag", result.error);
  }
};

const removeTag = async (tag: string) => {
  const group = selectedGroup.value;
  if (!group || !selected.value) return;
  const result = await commands.removeGroupTag(group.group_name, tag);
  if (result.status === "ok") {
    await refreshSelected();
    await refreshMeta();
  } else {
    toastStore.reportError("Failed to remove tag", result.error);
  }
};

/** Re-fetch the group's members so tag/detail edits show up immediately. */
const refreshSelected = async () => {
  const group = selectedGroup.value;
  const key = selected.value ? memberKey(selected.value) : undefined;
  await runSearch();
  if (!group) return;
  const result = await commands.getCatalogGroupMembers(group.group_name);
  if (result.status !== "ok") return;
  members.value = result.data;
  const updated = key
    ? members.value.find((m) => memberKey(m) === key)
    : undefined;
  if (updated) selected.value = updated;
};

/* ---- transparent use: materialize packed bytes just-in-time ---- */
// Resolves when every path is readable on disk. Returns the ephemeral
// extracts (what cleanup may take back afterwards), or null on failure.
// Progress/cancel ride the same PackStatus stream as pack jobs.
const ensureLoose = async (paths: string[]): Promise<string[] | null> => {
  if (!paths.length) return [];
  const result = await commands.ensureModelFiles(paths);
  if (result.status !== "ok") {
    toastStore.reportError(
      "Failed to extract from the pack archive",
      result.error,
    );
    return null;
  }
  return result.data.extracted;
};

// The user's cleanup-after preference, mirrored from settings; the print
// modal's checkbox writes it back. Default true — "print straight from the
// bundle" is the point of packing.
const packCleanupAfter = ref(true);
onMounted(async () => {
  const result = await commands.getSettings();
  if (result.status === "ok") {
    packCleanupAfter.value = result.data.pack_cleanup_after ?? true;
  }
});
const persistCleanupAfter = async () => {
  const result = await commands.getSettings();
  if (result.status !== "ok") return;
  await commands.setSettings({
    ...result.data,
    pack_cleanup_after: packCleanupAfter.value,
  });
};

// Paths a handed-off render still depends on: Blender reads them from disk
// in the Render tab for minutes, so print/3D cleanups of the same member
// must not touch them. Held until app exit (the exit sweep owns them) —
// there is no render-finished signal in this tab.
const renderHeldPaths = ref<Set<string>>(new Set());

// Every cleanup in this view goes through here so render-held paths are
// exempt in ONE place instead of at each call site
const cleanupEphemeralSafe = async (paths: string[]) => {
  const safe = paths.filter((p) => !renderHeldPaths.value.has(p));
  if (!safe.length) return;
  const result = await commands.cleanupEphemeralFiles(safe);
  if (result.status === "ok") {
    for (const kept of result.data.errors) toastStore.addToast(kept, "info");
  }
};

// Deleting the copies right after openWithDefaultApp returns would race the
// slicer's own read. The delay covers a normal open; a file the slicer still
// holds (Windows) refuses deletion and stays registered for the exit sweep,
// and the size+mtime guard keeps anything the slicer saved over.
const SLICER_CLEANUP_DELAY_MS = 15_000;
const scheduleSlicerCleanup = (paths: string[]) => {
  setTimeout(() => {
    cleanupEphemeralSafe(paths);
  }, SLICER_CLEANUP_DELAY_MS);
};

/* ---- bulk pack: compress everything under the current scope ---- */
// Scope order: explicit card selection > the designer facet > the whole
// catalog. The backend job is sequential and per-folder atomic, so a
// cancelled designer-wide run resumes by clicking Pack… again.
const bulkPack = async (groupNames: string[] = []) => {
  if (isPacking.value) return;
  const candidates = await commands.getPackCandidates(
    groupNames.length ? null : designerFilter.value || null,
    groupNames,
  );
  if (candidates.status !== "ok") {
    toastStore.reportError("Failed to list pack candidates", candidates.error);
    return;
  }
  const dirs = candidates.data;
  if (!dirs.length) {
    toastStore.addToast(
      "Nothing to pack — everything in scope is already packed",
      "info",
    );
    return;
  }
  const scope = groupNames.length
    ? `${groupNames.length} selected model${groupNames.length === 1 ? "" : "s"}`
    : designerFilter.value
      ? `every ${designerFilter.value} model`
      : "the whole catalog";
  const confirmed = await confirm(
    `Compress ${dirs.length} folder${dirs.length === 1 ? "" : "s"} — ${scope} — into pack archives?\n\n` +
      "Runs one folder at a time and is safe to cancel: finished folders stay packed, and re-running Pack… resumes where it left off.",
    { title: "Pack models", kind: "info" },
  );
  if (!confirmed) return;
  const result = await startPack(dirs);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to start packing", result.error);
  }
};

/* ---- compressed at rest: pack/unpack the open model ---- */
// Packing is per member FOLDER (nested variant folders pack themselves), so
// the group-level action fans out to every member dir in the relevant state.
const packableDirs = computed(() => [
  ...new Set(members.value.filter((m) => !m.packed).map((m) => m.dir_path)),
]);
const packedDirs = computed(() => [
  ...new Set(members.value.filter((m) => m.packed).map((m) => m.dir_path)),
]);

const packSelectedGroup = async () => {
  const group = selectedGroup.value;
  if (!group || !packableDirs.value.length || isPacking.value) return;
  const confirmed = await confirm(
    `Compress “${group.group_name}” (${formatFileSize(group.total_size_bytes)}) into pack archives?\n\n` +
      "The model stays in the catalog and unpacks on demand; printing, 3D preview and rendering need an unpack first.",
    { title: "Pack model", kind: "info" },
  );
  if (!confirmed) return;
  const result = await startPack(packableDirs.value);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to start packing", result.error);
  }
};

const unpackSelectedGroup = async () => {
  if (!packedDirs.value.length || isPacking.value) return;
  const result = await startUnpack(packedDirs.value);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to start unpacking", result.error);
  }
};

// Any terminal state changed disk state for the folders that DID finish —
// refresh files, members and stats regardless of how the job ended
watch(packFinishedCount, async () => {
  // extractions ride the same event stream but change nothing the index
  // knows about, and their outcome is handled where ensureLoose was awaited
  // — refreshing here would blank the drawer mid-print
  if (lastAction.value === "extract") return;
  if (packSummary.value) {
    const { action, succeeded, kept_files } = packSummary.value;
    toastStore.addToast(
      `${action === "unpack" ? "Unpacked" : "Packed"} ${succeeded} folder${succeeded === 1 ? "" : "s"}`,
      "success",
    );
    // files that changed between compression and delete stay loose — say so
    for (const kept of kept_files) {
      toastStore.addToast(
        `Kept on disk (changed while packing): ${kept}`,
        "info",
      );
    }
  } else if (packError.value) {
    toastStore.reportError("Pack job failed", packError.value);
  } else if (packCancelled.value) {
    toastStore.addToast(
      `Cancelled — ${packCancelled.value.succeeded} folder${packCancelled.value.succeeded === 1 ? "" : "s"} already done (re-run to resume)`,
      "info",
    );
  }
  await refreshSelected();
  await refreshMeta();
  if (selected.value) await selectEntry(selected.value);
});

// Carries the model's dir_path AND variant_key so the finished render comes
// back as THIS pose's preview, not the whole folder's (poses in one dump
// folder share a dir_path — only the variant_key tells them apart)
const renderSelected = async () => {
  if (!selected.value) return;
  if (selected.value.packed) {
    // Blender reads the STLs from disk in the Render tab, so they must
    // exist before the handoff. No active cleanup here: the render needs
    // them until it finishes elsewhere — the exit sweep takes them back,
    // and marking them held stops a 3D-close or print cleanup of the same
    // member from deleting them mid-render.
    const extracted = await ensureLoose(stlPaths.value);
    if (extracted === null) return;
    for (const path of extracted) renderHeldPaths.value.add(path);
  }
  releasesStore.requestRender(
    stlPaths.value,
    selected.value.dir_path,
    undefined,
    selected.value.variant_key,
  );
};

/* ---- print: pick exactly which files go to the slicer ---- */
// Print-ready scene files beat raw geometry: a .lys/.chitu already carries
// supports and plate layout, so when a member has both, those are what
// the modal pre-checks.
const SLICED_EXTS = ["lys", "chitu", "chitubox"];
const RAW_EXTS = ["stl", "obj", "3mf"];

// What the modal offers: everything a slicer could eat. Images, licences
// and archives stay out — offering them would only invite mis-ticks.
const printCandidates = computed(() =>
  files.value.filter((f) =>
    [...SLICED_EXTS, ...RAW_EXTS].includes(f.extension),
  ),
);

const printablePaths = computed(() => {
  const sliced = printCandidates.value.filter((f) =>
    SLICED_EXTS.includes(f.extension),
  );
  const pool = sliced.length ? sliced : printCandidates.value;
  return pool.map((f) => f.path);
});

const showPrintModal = ref(false);
const printSelection = ref<string[]>([]);
const printBusy = ref(false);

// any ticked file still inside the archive → the modal offers cleanup-after
const printSelectionPacked = computed(() =>
  files.value.some((f) => f.packed && printSelection.value.includes(f.path)),
);

const togglePrintFile = (path: string) => {
  printSelection.value = printSelection.value.includes(path)
    ? printSelection.value.filter((p) => p !== path)
    : [...printSelection.value, path];
};

const printModel = async () => {
  if (!selected.value) return;
  const settingsResult = await commands.getSettings();
  const action =
    (settingsResult.status === "ok" && settingsResult.data.print_action) ||
    "open-in-slicer";
  // Reveal-folder users keep the direct flow: reveal takes no file list,
  // so a picker would be a pointless extra click for them. Same fallback
  // when there's nothing a slicer could open. A packed file's path has no
  // bytes on disk to reveal — fall through to a loose file or the folder.
  if (action === "reveal-folder" || !printCandidates.value.length) {
    await reveal(
      files.value.find((f) => !f.packed)?.path ?? selected.value.dir_path,
    );
    return;
  }
  printSelection.value = printablePaths.value;
  showPrintModal.value = true;
};

const sendToSlicer = async () => {
  if (!printSelection.value.length) return;
  // Snapshot: extraction takes real time, and the modal's checkboxes stay
  // live — what opens must be exactly what was extracted
  const selection = [...printSelection.value];
  printBusy.value = true;
  try {
    // Packed files materialize just-in-time — only the ticked entries are
    // pulled from the archive, "print straight from the bundle"
    const extracted = await ensureLoose(selection);
    if (extracted === null) return;
    if (!showPrintModal.value) {
      // the user cancelled the modal mid-extraction — don't surprise them
      // with a slicer window; just take the copies back
      if (extracted.length) cleanupEphemeralSafe(extracted);
      return;
    }
    // Our own command, not the opener plugin: its open_path is
    // fire-and-forget and reports success even when the OS has no app
    // for the file type — a print button that silently does nothing
    const result = await commands.openWithDefaultApp(selection);
    if (result.status === "ok") {
      showPrintModal.value = false;
      if (extracted.length && packCleanupAfter.value) {
        scheduleSlicerCleanup(extracted);
      }
      return;
    }
    // No slicer owns the extension: show why, then still be useful —
    // the modal stays open with Reveal folder one click away
    toastStore.reportError("Couldn't open in a slicer", result.error);
    toastStore.addToast(
      "Associate the files with your slicer, or use Reveal folder below",
      "info",
    );
  } catch (error) {
    toastStore.reportError("Failed to send to slicer", error);
  } finally {
    printBusy.value = false;
  }
};

const revealFromPrintModal = async () => {
  // packed selections point at paths with no bytes on disk — reveal a
  // loose file when there is one, else the model folder itself
  const target =
    printSelection.value.find(
      (path) => !files.value.some((f) => f.path === path && f.packed),
    ) ??
    files.value.find((f) => !f.packed)?.path ??
    selected.value?.dir_path;
  showPrintModal.value = false;
  if (target) await reveal(target);
};

const reveal = async (path: string) => {
  try {
    await revealItemInDir(path);
  } catch (error) {
    toastStore.reportError("Failed to reveal file", error);
  }
};

/* ---- normalizer: make the disk match the curated catalog ---- */
// null = still checking (or not checked yet) — the drawer button shows a
// disabled "checking…" state rather than flashing dirty-then-clean.
const structureClean = ref<boolean | null>(null);

/** Dry-run the plan for one model to decide the drawer's badge/button. */
const checkStructure = async (groupName: string) => {
  structureClean.value = null;
  if (!catalogRoot.value) return;
  const result = await commands.planNormalize(
    catalogRoot.value,
    null,
    groupName,
  );
  // the user may have clicked to a different model while this was in
  // flight — a stale answer must never paint over the new selection
  if (selectedGroup.value?.group_name !== groupName) return;
  // Fail open on an error: showing the "fix" button is harmless (the plan
  // dialog will just fail again with the same error visible), but getting
  // stuck on "checking…" forever would hide a real problem
  structureClean.value =
    result.status === "ok" ? result.data.groups.length === 0 : false;
};

/* Re-run finalize WITHOUT moving anything: re-writes model.json for
   already-clean models from current catalog state, then rescans so the
   catalog re-reads them. The repair path when a Plinth update improves
   what the sidecar carries (e.g. the image lookup that used to write
   empty images lists) — otherwise clean models could never heal, since
   the normal flow only finalizes groups that had moves. */
const refreshingSidecars = ref(false);
const refreshSidecars = async (groupNames: string[]) => {
  const names = groupNames.filter(Boolean);
  if (!names.length || !catalogRoot.value || refreshingSidecars.value) return;
  refreshingSidecars.value = true;
  try {
    const result = await commands.finalizeNormalize(
      catalogRoot.value,
      names,
      [],
    );
    if (result.status !== "ok") {
      toastStore.reportError("Failed to refresh metadata", result.error);
      return;
    }
    for (const warning of result.data) toastStore.addToast(warning, "error");
    toastStore.addToast(
      `Metadata re-written for ${names.length} model${names.length === 1 ? "" : "s"}`,
      "success",
    );
    // the sidecars changed on disk — only a rescan makes the catalog see it
    await scan();
  } finally {
    refreshingSidecars.value = false;
  }
};

// Everything is planned read-only first and shown as a reviewable move
// list; nothing touches the NAS until "Move" is clicked. Ops are applied
// in chunks so big batches show progress instead of a silent hang.
const showNormalize = ref(false);
const normalizePlanData = ref<NormalizePlan | null>(null);
const normalizePlanning = ref(false);
const normalizeChecked = ref<string[]>([]);
const normalizeBusy = ref(false);
const normalizeDone = ref(0);
const normalizeTotal = ref(0);
const normalizeIssues = ref<string[]>([]);
const expandedPlanGroup = ref<string | null>(null);
// non-null = the drawer asked to clean ONE model; null = whole catalog
const normalizeScope = ref<string | null>(null);

const openNormalize = async (group?: string) => {
  if (!catalogRoot.value) {
    toastStore.addToast("Choose a catalog folder first", "info");
    return;
  }
  normalizeScope.value = group ?? null;
  showNormalize.value = true;
  normalizePlanData.value = null;
  normalizePlanning.value = true;
  normalizeIssues.value = [];
  normalizeDone.value = 0;
  normalizeTotal.value = 0;
  // the dry run respects the toolbar's designer facet (whole-catalog mode
  // only — a model cleanup must not be excluded by an unrelated filter),
  // so a NAS cleanup can proceed one designer at a time
  const result = await commands.planNormalize(
    catalogRoot.value,
    group ? null : designerFilter.value || null,
    group ?? null,
  );
  normalizePlanning.value = false;
  if (result.status !== "ok") {
    toastStore.reportError("Failed to plan the cleanup", result.error);
    showNormalize.value = false;
    return;
  }
  normalizePlanData.value = result.data;
  normalizeChecked.value = result.data.groups.map((g) => g.group_name);
};

const toggleNormalizeGroup = (name: string) => {
  normalizeChecked.value = normalizeChecked.value.includes(name)
    ? normalizeChecked.value.filter((n) => n !== name)
    : [...normalizeChecked.value, name];
};

const allPlanChecked = computed(
  () =>
    !!normalizePlanData.value?.groups.length &&
    normalizeChecked.value.length === normalizePlanData.value.groups.length,
);

const toggleAllPlan = () => {
  normalizeChecked.value = allPlanChecked.value
    ? []
    : (normalizePlanData.value?.groups.map((g) => g.group_name) ?? []);
};

const opLabel = (from: string, to: string) => {
  // show the shared prefix only once — the interesting part is what changes
  const fromParts = from.split(/[/\\]/);
  const toParts = to.split(/[/\\]/);
  let shared = 0;
  while (
    shared < fromParts.length - 1 &&
    shared < toParts.length - 1 &&
    fromParts[shared] === toParts[shared]
  ) {
    shared++;
  }
  return `${fromParts.slice(shared).join("/")} → ${toParts.slice(shared).join("/")}`;
};

const applyNormalizePlan = async () => {
  const plan = normalizePlanData.value;
  if (!plan || normalizeBusy.value) return;
  const chosen = plan.groups.filter((g) =>
    normalizeChecked.value.includes(g.group_name),
  );
  const ops = chosen.flatMap((g) => g.ops);
  if (!ops.length) return;
  normalizeBusy.value = true;
  normalizeTotal.value = ops.length;
  normalizeDone.value = 0;
  normalizeIssues.value = [];
  try {
    const CHUNK = 100;
    // Sequential on purpose: moves must land in plan order (a folder
    // rename precedes the file moves inside it), and the chunking exists
    // to surface progress — parallelizing would break both.
    for (let i = 0; i < ops.length; i += CHUNK) {
      // oxlint-disable-next-line no-await-in-loop
      const result = await commands.applyNormalize(ops.slice(i, i + CHUNK));
      if (result.status !== "ok") {
        toastStore.reportError("Cleanup stopped", result.error);
        return;
      }
      normalizeIssues.value.push(...result.data.errors);
      normalizeDone.value = Math.min(ops.length, i + CHUNK);
    }
    const finalize = await commands.finalizeNormalize(
      catalogRoot.value,
      chosen.map((g) => g.group_name),
      chosen.flatMap((g) => g.old_dirs),
    );
    if (finalize.status === "ok") {
      normalizeIssues.value.push(...finalize.data);
    } else {
      toastStore.reportError("Cleanup bookkeeping failed", finalize.error);
    }
    toastStore.addToast(
      `Cleaned up ${chosen.length} model${chosen.length === 1 ? "" : "s"}`,
      "success",
    );
    // the rescan re-reads the fresh model.json sidecars — completion also
    // refreshes search/stats via the existing scanCompletedCount watcher
    await scan();
    if (!normalizeIssues.value.length) {
      showNormalize.value = false;
    } else {
      normalizePlanData.value = null;
    }
  } finally {
    normalizeBusy.value = false;
  }
};

// packed_paths is additive in the bindings (older payloads omit it)
const packedIn = (group: DuplicateGroup) => group.packed_paths ?? [];

// The keeper must be a loose path: a packed keeper can't donate a hardlink
// (merge refuses it) and 'delete copies' would remove every loose copy. A
// stored choice that has since been packed is ignored, not honored.
const keepFor = (group: DuplicateGroup) => {
  const stored = keepChoice.value[group.hash];
  if (stored && !packedIn(group).includes(stored)) return stored;
  return (
    group.paths.find((path) => !packedIn(group).includes(path)) ??
    group.paths[0]
  );
};

// What merge/delete can actually touch: everything except the keeper and
// the packed copies (those have no loose bytes on disk)
const actionableOthers = (group: DuplicateGroup) =>
  group.paths.filter(
    (path) => path !== keepFor(group) && !packedIn(group).includes(path),
  );

// A packed path has no file on disk to reveal — show its folder instead
const revealDupPath = (group: DuplicateGroup, path: string) =>
  reveal(
    packedIn(group).includes(path)
      ? (path.replace(/[\\/][^\\/]*$/, "") ?? path)
      : path,
  );

// Probed with a real hardlink attempt next to the first duplicate (NAS and
// exFAT support can't be guessed from names) — gates the merge buttons so
// link-less volumes get delete-only instead of a button that can't work.
const linkSupport = ref<boolean | null>(null);
watch(showDups, async (open) => {
  const probePath = dupGroups.value[0]?.paths[0];
  if (!open || linkSupport.value !== null || !probePath) return;
  const result = await commands.supportsFileLinks(probePath);
  linkSupport.value = result.status === "ok" ? result.data : false;
});

const runMerge = async (group: DuplicateGroup) => {
  const keep = keepFor(group);
  const others = actionableOthers(group);
  if (!others.length) return 0;
  const result = await commands.mergeDuplicateFiles(keep, others);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to merge duplicates", result.error);
    return 0;
  }
  for (const error of result.data.errors) toastStore.addToast(error, "error");
  return result.data.succeeded;
};

const mergeGroup = async (group: DuplicateGroup) => {
  const confirmed = await confirm(
    `Merge ${group.paths.length} identical files so they share one copy on disk?\n\nEvery variant keeps a working file — ${formatFileSize(reclaimableBytes(group))} is freed.`,
    { title: "Merge duplicates", kind: "info" },
  );
  if (!confirmed) return;
  reclaimBusy.value = true;
  try {
    const merged = await runMerge(group);
    if (merged) {
      toastStore.addToast(
        `Merged into one shared copy — ${formatFileSize(reclaimableBytes(group))} freed`,
        "success",
      );
    }
    await refreshMeta();
  } finally {
    reclaimBusy.value = false;
  }
};

const mergeAllGroups = async () => {
  const targets = reclaimableGroups.value;
  const confirmed = await confirm(
    `Merge all ${targets.length} duplicate groups so identical files share one copy on disk?\n\nNothing disappears from any folder — ${formatFileSize(wastedBytes.value)} is freed.`,
    { title: "Merge all duplicates", kind: "info" },
  );
  if (!confirmed) return;
  reclaimBusy.value = true;
  try {
    let merged = 0;
    // Sequential on purpose: every merge re-hashes whole files on the same
    // disk (often a NAS) — concurrency would only add seek thrash
    // oxlint-disable-next-line no-await-in-loop
    for (const group of targets) merged += await runMerge(group);
    if (merged) {
      toastStore.addToast(
        `Merged ${merged} duplicate file${merged === 1 ? "" : "s"} into shared copies`,
        "success",
      );
    }
    await refreshMeta();
  } finally {
    reclaimBusy.value = false;
  }
};

const reclaimGroup = async (group: DuplicateGroup) => {
  const keep = keepFor(group);
  const doomed = actionableOthers(group);
  if (!doomed.length) return;
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

const toggleCheckedGroup = (groupName: string) => {
  checkedGroups.value = checkedGroups.value.includes(groupName)
    ? checkedGroups.value.filter((g) => g !== groupName)
    : [...checkedGroups.value, groupName];
};

const clearSelection = () => {
  checkedGroups.value = [];
  combining.value = false;
};

const startCombine = () => {
  combineName.value = checkedGroups.value[0] ?? "";
  combining.value = true;
};

// The manual counterpart to folder inference: creators structure their
// libraries every which way, so combining can never depend on the scanner
// having guessed right — pick the cards, give them one name.
const combineChecked = async () => {
  const names = [...checkedGroups.value];
  const target = combineName.value.trim();
  if (!target || names.length < 2) return;
  const result = await commands.combineCatalogGroups(names, target);
  combining.value = false;
  if (result.status !== "ok") {
    toastStore.reportError("Failed to combine models", result.error);
    return;
  }
  toastStore.addToast(
    `Combined ${names.length} models into "${target}"`,
    "success",
  );
  checkedGroups.value = [];
  await Promise.all([runSearch(), refreshMeta()]);
  const merged = groups.value.find(
    (g) => g.group_name.toLowerCase() === target.toLowerCase(),
  );
  if (merged) await selectGroup(merged);
};

// The selected pose's own image, else the SAME pose from another support
// variant — nobody renders the supported copy separately, so supported/
// unsupported share pictures automatically. Sharing stops at the pose
// boundary: pose B never borrows pose A's picture, they're different minis.
const drawerPreview = computed(() => {
  const entry = selected.value;
  if (!entry) return null;
  if (entry.preview_path) return entry.preview_path;
  const poseKey = entry.pose ?? entry.name;
  return (
    members.value.find((m) => m.preview_path && (m.pose ?? m.name) === poseKey)
      ?.preview_path ?? null
  );
});

const moveChecked = async () => {
  const dest = await selectDirectory({ title: "Move selected models into…" });
  if (!dest) return;
  // a checked group means ALL of its variant folders move
  const memberResults = await Promise.all(
    checkedGroups.value.map((name) => commands.getCatalogGroupMembers(name)),
  );
  const dirs = memberResults.flatMap((result) =>
    result.status === "ok" ? result.data.map((m) => m.dir_path) : [],
  );
  const sep = dest.includes("\\") ? "\\" : "/";
  const operations = dirs
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
    `Move ${operations.length} folder${operations.length === 1 ? "" : "s"} (${checkedGroups.value.length} model${checkedGroups.value.length === 1 ? "" : "s"}) into:\n${dest}`,
    { title: "Reorganize models", kind: "warning" },
  );
  if (!confirmed) return;
  const result = await commands.batchMoveModels(operations);
  if (result.status === "ok") {
    const { succeeded, errors } = result.data;
    if (succeeded) {
      toastStore.addToast(
        `Moved ${succeeded} folder${succeeded === 1 ? "" : "s"}`,
        "success",
      );
    }
    for (const error of errors) toastStore.addToast(error, "error");
    checkedGroups.value = [];
    // the selected entries' dir_paths may have just changed
    selectedGroup.value = null;
    selected.value = null;
    members.value = [];
    files.value = [];
    await Promise.all([runSearch(), refreshMeta()]);
  } else {
    toastStore.reportError("Failed to move models", result.error);
  }
};

watch(selected, (entry) => {
  metaDraft.value = {
    // NAME is the card/sort name — i.e. the GROUP name — not the per-variant
    // name. Variants are told apart by their pose, so this one field renames
    // the whole model regardless of how many poses it has.
    name: selectedGroup.value?.group_name ?? entry?.name ?? "",
    variant: entry?.variant ?? "",
    pose: entry?.pose ?? "",
    scale: entry?.scale ?? "",
    support_status: entry?.support_status ?? "",
    release_date: entry?.release_date ?? "",
    designer: entry?.designer ?? "",
    sculptor: entry?.sculptor ?? "",
    release_name: entry?.release_name ?? "",
    base_round_mm: entry?.base_round_mm ?? "",
    base_square_mm: entry?.base_square_mm ?? "",
  };
  // fresh member: drop any ticks, and seed the assign boxes with this member's
  // facets so filing more files under the same bucket is one tap
  checkedFiles.value = [];
  variantAssignDraft.value = entry?.variant ?? "";
  poseAssignDraft.value = entry?.pose ?? "";
});

const metaDirty = computed(() => {
  const entry = selected.value;
  if (!entry) return false;
  const draft = metaDraft.value;
  return (
    draft.name !== (selectedGroup.value?.group_name ?? entry.name) ||
    draft.variant !== (entry.variant ?? "") ||
    draft.pose !== (entry.pose ?? "") ||
    draft.scale !== (entry.scale ?? "") ||
    draft.support_status !== (entry.support_status ?? "") ||
    draft.release_date !== (entry.release_date ?? "") ||
    draft.designer !== (entry.designer ?? "") ||
    draft.sculptor !== (entry.sculptor ?? "") ||
    draft.release_name !== (entry.release_name ?? "") ||
    draft.base_round_mm !== (entry.base_round_mm ?? "") ||
    draft.base_square_mm !== (entry.base_square_mm ?? "")
  );
});

const saveMetadata = async () => {
  const entry = selected.value;
  const group = selectedGroup.value;
  if (!entry || !group) return;
  const draft = metaDraft.value;
  // A file-split member's variant/pose/support live in file_variants, not
  // model_user_meta — writing them there would silently revert on reload.
  const isVariant = !!entry.variant_key;
  const newVariant = draft.variant.trim();
  const newPose = draft.pose.trim();
  const bucketChanged =
    isVariant &&
    (newVariant !== (entry.variant ?? "") ||
      newPose !== (entry.pose ?? "") ||
      orNull(draft.support_status) !== (entry.support_status ?? null));

  if (bucketChanged) {
    // re-file this member's files under the edited facets (or unfile them
    // back to the pool when both variant and pose are cleared)
    const paths = files.value.map((file) => file.path);
    const refiled =
      newVariant || newPose
        ? await commands.assignFilesToPose(
            paths,
            newVariant || null,
            newPose || null,
            orNull(draft.support_status),
          )
        : await commands.clearFilePose(paths);
    if (refiled.status !== "ok") {
      toastStore.reportError("Failed to re-file member", refiled.error);
      return;
    }
  }

  // Model-level metadata (shared by every member of the folder). custom_name
  // is preserved — NAME drives the group name below. For a variant member
  // variant/pose/support are null here; they went to file_variants.
  const result = await commands.updateModelMetadata(entry.dir_path, {
    custom_name: entry.custom_name ?? null,
    variant: isVariant ? null : orNull(draft.variant),
    pose: isVariant ? null : orNull(draft.pose),
    scale: orNull(draft.scale),
    support_status: isVariant ? null : orNull(draft.support_status),
    release_date: orNull(draft.release_date),
    designer: orNull(draft.designer),
    sculptor: orNull(draft.sculptor),
    release_name: orNull(draft.release_name),
    base_round_mm: mmOrNull(draft.base_round_mm),
    base_square_mm: mmOrNull(draft.base_square_mm),
  });
  if (result.status !== "ok") {
    toastStore.reportError("Failed to save details", result.error);
    return;
  }
  // variant/pose/scale were also applied to this sculpt's other support
  // builds (exact folder twins) — say so, since the user didn't click them
  const twinCount = result.data;
  const savedToast = () =>
    toastStore.addToast(
      twinCount
        ? `Details saved · also applied to ${twinCount} matching build${twinCount === 1 ? "" : "s"}`
        : "Details saved",
      "success",
    );

  // NAME edits the group/card name (the sort key) for every model.
  const newName = draft.name.trim();
  if (newName && newName !== group.group_name) {
    const renamed = await commands.renameCatalogGroup(
      group.group_name,
      newName,
    );
    if (renamed.status !== "ok") {
      toastStore.reportError("Saved details, but rename failed", renamed.error);
      await refreshSelected();
      return;
    }
    savedToast();
    // the card moved to its new name — re-open it there
    await Promise.all([runSearch(), refreshMeta()]);
    const found = groups.value.find(
      (g) => g.group_name.toLowerCase() === newName.toLowerCase(),
    );
    if (found) await selectGroup(found);
    return;
  }

  savedToast();
  // land on the re-filed bucket (or the pool, if both facets were cleared)
  if (bucketChanged) {
    await Promise.all([
      runSearch(),
      reloadMembers(variantKeyFor(entry.dir_path, newVariant, newPose)),
    ]);
  } else {
    await refreshSelected();
  }
};

const pickPreviewImage = async () => {
  const entry = selected.value;
  if (!entry) return;
  const picked = await selectFiles({
    accept: "image/*",
    multiple: false,
    title: "Choose a preview image",
  });
  const image = picked?.[0];
  if (!image) return;
  // The backend copies the file into the app's previews dir, so the
  // catalog doesn't break if the original moves or gets deleted. variant_key
  // keeps the pick on this pose alone when the folder holds several.
  const result = await commands.setModelPreview(
    entry.dir_path,
    image.path,
    entry.variant_key,
  );
  if (result.status === "ok") {
    toastStore.addToast("Preview updated", "success");
    await refreshSelected();
  } else {
    toastStore.reportError("Failed to set preview", result.error);
  }
};

const displayPath = computed(() => {
  const entry = selected.value;
  if (!entry) return "";
  const root = catalogRoot.value;
  return root && entry.dir_path.startsWith(root)
    ? entry.dir_path.slice(root.length).replace(/^[/\\]/, "")
    : entry.dir_path;
});

/**
 * Stage a catalog model using its source paths. The release builder copies
 * it only after the user has chosen the release details.
 */
const addToDraftRelease = async () => {
  if (!selected.value) return;
  try {
    const entry = selected.value;
    const groupName = selectedGroup.value?.group_name ?? entry.name;
    const variants = members.value.length ? members.value : [entry];
    const newVariants = variants.filter(
      (variant) =>
        !releasesStore.models.some(
          (draft) => draft.source_dir === variant.dir_path,
        ),
    );
    const fileResults = await Promise.all(
      newVariants.map((variant) =>
        commands.getCatalogModelFiles(variant.dir_path, variant.variant_key),
      ),
    );
    // Per-file pose assignments ride along so a curated dump folder
    // reappears already split on the receiving side (docs/3PK.md)
    const assignmentResults = await Promise.all(
      newVariants.map((variant) => commands.getFileVariants(variant.dir_path)),
    );
    for (const [index, variant] of newVariants.entries()) {
      const fileResult = fileResults[index];
      if (fileResult.status !== "ok") throw fileResult.error;
      const fileNames = new Set(fileResult.data.map((file) => file.file_name));
      const assignments = assignmentResults[index];
      const filePoses = (assignments.status === "ok" ? assignments.data : [])
        .filter((assignment) => fileNames.has(basename(assignment.path)))
        .map((assignment) => ({
          name: basename(assignment.path),
          variant: assignment.variant,
          pose: assignment.pose,
          support_status: assignment.support_status,
        }));
      const poseKey = variant.pose ?? variant.name;
      // Mirror the catalog drawer's preview resolution so the render the user
      // sees on the card actually rides along: the pose's own image, else a
      // sibling variant sharing the pose, else the group's aggregate preview.
      const preview =
        variant.preview_path ??
        variants.find(
          (candidate) =>
            candidate.preview_path &&
            (candidate.pose ?? candidate.name) === poseKey,
        )?.preview_path ??
        selectedGroup.value?.preview_path ??
        null;
      releasesStore.models.push({
        id: `draft-${Date.now()}-${releasesStore.models.length}`,
        name: variant.name,
        description: variant.description,
        tags: [...variant.tags],
        images: preview ? [preview] : [],
        model_files: fileResult.data.map((file) => file.path),
        group: variants.length > 1 ? groupName : null,
        source_dir: variant.dir_path,
        source_group: groupName,
        // The full curation travels: model.json → manifest → another
        // user's catalog (the whole point of the 3pk format)
        variant: variant.variant,
        pose: variant.pose,
        scale: variant.scale,
        support_status: variant.support_status,
        release_date: variant.release_date,
        designer: variant.designer,
        sculptor: variant.sculptor,
        release_name: variant.release_name,
        base_round_mm: variant.base_round_mm,
        base_square_mm: variant.base_square_mm,
        file_poses: filePoses,
      });
    }
    toastStore.addToast(
      newVariants.length
        ? `Added “${groupName}” with ${newVariants.length} pose${newVariants.length === 1 ? "" : "s"}`
        : `“${groupName}” is already in the release`,
      newVariants.length ? "success" : "info",
    );
  } catch (error) {
    toastStore.reportError("Failed to add model to release", error);
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
  // The open drawer shows pre-scan members otherwise — a rescan that
  // regroups models (or removes dirs) must be visible immediately, not
  // after a reopen
  const openGroup = selectedGroup.value;
  if (!openGroup) return;
  const fresh = groups.value.find(
    (g) => g.group_name.toLowerCase() === openGroup.group_name.toLowerCase(),
  );
  if (fresh) {
    await selectGroup(fresh);
  } else {
    selectedGroup.value = null;
    selected.value = null;
    members.value = [];
  }
});

watch(dupCompletedCount, async () => {
  const dupResult = await commands.getDuplicateGroups();
  if (dupResult.status === "ok") {
    dupGroups.value = dupResult.data;
    // Same filter as the footer and the panel: already-merged (shared)
    // groups are done, not news — counting them here made the toast and
    // the summary disagree after a few merges
    const actionable = reclaimableGroups.value.length;
    const shared = dupGroups.value.length - actionable;
    showDups.value = actionable > 0;
    toastStore.addToast(
      actionable
        ? `Found ${actionable} duplicate group${actionable === 1 ? "" : "s"}${shared ? ` (${shared} already merged)` : ""}`
        : "No duplicates found",
      actionable ? "warning" : "success",
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

// The tab is kept alive (KeepAlive in App.vue), so onMounted only fires
// once — refresh on every return so previews set from the Render tab and
// other cross-tab changes show up without a manual rescan. When a group is
// open, refreshSelected re-fetches its members too, so a render promoted to
// this pose's preview shows up in the drawer without reselecting the card.
onActivated(async () => {
  await Promise.all([
    selectedGroup.value ? refreshSelected() : runSearch(),
    refreshMeta(),
  ]);
});
</script>
