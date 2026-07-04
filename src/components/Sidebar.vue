<script setup lang="ts">
import { onMounted, ref } from "vue";
import { commands } from "../bindings";
import { useOS } from "../composables/useOS";
import { useReleasesStore } from "../stores/releasesStore";
import type { ReleaseStep } from "../stores/releasesStore";
import { useThemeStore } from "../stores/themeStore";
import { formatFileSize } from "../utils/format";

const releasesStore = useReleasesStore();
const themeStore = useThemeStore();
const { osType } = useOS();
// The window's titlebar is transparent (titleBarStyle: Overlay), so the
// sidebar bg flows up behind the traffic lights. Only macOS parks lights
// in our top-left corner — drop the logo below them there.
const isMac = osType.value === "macos";

const catalogRoot = ref("");
const libraryLine = ref("");

onMounted(async () => {
  const [settings, stats] = await Promise.all([
    commands.getSettings(),
    commands.getCatalogStats(),
  ]);
  if (settings.status === "ok")
    catalogRoot.value = settings.data.catalog_root ?? "";
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
    class="w-[220px] shrink-0 bg-base-300 flex flex-col py-4 border-r border-base-content/10"
  >
    <!-- The frameless window (titleBarStyle: Overlay) has no OS titlebar to
         grab, so the sidebar IS the drag handle: this root and the header
         carry data-tauri-drag-region, and their non-interactive children get
         pointer-events-none so a press lands on the draggable ancestor.
         Buttons keep their own pointer events, so nav still clicks. -->
    <div
      data-tauri-drag-region
      class="flex items-center gap-[7px] px-[18px] mb-[22px] [&>span]:pointer-events-none"
      :class="isMac ? 'pt-[22px]' : ''"
    >
      <span class="font-display text-[15px] tracking-[0.06em]">PLINTH</span>
      <span class="w-1.5 h-1.5 bg-primary"></span>
      <span class="flex-1"></span>
      <span class="font-mono text-[10px] text-base-content/40">v0.1</span>
    </div>

    <button
      type="button"
      class="flex items-center gap-[10px] px-[18px] py-[9px] font-semibold text-[13px] cursor-pointer border-l-2 text-left"
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
      class="flex items-center gap-[10px] px-[18px] py-[9px] font-semibold text-[13px] cursor-pointer border-l-2 text-left"
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
      class="mx-[14px] mt-2 bg-base-200 border border-base-content/10 rounded-box px-3 pt-3 pb-[9px]"
    >
      <div class="flex items-center gap-[6px] mb-2">
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
          class="w-[15px] h-[15px] shrink-0 rounded-full font-bold text-[9px] flex items-center justify-center border box-border"
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
      class="flex items-center gap-[10px] px-[18px] py-[9px] mt-2 font-semibold text-[13px] cursor-pointer border-l-2 text-left"
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

    <span class="flex-1"></span>

    <div
      v-if="catalogRoot"
      class="px-[18px] pb-[10px] font-mono text-[10.5px] text-base-content/40 leading-[1.7] truncate"
      :title="catalogRoot"
    >
      {{ catalogRoot }}<br />
      {{ libraryLine }}
    </div>

    <div
      class="flex gap-1 mx-4 mb-2 bg-base-200 border border-base-content/10 rounded-full p-[3px]"
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
      class="flex items-center gap-[10px] px-[18px] py-[9px] font-semibold text-[13px] cursor-pointer border-l-2 text-left"
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
