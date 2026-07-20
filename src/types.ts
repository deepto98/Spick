export type ViewId = "today" | "engines" | "vocabulary" | "settings";

export type HudState =
  "idle" | "listening" | "processing" | "inserting" | "success" | "error";

export type EngineKind = "local" | "cloud";

export type TranscriptionSource =
  "local" | "localWithCloudFallback" | "cloud" | "loading" | "preview";

export type EngineStatus = "active" | "ready" | "available" | "invalid";

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
  usable?: boolean;
  recommended?: boolean;
  origin?: "curated" | "imported";
}

export interface AppSettings {
  hotkey: string;
  language: string;
  microphone: string;
  showWidget: boolean;
  keepHistory: boolean;
  cloudFallback: boolean;
  cleanupLevel: "Verbatim" | "Clean";
}
