<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { commands } from "../bindings";
import { useMinihoard } from "../composables/useMinihoard";
import { useOS } from "../composables/useOS";
import { useReleasesStore } from "../stores/releasesStore";
import type { ReleaseStep } from "../stores/releasesStore";
import { useThemeStore } from "../stores/themeStore";
import { formatFileSize } from "../utils/format";

const releasesStore = useReleasesStore();
const themeStore = useThemeStore();
// The easter egg: this stays null (and the menu invisible) unless the
// minihoard CLI is actually installed on this machine.
const { info: minihoardInfo, detect: detectMinihoard } = useMinihoard();
const { osType } = useOS();
// The window's titlebar is transparent (titleBarStyle: Overlay), so the
// sidebar bg flows up behind the traffic lights. Only macOS parks lights
// in our top-left corner — drop the logo below them there.
const isMac = osType.value === "macos";

const catalogRoots = ref<string[]>([]);
const libraryLine = ref("");
// One folder shows its path; several collapse to a count (full list in
// the tooltip) — the sidebar column can't fit a NAS path per designer.
const rootsLine = computed(() =>
  catalogRoots.value.length === 1
    ? catalogRoots.value[0]
    : `${catalogRoots.value.length} catalog folders`,
);

onMounted(async () => {
  detectMinihoard();
  const [settings, stats] = await Promise.all([
    commands.getSettings(),
    commands.getCatalogStats(),
  ]);
  if (settings.status === "ok") {
    catalogRoots.value =
      settings.data.catalog_roots ??
      (settings.data.catalog_root ? [settings.data.catalog_root] : []);
  }
  if (stats.status === "ok") {
    libraryLine.value = `${stats.data.total_models.toLocaleString()} models · ${formatFileSize(stats.data.total_size_bytes)}`;
  }
});

const stepDefs: { step: ReleaseStep; label: string }[] = [
  { step: 1, label: "Models" },
  { step: 2, label: "Release details" },
  { step: 3, label: "Pack" },
];

const stepState = (step: ReleaseStep) => {
  const active =
    releasesStore.activeTab === "releases" &&
    releasesStore.releaseStep === step;
  const done = step < releasesStore.releaseStep;
  return { active, done };
};
</script>

<template>
  <div
    data-tauri-drag-region
    class="w-55 shrink-0 bg-base-300 flex flex-col py-4 border-r border-base-content/10"
  >
    <!-- The frameless window (titleBarStyle: Overlay) has no OS titlebar to
         grab, so the sidebar IS the drag handle: this root and the header
         carry data-tauri-drag-region, and their non-interactive children get
         pointer-events-none so a press lands on the draggable ancestor.
         Buttons keep their own pointer events, so nav still clicks. -->
    <div
      data-tauri-drag-region
      class="flex items-center gap-1.75 px-4.5 mb-5.5 [&>span]:pointer-events-none"
      :class="isMac ? 'pt-5.5' : ''"
    >
      <span class="font-display text-[15px] tracking-[0.06em]">PLINTH</span>
      <span class="w-1.5 h-1.5 bg-primary"></span>
      <span class="flex-1"></span>
      <span class="font-mono text-[10px] text-base-content/40">v0.1</span>
    </div>

    <button
      type="button"
      class="flex items-center gap-2.5 px-4.5 py-2.25 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
      :class="
        releasesStore.activeTab === 'catalog'
          ? 'bg-base-content/5 border-primary'
          : 'border-transparent text-base-content/60 hover:text-base-content'
      "
      @click="releasesStore.setActiveTab('catalog')"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
      >
        <rect x="3" y="3" width="7" height="7" rx="1.5"></rect>
        <rect x="14" y="3" width="7" height="7" rx="1.5"></rect>
        <rect x="3" y="14" width="7" height="7" rx="1.5"></rect>
        <rect x="14" y="14" width="7" height="7" rx="1.5"></rect>
      </svg>
      Catalog
    </button>

    <button
      type="button"
      class="flex items-center gap-2.5 px-4.5 py-2.25 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
      :class="
        releasesStore.activeTab === 'releases'
          ? 'bg-base-content/5 border-primary'
          : 'border-transparent text-base-content/60 hover:text-base-content'
      "
      @click="releasesStore.setActiveTab('releases')"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
      >
        <path d="M21 8v13H3V8M1 3h22v5H1zM10 12h4"></path>
      </svg>
      Release builder
    </button>

    <!-- draft stepper -->
    <div
      v-if="releasesStore.modelCount"
      class="mx-3.5 mt-2 bg-base-200 border border-base-content/10 rounded-box px-3 pt-3 pb-2.25"
    >
      <div class="flex items-center gap-1.5 mb-2">
        <span
          class="font-mono font-semibold text-[9.5px] tracking-[0.12em] text-primary"
          >DRAFT</span
        >
        <span class="font-semibold text-[11.5px] truncate">{{
          releasesStore.release?.name || "Untitled release"
        }}</span>
      </div>
      <button
        v-for="s in stepDefs"
        :key="s.step"
        type="button"
        class="flex items-center gap-2 py-1 text-[11px] w-full text-left cursor-pointer"
        :class="
          stepState(s.step).active
            ? 'font-semibold'
            : 'font-normal text-base-content/60'
        "
        @click="releasesStore.setReleaseStep(s.step)"
      >
        <span
          class="w-3.75 h-3.75 shrink-0 rounded-full font-bold text-[9px] flex items-center justify-center border box-border"
          :class="
            stepState(s.step).done
              ? 'bg-success border-success text-success-content'
              : stepState(s.step).active
                ? 'bg-primary border-primary text-primary-content'
                : 'border-base-content/20 text-base-content/40'
          "
        >
          {{ stepState(s.step).done ? "✓" : s.step }}
        </span>
        {{ s.label }}
        <span
          v-if="s.step === 1"
          class="font-mono text-[10px] text-base-content/50 ml-auto"
          >{{ releasesStore.modelCount }} added</span
        >
      </button>
    </div>

    <button
      type="button"
      class="flex items-center gap-2.5 px-4.5 py-2.25 mt-2 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
      :class="
        releasesStore.activeTab === 'render'
          ? 'bg-base-content/5 border-primary'
          : 'border-transparent text-base-content/60 hover:text-base-content'
      "
      @click="releasesStore.setActiveTab('render')"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
      >
        <path
          d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z"
        ></path>
        <circle cx="12" cy="13" r="4"></circle>
      </svg>
      Render studio
    </button>

    <button
      type="button"
      class="flex items-center gap-2.5 px-4.5 py-2.25 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
      :class="
        releasesStore.activeTab === 'basecutter'
          ? 'bg-base-content/5 border-primary'
          : 'border-transparent text-base-content/60 hover:text-base-content'
      "
      @click="releasesStore.setActiveTab('basecutter')"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
      >
        <!-- a round cutter stamped through a slab — the base-cut plug -->
        <circle cx="12" cy="9" r="6"></circle>
        <path d="M6 9v9a6 6 0 0 0 12 0V9"></path>
      </svg>
      Base Cutter
    </button>

    <!-- easter egg: only exists when the sibling minihoard CLI is installed -->
    <button
      v-if="minihoardInfo"
      type="button"
      class="flex items-center gap-2.5 px-4.5 py-2.25 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
      :class="
        releasesStore.activeTab === 'minihoard'
          ? 'bg-base-content/5 border-primary'
          : 'border-transparent text-base-content/60 hover:text-base-content'
      "
      @click="releasesStore.setActiveTab('minihoard')"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
      >
        <!-- a treasure chest, for the hoard -->
        <path
          d="M3 10V8a3 3 0 0 1 3-3h12a3 3 0 0 1 3 3v2M3 10v9h18v-9M3 10h18"
        ></path>
        <path d="M12 10v4m-1.5-4h3"></path>
      </svg>
      Minihoard
    </button>

    <span class="flex-1"></span>

    <div
      v-if="catalogRoots.length"
      class="px-4.5 pb-2.5 font-mono text-[10.5px] text-base-content/40 leading-[1.7] truncate"
      :title="catalogRoots.join('\n')"
    >
      {{ rootsLine }}<br />
      {{ libraryLine }}
    </div>

    <div
      class="flex gap-1 mx-4 mb-2 bg-base-200 border border-base-content/10 rounded-full p-0.75"
    >
      <button
        type="button"
        class="flex-1 text-center font-semibold text-[10.5px] py-1 rounded-full cursor-pointer"
        :class="
          themeStore.isDark()
            ? 'bg-primary text-primary-content'
            : 'text-base-content/60'
        "
        @click="themeStore.setDark"
      >
        Dark
      </button>
      <button
        type="button"
        class="flex-1 text-center font-semibold text-[10.5px] py-1 rounded-full cursor-pointer"
        :class="
          !themeStore.isDark()
            ? 'bg-primary text-primary-content'
            : 'text-base-content/60'
        "
        @click="themeStore.setLight"
      >
        Light
      </button>
    </div>

    <button
      type="button"
      class="flex items-center gap-2.5 px-4.5 py-2.25 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
      :class="
        releasesStore.activeTab === 'settings'
          ? 'bg-base-content/5 border-primary'
          : 'border-transparent text-base-content/60 hover:text-base-content'
      "
      @click="releasesStore.setActiveTab('settings')"
    >
      <svg
        width="15"
        height="15"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="1.8"
      >
        <circle cx="12" cy="12" r="3"></circle>
        <path
          d="M12 1v4m0 14v4M4.2 4.2l2.8 2.8m10 10l2.8 2.8M1 12h4m14 0h4M4.2 19.8l2.8-2.8m10-10l2.8-2.8"
        ></path>
      </svg>
      Settings
    </button>
  </div>
</template>
