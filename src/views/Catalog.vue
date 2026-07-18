<template>
  <main class="bg-base-100 text-base-content flex flex-col h-full p-4 gap-3">
    <CatalogToolbar />
    <CatalogStatusBanners />
    <CatalogResultBar />
    <CatalogBatchBar />

    <!-- Content -->
    <div class="flex flex-1 gap-3 min-h-0">
      <CatalogBrowser />
      <CatalogDrawer />
    </div>

    <CatalogFooter />
    <CatalogDuplicatesPanel />

    <CatalogViewerModals />
    <CatalogDeleteModal />
    <CatalogPrintModal />
    <CatalogBatchRenderModal />
    <CatalogNormalizeModal />
  </main>
</template>

<script setup lang="ts">
import { onActivated, onMounted } from "vue";
import CatalogBatchBar from "../components/catalog/CatalogBatchBar.vue";
import CatalogBatchRenderModal from "../components/catalog/CatalogBatchRenderModal.vue";
import CatalogBrowser from "../components/catalog/CatalogBrowser.vue";
import CatalogDeleteModal from "../components/catalog/CatalogDeleteModal.vue";
import CatalogDrawer from "../components/catalog/CatalogDrawer.vue";
import CatalogDuplicatesPanel from "../components/catalog/CatalogDuplicatesPanel.vue";
import CatalogFooter from "../components/catalog/CatalogFooter.vue";
import CatalogNormalizeModal from "../components/catalog/CatalogNormalizeModal.vue";
import CatalogPrintModal from "../components/catalog/CatalogPrintModal.vue";
import CatalogResultBar from "../components/catalog/CatalogResultBar.vue";
import CatalogStatusBanners from "../components/catalog/CatalogStatusBanners.vue";
import CatalogToolbar from "../components/catalog/CatalogToolbar.vue";
import CatalogViewerModals from "../components/catalog/CatalogViewerModals.vue";
import { useCatalogStore } from "../stores/catalogStore";

const store = useCatalogStore();

onMounted(() => store.init());

// The tab is kept alive (KeepAlive in App.vue), so onMounted only fires
// once — refresh on every return so previews set from the Render tab and
// other cross-tab changes show up without a manual rescan. When a group is
// open, refreshSelected re-fetches its members too, so a render promoted to
// this pose's preview shows up in the drawer without reselecting the card.
onActivated(() => store.onReactivated());
</script>
