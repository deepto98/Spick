import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../types";
import { Onboarding } from "./Onboarding";

const optionSettings: AppSettings = {
  hotkey: "⌥",
  language: "English",
  microphone: "System default microphone",
  showWidget: true,
  keepHistory: false,
  cloudFallback: false,
  cleanupLevel: "Verbatim",
};

function renderPractice(
  settings: AppSettings = optionSettings,
  shortcutStatus = {
    optionSelected: settings.hotkey === "⌥",
    optionListenerActive: settings.hotkey === "⌥",
    inputMonitoringGranted: true,
    fallbackShortcut: null as string | null,
  },
) {
  render(
    <Onboarding
      accessibilityPending={false}
      accessibilityStatus={{ state: "granted", canRequest: true }}
      microphonePending={false}
      microphoneStatus={{ state: "granted", canRequest: false }}
      shortcutPending={false}
      shortcutStatus={shortcutStatus}
      settings={settings}
      settingsReady
      settingsSaving={false}
      transcriptionSource="local"
      engineName="Whisper Small"
      engineReady
      practiceDictationState="idle"
      practiceTranscript={null}
      onRequestAccessibility={vi.fn()}
      onRefreshAccessibility={vi.fn()}
      onRequestMicrophone={vi.fn()}
      onRefreshMicrophone={vi.fn()}
      onRefreshShortcut={vi.fn()}
      onRequestInputMonitoring={vi.fn()}
      onRetrySettings={vi.fn()}
      onSettingsChange={vi.fn()}
      onFinalStep={vi.fn()}
      onPracticeModeChange={vi.fn()}
      onComplete={vi.fn()}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  return screen.getByRole("status");
}

describe("onboarding shortcut practice", () => {
  beforeEach(() => vi.useFakeTimers());

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("demonstrates Option tap-to-start and tap-to-stop", () => {
    const status = renderPractice();
    expect(status).toHaveAttribute("aria-live", "polite");
    expect(status).toHaveTextContent(
      "Tap Option once to start and again to finish",
    );

    fireEvent.keyDown(window, {
      altKey: true,
      code: "AltLeft",
      key: "Alt",
    });
    expect(status).toHaveTextContent("Option is down");
    fireEvent.keyUp(window, { code: "AltLeft", key: "Alt" });

    expect(status).toHaveTextContent("Listening");
    expect(status).toHaveTextContent("Tap Option again to finish");

    fireEvent.keyDown(window, {
      altKey: true,
      code: "AltLeft",
      key: "Alt",
    });
    fireEvent.keyUp(window, { code: "AltLeft", key: "Alt" });

    expect(status).toHaveTextContent("Try it here");
    expect(status).toHaveTextContent(
      "Tap Option once to start and again to finish",
    );
  });

  it("demonstrates Option hold-to-talk at the native threshold", () => {
    const status = renderPractice();

    fireEvent.keyDown(window, {
      altKey: true,
      code: "AltLeft",
      key: "Alt",
    });
    act(() => vi.advanceTimersByTime(279));
    expect(status).toHaveTextContent("Option is down");

    act(() => vi.advanceTimersByTime(1));
    expect(status).toHaveTextContent("Listening");
    expect(status).toHaveTextContent("Release Option to finish");

    fireEvent.keyUp(window, { code: "AltLeft", key: "Alt" });
    expect(status).toHaveTextContent("Try it here");
  });

  it("activates only for the exact saved custom chord", () => {
    const settings = { ...optionSettings, hotkey: "⌘+⇧+D" };
    const status = renderPractice(settings);
    expect(status).toHaveTextContent("Hold ⌘+⇧+D to start");

    fireEvent.keyDown(window, {
      code: "KeyK",
      key: "K",
      metaKey: true,
      shiftKey: true,
    });
    expect(status).toHaveTextContent("That wasn’t your shortcut");
    expect(status).toHaveTextContent("Press exactly ⌘+⇧+D");

    fireEvent.keyDown(window, {
      code: "KeyD",
      key: "D",
      metaKey: true,
      shiftKey: true,
    });
    expect(status).toHaveTextContent("Listening");
    expect(status).toHaveTextContent("Release the main key in ⌘+⇧+D");

    fireEvent.keyUp(window, {
      code: "KeyD",
      key: "D",
      metaKey: true,
      shiftKey: true,
    });
    expect(status).toHaveTextContent("Try it here");
  });

  it("practices the registered fallback when Option is not active", () => {
    const status = renderPractice(optionSettings, {
      optionSelected: true,
      optionListenerActive: false,
      inputMonitoringGranted: false,
      fallbackShortcut: "CommandOrControl+Shift+Space",
    });

    expect(status).toHaveTextContent("Hold ⌘+⇧+Space to start");

    fireEvent.keyDown(window, {
      altKey: true,
      code: "AltLeft",
      key: "Alt",
    });
    expect(status).toHaveTextContent("Try it here");

    fireEvent.keyDown(window, {
      code: "Space",
      key: " ",
      metaKey: true,
      shiftKey: true,
    });
    expect(status).toHaveTextContent("Listening");
    expect(status).toHaveTextContent(
      "Release the main key in ⌘+⇧+Space to finish",
    );

    fireEvent.keyUp(window, {
      code: "Space",
      key: " ",
      metaKey: true,
      shiftKey: true,
    });
    expect(status).toHaveTextContent("Try it here");
  });

  it("does not advance past permissions without Option or a fallback", () => {
    render(
      <Onboarding
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        microphonePending={false}
        microphoneStatus={{ state: "granted", canRequest: false }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: false,
          inputMonitoringGranted: false,
          fallbackShortcut: null,
        }}
        settings={optionSettings}
        settingsReady
        settingsSaving={false}
        transcriptionSource="local"
        engineName="Whisper Small"
        engineReady
        practiceDictationState="idle"
        practiceTranscript={null}
        onRequestAccessibility={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRequestMicrophone={vi.fn()}
        onRefreshMicrophone={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRetrySettings={vi.fn()}
        onSettingsChange={vi.fn()}
        onFinalStep={vi.fn()}
        onPracticeModeChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));

    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /Allow in System Settings/ }),
    ).toBeVisible();
  });

  it("does not advance until microphone access is granted", () => {
    const onRequestMicrophone = vi.fn();
    render(
      <Onboarding
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        microphonePending={false}
        microphoneStatus={{ state: "missing", canRequest: true }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        settings={optionSettings}
        settingsReady
        settingsSaving={false}
        transcriptionSource="local"
        engineName="Whisper Small"
        engineReady
        practiceDictationState="idle"
        practiceTranscript={null}
        onRequestAccessibility={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRequestMicrophone={onRequestMicrophone}
        onRefreshMicrophone={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRetrySettings={vi.fn()}
        onSettingsChange={vi.fn()}
        onFinalStep={vi.fn()}
        onPracticeModeChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));

    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Allow microphone" }));
    expect(onRequestMicrophone).toHaveBeenCalledOnce();
  });

  it("describes cloud transcription without claiming audio stays local", () => {
    render(
      <Onboarding
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        microphonePending={false}
        microphoneStatus={{ state: "granted", canRequest: false }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        settings={optionSettings}
        settingsReady
        settingsSaving={false}
        transcriptionSource="cloud"
        engineName="GPT-4o Transcribe"
        engineReady
        practiceDictationState="idle"
        practiceTranscript={null}
        onRequestAccessibility={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRequestMicrophone={vi.fn()}
        onRefreshMicrophone={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRetrySettings={vi.fn()}
        onSettingsChange={vi.fn()}
        onFinalStep={vi.fn()}
        onPracticeModeChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );

    expect(
      screen.getByText("Cloud transcription · GPT-4o Transcribe"),
    ).toBeVisible();
    expect(screen.queryByText(/audio stays on this Mac/i)).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      screen.getByText(/Audio leaves this Mac for transcription/i),
    ).toBeVisible();
  });

  it("describes the local cleaner without promising sentence rewriting", () => {
    render(
      <Onboarding
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        microphonePending={false}
        microphoneStatus={{ state: "granted", canRequest: false }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        settings={{ ...optionSettings, cleanupLevel: "Clean" }}
        settingsReady
        settingsSaving={false}
        transcriptionSource="local"
        engineName="Whisper Small"
        engineReady
        practiceDictationState="idle"
        practiceTranscript={null}
        onRequestAccessibility={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRequestMicrophone={vi.fn()}
        onRefreshMicrophone={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRetrySettings={vi.fn()}
        onSettingsChange={vi.fn()}
        onFinalStep={vi.fn()}
        onPracticeModeChange={vi.fn()}
        onComplete={vi.fn()}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      screen.getByText("Local list · keeps quoted and named uses"),
    ).toBeVisible();
    expect(screen.getByText(/short language-specific list/i)).toBeVisible();
    expect(screen.getByText(/no sentence rewriting/i)).toBeVisible();
  });
});
