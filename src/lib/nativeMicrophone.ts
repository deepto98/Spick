import { invoke } from "@tauri-apps/api/core";

export type MicrophonePermissionState =
  "granted" | "missing" | "restricted" | "unsupported";

export interface MicrophonePermissionStatus {
  state: MicrophonePermissionState;
  /** True when macOS can still show its first-use permission prompt. */
  canRequest: boolean;
}

export function getMicrophonePermissionStatus() {
  return invoke<MicrophonePermissionStatus>("get_microphone_permission_status");
}

export function requestMicrophonePermission() {
  return invoke<MicrophonePermissionStatus>("request_microphone_permission");
}
