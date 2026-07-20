import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CloudProviderId, CloudProviderStatus } from "../lib/nativeCloud";
import type { NativeAppSettings } from "../lib/nativeSettings";
import { useCloudProviders } from "./useCloudProviders";

const mocks = vi.hoisted(() => ({
  activate: vi.fn(),
  configure: vi.fn(),
  list: vi.fn(),
  remove: vi.fn(),
}));

vi.mock("../lib/nativeCloud", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/nativeCloud")>();
  return {
    ...actual,
    activateCloudProvider: mocks.activate,
    deleteCloudApiKey: mocks.remove,
    listCloudProviders: mocks.list,
    setCloudApiKey: mocks.configure,
  };
});

const statuses: Record<CloudProviderId, CloudProviderStatus> = {
  openAi: {
    provider: "openAi",
    providerName: "OpenAI",
    engineId: "openai-gpt-4o-transcribe",
    modelName: "GPT-4o Transcribe",
    configured: false,
    selected: false,
    experimental: false,
    description: "Dedicated multilingual speech-to-text.",
    languageSupport: "Multilingual batch transcription",
    cleanupBehavior: "Spick cleanup runs after transcription",
  },
  xAi: {
    provider: "xAi",
    providerName: "xAI",
    engineId: "xai-speech-to-text",
    modelName: "xAI Speech to Text",
    configured: false,
    selected: false,
    experimental: false,
    description: "Dedicated speech-to-text.",
    languageSupport: "Multilingual batch transcription",
    cleanupBehavior: "Filler handling follows your cleanup setting",
  },
  gemini: {
    provider: "gemini",
    providerName: "Google",
    engineId: "gemini-3-5-flash",
    modelName: "Gemini 3.5 Flash",
    configured: false,
    selected: false,
    experimental: true,
    description: "General audio understanding.",
    languageSupport: "Model-dependent multilingual audio",
    cleanupBehavior: "General audio response",
  },
};

const cloudSettings: NativeAppSettings = {
  schemaVersion: 3,
  pushToTalkShortcut: "Option",
  languagePolicy: { mode: "auto" },
  transcriptionEngine: {
    provider: "openAi",
    model: "gpt-4o-transcribe",
    location: "cloud",
  },
  cleanupEngine: null,
  hud: {
    position: "bottomRight",
    presentation: "expanded",
    customPosition: null,
  },
  allowCloudFallback: false,
  saveTranscriptHistory: false,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, reject, resolve };
}

describe("useCloudProviders", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.list.mockResolvedValue([
      statuses.gemini,
      statuses.openAi,
      statuses.xAi,
    ]);
  });

  afterEach(cleanup);

  it("loads providers in deterministic fallback order", async () => {
    const { result } = renderHook(() => useCloudProviders(true));

    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(result.current.providers.map(({ provider }) => provider)).toEqual([
      "openAi",
      "xAi",
      "gemini",
    ]);
    expect(result.current.error).toBeNull();
  });

  it("keeps a credential out of errors and applies the acknowledged status", async () => {
    const credential = ["not", "for", "display"].join("-");
    mocks.configure.mockRejectedValueOnce(new Error(credential));
    const { result } = renderHook(() => useCloudProviders(true));
    await waitFor(() => expect(result.current.loading).toBe(false));

    let saved = true;
    await act(async () => {
      saved = await result.current.configure("openAi", credential);
    });

    expect(saved).toBe(false);
    expect(result.current.error).toBe(
      "Couldn’t save that API key. Check the key and try again.",
    );
    expect(result.current.error).not.toContain(credential);

    mocks.configure.mockResolvedValueOnce({
      ...statuses.openAi,
      configured: true,
    });
    mocks.list.mockResolvedValueOnce([
      { ...statuses.openAi, configured: true },
      statuses.xAi,
      statuses.gemini,
    ]);
    await act(async () => {
      saved = await result.current.configure("openAi", credential);
    });

    expect(saved).toBe(true);
    expect(mocks.configure).toHaveBeenLastCalledWith("openAi", credential);
    expect(result.current.providers[0]?.configured).toBe(true);
  });

  it("does not let a stale list replace a newer credential acknowledgement", async () => {
    const initialList = deferred<CloudProviderStatus[]>();
    const reconciliation = deferred<CloudProviderStatus[]>();
    mocks.list
      .mockReturnValueOnce(initialList.promise)
      .mockReturnValueOnce(reconciliation.promise);
    mocks.configure.mockResolvedValue({
      ...statuses.openAi,
      configured: true,
    });
    const { result } = renderHook(() => useCloudProviders(true));
    await waitFor(() => expect(mocks.list).toHaveBeenCalledOnce());

    await act(async () => {
      await result.current.configure("openAi", "short-lived-value");
    });
    expect(result.current.providers[0]?.configured).toBe(true);

    await act(async () => {
      initialList.resolve([statuses.openAi, statuses.xAi, statuses.gemini]);
      await initialList.promise;
    });
    expect(result.current.providers[0]?.configured).toBe(true);

    await act(async () => {
      reconciliation.resolve([
        { ...statuses.openAi, configured: true },
        statuses.xAi,
        statuses.gemini,
      ]);
      await reconciliation.promise;
    });
  });

  it("serializes mutations and applies delete and activation acknowledgements", async () => {
    const removal = deferred<CloudProviderStatus>();
    mocks.list.mockResolvedValue([
      { ...statuses.openAi, configured: true },
      { ...statuses.xAi, configured: true },
      statuses.gemini,
    ]);
    mocks.remove.mockReturnValue(removal.promise);
    mocks.activate.mockResolvedValue(cloudSettings);
    const { result } = renderHook(() => useCloudProviders(true));
    await waitFor(() => expect(result.current.loading).toBe(false));

    let removeRequest!: Promise<boolean>;
    act(() => {
      removeRequest = result.current.removeCredential("openAi");
    });
    await waitFor(() =>
      expect(result.current.pending).toEqual({
        provider: "openAi",
        action: "delete",
      }),
    );
    await act(async () => {
      expect(await result.current.activate("xAi")).toBeNull();
    });
    expect(mocks.activate).not.toHaveBeenCalled();

    await act(async () => {
      mocks.list.mockResolvedValueOnce([
        statuses.openAi,
        { ...statuses.xAi, configured: true },
        statuses.gemini,
      ]);
      removal.resolve(statuses.openAi);
      expect(await removeRequest).toBe(true);
    });
    expect(result.current.providers[0]?.configured).toBe(false);

    let saved: NativeAppSettings | null = null;
    await act(async () => {
      mocks.list.mockResolvedValueOnce([
        statuses.openAi,
        { ...statuses.xAi, configured: true, selected: true },
        statuses.gemini,
      ]);
      saved = await result.current.activate("xAi");
    });
    expect(saved).toEqual(cloudSettings);
    expect(
      result.current.providers.find(({ provider }) => provider === "xAi")
        ?.selected,
    ).toBe(true);
  });

  it("surfaces list failures and can retry", async () => {
    mocks.list.mockRejectedValueOnce(new Error("native bridge unavailable"));
    const { result } = renderHook(() => useCloudProviders(true));

    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toContain("native bridge unavailable");

    mocks.list.mockResolvedValueOnce(Object.values(statuses));
    await act(async () => {
      expect(await result.current.refresh()).toBe(true);
    });
    expect(result.current.error).toBeNull();
    expect(result.current.providers).toHaveLength(3);
  });
});
