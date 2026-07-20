import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  getShortcutStatus,
  requestInputMonitoringPermission,
} from "./nativeShortcut";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("native Option shortcut", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("reads listener and fallback state", async () => {
    const status = {
      optionSelected: true,
      optionListenerActive: false,
      inputMonitoringGranted: false,
      inputMonitoringAccess: "denied" as const,
      fallbackShortcut: "CommandOrControl+Shift+Space",
    };
    vi.mocked(invoke).mockResolvedValueOnce(status);

    await expect(getShortcutStatus()).resolves.toEqual(status);
    expect(invoke).toHaveBeenCalledWith("get_shortcut_status");
  });

  it("requests Input Monitoring through the native process", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(true);

    await expect(requestInputMonitoringPermission()).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("request_input_monitoring_permission");
  });
});
