<template>
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
</template>

<script setup lang="ts">
import { defineAsyncComponent } from "vue";
import { convertFileSrc } from "@tauri-apps/api/core";
import { storeToRefs } from "pinia";
import ModalView from "../ModalView.vue";
// Lazy: StlViewport drags three.js with it, and this component rides the
// eager Catalog boot chunk — the viewport is only needed once a preview
// actually opens (the v-if), not at first paint.
const StlViewport = defineAsyncComponent(() => import("../StlViewport.vue"));
import { useCatalogStore } from "../../stores/catalogStore";

const store = useCatalogStore();
const { show3dModal, stlPaths, showImageModal, drawerPreview } =
  storeToRefs(store);
</script>
