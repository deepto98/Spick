import { invoke } from "@tauri-apps/api/core";
import type { NativeAppSettings, NativeHudSettings, NativeLanguagePolicy } from "./nativeSettings";

export function getHudSettings() {
  return invoke<NativeHudSettings>("get_hud_settings");
}

export function markHudRendererReady() {
  return invoke<void>("mark_hud_renderer_ready");
}

export function setHudPresentation(
  presentation: NativeHudSettings["presentation"],
) {
  return invoke<NativeHudSettings>("set_hud_presentation", { presentation });
}

export function startHudDrag() {
  return invoke<void>("start_hud_drag");
}

export function setHudHovered(hovered: boolean) {
  return invoke<void>("set_hud_hovered", { hovered });
}

export function updateHudPreferences(
  languagePolicy: NativeLanguagePolicy,
  polished: boolean,
) {
  return invoke<NativeAppSettings>("update_hud_preferences", {
    languagePolicy,
    polished,
  });
}

export function openDashboardView(view: "engines") {
  return invoke<void>("open_dashboard_view", { view });
}
