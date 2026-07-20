import { invoke } from "@tauri-apps/api/core";
import type { NativeHudSettings } from "./nativeSettings";

export function getHudSettings() {
  return invoke<NativeHudSettings>("get_hud_settings");
}

export function setHudPresentation(
  presentation: NativeHudSettings["presentation"],
) {
  return invoke<NativeHudSettings>("set_hud_presentation", { presentation });
}

export function startHudDrag() {
  return invoke<void>("start_hud_drag");
}
