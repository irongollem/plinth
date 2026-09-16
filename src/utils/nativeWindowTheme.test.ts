import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  isTauri: vi.fn(),
  setTheme: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ isTauri: mocks.isTauri }));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({ setTheme: mocks.setTheme }),
}));

import { nativeThemeFor, syncNativeWindowTheme } from "./nativeWindowTheme";

describe("native window theme", () => {
  beforeEach(() => {
    mocks.isTauri.mockReset();
    mocks.setTheme.mockReset();
  });

  it("maps both Plinth themes to Tauri's native theme names", () => {
    expect(nativeThemeFor("plinth")).toBe("dark");
    expect(nativeThemeFor("plinth-light")).toBe("light");
  });

  it.each([
    ["plinth", "dark"],
    ["plinth-light", "light"],
  ] as const)("synchronizes %s to the native window", async (theme, native) => {
    mocks.isTauri.mockReturnValue(true);
    await syncNativeWindowTheme(theme);
    expect(mocks.setTheme).toHaveBeenCalledOnce();
    expect(mocks.setTheme).toHaveBeenCalledWith(native);
  });

  it("does nothing in a browser-only preview", async () => {
    mocks.isTauri.mockReturnValue(false);
    await syncNativeWindowTheme("plinth");
    expect(mocks.setTheme).not.toHaveBeenCalled();
  });
});
