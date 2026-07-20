import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  activateCloudProvider,
  deleteCloudApiKey,
  listCloudProviders,
  setCloudApiKey,
} from "./nativeCloud";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("native cloud-provider commands", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("uses the provider-status command without synthetic arguments", async () => {
    vi.mocked(invoke).mockResolvedValue([]);

    await listCloudProviders();

    expect(invoke).toHaveBeenCalledWith("list_cloud_providers");
  });

  it("passes credentials only to the native credential command", async () => {
    vi.mocked(invoke).mockResolvedValue({});
    const credential = ["test", "credential", "value"].join("-");

    await setCloudApiKey("openAi", credential);

    expect(invoke).toHaveBeenCalledWith("set_cloud_api_key", {
      provider: "openAi",
      apiKey: credential,
    });
  });

  it("passes provider IDs to delete and activation commands", async () => {
    vi.mocked(invoke).mockResolvedValue({});

    await deleteCloudApiKey("xAi");
    await activateCloudProvider("gemini");

    expect(invoke).toHaveBeenNthCalledWith(1, "delete_cloud_api_key", {
      provider: "xAi",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "activate_cloud_provider", {
      provider: "gemini",
    });
  });
});
