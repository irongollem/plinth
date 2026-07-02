import { defineStore } from "pinia";
import { ref } from "vue";
import { describeError } from "../utils/format";

export type ToastType = "success" | "error" | "warning" | "info";
export interface Toast {
  id: string;
  message: string;
  type: ToastType;
  duration: number;
}

export const useToastStore = defineStore("toast", () => {
  const toasts = ref<Toast[]>([]);
  let nextId = 0;

  const addToast = (
    message: string,
    type: ToastType = "info",
    duration = 5000,
  ) => {
    const id = `toast-${nextId++}`;
    const toast = { id, message, type, duration };
    toasts.value.push(toast);
    if (duration > 0) {
      setTimeout(() => {
        removeToast(id);
      }, duration);
    }
    return id;
  };

  const removeToast = (id: string) => {
    toasts.value = toasts.value.filter((toast) => toast.id !== id);
  };

  /**
   * One place for the "console.error + sticky error toast" pattern, with
   * AppError objects rendered readably instead of [object Object].
   */
  const reportError = (message: string, error: unknown) => {
    console.error(message, error);
    return addToast(`${message}: ${describeError(error)}`, "error", 0);
  };

  return { toasts, addToast, removeToast, reportError };
});
