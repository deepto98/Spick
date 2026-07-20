import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  NativeAppSettings,
  NativeLanguagePolicy,
} from "./lib/nativeSettings";
import type { NativeDictationTranscript } from "./lib/nativeDictation";
import type { CloudProviderStatus } from "./lib/nativeCloud";
import type { LocalModelSummary } from "./lib/nativeModels";
import App from "./App";

const nativeMocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
}));

const localDataMocks = vi.hoisted(() => ({
  clearData: vi.fn(),
  lastTranscript: null as NativeDictationTranscript | null,
}));

const cloudMocks = vi.hoisted(() => ({
  activate: vi.fn(),
  configure: vi.fn(),
  providers: [] as CloudProviderStatus[],
  refresh: vi.fn(),
  removeCredential: vi.fn(),
}));

const modelMocks = vi.hoisted(() => ({
  importModel: vi.fn(),
  list: vi.fn(),
  remove: vi.fn(),
}));

const audioMocks = vi.hoisted(() => ({
  list: vi.fn(),
}));

vi.mock("./lib/nativeSettings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./lib/nativeSettings")>();
  return {
    ...actual,
    getNativeSettings: nativeMocks.getSettings,
    updateNativeSettings: nativeMocks.updateSettings,
  };
});

vi.mock("./lib/nativeAudio", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./lib/nativeAudio")>();
  return {
    ...actual,
    listNativeAudioInputDevices: audioMocks.list,
  };
});

vi.mock("./hooks/useDictationController", () => ({
  useDictationController: () => ({
    delivery: null,
    error: null,
    language: "AUTO",
    lastLatency: null,
    lastTranscript: localDataMocks.lastTranscript,
    native: true,
    pending: false,
    state: "idle",
    transitionTo: vi.fn(),
  }),
}));

vi.mock("./hooks/useLocalData", () => ({
  useLocalData: () => ({
    dashboard: null,
    dashboardError: null,
    dashboardLoading: false,
    subscriptionError: null,
    history: [],
    historyError: null,
    historyLoading: false,
    historyLoadingMore: false,
    historyNextCursor: null,
    vocabulary: [],
    vocabularyError: null,
    vocabularyLoading: false,
    vocabularyPendingIds: new Set<string>(),
    clearError: null,
    clearPendingScope: null,
    lastClearResult: null,
    refreshDashboardAndHistory: vi.fn(),
    loadOlderHistory: vi.fn(),
    refreshVocabulary: vi.fn(),
    createVocabulary: vi.fn(),
    updateVocabulary: vi.fn(),
    deleteVocabulary: vi.fn(),
    clearData: localDataMocks.clearData,
  }),
}));

vi.mock("./hooks/useCloudProviders", () => ({
  useCloudProviders: () => ({
    activate: cloudMocks.activate,
    configure: cloudMocks.configure,
    error: null,
    loading: false,
    pending: null,
    providers: cloudMocks.providers,
    refresh: cloudMocks.refresh,
    removeCredential: cloudMocks.removeCredential,
  }),
}));

vi.mock("./hooks/useAccessibilityPermission", () => ({
  useAccessibilityPermission: () => ({
    error: null,
    pending: false,
    refresh: vi.fn(),
    request: vi.fn(),
    status: { state: "granted", canRequest: true },
  }),
}));

vi.mock("./hooks/useShortcutStatus", () => ({
  useShortcutStatus: () => ({
    error: null,
    pending: false,
    refresh: vi.fn(),
    request: vi.fn(),
    status: {
      optionSelected: true,
      optionListenerActive: true,
      inputMonitoringGranted: true,
      fallbackShortcut: null,
    },
  }),
}));

vi.mock("./hooks/useAudioLevel", () => ({
  useAudioLevel: () => null,
}));

vi.mock("./lib/nativeModels", () => ({
  activateLocalModel: vi.fn(),
  cancelLocalModelInstall: vi.fn(),
  formatModelBytes: vi.fn(() => "0 MB"),
  importLocalModel: modelMocks.importModel,
  installLocalModel: vi.fn(),
  listLocalModels: modelMocks.list,
  modelStatus: vi.fn(() => "available"),
  removeLocalModel: modelMocks.remove,
  subscribeToModelDownload: vi.fn(async () => () => undefined),
}));

const baseSettings: NativeAppSettings = {
  schemaVersion: 5,
  pushToTalkShortcut: "Option",
  languagePolicy: { mode: "auto" },
  transcriptionEngine: {
    provider: "whisperCpp",
    model: "whisper-small-multilingual-q5-1",
    location: "local",
  },
  cleanupEngine: null,
  inputDeviceName: null,
  hud: {
    position: "bottomRight",
    presentation: "expanded",
    customPosition: null,
    visible: true,
  },
  allowCloudFallback: false,
  saveTranscriptHistory: false,
};

function settingsWith(
  languagePolicy: NativeAppSettings["languagePolicy"],
  clean: boolean,
): NativeAppSettings {
  return {
    ...baseSettings,
    languagePolicy,
    cleanupEngine: clean
      ? { provider: "builtIn", model: "readable-v1", location: "local" }
      : null,
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, reject, resolve };
}

async function openLanguageSettings(expectedLanguage = "Auto-detect") {
  render(<App />);
  await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Language & cleanup" }));
  await waitFor(() =>
    expect(screen.getByRole("combobox")).toHaveValue(expectedLanguage),
  );
}

async function openDictationSettings() {
  render(<App />);
  await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Dictation" }));
  await waitFor(() =>
    expect(screen.getAllByLabelText("Shortcut ⌥").length).toBeGreaterThan(1),
  );
}

async function openPrivacySettings() {
  render(<App />);
  await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "Settings" }));
  fireEvent.click(screen.getByRole("button", { name: "Privacy & history" }));
  await waitFor(() =>
    expect(
      screen.getByRole("switch", { name: "Keep transcript history" }),
    ).toBeInTheDocument(),
  );
}

describe("native language and cleanup persistence", () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem("spick-onboarding-complete", "true");
    window.history.replaceState({}, "", "/");
    nativeMocks.getSettings.mockReset();
    nativeMocks.updateSettings.mockReset();
    localDataMocks.clearData.mockReset();
    localDataMocks.lastTranscript = null;
    cloudMocks.activate.mockReset();
    cloudMocks.configure.mockReset();
    cloudMocks.providers = [];
    cloudMocks.refresh.mockReset();
    cloudMocks.refresh.mockResolvedValue(true);
    cloudMocks.removeCredential.mockReset();
    modelMocks.list.mockReset();
    modelMocks.list.mockResolvedValue([]);
    modelMocks.importModel.mockReset();
    modelMocks.importModel.mockResolvedValue(null);
    modelMocks.remove.mockReset();
    modelMocks.remove.mockResolvedValue(undefined);
    audioMocks.list.mockReset();
    audioMocks.list.mockResolvedValue([]);
    nativeMocks.getSettings.mockResolvedValue(baseSettings);
  });

  afterEach(() => {
    cleanup();
  });

  it("keeps a partial imported-model removal warning after refreshing", async () => {
    const imported: LocalModelSummary = {
      active: false,
      installedBytes: 123,
      state: "notInstalled",
      manifest: {
        id: `whisper-imported-${"a".repeat(64)}`,
        displayName: "Imported Small Q5_1 · aaaaaaaa",
        fileName: `whisper-imported-${"a".repeat(64)}.bin`,
        family: "small",
        languages: "multilingual",
        quantization: "q5_1",
        downloadBytes: 123,
        sha256: "a".repeat(64),
        origin: "imported",
      },
    };
    modelMocks.list.mockResolvedValue([imported]);
    modelMocks.remove.mockRejectedValueOnce(
      new Error("model was removed, but some local files remain"),
    );

    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Engines" }));
    fireEvent.click(
      await screen.findByRole("button", { name: "Remove model" }),
    );

    await waitFor(() => expect(modelMocks.list).toHaveBeenCalledTimes(2));
    expect(screen.getByText(/some local files remain/i)).toBeInTheDocument();
  });

  it("refreshes without hiding an import error after a possible native commit", async () => {
    modelMocks.importModel.mockRejectedValueOnce(
      new Error("verification cache was unavailable"),
    );

    render(<App />);
    fireEvent.click(screen.getByRole("button", { name: "Engines" }));
    const importButton = await screen.findByRole("button", {
      name: "Import model",
    });
    await waitFor(() => expect(importButton).toBeEnabled());
    fireEvent.click(importButton);

    await waitFor(() => expect(modelMocks.list).toHaveBeenCalledTimes(2));
    expect(
      screen.getByText(/verification cache was unavailable/i),
    ).toBeInTheDocument();
  });

  it("renders a native-backed choice only after the save acknowledges it", async () => {
    const save = deferred<NativeAppSettings>();
    nativeMocks.updateSettings.mockReturnValueOnce(save.promise);
    await openLanguageSettings();

    const language = screen.getByRole("combobox");
    fireEvent.change(language, { target: { value: "English" } });

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );
    expect(language).toHaveValue("Auto-detect");
    expect(screen.getAllByText("Saving…").length).toBeGreaterThan(0);

    await act(async () => {
      save.resolve(settingsWith({ mode: "fixed", language: "en" }, false));
      await save.promise;
    });

    await waitFor(() => expect(language).toHaveValue("English"));
    expect(screen.getByText("Saved on this Mac")).toBeInTheDocument();
  });

  it("keeps the acknowledged cleanup choice when persistence fails", async () => {
    const save = deferred<NativeAppSettings>();
    nativeMocks.updateSettings.mockReturnValueOnce(save.promise);
    await openLanguageSettings();

    const asTranscribed = screen.getByRole("button", {
      name: /As transcribed/i,
    });
    fireEvent.click(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    );

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );
    expect(asTranscribed).toHaveClass("active");

    await act(async () => {
      save.reject(new Error("disk is read-only"));
      await save.promise.catch(() => undefined);
    });

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent("disk is read-only"),
    );
    expect(asTranscribed).toHaveClass("active");
    expect(screen.getByText("Saved on this Mac")).toBeInTheDocument();
  });

  it("publishes a custom shortcut only after native acknowledgement", async () => {
    const save = deferred<NativeAppSettings>();
    nativeMocks.updateSettings.mockReturnValueOnce(save.promise);
    await openDictationSettings();

    fireEvent.click(screen.getByRole("button", { name: "Custom" }));
    fireEvent.keyDown(window, {
      code: "KeyD",
      key: "D",
      metaKey: true,
      shiftKey: true,
    });

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );
    expect(nativeMocks.updateSettings.mock.calls[0]?.[0]).toMatchObject({
      pushToTalkShortcut: "Command+Shift+KeyD",
    });
    expect(screen.getAllByText("Saving…").length).toBeGreaterThan(0);
    expect(screen.queryByLabelText("Shortcut ⌘+⇧+D")).not.toBeInTheDocument();
    expect(screen.getAllByLabelText("Shortcut ⌥").length).toBeGreaterThan(1);

    await act(async () => {
      save.resolve({
        ...baseSettings,
        pushToTalkShortcut: "Command+Shift+KeyD",
      });
      await save.promise;
    });

    await waitFor(() =>
      expect(screen.getAllByLabelText("Shortcut ⌘+⇧+D").length).toBeGreaterThan(
        1,
      ),
    );
    expect(screen.queryByLabelText("Shortcut ⌥")).not.toBeInTheDocument();
    expect(screen.getByText("Saved on this Mac")).toBeInTheDocument();
  });

  it("persists the selected microphone for the next capture", async () => {
    audioMocks.list.mockResolvedValue([
      { name: "MacBook Microphone", isDefault: true },
      { name: "Desk Mic", isDefault: false },
    ]);
    nativeMocks.updateSettings.mockImplementation(
      async (saved: NativeAppSettings) => saved,
    );
    await openDictationSettings();

    const microphone = await screen.findByRole("combobox", {
      name: "Microphone",
    });
    await waitFor(() =>
      expect(screen.getByRole("option", { name: "Desk Mic" })).toBeVisible(),
    );
    fireEvent.change(microphone, { target: { value: "Desk Mic" } });

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );
    expect(nativeMocks.updateSettings.mock.calls[0]?.[0]).toMatchObject({
      inputDeviceName: "Desk Mic",
    });
    await waitFor(() => expect(microphone).toHaveValue("Desk Mic"));

    fireEvent.change(microphone, {
      target: { value: "System default microphone" },
    });
    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledTimes(2),
    );
    expect(nativeMocks.updateSettings.mock.calls[1]?.[0]).toMatchObject({
      inputDeviceName: null,
    });
    await waitFor(() =>
      expect(microphone).toHaveValue("System default microphone"),
    );
  });

  it("ignores a stale microphone list after a newer refresh finishes", async () => {
    const firstList = deferred<Array<{ name: string; isDefault: boolean }>>();
    const secondList = deferred<Array<{ name: string; isDefault: boolean }>>();
    audioMocks.list
      .mockReturnValueOnce(firstList.promise)
      .mockReturnValueOnce(secondList.promise);
    render(<App />);
    await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await waitFor(() => expect(audioMocks.list).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    await waitFor(() => expect(audioMocks.list).toHaveBeenCalledTimes(2));
    fireEvent.click(screen.getByRole("button", { name: "Dictation" }));

    await act(async () => {
      secondList.resolve([{ name: "New Mic", isDefault: true }]);
      await secondList.promise;
    });
    await waitFor(() =>
      expect(screen.getByRole("option", { name: "New Mic" })).toBeVisible(),
    );

    await act(async () => {
      firstList.resolve([{ name: "Stale Mic", isDefault: true }]);
      await firstList.promise;
    });
    expect(screen.getByRole("option", { name: "New Mic" })).toBeVisible();
    expect(screen.queryByRole("option", { name: "Stale Mic" })).toBeNull();
  });

  it("persists visibility for the native floating HUD", async () => {
    nativeMocks.updateSettings.mockImplementation(
      async (saved: NativeAppSettings) => saved,
    );
    render(<App />);
    await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(
      screen.getByRole("switch", { name: "Show floating widget" }),
    );

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );
    expect(nativeMocks.updateSettings.mock.calls[0]?.[0]).toMatchObject({
      hud: { visible: false },
    });
    await waitFor(() =>
      expect(
        screen.getByRole("switch", { name: "Show floating widget" }),
      ).toHaveAttribute("aria-checked", "false"),
    );
    expect(screen.getByText(/Turn this on to show it now/i)).toBeVisible();
  });

  it("uses an acknowledged cloud engine in the next settings write", async () => {
    const provider: CloudProviderStatus = {
      provider: "openAi",
      providerName: "OpenAI",
      engineId: "openai-gpt-4o-transcribe",
      modelName: "GPT-4o Transcribe",
      configured: true,
      selected: false,
      experimental: false,
      description: "Dedicated multilingual speech-to-text.",
      languageSupport: "Multilingual batch transcription",
      cleanupBehavior: "Spick cleanup runs after transcription",
    };
    const cloudSettings: NativeAppSettings = {
      ...baseSettings,
      transcriptionEngine: {
        provider: "openAi",
        model: "gpt-4o-transcribe",
        location: "cloud",
      },
    };
    cloudMocks.providers = [provider];
    cloudMocks.activate.mockResolvedValue(cloudSettings);
    nativeMocks.updateSettings.mockImplementation(
      async (saved: NativeAppSettings) => saved,
    );

    render(<App />);
    await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Engines" }));
    fireEvent.click(screen.getByRole("tab", { name: /Cloud providers/i }));
    fireEvent.click(screen.getByRole("button", { name: "Use provider" }));
    await waitFor(() =>
      expect(cloudMocks.activate).toHaveBeenCalledWith("openAi"),
    );

    fireEvent.click(screen.getByRole("button", { name: "Settings" }));
    fireEvent.click(screen.getByRole("button", { name: "Privacy & history" }));
    fireEvent.click(
      screen.getByRole("switch", { name: "Keep transcript history" }),
    );

    await waitFor(() => expect(nativeMocks.updateSettings).toHaveBeenCalled());
    const latestSave =
      nativeMocks.updateSettings.mock.calls[
        nativeMocks.updateSettings.mock.calls.length - 1
      ]?.[0];
    expect(latestSave).toMatchObject({
      transcriptionEngine: cloudSettings.transcriptionEngine,
      saveTranscriptHistory: true,
    });
  });

  it("keeps the previous shortcut and shows a native save error", async () => {
    const save = deferred<NativeAppSettings>();
    nativeMocks.updateSettings.mockReturnValueOnce(save.promise);
    await openDictationSettings();

    fireEvent.click(screen.getByRole("button", { name: "Custom" }));
    fireEvent.keyDown(window, {
      altKey: true,
      code: "Space",
      key: " ",
    });
    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );

    await act(async () => {
      save.reject(new Error("that shortcut is already in use"));
      await save.promise.catch(() => undefined);
    });

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        "that shortcut is already in use",
      ),
    );
    expect(screen.queryByLabelText("Shortcut ⌥+Space")).not.toBeInTheDocument();
    expect(screen.getAllByLabelText("Shortcut ⌥").length).toBeGreaterThan(1);
    expect(screen.getByText("Saved on this Mac")).toBeInTheDocument();
  });

  it.each([
    ["transcript history", "Keep transcript history", "saveTranscriptHistory"],
    ["cloud fallback", "Allow cloud fallback", "allowCloudFallback"],
  ] as const)(
    "publishes the %s choice only after native acknowledgement",
    async (_label, controlName, nativeField) => {
      const save = deferred<NativeAppSettings>();
      nativeMocks.updateSettings.mockReturnValueOnce(save.promise);
      await openPrivacySettings();

      const control = screen.getByRole("switch", { name: controlName });
      expect(control).toHaveAttribute("aria-checked", "false");
      fireEvent.click(control);

      await waitFor(() =>
        expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
      );
      expect(nativeMocks.updateSettings.mock.calls[0]?.[0]).toMatchObject({
        [nativeField]: true,
      });
      expect(control).toHaveAttribute("aria-checked", "false");
      expect(control).toBeDisabled();

      await act(async () => {
        save.resolve({ ...baseSettings, [nativeField]: true });
        await save.promise;
      });

      await waitFor(() =>
        expect(control).toHaveAttribute("aria-checked", "true"),
      );
      expect(control).toBeEnabled();
    },
  );

  it("hides the exact cleared recovery transcript even if React had not identified it", async () => {
    localDataMocks.lastTranscript = {
      sessionId: "cleared-session",
      engineId: "whisper-local",
      transcript: {
        text: "This cleared recovery text must stay gone.",
        segments: [],
        detectedLanguage: "en",
        confidence: null,
        isFinal: true,
      },
      delivery: {
        status: "focusChanged",
        transcriptAvailable: true,
        targetApp: "Notes",
        caretRepositioned: null,
      },
    };
    localDataMocks.clearData.mockResolvedValue({
      scope: "all",
      deletedUsageSessions: 1,
      deletedTranscripts: 1,
      deletedVocabularyEntries: 0,
      clearedLatestTranscript: true,
      clearedLatestSessionId: "cleared-session",
      storageCleanupComplete: true,
      storageCleanupWarning: null,
      memoryCleanupComplete: true,
      memoryCleanupWarning: null,
      clearedAtMs: 10,
    });
    await openPrivacySettings();

    fireEvent.click(screen.getByRole("button", { name: "Reset local data" }));
    expect(localDataMocks.clearData).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Confirm reset" }));

    await waitFor(() =>
      expect(localDataMocks.clearData).toHaveBeenCalledWith("all"),
    );
    fireEvent.click(screen.getByRole("button", { name: "Today" }));
    expect(
      screen.queryByText("This cleared recovery text must stay gone."),
    ).not.toBeInTheDocument();
  });

  it.each<[string, NativeLanguagePolicy, string]>([
    ["regional fixed", { mode: "fixed", language: "en-IN" }, "English"],
    [
      "preferred",
      { mode: "preferred", languages: ["en-IN", "hi-IN"] },
      "Auto-detect",
    ],
  ])(
    "preserves a %s language policy during a cleanup-only save",
    async (_label, languagePolicy, displayedLanguage) => {
      nativeMocks.getSettings.mockResolvedValue({
        ...baseSettings,
        languagePolicy,
      });
      nativeMocks.updateSettings.mockImplementation(
        async (saved: NativeAppSettings) => saved,
      );
      await openLanguageSettings(displayedLanguage);

      fireEvent.click(
        screen.getByRole("button", { name: /Trim obvious fillers/i }),
      );

      await waitFor(() =>
        expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
      );
      expect(
        nativeMocks.updateSettings.mock.calls[0]?.[0].languagePolicy,
      ).toEqual(languagePolicy);
    },
  );

  it("locks choices until each native acknowledgement", async () => {
    const firstSave = deferred<NativeAppSettings>();
    const secondSave = deferred<NativeAppSettings>();
    nativeMocks.updateSettings
      .mockReturnValueOnce(firstSave.promise)
      .mockReturnValueOnce(secondSave.promise);
    await openLanguageSettings();

    fireEvent.change(screen.getByRole("combobox"), {
      target: { value: "English" },
    });

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledTimes(1),
    );
    expect(screen.getByRole("combobox")).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    ).toBeDisabled();

    await act(async () => {
      firstSave.resolve(settingsWith({ mode: "fixed", language: "en" }, false));
      await firstSave.promise;
    });

    await waitFor(() =>
      expect(screen.getByRole("combobox")).toHaveValue("English"),
    );
    expect(screen.getByRole("combobox")).toBeEnabled();
    fireEvent.click(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    );

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledTimes(2),
    );
    expect(nativeMocks.updateSettings.mock.calls[1]?.[0]).toMatchObject({
      languagePolicy: { mode: "fixed", language: "en" },
      cleanupEngine: {
        provider: "builtIn",
        model: "readable-v1",
        location: "local",
      },
    });
    expect(screen.getByRole("button", { name: /As transcribed/i })).toHaveClass(
      "active",
    );
    expect(
      screen.getByRole("button", { name: /As transcribed/i }),
    ).toBeDisabled();

    await act(async () => {
      secondSave.resolve(settingsWith({ mode: "fixed", language: "en" }, true));
      await secondSave.promise;
    });

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /Trim obvious fillers/i }),
      ).toHaveClass("active"),
    );
    expect(screen.getByText("Saved on this Mac")).toBeInTheDocument();
  });

  it("does not advance onboarding while its language choice is unacknowledged", async () => {
    window.localStorage.removeItem("spick-onboarding-complete");
    const save = deferred<NativeAppSettings>();
    nativeMocks.updateSettings.mockReturnValueOnce(save.promise);
    render(<App />);
    await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    expect(
      screen.getByRole("button", { name: /As transcribed/i }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    ).toBeInTheDocument();
    fireEvent.change(
      screen.getByRole("combobox", { name: "Speech language" }),
      { target: { value: "English" } },
    );

    await waitFor(() =>
      expect(nativeMocks.updateSettings).toHaveBeenCalledOnce(),
    );
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    expect(screen.getByText("Saving this choice…")).toBeInTheDocument();

    await act(async () => {
      save.resolve(settingsWith({ mode: "fixed", language: "en" }, false));
      await save.promise;
    });

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled(),
    );
    expect(
      screen.getByRole("combobox", { name: "Speech language" }),
    ).toHaveValue("English");
  });

  it("uses the saved custom shortcut when setup is opened again", async () => {
    window.localStorage.removeItem("spick-onboarding-complete");
    nativeMocks.getSettings.mockResolvedValue({
      ...baseSettings,
      pushToTalkShortcut: "Command+Shift+KeyD",
    });

    render(<App />);

    await waitFor(() =>
      expect(
        screen.getByText(/Hold your saved shortcut and speak/),
      ).toBeInTheDocument(),
    );
    expect(
      screen.queryByText(/Tap or hold Option and speak/),
    ).not.toBeInTheDocument();
  });

  it("blocks personalization until saved settings can be loaded", async () => {
    window.localStorage.removeItem("spick-onboarding-complete");
    const firstRead = deferred<NativeAppSettings>();
    nativeMocks.getSettings
      .mockReturnValueOnce(firstRead.promise)
      .mockResolvedValueOnce(baseSettings);
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    expect(screen.getByText("Loading your saved choices…")).toBeInTheDocument();

    await act(async () => {
      firstRead.reject(new Error("settings file is unavailable"));
      await firstRead.promise.catch(() => undefined);
    });

    await waitFor(() =>
      expect(screen.getByRole("alert")).toHaveTextContent(
        "settings file is unavailable",
      ),
    );
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));

    await waitFor(() =>
      expect(nativeMocks.getSettings).toHaveBeenCalledTimes(2),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled(),
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("marks main settings unacknowledged until a failed load is retried", async () => {
    nativeMocks.getSettings
      .mockRejectedValueOnce(new Error("settings database is busy"))
      .mockResolvedValueOnce(baseSettings);
    render(<App />);

    await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Settings" }));

    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent(
        "Settings not loaded",
      ),
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "settings database is busy",
    );

    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    await waitFor(() =>
      expect(nativeMocks.getSettings).toHaveBeenCalledTimes(2),
    );
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("Saved on this Mac"),
    );
  });

  it("does not invent local models while the native catalog loads", async () => {
    const catalog = deferred<[]>();
    modelMocks.list.mockReturnValueOnce(catalog.promise);
    render(<App />);
    await waitFor(() => expect(nativeMocks.getSettings).toHaveBeenCalledOnce());
    fireEvent.click(screen.getByRole("button", { name: "Engines" }));

    expect(screen.getByText("Loading local models…")).toBeVisible();
    expect(screen.queryByText("Whisper Small")).toBeNull();

    await act(async () => {
      catalog.resolve([]);
      await catalog.promise;
    });
    await waitFor(() =>
      expect(screen.getByText("No local models found")).toBeVisible(),
    );
  });
});
