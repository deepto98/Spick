import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  NativeDictationLatencyEvent,
  NativeDictationStateEvent,
} from "../lib/nativeDictation";
import { useDictationController } from "./useDictationController";

const nativeMocks = vi.hoisted(() => ({
  getLatency: vi.fn(),
  getSession: vi.fn(),
  getTranscript: vi.fn(),
  subscribeLatency: vi.fn(),
  subscribeState: vi.fn(),
  subscribeTranscript: vi.fn(),
  stateHandler: null as ((event: NativeDictationStateEvent) => void) | null,
  latencyHandler: null as ((event: NativeDictationLatencyEvent) => void) | null,
  startSession: vi.fn(),
  unlistenLatency: vi.fn(),
  unlistenState: vi.fn(),
  unlistenTranscript: vi.fn(),
}));

vi.mock("../lib/nativeDictation", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../lib/nativeDictation")>();
  return {
    ...actual,
    getDictationSession: nativeMocks.getSession,
    getLastDictationLatency: nativeMocks.getLatency,
    getLastTranscript: nativeMocks.getTranscript,
    hasNativeRuntime: () => true,
    startDictationSession: nativeMocks.startSession,
    subscribeToDictationLatency: nativeMocks.subscribeLatency,
    subscribeToDictationState: nativeMocks.subscribeState,
    subscribeToDictationTranscript: nativeMocks.subscribeTranscript,
  };
});

const idle: NativeDictationStateEvent = {
  revision: 0,
  state: "idle",
  session: null,
};

const latency: NativeDictationLatencyEvent = {
  sessionId: "opaque-session",
  revision: 2,
  outcome: "completed",
  targetCaptureMs: 8,
  startToTargetCaptureReturnMs: 10,
  startToAudioOwnerSpawnMs: 14,
  startToStartingEmittedMs: 15,
  startToHudShowReturnMs: 21,
  startToMicrophoneReadyMs: 19,
  startToListeningEmittedMs: 23,
  audioDurationMs: 1_900,
  stopToProcessingMs: 3,
  captureFinalizeMs: 12,
  transcriptionMs: 90,
  deliveryMs: 5,
  stopToDeliveryMs: 112,
  processingTotalMs: 116,
};

describe("dictation latency diagnostics", () => {
  beforeEach(() => {
    nativeMocks.stateHandler = null;
    nativeMocks.latencyHandler = null;
    nativeMocks.getSession.mockReset();
    nativeMocks.getSession.mockResolvedValue(idle);
    nativeMocks.getLatency.mockReset();
    nativeMocks.getLatency.mockResolvedValue(null);
    nativeMocks.getTranscript.mockReset();
    nativeMocks.getTranscript.mockResolvedValue(null);
    nativeMocks.startSession.mockReset();
    nativeMocks.startSession.mockResolvedValue({
      ...idle,
      revision: 1,
      state: "starting",
    });
    nativeMocks.subscribeState.mockReset();
    nativeMocks.subscribeState.mockImplementation(async (handler) => {
      nativeMocks.stateHandler = handler;
      return nativeMocks.unlistenState;
    });
    nativeMocks.subscribeTranscript.mockReset();
    nativeMocks.subscribeTranscript.mockResolvedValue(
      nativeMocks.unlistenTranscript,
    );
    nativeMocks.subscribeLatency.mockReset();
    nativeMocks.subscribeLatency.mockImplementation(async (handler) => {
      nativeMocks.latencyHandler = handler;
      return nativeMocks.unlistenLatency;
    });
    nativeMocks.unlistenLatency.mockReset();
    nativeMocks.unlistenState.mockReset();
    nativeMocks.unlistenTranscript.mockReset();
  });

  afterEach(cleanup);

  it("keeps the newest valid processing result and removes every listener", async () => {
    const { result, unmount } = renderHook(() => useDictationController(true));
    await waitFor(() => expect(nativeMocks.latencyHandler).not.toBeNull());

    act(() => {
      // Recorder revisions can legitimately advance before the previous
      // terminal timing is delivered; only timing revisions order timings.
      nativeMocks.stateHandler?.({ ...idle, revision: 3, state: "listening" });
      nativeMocks.latencyHandler?.(latency);
    });
    expect(result.current.lastLatency).toEqual(latency);

    act(() => {
      nativeMocks.stateHandler?.({ ...idle, revision: 3, state: "listening" });
      nativeMocks.latencyHandler?.({ ...latency, revision: 1 });
      nativeMocks.latencyHandler?.({
        ...latency,
        revision: 4,
        transcriptionMs: -1,
      });
    });
    expect(result.current.lastLatency).toEqual(latency);

    unmount();
    expect(nativeMocks.unlistenState).toHaveBeenCalledOnce();
    expect(nativeMocks.unlistenTranscript).toHaveBeenCalledOnce();
    expect(nativeMocks.unlistenLatency).toHaveBeenCalledOnce();
  });

  it("subscribes before replaying the latest in-memory result", async () => {
    nativeMocks.getLatency.mockResolvedValueOnce(latency);
    const { result } = renderHook(() => useDictationController(true));

    await waitFor(() => expect(result.current.lastLatency).toEqual(latency));
    expect(nativeMocks.subscribeLatency).toHaveBeenCalledOnce();
    expect(nativeMocks.getLatency).toHaveBeenCalledOnce();
    expect(
      nativeMocks.subscribeLatency.mock.invocationCallOrder[0],
    ).toBeLessThan(nativeMocks.getLatency.mock.invocationCallOrder[0]);
  });

  it("does not let a slower cache replay replace a newer live timing", async () => {
    let resolveCached!: (value: unknown) => void;
    nativeMocks.getLatency.mockReturnValueOnce(
      new Promise<unknown>((resolve) => {
        resolveCached = resolve;
      }),
    );
    const { result } = renderHook(() => useDictationController(true));
    await waitFor(() => expect(nativeMocks.latencyHandler).not.toBeNull());
    const newer = { ...latency, revision: latency.revision + 1 };

    act(() => nativeMocks.latencyHandler?.(newer));
    await act(async () => resolveCached(latency));

    expect(result.current.lastLatency).toEqual(newer);
  });

  it("keeps optional diagnostics away from the HUD connection", async () => {
    renderHook(() => useDictationController(false));
    await waitFor(() => expect(nativeMocks.getSession).toHaveBeenCalledOnce());

    expect(nativeMocks.subscribeTranscript).not.toHaveBeenCalled();
    expect(nativeMocks.subscribeLatency).not.toHaveBeenCalled();
    expect(nativeMocks.getLatency).not.toHaveBeenCalled();
  });

  it("keeps the UI in startup until native microphone readiness arrives", async () => {
    const { result } = renderHook(() => useDictationController(true));
    await waitFor(() => expect(nativeMocks.getSession).toHaveBeenCalledOnce());

    act(() => result.current.transitionTo("listening"));

    await waitFor(() => expect(result.current.state).toBe("starting"));
    expect(nativeMocks.startSession).toHaveBeenCalledOnce();
  });

  it("does not turn a diagnostics-listener failure into a recorder error", async () => {
    nativeMocks.subscribeLatency.mockRejectedValueOnce(
      new Error("diagnostics unavailable"),
    );
    const { result } = renderHook(() => useDictationController(true));

    await waitFor(() => expect(nativeMocks.getSession).toHaveBeenCalledOnce());
    expect(nativeMocks.getLatency).toHaveBeenCalledOnce();
    expect(result.current.error).toBeNull();
  });
});
