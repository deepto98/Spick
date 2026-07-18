import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { EngineStatus } from "../types";
import type { NativeAppSettings } from "./nativeSettings";

export const MODEL_DOWNLOAD_PROGRESS_EVENT = "models://download-progress";

export type ModelInstallationState =
  "notInstalled" | "needsVerification" | "installed" | "invalid";

export type ModelDownloadPhase = "downloading" | "verifying" | "installed";

export interface WhisperModelManifest {
  id: string;
  displayName: string;
  fileName: string;
  family: string;
  languages: "multilingual" | "englishOnly";
  quantization: string | { other: string };
  downloadBytes: number;
  sha256: string;
  sourceUrl: string;
}

export interface LocalModelSummary {
  manifest: WhisperModelManifest;
  state: ModelInstallationState;
  installedBytes: number;
  active: boolean;
}

export interface ModelDownloadProgress {
  modelId: string;
  phase: ModelDownloadPhase;
  downloadedBytes: number;
  totalBytes: number;
}

export function listLocalModels() {
  return invoke<LocalModelSummary[]>("list_local_models");
}

export function installLocalModel(modelId: string) {
  return invoke<LocalModelSummary>("install_local_model", { modelId });
}

export function cancelLocalModelInstall(modelId: string) {
  return invoke<boolean>("cancel_local_model_install", { modelId });
}

export function activateLocalModel(modelId: string) {
  return invoke<NativeAppSettings>("activate_local_model", { modelId });
}

export function removeLocalModel(modelId: string) {
  return invoke<void>("remove_local_model", { modelId });
}

export function subscribeToModelDownload(
  handler: (progress: ModelDownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<ModelDownloadProgress>(MODEL_DOWNLOAD_PROGRESS_EVENT, (event) =>
    handler(event.payload),
  );
}

export function modelStatus(summary: LocalModelSummary): EngineStatus {
  if (summary.state === "installed") {
    return summary.active ? "active" : "ready";
  }
  if (summary.state === "needsVerification") return "ready";
  if (summary.state === "invalid") return "invalid";
  return "available";
}

export function formatModelBytes(bytes: number) {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  return `${(bytes / 1_000_000).toFixed(1)} MB`;
}
