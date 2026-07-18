import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getAccessibilityPermissionStatus,
  requestAccessibilityPermission,
} from "./nativeAccessibility";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

describe("native Accessibility permission", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
  });

  it("queries without prompting", async () => {
    vi.mocked(invoke).mockResolvedValue({ state: "missing", canRequest: true });

    await expect(getAccessibilityPermissionStatus()).resolves.toEqual({
      state: "missing",
      canRequest: true,
    });
    expect(invoke).toHaveBeenCalledWith("get_accessibility_permission_status");
  });

  it("uses a separate explicit request command", async () => {
    vi.mocked(invoke).mockResolvedValue({
      state: "granted",
      canRequest: false,
    });

    await requestAccessibilityPermission();

    expect(invoke).toHaveBeenCalledWith("request_accessibility_permission");
  });
});
