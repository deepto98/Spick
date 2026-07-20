import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getMicrophonePermissionStatus,
  requestMicrophonePermission,
} from "./nativeMicrophone";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("native microphone permission", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("checks permission without prompting", async () => {
    const status = { state: "missing" as const, canRequest: true };
    vi.mocked(invoke).mockResolvedValue(status);

    await expect(getMicrophonePermissionStatus()).resolves.toEqual(status);
    expect(invoke).toHaveBeenCalledWith("get_microphone_permission_status");
  });

  it("keeps the explicit request separate", async () => {
    const status = { state: "granted" as const, canRequest: false };
    vi.mocked(invoke).mockResolvedValue(status);

    await expect(requestMicrophonePermission()).resolves.toEqual(status);
    expect(invoke).toHaveBeenCalledWith("request_microphone_permission");
  });
});
