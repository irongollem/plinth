import type { UnlistenFn } from "@tauri-apps/api/event";
import { computed, onMounted, onUnmounted, ref } from "vue";
import {
  events,
  type RenderOptions,
  type RenderStatus,
  commands,
} from "../bindings";

/**
 * Tracks the lifecycle of a Blender render job driven by the Rust render
 * engine (start_render / cancel_render + render-status events).
 */
export function useRenderStatus() {
  const status = ref<RenderStatus | null>(null);
  const activeJobId = ref<string>("");
  let unlisten: UnlistenFn | null = null;

  onMounted(async () => {
    unlisten = await events.renderStatus.listen((event) => {
      const payload = event.payload;
      const jobId =
        "Started" in payload
          ? payload.Started.job_id
          : "Progress" in payload
            ? payload.Progress.job_id
            : "Completed" in payload
              ? payload.Completed.job_id
              : "Failed" in payload
                ? payload.Failed.job_id
                : payload.Cancelled.job_id;
      if (activeJobId.value && jobId !== activeJobId.value) return;
      status.value = payload;
      if (
        "Completed" in payload ||
        "Failed" in payload ||
        "Cancelled" in payload
      ) {
        activeJobId.value = "";
      }
    });
  });

  onUnmounted(() => {
    unlisten?.();
    unlisten = null;
  });

  const isRendering = computed(
    () =>
      !!activeJobId.value ||
      (status.value !== null &&
        ("Started" in status.value || "Progress" in status.value)),
  );

  const percent = computed(() => {
    if (status.value && "Progress" in status.value) {
      return status.value.Progress.percent;
    }
    if (status.value && "Completed" in status.value) {
      return 100;
    }
    return 0;
  });

  const resultPath = computed(() =>
    status.value && "Completed" in status.value
      ? status.value.Completed.output_path
      : null,
  );

  const elapsedSeconds = computed(() =>
    status.value && "Completed" in status.value
      ? status.value.Completed.elapsed_seconds
      : null,
  );

  const errorMessage = computed(() =>
    status.value && "Failed" in status.value ? status.value.Failed.error : null,
  );

  const start = async (parts: string[], options: RenderOptions) => {
    status.value = null;
    const result = await commands.startRender(parts, options);
    if (result.status === "ok") {
      activeJobId.value = result.data;
    }
    return result;
  };

  const cancel = async () => {
    if (!activeJobId.value) return;
    await commands.cancelRender(activeJobId.value);
  };

  const reset = () => {
    status.value = null;
    activeJobId.value = "";
  };

  return {
    status,
    activeJobId,
    isRendering,
    percent,
    resultPath,
    elapsedSeconds,
    errorMessage,
    start,
    cancel,
    reset,
  };
}
