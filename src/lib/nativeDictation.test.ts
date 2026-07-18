import { describe, expect, it } from "vitest";
import type { HudState } from "../types";
import { toHudState, type NativeSessionState } from "./nativeDictation";

describe("native dictation state mapping", () => {
  it.each<[NativeSessionState, HudState]>([
    ["idle", "idle"],
    ["listening", "listening"],
    ["processing", "processing"],
    ["completed", "success"],
    ["cancelled", "idle"],
    ["failed", "idle"],
  ])("maps %s to the %s HUD state", (nativeState, hudState) => {
    expect(toHudState(nativeState)).toBe(hudState);
  });
});
