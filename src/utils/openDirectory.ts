import { openPath } from "@tauri-apps/plugin-opener";

/** Open a directory itself; unlike revealItemInDir this also has sensible
 * semantics for filesystem roots such as `Z:\\`. Keep paths opaque so mapped
 * drives, UNC shares, and POSIX folders reach the platform plugin unchanged. */
export const openDirectoryPath = (path: string) => openPath(path);
