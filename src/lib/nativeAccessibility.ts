import { invoke } from "@tauri-apps/api/core";

export type AccessibilityPermissionState =
  "granted" | "missing" | "unsupported";

export interface AccessibilityPermissionStatus {
  state: AccessibilityPermissionState;
  canRequest: boolean;
}

export function getAccessibilityPermissionStatus() {
  return invoke<AccessibilityPermissionStatus>(
    "get_accessibility_permission_status",
  );
}

export function requestAccessibilityPermission() {
  return invoke<AccessibilityPermissionStatus>(
    "request_accessibility_permission",
  );
}
