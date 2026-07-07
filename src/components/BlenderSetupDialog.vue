<template>
  <div
    v-if="dialogVisible"
    class="fixed inset-0 bg-black/60 backdrop-blur-sm flex items-center justify-center z-50"
  >
    <div
      class="bg-base-100 border border-base-content/10 rounded-xl shadow-xl w-105 max-w-[90vw] p-5 flex flex-col gap-4"
    >
      <div class="flex flex-col gap-1">
        <span
          class="font-mono font-semibold text-[10px] tracking-widest text-base-content/40"
          >RENDER ENGINE</span
        >
        <span class="font-bold text-[15px]">{{ title }}</span>
      </div>

      <!-- Scanning -->
      <div v-if="stage === 'scanning'" class="flex items-center gap-3 py-2">
        <span class="loading loading-spinner loading-sm"></span>
        <span class="text-[13px] text-base-content/70"
          >Looking for a Blender installation…</span
        >
      </div>

      <!-- Verdict report -->
      <template v-else-if="stage === 'report'">
        <div
          v-if="blenderInfo"
          class="flex flex-col gap-0.5 bg-base-200 border border-base-content/10 rounded-lg px-3 py-2"
        >
          <span
            class="font-mono text-[12px]"
            :class="verdict === 'Ok' ? 'text-success' : 'text-warning'"
            >{{ verdict === "Ok" ? "✓" : "△" }} {{ blenderInfo.version }}</span
          >
          <span
            class="font-mono text-[11px] text-base-content/50 truncate"
            :title="blenderInfo.path"
            >{{ blenderInfo.path }}</span
          >
        </div>

        <p class="text-[12.5px] text-base-content/70 leading-relaxed">
          <template v-if="verdict === 'Ok'">
            Your Blender is ready — model previews render with it as-is.
          </template>
          <template v-else-if="verdict === 'Outdated'">
            This works, but previews are tuned for Blender
            {{ recommendedSeries }} — older versions light and tone-map them
            differently. You can keep yours, or let stl-pack fetch its own
            {{ managedVersion }} alongside it (~350&nbsp;MB, yours stays
            untouched).
          </template>
          <template v-else-if="verdict === 'TooOld'">
            This version is too old to drive the render engine (4.2 is the
            minimum). stl-pack can download its own Blender
            {{ managedVersion }} without touching your install.
          </template>
          <template v-else>
            No Blender found. Rendering model previews needs one — stl-pack can
            download its own copy (~350&nbsp;MB), or you can point it at an
            existing install in Settings.
          </template>
        </p>

        <div class="flex justify-end gap-2">
          <template v-if="verdict === 'Ok'">
            <button
              type="button"
              class="btn btn-sm btn-primary"
              @click="finish"
            >
              Continue
            </button>
          </template>
          <template v-else-if="verdict === 'Outdated'">
            <button type="button" class="btn btn-sm" @click="finish">
              Keep mine
            </button>
            <button
              type="button"
              class="btn btn-sm btn-primary"
              @click="startDownload"
            >
              Download {{ managedVersion }}
            </button>
          </template>
          <template v-else>
            <button type="button" class="btn btn-sm" @click="goToSettings">
              Choose location…
            </button>
            <button
              type="button"
              class="btn btn-sm btn-primary"
              @click="startDownload"
            >
              Download Blender {{ managedVersion }}
            </button>
          </template>
        </div>
      </template>

      <!-- Downloading / extracting -->
      <template v-else-if="stage === 'downloading'">
        <div class="flex flex-col gap-1.5">
          <progress
            class="progress progress-primary w-full"
            :value="percent"
            max="100"
          ></progress>
          <span class="font-mono text-[11px] text-base-content/50">
            <template v-if="phase">{{ phaseLabel }}</template>
            <template v-else-if="totalBytes"
              >{{ mb(downloadedBytes) }} / {{ mb(totalBytes) }} MB —
              {{ percent }}%</template
            >
            <template v-else>Contacting download.blender.org…</template>
          </span>
        </div>
        <div class="flex justify-end">
          <button
            type="button"
            class="btn btn-sm"
            :disabled="!!phase"
            @click="cancelDownload"
          >
            Cancel
          </button>
        </div>
      </template>

      <!-- Done -->
      <template v-else-if="stage === 'done'">
        <div
          class="flex flex-col gap-0.5 bg-base-200 border border-base-content/10 rounded-lg px-3 py-2"
        >
          <span class="font-mono text-[12px] text-success"
            >✓ {{ installedInfo?.version }} installed</span
          >
          <span
            class="font-mono text-[11px] text-base-content/50 truncate"
            :title="installedInfo?.path"
            >{{ installedInfo?.path }}</span
          >
        </div>
        <div class="flex justify-end">
          <button type="button" class="btn btn-sm btn-primary" @click="finish">
            Continue
          </button>
        </div>
      </template>

      <!-- Failed / cancelled -->
      <template v-else>
        <p class="text-[12.5px] text-error leading-relaxed">
          {{ errorMessage || "Download cancelled." }}
        </p>
        <div class="flex justify-end gap-2">
          <button type="button" class="btn btn-sm" @click="finish">
            Not now
          </button>
          <button
            type="button"
            class="btn btn-sm btn-primary"
            @click="startDownload"
          >
            Retry
          </button>
        </div>
      </template>

      <p
        v-if="stage !== 'scanning'"
        class="text-[10px] text-base-content/35 leading-relaxed"
      >
        Blender is free software under the GNU GPL, downloaded from the official
        blender.org mirror.
        <a
          class="link"
          href="#"
          @click.prevent="openUrl('https://www.blender.org/about/license/')"
          >License</a
        >
        ·
        <a
          class="link"
          href="#"
          @click.prevent="
            openUrl('https://projects.blender.org/blender/blender')
          "
          >Source</a
        >
      </p>
    </div>
  </div>
</template>

<script setup lang="ts">
import { openUrl } from "@tauri-apps/plugin-opener";
import { computed, onMounted, watch } from "vue";
import { commands } from "../bindings";
import { useBlenderProvision } from "../composables/useBlenderProvision";
import { useReleasesStore } from "../stores/releasesStore";

const releasesStore = useReleasesStore();
const {
  check,
  checking,
  verdict,
  blenderInfo,
  managedVersion,
  isDownloading,
  percent,
  downloadedBytes,
  totalBytes,
  phase,
  errorMessage,
  installedInfo,
  status,
  runCheck,
  startDownload,
  cancelDownload,
  acknowledge,
  clearBlenderPathSetting,
  dialogVisible,
  openDialog,
  closeDialog,
} = useBlenderProvision();

// "5.1.2" -> "5.1", for copy about the look-locked series
const recommendedSeries = computed(() =>
  managedVersion.value.split(".").slice(0, 2).join("."),
);

const stage = computed(() => {
  if (isDownloading.value) return "downloading";
  if (status.value && "Completed" in status.value) return "done";
  if (status.value && ("Failed" in status.value || "Cancelled" in status.value))
    return "failed";
  if (checking.value || !check.value) return "scanning";
  return "report";
});

const title = computed(() => {
  switch (stage.value) {
    case "scanning":
      return "Checking for Blender";
    case "downloading":
      return `Downloading Blender ${managedVersion.value}`;
    case "done":
      return "Blender is ready";
    case "failed":
      return "Download didn't finish";
    default:
      switch (verdict.value) {
        case "Ok":
          return "Blender found";
        case "Outdated":
          return "Blender found — newer look available";
        case "TooOld":
          return "Blender found — too old to render";
        default:
          return "No Blender found";
      }
  }
});

const phaseLabel = computed(() => {
  switch (phase.value) {
    case "verify":
      return "Verifying checksum…";
    case "extract":
      return "Extracting…";
    case "install":
      return "Installing…";
    default:
      return "";
  }
});

const mb = (bytes: number) => Math.round(bytes / (1024 * 1024)).toString();

// A finished install is durable state: record the ack and drop a stale
// explicit blender_path (it would keep outranking the managed copy) even
// if the user never clicks Continue.
watch(installedInfo, async (info) => {
  if (!info) return;
  await clearBlenderPathSetting();
  await acknowledge(managedVersion.value);
});

const finish = async () => {
  if (managedVersion.value) await acknowledge(managedVersion.value);
  closeDialog();
};

const goToSettings = () => {
  // Deliberately no ack: setup isn't done, the dialog should return next
  // launch if they don't end up configuring a Blender
  releasesStore.setActiveTab("settings");
  closeDialog();
};

// First-run gate: one probe per launch feeds every surface's verdict; the
// dialog itself only appears until the user acknowledges this pinned
// version — bumping the pin re-offers exactly once.
onMounted(async () => {
  const settings = await commands.getSettings();
  const acknowledged =
    settings.status === "ok" ? settings.data.blender_setup_acknowledged : null;
  const result = await runCheck();
  if (result && acknowledged !== result.managed_version) {
    openDialog();
  }
});
</script>
