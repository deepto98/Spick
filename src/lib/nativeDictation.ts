import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { HudState } from "../types";
import type { NativeLanguagePolicy } from "./nativeSettings";

export const DICTATION_STATE_EVENT = "dictation://state";
export const DICTATION_TRANSCRIPT_EVENT = "dictation://transcript";
export const DICTATION_LATENCY_EVENT = "dictation://latency";

export type NativeSessionState =
  | "idle"
  | "starting"
  | "listening"
  | "processing"
  | "inserting"
  | "completed"
  | "cancelled"
  | "failed";

export type NativeDeliveryStatus =
  | "inserted"
  | "focusChanged"
  | "secureField"
  | "accessibilityMissing"
  | "unsupported"
  | "failed"
  | "indeterminate";

export interface NativeDeliveryOutcome {
  status: NativeDeliveryStatus;
  transcriptAvailable: boolean;
  targetApp: string | null;
  caretRepositioned: boolean | null;
}

export interface NativeDictationSession {
  id: string;
  state: NativeSessionState;
  languagePolicy: NativeLanguagePolicy;
  startedAtMs: number;
  endedAtMs: number | null;
  cancelReason: string | null;
  error: string | null;
  delivery: NativeDeliveryOutcome | null;
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
  delivery: NativeDeliveryOutcome;
}

export type NativeDictationLatencyOutcome =
  "completed" | "failed" | "cancelled";

export interface NativeDictationLatencyEvent {
  sessionId: string;
  revision: number;
  outcome: NativeDictationLatencyOutcome;
  targetCaptureMs: number | null;
  startToTargetCaptureReturnMs: number | null;
  startToAudioOwnerSpawnMs: number | null;
  startToStartingEmittedMs: number | null;
  startToHudShowReturnMs: number | null;
  startToMicrophoneReadyMs: number | null;
  startToListeningEmittedMs: number | null;
  audioDurationMs: number | null;
  stopToProcessingMs: number | null;
  captureFinalizeMs: number | null;
  transcriptionMs: number | null;
  deliveryMs: number | null;
  stopToDeliveryMs: number | null;
  processingTotalMs: number | null;
}

function isDuration(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

function isOptionalDuration(value: unknown) {
  return value === null || isDuration(value);
}

export function isValidDictationLatencyEvent(
  event: unknown,
): event is NativeDictationLatencyEvent {
  if (typeof event !== "object" || event === null) return false;
  const candidate = event as Record<string, unknown>;
  const validOutcome =
    candidate.outcome === "completed" ||
    candidate.outcome === "failed" ||
    candidate.outcome === "cancelled";
  if (
    typeof candidate.sessionId !== "string" ||
    !candidate.sessionId ||
    candidate.sessionId.length > 160 ||
    !isDuration(candidate.revision) ||
    !validOutcome ||
    !isOptionalDuration(candidate.targetCaptureMs) ||
    !isOptionalDuration(candidate.startToTargetCaptureReturnMs) ||
    !isOptionalDuration(candidate.startToAudioOwnerSpawnMs) ||
    !isOptionalDuration(candidate.startToStartingEmittedMs) ||
    !isOptionalDuration(candidate.startToHudShowReturnMs) ||
    !isOptionalDuration(candidate.startToMicrophoneReadyMs) ||
    !isOptionalDuration(candidate.startToListeningEmittedMs) ||
    !isOptionalDuration(candidate.stopToProcessingMs) ||
    !isOptionalDuration(candidate.processingTotalMs) ||
    !isOptionalDuration(candidate.audioDurationMs) ||
    !isOptionalDuration(candidate.captureFinalizeMs) ||
    !isOptionalDuration(candidate.transcriptionMs) ||
    !isOptionalDuration(candidate.deliveryMs) ||
    !isOptionalDuration(candidate.stopToDeliveryMs)
  ) {
    return false;
  }

  const targetCaptureMs = candidate.targetCaptureMs as number | null;
  const targetReturnMs = candidate.startToTargetCaptureReturnMs as
    number | null;
  const audioSpawnMs = candidate.startToAudioOwnerSpawnMs as number | null;
  const startingMs = candidate.startToStartingEmittedMs as number | null;
  const hudReturnMs = candidate.startToHudShowReturnMs as number | null;
  const microphoneReadyMs = candidate.startToMicrophoneReadyMs as number | null;
  const listeningMs = candidate.startToListeningEmittedMs as number | null;
  const processingTotalMs = candidate.processingTotalMs as number | null;
  const processingStages = [
    candidate.stopToProcessingMs as number | null,
    candidate.captureFinalizeMs as number | null,
    candidate.transcriptionMs as number | null,
    candidate.deliveryMs as number | null,
    candidate.stopToDeliveryMs as number | null,
  ];

  if (
    (targetCaptureMs !== null &&
      targetReturnMs !== null &&
      targetCaptureMs > targetReturnMs) ||
    (targetReturnMs !== null &&
      audioSpawnMs !== null &&
      targetReturnMs > audioSpawnMs) ||
    (audioSpawnMs !== null &&
      startingMs !== null &&
      audioSpawnMs > startingMs) ||
    (startingMs !== null && hudReturnMs !== null && startingMs > hudReturnMs) ||
    (audioSpawnMs !== null &&
      microphoneReadyMs !== null &&
      audioSpawnMs > microphoneReadyMs) ||
    (microphoneReadyMs !== null &&
      listeningMs !== null &&
      microphoneReadyMs > listeningMs) ||
    (hudReturnMs !== null && listeningMs !== null && hudReturnMs > listeningMs)
  ) {
    return false;
  }

  return processingTotalMs === null
    ? processingStages.every((duration) => duration === null)
    : processingStages.every(
        (duration) => duration === null || duration <= processingTotalMs,
      );
}

export function hasNativeRuntime() {
  return isTauri();
}

export function toHudState(state: NativeSessionState): HudState {
  switch (state) {
    case "starting":
      return "starting";
    case "listening":
      return "listening";
    case "processing":
      return "processing";
    case "inserting":
      return "inserting";
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

export function getLastDictationLatency(): Promise<unknown> {
  return invoke<unknown>("get_last_dictation_latency");
}

export function startDictationSession() {
  return invoke<NativeDictationStateEvent>("start_dictation_session");
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

export function subscribeToDictationLatency(
  handler: (latency: unknown) => void,
): Promise<UnlistenFn> {
  return listen<unknown>(DICTATION_LATENCY_EVENT, (event) =>
    handler(event.payload),
  );
}
