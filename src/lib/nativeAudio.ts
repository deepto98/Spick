import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const AUDIO_LEVEL_EVENT = "dictation://audio-level";

export interface AudioLevelFrame {
  level: number;
  peak: number;
  capturedMs: number;
}

export interface NativeAudioInputDevice {
  name: string;
  isDefault: boolean;
}

export function listNativeAudioInputDevices() {
  return invoke<NativeAudioInputDevice[]>("list_audio_input_devices");
}

export function clampAudioLevel(value: number) {
  if (!Number.isFinite(value)) return 0;
  return Math.min(1, Math.max(0, value));
}

export function subscribeToAudioLevel(
  handler: (frame: AudioLevelFrame) => void,
): Promise<UnlistenFn> {
  return listen<AudioLevelFrame>(AUDIO_LEVEL_EVENT, (event) => {
    handler({
      ...event.payload,
      level: clampAudioLevel(event.payload.level),
      peak: clampAudioLevel(event.payload.peak),
    });
  });
}
