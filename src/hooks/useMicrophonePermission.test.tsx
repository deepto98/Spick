import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useMicrophonePermission } from "./useMicrophonePermission";

const nativeMocks = vi.hoisted(() => ({
  getStatus: vi.fn(),
  request: vi.fn(),
}));

vi.mock("../lib/nativeMicrophone", () => ({
  getMicrophonePermissionStatus: nativeMocks.getStatus,
  requestMicrophonePermission: nativeMocks.request,
}));

describe("microphone permission state", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    nativeMocks.getStatus.mockReset();
    nativeMocks.request.mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("does not prompt during a status refresh", async () => {
    const missing = { state: "missing" as const, canRequest: true };
    nativeMocks.getStatus.mockResolvedValue(missing);
    const { result } = renderHook(() => useMicrophonePermission(true));

    await act(async () => result.current.refresh());

    expect(result.current.status).toEqual(missing);
    expect(nativeMocks.request).not.toHaveBeenCalled();
  });

  it("updates from the explicit permission result", async () => {
    const granted = { state: "granted" as const, canRequest: false };
    nativeMocks.request.mockResolvedValue(granted);
    const { result } = renderHook(() => useMicrophonePermission(true));

    await act(async () => result.current.request());

    expect(result.current.status).toEqual(granted);
  });

  it("reports unsupported outside the native app", async () => {
    const { result } = renderHook(() => useMicrophonePermission(false));

    await act(async () => result.current.refresh());

    expect(result.current.status).toEqual({
      state: "unsupported",
      canRequest: false,
    });
    expect(nativeMocks.getStatus).not.toHaveBeenCalled();
  });
});
