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
import App from "./App";

const nativeMocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
}));

vi.mock("./lib/nativeSettings", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./lib/nativeSettings")>();
  return {
    ...actual,
    getNativeSettings: nativeMocks.getSettings,
    updateNativeSettings: nativeMocks.updateSettings,
  };
});

vi.mock("./hooks/useDictationController", () => ({
  useDictationController: () => ({
    delivery: null,
    error: null,
    language: "AUTO",
    lastTranscript: null,
    native: true,
    pending: false,
    state: "idle",
    transitionTo: vi.fn(),
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
  installLocalModel: vi.fn(),
  listLocalModels: vi.fn(async () => []),
  modelStatus: vi.fn(() => "available"),
  removeLocalModel: vi.fn(),
  subscribeToModelDownload: vi.fn(async () => () => undefined),
}));

const baseSettings: NativeAppSettings = {
  schemaVersion: 3,
  pushToTalkShortcut: "Option",
  languagePolicy: { mode: "auto" },
  transcriptionEngine: {
    provider: "whisperCpp",
    model: "whisper-small-multilingual-q5-1",
    location: "local",
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

describe("native language and cleanup persistence", () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.localStorage.setItem("spick-onboarding-complete", "true");
    window.history.replaceState({}, "", "/");
    nativeMocks.getSettings.mockReset();
    nativeMocks.updateSettings.mockReset();
    nativeMocks.getSettings.mockResolvedValue(baseSettings);
  });

  afterEach(() => {
    cleanup();
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
    fireEvent.click(screen.getByRole("button", { name: /^EN English$/ }));

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
    expect(screen.getByRole("button", { name: /^EN English$/ })).toHaveClass(
      "active",
    );
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
});
