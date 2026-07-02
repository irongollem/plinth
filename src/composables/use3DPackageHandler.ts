import { listen } from "@tauri-apps/api/event";
import { onMounted, onUnmounted } from "vue";
import { commands } from "../bindings";
import { useToastStore } from "../stores/toastStore";
// import { useReleasesStore } from "../stores/releasesStore";

export function use3DPackageHandler() {
  const toastStore = useToastStore();
  // const releasesStore = useReleasesStore();
  let unlistenFn: (() => void) | null = null;

  const handle3DPackage = (filePath: string) => {
    try {
      toastStore.addToast(`3D package detected: ${filePath}`, "info");
      console.log(`3D package file received: ${filePath}`);

      // Here you would handle the UI navigation/display:
      // For example, you might want to:
      // 1. Navigate to a specific tab
      // 2. Pass the file path to a store
      // 3. Start the loading process

      // This is where your app-specific logic would go
      // releasesStore.loadPackageFromFile(filePath);
    } catch (error) {
      toastStore.addToast(`Failed to handle 3D package: ${error}`, "error", 0);
      console.error("Error handling 3D package:", error);
    }
  };

  onMounted(async () => {
    // Set up listener for 3D package open events (drag-drop on a running app)
    unlistenFn = await listen("3dpak-open", (event) => {
      const filePath = event.payload as string;
      handle3DPackage(filePath);
    });

    // A file opened via OS file association arrives before this listener
    // exists (Tauri events don't queue), so the backend parks it for us
    try {
      const pending = await commands.getPending3dpak();
      if (pending) {
        handle3DPackage(pending);
      }
    } catch (error) {
      console.error("Failed to check for a pending 3D package:", error);
    }
  });

  onUnmounted(() => {
    // Clean up listener when component unmounts
    if (unlistenFn) {
      unlistenFn();
    }
  });

  return {
    handle3DPackage,
  };
}
