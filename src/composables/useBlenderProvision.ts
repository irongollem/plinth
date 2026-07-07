import { computed, ref } from "vue";
import {
  type BlenderCheck,
  type BlenderProvisionStatus,
  type BlenderVerdict,
  commands,
  events,
} from "../bindings";

/**
 * Blender detection verdict + managed-download lifecycle, shared app-wide.
 *
 * State lives at module scope on purpose: the setup dialog is mounted once
 * in App.vue, but Settings and the Render view both need to open it and to
 * see the verdict flip after an install — a per-component instance (the
 * useRenderStatus pattern) would leave Render's KeepAlive'd badge stale.
 */
const check = ref<BlenderCheck | null>(null);
const checking = ref(false);
const status = ref<BlenderProvisionStatus | null>(null);
const activeJobId = ref("");
const dialogVisible = ref(false);

let listenerStarted = false;
const ensureListener = () => {
  if (listenerStarted) return;
  listenerStarted = true;
  // App-lifetime listener — never unlistened, matching the singleton state
  events.blenderProvisionStatus.listen((event) => {
    const payload = event.payload;
    const jobId =
      "Started" in payload
        ? payload.Started.job_id
        : "Progress" in payload
          ? payload.Progress.job_id
          : "Extracting" in payload
            ? payload.Extracting.job_id
            : "Completed" in payload
              ? payload.Completed.job_id
              : "Failed" in payload
                ? payload.Failed.job_id
                : payload.Cancelled.job_id;
    if (activeJobId.value && jobId !== activeJobId.value) return;
    status.value = payload;
    if ("Completed" in payload) {
      activeJobId.value = "";
      // The backend re-detected on install; refresh the verdict every
      // surface renders from
      void runCheck();
    } else if ("Failed" in payload || "Cancelled" in payload) {
      activeJobId.value = "";
    }
  });
};

const runCheck = async (): Promise<BlenderCheck | null> => {
  checking.value = true;
  try {
    const result = await commands.checkBlender();
    check.value = result.status === "ok" ? result.data : null;
    return check.value;
  } finally {
    checking.value = false;
  }
};

const verdict = computed<BlenderVerdict | null>(
  () => check.value?.verdict ?? null,
);
const blenderInfo = computed(() => check.value?.info ?? null);
const managedVersion = computed(() => check.value?.managed_version ?? "");
/** Rendering is only impossible without a usable Blender — Outdated still renders. */
const renderBlocked = computed(
  () => verdict.value === "Missing" || verdict.value === "TooOld",
);

const isDownloading = computed(
  () =>
    !!activeJobId.value ||
    (status.value !== null &&
      ("Started" in status.value ||
        "Progress" in status.value ||
        "Extracting" in status.value)),
);
const percent = computed(() => {
  if (status.value && "Progress" in status.value)
    return status.value.Progress.percent;
  if (
    status.value &&
    ("Extracting" in status.value || "Completed" in status.value)
  )
    return 100;
  return 0;
});
const downloadedBytes = computed(() =>
  status.value && "Progress" in status.value
    ? status.value.Progress.downloaded_bytes
    : 0,
);
const totalBytes = computed(() =>
  status.value && "Progress" in status.value
    ? status.value.Progress.total_bytes
    : 0,
);
/** Post-download phase: "verify" | "extract" | "install", else null. */
const phase = computed(() =>
  status.value && "Extracting" in status.value
    ? status.value.Extracting.phase
    : null,
);
const errorMessage = computed(() =>
  status.value && "Failed" in status.value ? status.value.Failed.error : null,
);
const installedInfo = computed(() =>
  status.value && "Completed" in status.value
    ? status.value.Completed.info
    : null,
);

const startDownload = async () => {
  ensureListener();
  status.value = null;
  const result = await commands.downloadBlender();
  if (result.status === "ok") {
    activeJobId.value = result.data;
  }
  return result;
};

const cancelDownload = async () => {
  if (!activeJobId.value) return;
  await commands.cancelBlenderDownload(activeJobId.value);
};

/**
 * Record that the user completed or dismissed setup for this managed
 * version, so the first-run dialog stays quiet until the pin bumps.
 */
const acknowledge = async (version: string) => {
  const current = await commands.getSettings();
  if (current.status !== "ok") return;
  await commands.setSettings({
    ...current.data,
    blender_setup_acknowledged: version,
  });
};

/** A managed install just landed — an explicit blender_path would keep outranking it. */
const clearBlenderPathSetting = async () => {
  const current = await commands.getSettings();
  if (current.status !== "ok" || !current.data.blender_path) return;
  await commands.setSettings({ ...current.data, blender_path: null });
};

const openDialog = () => {
  ensureListener();
  dialogVisible.value = true;
};
const closeDialog = () => {
  dialogVisible.value = false;
};

export function useBlenderProvision() {
  ensureListener();
  return {
    check,
    checking,
    verdict,
    blenderInfo,
    managedVersion,
    renderBlocked,
    status,
    isDownloading,
    percent,
    downloadedBytes,
    totalBytes,
    phase,
    errorMessage,
    installedInfo,
    runCheck,
    startDownload,
    cancelDownload,
    acknowledge,
    clearBlenderPathSetting,
    dialogVisible,
    openDialog,
    closeDialog,
  };
}
