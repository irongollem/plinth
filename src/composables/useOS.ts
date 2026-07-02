import { type } from "@tauri-apps/plugin-os";
import { ref } from "vue";

export function useOS() {
  const osType = ref(type());
  const fileExplorerName = ref("File Explorer");

  (async () => {
    switch (osType.value) {
      case "windows":
        fileExplorerName.value = "File Explorer";
        break;
      case "linux":
        fileExplorerName.value = "Nautilus";
        break;
      case "macos":
        fileExplorerName.value = "Finder";
        break;
      default:
        fileExplorerName.value = "Unknown";
    }
  })();

  return {
    fileExplorerName,
    osType,
  };
}
