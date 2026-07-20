import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { HudState } from "../types";
import {
  DICTATION_LATENCY_EVENT,
  getLastDictationLatency,
  isValidDictationLatencyEvent,
  startDictationSession,
  subscribeToDictationLatency,
  toHudState,
  type NativeDictationLatencyEvent,
  type NativeSessionState,
} from "./nativeDictation";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(),
}));

const latency: NativeDictationLatencyEvent = {
  sessionId: "opaque-session",
  revision: 4,
  outcome: "completed",
  audioDurationMs: 2_400,
  stopToProcessingMs: 3,
  captureFinalizeMs: 10,
  transcriptionMs: 80,
  deliveryMs: 6,
  stopToDeliveryMs: 101,
  processingTotalMs: 104,
};

describe("native dictation state mapping", () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset();
    vi.mocked(listen).mockReset();
  });

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

  it("forwards privacy-safe latency events from the native listener", async () => {
    const handler = vi.fn();
    const unlisten = vi.fn();
    vi.mocked(listen).mockResolvedValue(unlisten);

    await subscribeToDictationLatency(handler);

    expect(listen).toHaveBeenCalledWith(
      DICTATION_LATENCY_EVENT,
      expect.any(Function),
    );
    const listener = vi.mocked(listen).mock.calls[0]?.[1] as (event: {
      payload: NativeDictationLatencyEvent;
    }) => void;
    listener({ payload: latency });
    expect(handler).toHaveBeenCalledWith(latency);
  });

  it("reads the latest process-memory timing without persistence", async () => {
    vi.mocked(invoke).mockResolvedValue(latency);

    await expect(getLastDictationLatency()).resolves.toEqual(latency);
    expect(invoke).toHaveBeenCalledWith("get_last_dictation_latency");
  });

  it("rejects malformed or internally inconsistent latency payloads", () => {
    expect(isValidDictationLatencyEvent(latency)).toBe(true);
    expect(
      isValidDictationLatencyEvent({ ...latency, transcriptionMs: -1 }),
    ).toBe(false);
    expect(
      isValidDictationLatencyEvent({
        ...latency,
        deliveryMs: latency.processingTotalMs + 1,
      }),
    ).toBe(false);
    expect(
      isValidDictationLatencyEvent({
        ...latency,
        revision: Number.MAX_SAFE_INTEGER + 1,
      }),
    ).toBe(false);
    expect(isValidDictationLatencyEvent(null)).toBe(false);
    expect(isValidDictationLatencyEvent({})).toBe(false);
  });
});
