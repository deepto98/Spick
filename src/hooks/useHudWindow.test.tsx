import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { StrictMode, type ReactNode } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NativeHudSettings } from "../lib/nativeSettings";
import { useHudWindow } from "./useHudWindow";

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  markReady: vi.fn(),
  setPresentation: vi.fn(),
  setHovered: vi.fn(),
  startDrag: vi.fn(),
}));

vi.mock("../lib/nativeHud", () => ({
  getHudSettings: mocks.getSettings,
  markHudRendererReady: mocks.markReady,
  setHudPresentation: mocks.setPresentation,
  setHudHovered: mocks.setHovered,
  startHudDrag: mocks.startDrag,
}));

const compactSettings: NativeHudSettings = {
  position: "bottomRight",
  presentation: "compact",
  customPosition: { x: 120, y: 80 },
  visible: true,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, reject, resolve };
}

describe("HUD renderer hydration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.markReady.mockResolvedValue(undefined);
    mocks.setHovered.mockResolvedValue(undefined);
    mocks.startDrag.mockResolvedValue(undefined);
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("releases the native visibility gate only after settings are committed", async () => {
    const settings = deferred<NativeHudSettings>();
    mocks.getSettings.mockReturnValue(settings.promise);

    const { result } = renderHook(() => useHudWindow(true));

    expect(result.current.settings).toBeNull();
    expect(mocks.markReady).not.toHaveBeenCalled();

    await act(async () => settings.resolve(compactSettings));

    await waitFor(() => expect(result.current.compact).toBe(true));
    expect(mocks.markReady).toHaveBeenCalledOnce();
  });

  it("keeps the native HUD gated when settings hydration fails", async () => {
    mocks.getSettings.mockRejectedValue(new Error("settings unavailable"));

    const { result } = renderHook(() => useHudWindow(true));

    await waitFor(() =>
      expect(result.current.error).toContain("settings unavailable"),
    );
    expect(result.current.settings).toBeNull();
    expect(mocks.markReady).not.toHaveBeenCalled();
  });

  it("does not initialize the HUD bridge outside the HUD window", () => {
    renderHook(() => useHudWindow(false));

    expect(mocks.getSettings).not.toHaveBeenCalled();
    expect(mocks.markReady).not.toHaveBeenCalled();
  });

  it("retries a transient readiness acknowledgement", async () => {
    vi.useFakeTimers();
    mocks.getSettings.mockResolvedValue(compactSettings);
    mocks.markReady
      .mockRejectedValueOnce(new Error("bridge busy"))
      .mockResolvedValueOnce(undefined);

    const { result } = renderHook(() => useHudWindow(true));
    await act(async () => Promise.resolve());
    expect(mocks.markReady).toHaveBeenCalledOnce();

    await act(async () => vi.advanceTimersByTimeAsync(100));

    expect(mocks.markReady).toHaveBeenCalledTimes(2);
    expect(result.current.error).toBeNull();
  });

  it("still acknowledges after React replays layout effects in development", async () => {
    mocks.getSettings.mockResolvedValue(compactSettings);
    const wrapper = ({ children }: { children: ReactNode }) => (
      <StrictMode>{children}</StrictMode>
    );

    const { result } = renderHook(() => useHudWindow(true), { wrapper });

    await waitFor(() => expect(result.current.compact).toBe(true));
    await waitFor(() => expect(mocks.markReady).toHaveBeenCalled());
    expect(result.current.error).toBeNull();
  });

  it("lets a quick drag win over hover expansion", async () => {
    vi.useFakeTimers();
    mocks.getSettings.mockResolvedValue(compactSettings);
    const drag = deferred<void>();
    mocks.startDrag.mockReturnValue(drag.promise);

    const { result } = renderHook(() => useHudWindow(true));
    await act(async () => Promise.resolve());

    act(() => result.current.setHovered(true));
    act(() => result.current.beginDrag());
    await act(async () => vi.advanceTimersByTimeAsync(160));

    expect(mocks.setHovered).not.toHaveBeenCalled();
    act(() => result.current.setHovered(false));
    await act(async () => drag.resolve());
    expect(mocks.setHovered).toHaveBeenLastCalledWith(false);
  });

  it("expands only after a deliberate hover", async () => {
    vi.useFakeTimers();
    mocks.getSettings.mockResolvedValue(compactSettings);
    const { result } = renderHook(() => useHudWindow(true));
    await act(async () => Promise.resolve());

    act(() => result.current.setHovered(true));
    await act(async () => vi.advanceTimersByTimeAsync(159));
    expect(mocks.setHovered).not.toHaveBeenCalled();

    await act(async () => vi.advanceTimersByTimeAsync(1));
    expect(mocks.setHovered).toHaveBeenCalledWith(true);
  });
});
