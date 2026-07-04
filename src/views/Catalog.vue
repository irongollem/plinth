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
          <!-- div, not button: the row hosts a nested checkbox and
               interactive elements can't nest -->
          <div
            v-for="group in groups"
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

        <!-- GRID MODE -->
        <div
          v-else
          class="grid gap-3"
          style="grid-template-columns: repeat(auto-fill, minmax(10rem, 1fr))"
        >
          <CatalogCard
            v-for="group in groups"
            :key="group.group_name"
            :group="group"
            :selected="group.group_name === selectedGroup?.group_name"
            :checked="checkedGroups.includes(group.group_name)"
            @select="selectGroup"
            @toggle-check="toggleCheckedGroup($event.group_name)"
          />
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
              v-if="show3d"
              type="button"
              class="absolute top-1.5 right-1.5 btn btn-xs bg-base-100/70"
              title="Open large viewer"
              @click="show3dModal = true"
            >
              ⤢
            </button>
          </div>
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
                {{ member.pose ?? member.name }}
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

            <div class="flex gap-1.5">
              <button
                type="button"
                class="flex-1 text-center font-semibold text-[11px] tracking-wider bg-primary text-primary-content rounded-md py-2 cursor-pointer"
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
                :disabled="!stlPaths.length"
                @click="show3d = !show3d"
              >
                3D
              </button>
              <button
                type="button"
                class="flex-1 text-center font-semibold text-[11px] tracking-wider border border-base-content/15 rounded-md py-2 cursor-pointer disabled:opacity-40"
                :disabled="!stlPaths.length"
                @click="renderSelected"
              >
                RENDER
              </button>
            </div>

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

              <!-- Assignment bar: appears once files are ticked. Splits a
                   dump folder into pose members without moving anything. -->
              <div
                v-if="checkedFiles.length"
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
                    v-model="poseAssignDraft"
                    type="text"
                    class="input input-xs font-mono flex-1 min-w-0"
                    placeholder="pose e.g. A"
                  />
                  <button
                    type="submit"
                    class="btn btn-xs btn-primary"
                    :disabled="!poseAssignDraft.trim()"
                  >
                    assign
                  </button>
                </form>
                <button
                  type="button"
                  class="btn btn-xs btn-ghost"
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
const orNull = (value: string) => value.trim() || null;

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

const catalogRoot = ref("");
const query = ref("");
const viewMode = ref<"list" | "grid">("grid");
const selectedTags = ref<string[]>([]);
const allTags = ref<TagCount[]>([]);
// the browsable units: one group per logical model
const groups = ref<CatalogGroup[]>([]);
const total = ref(0);
const stats = ref<CatalogStats | null>(null);
// drill-down state: group -> its variant entries -> the active one
const selectedGroup = ref<CatalogGroup | null>(null);
const members = ref<CatalogEntry[]>([]);
const activeSupport = ref("");
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
  pose: "",
  scale: "",
  support_status: "",
  release_date: "",
  designer: "",
  sculptor: "",
  release_name: "",
});

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
  const offset = append ? groups.value.length : 0;
  const result = await commands.searchCatalogGroups(
    query.value,
    selectedTags.value,
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

// A folder split into poses yields several members sharing one dir_path;
// their variant_key disambiguates. Fall back to dir_path for whole-folder
// members (variant_key null).
const memberKey = (entry: CatalogEntry) => entry.variant_key ?? entry.dir_path;

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
    for (const v of variantResult.data) if (v.pose) map[v.path] = v.pose;
    fileVariantMap.value = map;
  }
};

/* ---- deliverable 3: assign files in a dump folder to poses ---- */
// checked file paths in the drawer's file list, and the pose to file them under
const checkedFiles = ref<string[]>([]);
const poseAssignDraft = ref("");
// path -> assigned pose, so already-sorted files show a badge
const fileVariantMap = ref<Record<string, string>>({});

const toggleCheckedFile = (path: string) => {
  checkedFiles.value = checkedFiles.value.includes(path)
    ? checkedFiles.value.filter((p) => p !== path)
    : [...checkedFiles.value, path];
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
  activeSupport.value = firstTab;
  const next =
    (preferKey
      ? members.value.find((m) => memberKey(m) === preferKey)
      : undefined) ??
    members.value.find((m) => (m.support_status ?? "") === firstTab) ??
    members.value[0];
  if (next) await selectEntry(next);
};

const assignChecked = async () => {
  const dir = selected.value?.dir_path;
  const pose = poseAssignDraft.value.trim();
  if (!dir || !pose || !checkedFiles.value.length) return;
  const count = checkedFiles.value.length;
  const result = await commands.assignFilesToPose(
    checkedFiles.value,
    pose,
    null,
  );
  if (result.status !== "ok") {
    toastStore.reportError("Failed to assign files", result.error);
    return;
  }
  toastStore.addToast(
    `Filed ${count} file${count === 1 ? "" : "s"} under pose “${pose}”`,
    "success",
  );
  checkedFiles.value = [];
  poseAssignDraft.value = "";
  // land on the pose we just created so the split is immediately visible
  await Promise.all([runSearch(), reloadMembers(`${dir}\u{1f}${pose}`)]);
};

const clearChecked = async () => {
  if (!checkedFiles.value.length) return;
  const result = await commands.clearFilePose(checkedFiles.value);
  if (result.status !== "ok") {
    toastStore.reportError("Failed to clear assignment", result.error);
    return;
  }
  toastStore.addToast("Unfiled the selected files", "success");
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

const tabMembers = computed(() =>
  members.value.filter((m) => (m.support_status ?? "") === activeSupport.value),
);

const setSupportTab = (tab: string) => {
  // keep the pose when hopping between supported/unsupported — you're
  // looking at the same mini, just the other build of it
  const currentPose = selected.value?.pose ?? null;
  activeSupport.value = tab;
  const next =
    (currentPose
      ? tabMembers.value.find((m) => m.pose === currentPose)
      : undefined) ?? tabMembers.value[0];
  if (next) selectEntry(next);
};

const selectGroup = async (group: CatalogGroup) => {
  selectedGroup.value = group;
  renamingGroup.value = false;
  members.value = [];
  selected.value = null;
  files.value = [];
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

// Carries the model's dir_path so the finished render comes back as this
// pose's catalog preview
const renderSelected = () => {
  if (!selected.value) return;
  releasesStore.requestRender(stlPaths.value, selected.value.dir_path);
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
    pose: entry?.pose ?? "",
    scale: entry?.scale ?? "",
    support_status: entry?.support_status ?? "",
    release_date: entry?.release_date ?? "",
    designer: entry?.designer ?? "",
    sculptor: entry?.sculptor ?? "",
    release_name: entry?.release_name ?? "",
  };
  // fresh member: drop any ticks, and seed the assign box with the scanner's
  // pose guess (the "pre-estimate") so filing more files under it is one tap
  checkedFiles.value = [];
  poseAssignDraft.value = entry?.pose ?? "";
});

const metaDirty = computed(() => {
  const entry = selected.value;
  if (!entry) return false;
  const draft = metaDraft.value;
  return (
    draft.name !== (selectedGroup.value?.group_name ?? entry.name) ||
    draft.pose !== (entry.pose ?? "") ||
    draft.scale !== (entry.scale ?? "") ||
    draft.support_status !== (entry.support_status ?? "") ||
    draft.release_date !== (entry.release_date ?? "") ||
    draft.designer !== (entry.designer ?? "") ||
    draft.sculptor !== (entry.sculptor ?? "") ||
    draft.release_name !== (entry.release_name ?? "")
  );
});

const saveMetadata = async () => {
  const entry = selected.value;
  const group = selectedGroup.value;
  if (!entry || !group) return;
  const draft = metaDraft.value;

  // Per-variant metadata. custom_name is preserved untouched — the NAME
  // field drives the group name below, not a per-variant override.
  const result = await commands.updateModelMetadata(
    entry.dir_path,
    entry.custom_name ?? null,
    orNull(draft.pose),
    orNull(draft.scale),
    orNull(draft.support_status),
    orNull(draft.release_date),
    orNull(draft.designer),
    orNull(draft.sculptor),
    orNull(draft.release_name),
  );
  if (result.status !== "ok") {
    toastStore.reportError("Failed to save details", result.error);
    return;
  }

  // NAME edits the group/card name (the sort key) for every model.
  const newName = draft.name.trim();
  if (newName && newName !== group.group_name) {
    const renamed = await commands.renameCatalogGroup(group.group_name, newName);
    if (renamed.status !== "ok") {
      toastStore.reportError("Saved details, but rename failed", renamed.error);
      await refreshSelected();
      return;
    }
    toastStore.addToast("Details saved", "success");
    // the card moved to its new name — re-open it there
    await Promise.all([runSearch(), refreshMeta()]);
    const found = groups.value.find(
      (g) => g.group_name.toLowerCase() === newName.toLowerCase(),
    );
    if (found) await selectGroup(found);
    return;
  }

  toastStore.addToast("Details saved", "success");
  await refreshSelected();
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
  // catalog doesn't break if the original moves or gets deleted
  const result = await commands.setModelPreview(entry.dir_path, image.path);
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
    for (const [index, variant] of newVariants.entries()) {
      const fileResult = fileResults[index];
      if (fileResult.status !== "ok") throw fileResult.error;
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
        pose: variant.pose,
        scale: variant.scale,
        support_status: variant.support_status,
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
