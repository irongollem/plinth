<script setup lang="ts">
import { computed } from "vue";
import BlenderSetupDialog from "./components/BlenderSetupDialog.vue";
import Sidebar from "./components/Sidebar.vue";
import ToastContainer from "./components/ToastContainer.vue";
import { use3DPackageHandler } from "./composables/use3DPackageHandler";
import { useReleasesStore } from "./stores/releasesStore";
import Catalog from "./views/Catalog.vue";
import Releases from "./views/Releases.vue";
import Render from "./views/Render.vue";
import Settings from "./views/Settings.vue";

use3DPackageHandler();
const releasesStore = useReleasesStore();

const currentTabComponent = computed(() => {
  switch (releasesStore.activeTab) {
    case "catalog":
      return Catalog;
    case "releases":
      return Releases;
    case "render":
      return Render;
    case "settings":
      return Settings;
    default:
      return Catalog;
  }
});
</script>

<template>
  <div class="h-screen flex bg-base-100 text-base-content">
    <Sidebar />
    <div class="flex-1 min-w-0 overflow-hidden">
      <KeepAlive>
        <component :is="currentTabComponent" class="h-full" />
      </KeepAlive>
    </div>
  </div>

  <ToastContainer />
  <!-- Outside the KeepAlive so first-run setup overlays every tab -->
  <BlenderSetupDialog />
</template>
