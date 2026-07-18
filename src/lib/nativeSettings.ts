import { invoke } from "@tauri-apps/api/core";

export type NativeLanguagePolicy =
  | { mode: "auto" }
  | { mode: "fixed"; language: string }
  | { mode: "preferred"; languages: string[] }
  | { mode: "mixed"; languages: string[] }
  | {
      mode: "translate";
      sourceLanguages: string[];
      outputLanguage: string;
    };

export interface NativeEngineConfig {
  provider: string;
  model: string;
  location: "local" | "cloud";
}

export interface NativeAppSettings {
  schemaVersion: number;
  pushToTalkShortcut: string;
  languagePolicy: NativeLanguagePolicy;
  transcriptionEngine: NativeEngineConfig;
  cleanupEngine: NativeEngineConfig | null;
  hud: { position: "bottomLeft" | "bottomCenter" | "bottomRight" };
  allowCloudFallback: boolean;
  saveTranscriptHistory: boolean;
}

const LANGUAGE_TAGS: Record<string, string> = {
  English: "en",
  Hindi: "hi",
  Bengali: "bn",
  Spanish: "es",
  French: "fr",
};

const LANGUAGE_NAMES = Object.fromEntries(
  Object.entries(LANGUAGE_TAGS).map(([name, tag]) => [tag, name]),
) as Record<string, string>;

export function getNativeSettings() {
  return invoke<NativeAppSettings>("get_settings");
}

export function updateNativeSettings(settings: NativeAppSettings) {
  return invoke<NativeAppSettings>("update_settings", { settings });
}

export function languagePolicyForName(
  name: string,
): NativeLanguagePolicy | null {
  if (name === "Auto-detect") return { mode: "auto" };
  const language = LANGUAGE_TAGS[name];
  return language ? { mode: "fixed", language } : null;
}

export function languagePolicyName(policy: NativeLanguagePolicy) {
  if (policy.mode === "auto") return "Auto-detect";
  if (policy.mode === "fixed") {
    return LANGUAGE_NAMES[baseLanguage(policy.language)] ?? policy.language;
  }
  if (policy.mode === "mixed") return "Mixed languages";
  if (policy.mode === "preferred") return "Auto-detect";
  return `Translate to ${policy.outputLanguage.toUpperCase()}`;
}

export function languagePolicyBadge(policy: NativeLanguagePolicy) {
  if (policy.mode === "auto" || policy.mode === "preferred") return "AUTO";
  if (policy.mode === "mixed") return "MIX";
  const language =
    policy.mode === "fixed" ? policy.language : policy.outputLanguage;
  return baseLanguage(language).toUpperCase();
}

function baseLanguage(language: string) {
  return language.split("-", 1)[0] ?? language;
}
