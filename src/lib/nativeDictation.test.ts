import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import type { HudState } from "../types";
import {
  startDictationSession,
  toHudState,
  type NativeSessionState,
} from "./nativeDictation";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));

describe("native dictation state mapping", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it.each<[NativeSessionState, HudState]>([
    ["idle", "idle"],
    ["listening", "listening"],
    ["processing", "processing"],
    ["inserting", "inserting"],
    ["completed", "success"],
    ["cancelled", "idle"],
    ["failed", "error"],
  ])("maps %s to the %s HUD state", (nativeState, hudState) => {
    expect(toHudState(nativeState)).toBe(hudState);
  });

  it("lets the native command derive targeting from the invoking window", async () => {
    vi.mocked(invoke).mockResolvedValue({ revision: 1, state: "listening" });

    await startDictationSession();

    expect(invoke).toHaveBeenCalledWith("start_dictation_session");
  });
});
