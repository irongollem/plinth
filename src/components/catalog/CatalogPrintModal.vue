<template>
  <!-- Print file picker: tick exactly what goes to the slicer -->
  <ModalView :is-open="showPrintModal" @close="showPrintModal = false">
    <div
      class="w-120 max-w-[85vw] bg-base-100 rounded-box p-4 flex flex-col gap-3"
    >
      <div>
        <div class="font-bold text-[15px]">Print — {{ selected?.name }}</div>
        <p class="text-[11px] text-base-content/50 mt-0.5">
          Ticked files open in your slicer. Slicer scenes keep supports and
          plate layout editable, so they're picked over raw geometry by default.
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
              class="w-4 h-4 shrink-0 flex items-center justify-center"
              :title="KIND_LABELS[fileKind(file)]"
            >
              <img
                v-if="KIND_LOGOS[fileKind(file)]"
                :src="KIND_LOGOS[fileKind(file)]"
                class="w-4 h-4 rounded-[3px]"
                alt=""
              />
              <span
                v-else-if="fileKind(file) === 'gcode'"
                class="text-[10px] font-bold leading-none opacity-50"
                >G</span
              >
              <!-- machine-ready sliced output without a vendor mark -->
              <svg
                v-else-if="fileKind(file) === 'machine'"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                class="opacity-50"
              >
                <path d="M6 9V3h12v6"></path>
                <path
                  d="M6 18H4a2 2 0 0 1-2-2v-5a2 2 0 0 1 2-2h16a2 2 0 0 1 2 2v5a2 2 0 0 1-2 2h-2"
                ></path>
                <rect x="6" y="14" width="12" height="7"></rect>
              </svg>
              <!-- supported raw mesh: object standing on support struts -->
              <svg
                v-else-if="fileKind(file) === 'supported'"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                class="opacity-50"
              >
                <path d="M12 3 L19 11 H5 Z"></path>
                <path d="M7.5 11v6.5M12 11v6.5M16.5 11v6.5"></path>
                <path d="M4 20.5h16"></path>
              </svg>
              <!-- bare raw mesh: plain geometry box -->
              <svg
                v-else
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                class="opacity-50"
              >
                <path d="M12 2.5 21 7.5v9L12 21.5 3 16.5v-9Z"></path>
                <path d="M3 7.5 12 12.5 21 7.5M12 12.5v9"></path>
              </svg>
            </span>
            <span
              class="flex-1 truncate font-mono text-[11.5px]"
              :title="file.path"
              >{{ file.file_name }}</span
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
</template>

<script setup lang="ts">
import { storeToRefs } from "pinia";
import ModalView from "../ModalView.vue";
import { useCatalogStore } from "../../stores/catalogStore";
import { formatFileSize } from "../../utils/format";
import type { CatalogFile } from "../../bindings";
import lycheeIcon from "../../assets/images/lychee.svg";
import chituboxIcon from "../../assets/images/chitubox.png";
import elegooIcon from "../../assets/images/elegoo.png";
import anycubicIcon from "../../assets/images/anycubic.png";

const store = useCatalogStore();
const {
  showPrintModal,
  selected,
  printCandidates,
  printSelection,
  printSelectionPacked,
  packCleanupAfter,
  printBusy,
} = storeToRefs(store);
const {
  MACHINE_EXTS,
  togglePrintFile,
  persistCleanupAfter,
  revealFromPrintModal,
  sendToSlicer,
} = store;

type FileKind =
  | "lychee"
  | "chitubox"
  | "chituPlate"
  | "elegoo"
  | "anycubic"
  | "gcode"
  | "machine"
  | "supported"
  | "raw";

const KIND_LABELS: Record<FileKind, string> = {
  lychee: "Lychee Slicer scene",
  chitubox: "ChituBox scene",
  chituPlate: "ChituBox sliced plate, printer-ready",
  elegoo: "Elegoo sliced plate, printer-ready",
  anycubic: "Anycubic sliced plate, printer-ready",
  gcode: "G-code, printer-ready",
  machine: "Sliced output, printer-ready",
  supported: "Raw mesh, supports baked in",
  raw: "Raw mesh",
};

// Kinds carrying a vendor logo; everything else draws an inline glyph.
const KIND_LOGOS: Partial<Record<FileKind, string>> = {
  lychee: lycheeIcon,
  chitubox: chituboxIcon,
  chituPlate: chituboxIcon,
  elegoo: elegooIcon,
  anycubic: anycubicIcon,
};

// Whether a raw mesh carries baked-in supports comes from the member's
// support_status (same vocabulary as layout::support_segment). Only when
// that's unset do we fall back to the filename convention — mind that
// "unsupported" contains "supported".
const fileKind = (file: CatalogFile): FileKind => {
  switch (file.extension) {
    case "lys":
      return "lychee";
    case "chitu":
    case "chitubox":
      return "chitubox";
    case "ctb":
    case "cbddlp":
      return "chituPlate";
    case "goo":
      return "elegoo";
    case "photon":
    case "pwmx":
    case "pwmo":
    case "pwms":
    case "pw0":
      return "anycubic";
    case "gcode":
    case "bgcode":
      return "gcode";
  }
  if (MACHINE_EXTS.includes(file.extension)) return "machine";
  switch (selected.value?.support_status?.toLowerCase()) {
    case "supported":
    case "presupported":
      return "supported";
    case "unsupported":
      return "raw";
  }
  const name = file.file_name.toLowerCase();
  return name.includes("supported") && !name.includes("unsupported")
    ? "supported"
    : "raw";
};
</script>
