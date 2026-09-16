import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow, type Theme } from "@tauri-apps/api/window";

export type ThemeName = "plinth" | "plinth-light";

export const nativeThemeFor = (theme: ThemeName): Theme =>
  theme === "plinth" ? "dark" : "light";

/** Keep Windows' native caption (and the equivalent native chrome on other
 * platforms) aligned with the theme rendered inside the webview. Browser-only
 * previews have no Tauri window and intentionally stop here. */
export const syncNativeWindowTheme = async (theme: ThemeName) => {
  if (!isTauri()) return;
  await getCurrentWindow().setTheme(nativeThemeFor(theme));
};
