import { openPath } from "@tauri-apps/plugin-opener";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { openDirectoryPath } from "./openDirectory";

vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: vi.fn(),
}));

describe("openDirectoryPath", () => {
  beforeEach(() => vi.mocked(openPath).mockReset());

  it.each(["Z:\\", "Z:\\Designer\\Release", "/Users/me/Models"])(
    "opens the directory itself without rewriting %s",
    async (path) => {
      await openDirectoryPath(path);
      expect(openPath).toHaveBeenCalledOnce();
      expect(openPath).toHaveBeenCalledWith(path);
    },
  );
});
