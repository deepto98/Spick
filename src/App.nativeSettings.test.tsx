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
  schemaVersion: 2,
  pushToTalkShortcut: "CommandOrControl+Shift+Space",
  languagePolicy: { mode: "auto" },
  transcriptionEngine: {
    provider: "whisperCpp",
    model: "whisper-small-multilingual-q5-1",
    location: "local",
  },
  cleanupEngine: null,
  hud: { position: "bottomRight" },
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
    expect(screen.getByText("Saving…")).toBeInTheDocument();

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
