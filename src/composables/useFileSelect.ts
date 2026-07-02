import { convertFileSrc } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { stat } from "@tauri-apps/plugin-fs";
import type { FileInfo } from "@tauri-apps/plugin-fs";
import { ref } from "vue";
import { formatFileSize } from "../utils/format";

export interface SelectedFile {
  path: string; // The full file path
  name: string; // The file name
  info: FileInfo; // The standard Tauri FileInfo
  fileType?: string; // Our custom file type/extension

  getPreviewUrl: () => string;
}

export interface FileFilter {
  name: string;
  extensions: string[];
}

/** Build SelectedFile entries from known paths (dialog-free). */
export async function filesFromPaths(paths: string[]): Promise<SelectedFile[]> {
  // Stat all paths concurrently; unreadable files drop out with a log
  const files = await Promise.all(
    paths.map(async (path) => {
      try {
        const fileInfo = await stat(path);
        const fileName = path.split(/[/\\]/).pop() || "";
        const extension = fileName.split(".").pop()?.toLowerCase() || "";

        const file: SelectedFile = {
          path,
          name: fileName,
          info: fileInfo,
          fileType: extension ? `.${extension}` : "Unknown",
          getPreviewUrl() {
            return convertFileSrc(this.path);
          },
        };
        return file;
      } catch (error) {
        console.error(`Failed to read file metadata for ${path}:`, error);
        return null;
      }
    }),
  );
  return files.filter((file): file is SelectedFile => file !== null);
}

/** Open a native directory picker; resolves to the chosen path or null. */
export async function selectDirectory(options: {
  title?: string;
}): Promise<string | null> {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: options.title || "Select Directory",
    });

    return selected ? (selected as string) : null;
  } catch (error) {
    console.error("Directory selection failed:", error);
    return null;
  }
}

export function useFileSelect() {
  const selectedFiles = ref<SelectedFile[]>([]);

  const createFiltersFromAccept = (
    accept: string,
  ): FileFilter[] | undefined => {
    if (!accept) return undefined;

    const filters: FileFilter[] = [];
    const allExtensions: string[] = [];

    const extensions = accept
      .split(",")
      .map((type) => type.trim())
      .map((type) => (type.startsWith(".") ? type.substring(1) : type));

    if (extensions.length) {
      allExtensions.push(...extensions);
    }

    const images = extensions.filter((ext) =>
      ["jpg", "jpeg", "png", "gif", "svg", "webp", "avif", "bmp"].includes(ext),
    );
    const models = extensions.filter((ext) =>
      ["stl", "obj", "3mf", "lys", "chitubox", "blend", "gcode"].includes(ext),
    );
    const documents = extensions.filter((ext) =>
      ["txt", "md", "json", "csv", "xml", "pdf"].includes(ext),
    );

    if (images.length) {
      filters.push({
        name: "Images",
        extensions: images,
      });
    }
    if (models.length) {
      filters.push({
        name: "Models",
        extensions: models,
      });
    }
    if (documents.length) {
      filters.push({
        name: "Documents",
        extensions: documents,
      });
    }

    if (allExtensions.length) {
      filters.push({
        name: "All Supported",
        extensions: allExtensions,
      });
    }

    return filters.length ? filters : undefined;
  };

  const selectFiles = async (options: {
    accept?: string;
    multiple?: boolean;
    title?: string;
  }): Promise<SelectedFile[] | null> => {
    try {
      const filters = options.accept
        ? createFiltersFromAccept(options.accept)
        : undefined;

      const selected = await open({
        multiple: options.multiple,
        filters,
        title: options.title || "Select Files",
      });

      if (!selected) return null;

      const paths = Array.isArray(selected) ? selected : [selected];
      return await filesFromPaths(paths);
    } catch (error) {
      console.error("Selection failed:", error);
      return null;
    }
  };

  return {
    selectedFiles,
    selectFiles,
    selectDirectory,
    formatFileSize,
    createFiltersFromAccept,
  };
}
