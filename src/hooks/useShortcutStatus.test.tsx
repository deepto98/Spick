import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useShortcutStatus } from "./useShortcutStatus";

const nativeMocks = vi.hoisted(() => ({
  getShortcutStatus: vi.fn(),
  requestInputMonitoringPermission: vi.fn(),
}));

vi.mock("../lib/nativeShortcut", () => ({
  getShortcutStatus: nativeMocks.getShortcutStatus,
  requestInputMonitoringPermission:
    nativeMocks.requestInputMonitoringPermission,
}));

describe("Option shortcut status", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    nativeMocks.getShortcutStatus.mockReset();
    nativeMocks.requestInputMonitoringPermission.mockReset();
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("activates Option when permission is granted without requiring a fallback", async () => {
    const waiting = {
      optionSelected: true,
      optionListenerActive: false,
      inputMonitoringGranted: true,
      fallbackShortcut: null,
    };
    const active = {
      ...waiting,
      optionListenerActive: true,
    };
    nativeMocks.getShortcutStatus
      .mockResolvedValueOnce(waiting)
      .mockResolvedValueOnce(active);
    nativeMocks.requestInputMonitoringPermission.mockResolvedValueOnce(true);

    const { result } = renderHook(() => useShortcutStatus(true));

    await act(async () => {
      await result.current.refresh();
    });

    expect(nativeMocks.requestInputMonitoringPermission).toHaveBeenCalledOnce();
    expect(nativeMocks.getShortcutStatus).toHaveBeenCalledTimes(2);
    expect(result.current.status).toEqual(active);
  });

  it("does not activate Option when a custom shortcut is selected", async () => {
    const custom = {
      optionSelected: false,
      optionListenerActive: false,
      inputMonitoringGranted: true,
      fallbackShortcut: null,
    };
    nativeMocks.getShortcutStatus.mockResolvedValueOnce(custom);

    const { result } = renderHook(() => useShortcutStatus(true));

    await act(async () => {
      await result.current.refresh();
    });

    expect(nativeMocks.requestInputMonitoringPermission).not.toHaveBeenCalled();
    expect(nativeMocks.getShortcutStatus).toHaveBeenCalledOnce();
    expect(result.current.status).toEqual(custom);
  });
});
