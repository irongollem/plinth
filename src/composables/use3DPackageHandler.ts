import { listen } from "@tauri-apps/api/event";
import { confirm } from "@tauri-apps/plugin-dialog";
import { onMounted, onUnmounted } from "vue";
import { commands } from "../bindings";
import { useToastStore } from "../stores/toastStore";
import { selectDirectory } from "./useFileSelect";

/**
 * The receiving end of the 3pk format: a `release.3pk` arriving via OS file
 * association or drag-drop imports into the library — component archives
 * verified against their manifest checksums, dedup-elided files
 * rematerialized — and a catalog scan restores the packed curation.
 */
export function use3DPackageHandler() {
  const toastStore = useToastStore();
  let unlistenFn: (() => void) | null = null;

  const handle3DPackage = async (filePath: string) => {
    try {
      const confirmed = await confirm(
        `Import this release into your library?\n\n${filePath}\n\nEvery file is verified against the release's checksums first.`,
        { title: "Import release", kind: "info" },
      );
      if (!confirmed) return;

      // Default destination: the catalog root, so the release is scanned
      // like everything else; ask only when none is configured
      const settings = await commands.getSettings();
      const library =
        (settings.status === "ok" && settings.data.catalog_root) ||
        (await selectDirectory({ title: "Import into which folder?" }));
      if (!library) return;

      const result = await commands.importRelease(filePath, library);
      if (result.status !== "ok") {
        toastStore.reportError("Import failed", result.error);
        return;
      }
      const outcome = result.data;
      for (const error of outcome.errors) toastStore.addToast(error, "error");
      toastStore.addToast(
        `Imported "${outcome.release_name}" by ${outcome.designer} — ${outcome.components} component${outcome.components === 1 ? "" : "s"}, ${outcome.files} files, verified`,
        "success",
      );

      // Index it right away when it landed inside the catalog root; the
      // scan also restores the packed curation from the model.json sidecars
      if (
        settings.status === "ok" &&
        settings.data.catalog_root &&
        library === settings.data.catalog_root
      ) {
        await commands.startCatalogScan(settings.data.catalog_root);
      }
    } catch (error) {
      toastStore.reportError("Failed to import 3D package", error);
    }
  };

  onMounted(async () => {
    // Drag-drop on a running app
    unlistenFn = await listen("3dpak-open", (event) => {
      handle3DPackage(event.payload as string);
    });

    // A file opened via OS file association arrives before this listener
    // exists (Tauri events don't queue), so the backend parks it for us
    try {
      const pending = await commands.getPending3dpak();
      if (pending) await handle3DPackage(pending);
    } catch (error) {
      console.error("Failed to check for a pending 3D package:", error);
    }
  });

  onUnmounted(() => {
    unlistenFn?.();
  });

  return {
    handle3DPackage,
  };
}
