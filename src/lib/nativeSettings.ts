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
  inputDeviceName: string | null;
  hud: NativeHudSettings;
  allowCloudFallback: boolean;
  saveTranscriptHistory: boolean;
}

export interface NativeHudSettings {
  position: "bottomLeft" | "bottomCenter" | "bottomRight";
  presentation: "expanded" | "compact";
  customPosition: { x: number; y: number } | null;
  visible: boolean;
}

export type NativeCleanupLevel = "Verbatim" | "Clean";

const BUILTIN_CLEANUP_ENGINE: NativeEngineConfig = {
  provider: "builtIn",
  model: "readable-v1",
  location: "local",
};

// Keep the labels stable instead of deriving them from the operating-system
// locale. That makes saved UI choices portable while covering the complete
// multilingual language table shared by the bundled whisper.cpp models.
const SPEECH_LANGUAGES = [
  { name: "Afrikaans", tag: "af" },
  { name: "Albanian", tag: "sq" },
  { name: "Amharic", tag: "am" },
  { name: "Arabic", tag: "ar" },
  { name: "Armenian", tag: "hy" },
  { name: "Assamese", tag: "as" },
  { name: "Azerbaijani", tag: "az" },
  { name: "Bashkir", tag: "ba" },
  { name: "Basque", tag: "eu" },
  { name: "Belarusian", tag: "be" },
  { name: "Bengali", tag: "bn" },
  { name: "Bosnian", tag: "bs" },
  { name: "Breton", tag: "br" },
  { name: "Bulgarian", tag: "bg" },
  { name: "Burmese", tag: "my" },
  { name: "Catalan", tag: "ca" },
  { name: "Chinese", tag: "zh" },
  { name: "Croatian", tag: "hr" },
  { name: "Czech", tag: "cs" },
  { name: "Danish", tag: "da" },
  { name: "Dutch", tag: "nl" },
  { name: "English", tag: "en" },
  { name: "Estonian", tag: "et" },
  { name: "Faroese", tag: "fo" },
  { name: "Finnish", tag: "fi" },
  { name: "French", tag: "fr" },
  { name: "Galician", tag: "gl" },
  { name: "Georgian", tag: "ka" },
  { name: "German", tag: "de" },
  { name: "Greek", tag: "el" },
  { name: "Gujarati", tag: "gu" },
  { name: "Haitian Creole", tag: "ht" },
  { name: "Hausa", tag: "ha" },
  { name: "Hawaiian", tag: "haw" },
  { name: "Hebrew", tag: "he" },
  { name: "Hindi", tag: "hi" },
  { name: "Hungarian", tag: "hu" },
  { name: "Icelandic", tag: "is" },
  { name: "Indonesian", tag: "id" },
  { name: "Italian", tag: "it" },
  { name: "Japanese", tag: "ja" },
  { name: "Javanese", tag: "jv" },
  { name: "Kannada", tag: "kn" },
  { name: "Kazakh", tag: "kk" },
  { name: "Khmer", tag: "km" },
  { name: "Korean", tag: "ko" },
  { name: "Lao", tag: "lo" },
  { name: "Latin", tag: "la" },
  { name: "Latvian", tag: "lv" },
  { name: "Lingala", tag: "ln" },
  { name: "Lithuanian", tag: "lt" },
  { name: "Luxembourgish", tag: "lb" },
  { name: "Macedonian", tag: "mk" },
  { name: "Malagasy", tag: "mg" },
  { name: "Malay", tag: "ms" },
  { name: "Malayalam", tag: "ml" },
  { name: "Maltese", tag: "mt" },
  { name: "Maori", tag: "mi" },
  { name: "Marathi", tag: "mr" },
  { name: "Mongolian", tag: "mn" },
  { name: "Nepali", tag: "ne" },
  { name: "Norwegian", tag: "no" },
  { name: "Norwegian Nynorsk", tag: "nn" },
  { name: "Occitan", tag: "oc" },
  { name: "Pashto", tag: "ps" },
  { name: "Persian", tag: "fa" },
  { name: "Polish", tag: "pl" },
  { name: "Portuguese", tag: "pt" },
  { name: "Punjabi", tag: "pa" },
  { name: "Romanian", tag: "ro" },
  { name: "Russian", tag: "ru" },
  { name: "Sanskrit", tag: "sa" },
  { name: "Serbian", tag: "sr" },
  { name: "Shona", tag: "sn" },
  { name: "Sindhi", tag: "sd" },
  { name: "Sinhala", tag: "si" },
  { name: "Slovak", tag: "sk" },
  { name: "Slovenian", tag: "sl" },
  { name: "Somali", tag: "so" },
  { name: "Spanish", tag: "es" },
  { name: "Sundanese", tag: "su" },
  { name: "Swahili", tag: "sw" },
  { name: "Swedish", tag: "sv" },
  { name: "Tagalog", tag: "fil" },
  { name: "Tajik", tag: "tg" },
  { name: "Tamil", tag: "ta" },
  { name: "Tatar", tag: "tt" },
  { name: "Telugu", tag: "te" },
  { name: "Thai", tag: "th" },
  { name: "Tibetan", tag: "bo" },
  { name: "Turkish", tag: "tr" },
  { name: "Turkmen", tag: "tk" },
  { name: "Ukrainian", tag: "uk" },
  { name: "Urdu", tag: "ur" },
  { name: "Uzbek", tag: "uz" },
  { name: "Vietnamese", tag: "vi" },
  { name: "Welsh", tag: "cy" },
  { name: "Yiddish", tag: "yi" },
  { name: "Yoruba", tag: "yo" },
] as const;

const LANGUAGE_TAGS = Object.fromEntries(
  SPEECH_LANGUAGES.map(({ name, tag }) => [name, tag]),
) as Record<string, string>;

export const SPEECH_LANGUAGE_OPTIONS = [
  "Auto-detect",
  ...SPEECH_LANGUAGES.map(({ name }) => name),
];

const LANGUAGE_NAMES = Object.fromEntries(
  Object.entries(LANGUAGE_TAGS).map(([name, tag]) => [tag, name]),
) as Record<string, string>;

// Imported settings may contain whisper.cpp's historical identifiers.
LANGUAGE_NAMES.jw = "Javanese";
LANGUAGE_NAMES.tl = "Tagalog";

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
