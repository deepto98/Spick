import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { HudState } from "../types";
import type { NativeLanguagePolicy } from "./nativeSettings";

export const DICTATION_STATE_EVENT = "dictation://state";
export const DICTATION_TRANSCRIPT_EVENT = "dictation://transcript";

export type NativeSessionState =
  "idle" | "listening" | "processing" | "completed" | "cancelled" | "failed";

export interface NativeDictationSession {
  id: string;
  state: NativeSessionState;
  languagePolicy: NativeLanguagePolicy;
  startedAtMs: number;
  endedAtMs: number | null;
  cancelReason: string | null;
  error: string | null;
}

export interface NativeDictationStateEvent {
  revision: number;
  state: NativeSessionState;
  session: NativeDictationSession | null;
}

export interface NativeTranscriptSegment {
  text: string;
  startMs: number;
  endMs: number;
  language: string | null;
  confidence: number | null;
}

export interface NativeTranscriptResult {
  text: string;
  segments: NativeTranscriptSegment[];
  detectedLanguage: string | null;
  confidence: number | null;
  isFinal: boolean;
}

export interface NativeDictationTranscript {
  sessionId: string;
  engineId: string;
  transcript: NativeTranscriptResult;
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
    case "failed":
      return "error";
    case "idle":
    case "cancelled":
      return "idle";
  }
}

export function getDictationSession() {
  return invoke<NativeDictationStateEvent>("get_dictation_session");
}

export function getLastTranscript() {
  return invoke<NativeDictationTranscript | null>("get_last_transcript");
}

export function startDictationSession() {
  return invoke<NativeDictationStateEvent>("start_dictation_session", {
    trigger: "userInterface",
  });
}

export function stopDictationSession() {
  return invoke<NativeDictationStateEvent>("stop_dictation_session");
}

export function cancelDictationSession(reason = "Cancelled by user") {
  return invoke<NativeDictationStateEvent>("cancel_dictation_session", {
    reason,
  });
}

export function subscribeToDictationState(
  handler: (state: NativeDictationStateEvent) => void,
): Promise<UnlistenFn> {
  return listen<NativeDictationStateEvent>(DICTATION_STATE_EVENT, (event) => {
    handler(event.payload);
  });
}

export function subscribeToDictationTranscript(
  handler: (transcript: NativeDictationTranscript) => void,
): Promise<UnlistenFn> {
  return listen<NativeDictationTranscript>(
    DICTATION_TRANSCRIPT_EVENT,
    (event) => handler(event.payload),
  );
}
