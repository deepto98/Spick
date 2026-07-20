import { invoke } from "@tauri-apps/api/core";
import type { NativeAppSettings } from "./nativeSettings";

export type CloudProviderId = "openAi" | "xAi" | "gemini";

export interface CloudProviderStatus {
  provider: CloudProviderId;
  providerName: string;
  engineId: string;
  modelName: string;
  configured: boolean;
  selected: boolean;
  experimental: boolean;
  description: string;
  languageSupport: string;
  cleanupBehavior: string;
}

export function listCloudProviders() {
  return invoke<CloudProviderStatus[]>("list_cloud_providers");
}

export function setCloudApiKey(provider: CloudProviderId, apiKey: string) {
  return invoke<CloudProviderStatus>("set_cloud_api_key", {
    provider,
    apiKey,
  });
}

export function deleteCloudApiKey(provider: CloudProviderId) {
  return invoke<CloudProviderStatus>("delete_cloud_api_key", { provider });
}

export function activateCloudProvider(provider: CloudProviderId) {
  return invoke<NativeAppSettings>("activate_cloud_provider", { provider });
}
