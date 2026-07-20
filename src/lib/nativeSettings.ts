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
  hud: NativeHudSettings;
  allowCloudFallback: boolean;
  saveTranscriptHistory: boolean;
}

export interface NativeHudSettings {
  position: "bottomLeft" | "bottomCenter" | "bottomRight";
  presentation: "expanded" | "compact";
  customPosition: { x: number; y: number } | null;
}

export type NativeCleanupLevel = "Verbatim" | "Clean";

const BUILTIN_CLEANUP_ENGINE: NativeEngineConfig = {
  provider: "builtIn",
  model: "readable-v1",
  location: "local",
};

const LANGUAGE_TAGS: Record<string, string> = {
  English: "en",
  Hindi: "hi",
  Bengali: "bn",
  Spanish: "es",
  French: "fr",
};

export const SPEECH_LANGUAGE_OPTIONS = [
  "Auto-detect",
  ...Object.keys(LANGUAGE_TAGS),
];

const LANGUAGE_NAMES = Object.fromEntries(
  Object.entries(LANGUAGE_TAGS).map(([name, tag]) => [tag, name]),
) as Record<string, string>;

export function getNativeSettings() {
  return invoke<NativeAppSettings>("get_settings");
}

export function updateNativeSettings(settings: NativeAppSettings) {
  return invoke<NativeAppSettings>("update_settings", { settings });
}

export function shortcutDisplayName(shortcut: string) {
  return shortcut
    .split("+")
    .map((token) => {
      const normalized = token.trim();
      const upper = normalized.toUpperCase();
      const modifier = {
        ALT: "⌥",
        CMD: "⌘",
        COMMAND: "⌘",
        COMMANDORCONTROL: "⌘",
        COMMANDORCTRL: "⌘",
        CONTROL: "⌃",
        CTRL: "⌃",
        OPTION: "⌥",
        SHIFT: "⇧",
        SUPER: "⌘",
      }[upper];
      if (modifier) return modifier;
      if (/^KEY[A-Z]$/.test(upper)) return upper.slice(3);
      if (/^DIGIT[0-9]$/.test(upper)) return upper.slice(5);
      return (
        {
          ARROWDOWN: "↓",
          ARROWLEFT: "←",
          ARROWRIGHT: "→",
          ARROWUP: "↑",
          BACKSPACE: "⌫",
          ENTER: "Return",
          SPACE: "Space",
          TAB: "⇥",
        }[upper] ?? normalized
      );
    })
    .join("+");
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

export function cleanupLevelForEngine(
  engine: NativeEngineConfig | null,
): NativeCleanupLevel | null {
  if (!engine) return "Verbatim";
  if (
    engine.provider === BUILTIN_CLEANUP_ENGINE.provider &&
    engine.model === BUILTIN_CLEANUP_ENGINE.model &&
    engine.location === BUILTIN_CLEANUP_ENGINE.location
  ) {
    return "Clean";
  }
  return null;
}

export function cleanupEngineForLevel(
  level: NativeCleanupLevel,
): NativeEngineConfig | null {
  if (level === "Verbatim") return null;
  return { ...BUILTIN_CLEANUP_ENGINE };
}

function baseLanguage(language: string) {
  return language.split("-", 1)[0] ?? language;
}
