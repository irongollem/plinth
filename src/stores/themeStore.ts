import { defineStore } from "pinia";
import { ref, watchEffect } from "vue";
import {
  syncNativeWindowTheme,
  type ThemeName,
} from "../utils/nativeWindowTheme";

export type { ThemeName } from "../utils/nativeWindowTheme";

const STORAGE_KEY = "plinth-theme";

const readStoredTheme = (): ThemeName => {
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === "plinth-light" ? "plinth-light" : "plinth";
};

/**
 * Dark/light appearance. This is a pure display preference — unlike
 * scratch_dir/target_dir etc. it doesn't affect file operations, so it's
 * kept as a frontend-only, localStorage-persisted setting rather than a
 * round-trip through the Rust settings store.
 */
export const useThemeStore = defineStore("theme", () => {
  const theme = ref<ThemeName>(readStoredTheme());

  watchEffect(() => {
    document.documentElement.setAttribute("data-theme", theme.value);
    localStorage.setItem(STORAGE_KEY, theme.value);
    void syncNativeWindowTheme(theme.value).catch((error) => {
      console.warn("Failed to synchronize the native window theme", error);
    });
  });

  const setDark = () => {
    theme.value = "plinth";
  };
  const setLight = () => {
    theme.value = "plinth-light";
  };
  const isDark = () => theme.value === "plinth";

  return { theme, setDark, setLight, isDark };
});
