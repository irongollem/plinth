export const fileLogos = {
  chitubox: "/chitubox.jpg",
  lychee: "/lychee.jpg",
  stl: "/stl.jpg",
};

/**
 * Single source of truth mapping file extensions (including slicer aliases)
 * to their logo. Add new formats here — AddSTL's icon list and any future
 * pickers read this map instead of keeping their own extension lists.
 */
export const extensionLogos: Record<string, string> = {
  stl: fileLogos.stl,
  chitu: fileLogos.chitubox,
  chitubox: fileLogos.chitubox,
  lyt: fileLogos.lychee,
  lys: fileLogos.lychee,
  lychee: fileLogos.lychee,
};

export const logoForFileName = (fileName: string): string => {
  const ext = fileName.split(".").pop()?.toLowerCase() ?? "";
  return extensionLogos[ext] ?? "tauri.svg";
};
