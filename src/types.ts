export type ViewId = "today" | "engines" | "vocabulary" | "settings";

export type HudState =
  "idle" | "listening" | "processing" | "success" | "error";

export type EngineKind = "local" | "cloud";

export type EngineStatus = "active" | "ready" | "available";

export interface Engine {
  id: string;
  name: string;
  provider: string;
  description: string;
  kind: EngineKind;
  status: EngineStatus;
  languageSupport: string;
  size?: string;
  performance: string;
  recommended?: boolean;
}

export interface VocabularyEntry {
  id: string;
  phrase: string;
  soundsLike?: string;
  category: "Name" | "Technical" | "Company" | "Replacement";
  language: string;
}

export interface DictationSession {
  id: string;
  application: string;
  text: string;
  timestamp: string;
  words: number;
  language: string;
  color: string;
}

export interface AppSettings {
  hotkey: string;
  language: string;
  microphone: string;
  launchAtLogin: boolean;
  playSounds: boolean;
  showWidget: boolean;
  keepHistory: boolean;
  cloudFallback: boolean;
  cleanupLevel: "Verbatim" | "Clean" | "Polished";
}
