import { invoke } from "@tauri-apps/api/core";

export interface NativeShortcutStatus {
  optionSelected: boolean;
  optionListenerActive: boolean;
  inputMonitoringGranted: boolean;
  fallbackShortcut: string | null;
}

export function getShortcutStatus() {
  return invoke<NativeShortcutStatus>("get_shortcut_status");
}

export function requestInputMonitoringPermission() {
  return invoke<boolean>("request_input_monitoring_permission");
}
