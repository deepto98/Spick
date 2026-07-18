import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { HudState } from "../types";

export const DICTATION_STATE_EVENT = "dictation://state";

export type NativeSessionState =
  "idle" | "listening" | "processing" | "completed" | "cancelled" | "failed";

export interface NativeDictationSession {
  id: string;
  state: NativeSessionState;
  startedAtMs: number;
  endedAtMs: number | null;
  cancelReason: string | null;
  error: string | null;
}

export interface NativeDictationStateEvent {
  state: NativeSessionState;
  session: NativeDictationSession | null;
}

export function hasNativeRuntime() {
  return isTauri();
}

export function toHudState(state: NativeSessionState): HudState {
  switch (state) {
    case "listening":
      return "listening";
    case "processing":
      return "processing";
    case "completed":
      return "success";
    case "idle":
    case "cancelled":
    case "failed":
      return "idle";
  }
}

export function getDictationSession() {
  return invoke<NativeDictationStateEvent>("get_dictation_session");
}

export function startDictationSession() {
  return invoke<NativeDictationStateEvent>("start_dictation_session", {
    trigger: "userInterface",
  });
}

export function stopDictationSession() {
  return invoke<NativeDictationStateEvent>("stop_dictation_session");
}

export function completeDictationSession() {
  return invoke<NativeDictationStateEvent>("complete_dictation_session");
}

export function subscribeToDictationState(
  handler: (state: NativeDictationStateEvent) => void,
): Promise<UnlistenFn> {
  return listen<NativeDictationStateEvent>(DICTATION_STATE_EVENT, (event) => {
    handler(event.payload);
  });
}
