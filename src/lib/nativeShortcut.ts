import { invoke } from "@tauri-apps/api/core";

export interface NativeShortcutStatus {
  optionSelected: boolean;
  optionListenerActive: boolean;
  inputMonitoringGranted: boolean;
  /** Present in current native builds; optional keeps older dev binaries readable. */
  inputMonitoringAccess?: "granted" | "denied" | "unknown";
  fallbackShortcut: string | null;
}

export function getShortcutStatus() {
  return invoke<NativeShortcutStatus>("get_shortcut_status");
}

export function requestInputMonitoringPermission() {
  return invoke<boolean>("request_input_monitoring_permission");
}
