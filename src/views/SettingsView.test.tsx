import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../types";
import { SettingsView } from "./SettingsView";

const settings: AppSettings = {
  hotkey: "⌥",
  language: "English",
  microphone: "System default microphone",
  showWidget: true,
  keepHistory: false,
  cloudFallback: false,
  cleanupLevel: "Clean",
};

afterEach(cleanup);

describe("cleanup settings", () => {
  it("uses real microphone choices and a persisted widget control", () => {
    const onChange = vi.fn();
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        audioInputDevices={[
          { name: "MacBook Microphone", isDefault: true },
          { name: "Desk Mic", isDefault: false },
        ]}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={onChange}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={settings}
        settingsSaving={false}
      />,
    );

    expect(screen.queryByText("Coming later")).not.toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("switch", { name: "Show floating widget" }),
    );
    expect(onChange).toHaveBeenCalledWith({
      ...settings,
      showWidget: false,
    });

    fireEvent.click(screen.getByRole("button", { name: "Dictation" }));
    const microphone = screen.getByRole("combobox", { name: "Microphone" });
    expect(microphone).toBeEnabled();
    expect(screen.getByRole("option", { name: "Desk Mic" })).toBeVisible();
    expect(screen.queryByText("External USB microphone")).toBeNull();
    expect(
      screen.getByText("System default: MacBook Microphone"),
    ).toBeVisible();
  });

  it("marks a saved disconnected microphone and keeps System default available", () => {
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        audioInputDevices={[{ name: "MacBook Microphone", isDefault: true }]}
        audioInputDevicesLoaded
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={vi.fn()}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={{ ...settings, microphone: "Desk Mic" }}
        settingsSaving={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Dictation" }));

    expect(
      screen.getByRole("option", { name: "Desk Mic — disconnected" }),
    ).toBeDisabled();
    expect(
      screen.getByRole("option", { name: "System default microphone" }),
    ).toBeEnabled();
    expect(
      screen.getByText(/Choose System default or a connected microphone/),
    ).toBeVisible();
  });

  it("disables the native floating-widget setting in browser preview", () => {
    const onChange = vi.fn();
    render(
      <SettingsView
        native={false}
        accessibilityPending={false}
        accessibilityStatus={{ state: "unsupported", canRequest: false }}
        shortcutPending={false}
        shortcutStatus={null}
        onChange={onChange}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={settings}
        settingsSaving={false}
      />,
    );

    const widget = screen.getByRole("switch", {
      name: "Show floating widget",
    });
    expect(widget).toBeDisabled();
    expect(
      screen.getByText("Available in the Tauri development app."),
    ).toBeVisible();
    fireEvent.click(widget);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("marks settings unacknowledged and offers a native load retry", () => {
    const onRetryNativeSettings = vi.fn();
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={vi.fn()}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        onRetryNativeSettings={onRetryNativeSettings}
        settings={settings}
        settingsAcknowledged={false}
        settingsLoading={false}
        settingsSaving={false}
        nativeError="Couldn’t read saved settings: database busy"
        nativeErrorTitle="Couldn’t load saved settings"
      />,
    );

    expect(screen.getByRole("status")).toHaveTextContent("Settings not loaded");
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Couldn’t load saved settings",
    );
    fireEvent.click(screen.getByRole("button", { name: "Language & cleanup" }));
    expect(screen.getByRole("combobox")).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRetryNativeSettings).toHaveBeenCalledOnce();
  });

  it("offers only the cleanup behavior the native pipeline can perform", () => {
    const onChange = vi.fn();
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={onChange}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={settings}
        settingsSaving={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Language & cleanup" }));

    expect(
      screen.getByRole("option", { name: "Japanese" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Yoruba" })).toBeInTheDocument();
    expect(
      screen.getByText(/fixed choices are checked against/i),
    ).toHaveTextContent(/xAI.*formatting-language list/i);
    expect(
      screen.getByRole("button", { name: /As transcribed/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        /When local cleanup runs, its on-device list covers English, Spanish, French, German, Hindi, Italian, Russian, Japanese, and Chinese\./,
      ),
    ).toBeInTheDocument();
    expect(screen.getByText(/does not rewrite sentences/i)).toBeVisible();
    expect(screen.queryByText("Polished")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/removes repeats|rewrites your words/i),
    ).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /As transcribed/i }));
    expect(onChange).toHaveBeenCalledWith({
      ...settings,
      cleanupLevel: "Verbatim",
    });
  });

  it("locks native-backed choices while a save is pending", () => {
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={vi.fn()}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={settings}
        settingsSaving
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Language & cleanup" }));

    expect(screen.getByRole("combobox")).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /As transcribed/i }),
    ).toBeDisabled();
    expect(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    ).toBeDisabled();
  });

  it("shows the temporary fallback until Input Monitoring is allowed", () => {
    const onRequestInputMonitoring = vi.fn();
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: false,
          inputMonitoringGranted: false,
          fallbackShortcut: "CommandOrControl+Shift+Space",
        }}
        onChange={vi.fn()}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={onRequestInputMonitoring}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={settings}
        settingsSaving={false}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Dictation" }));
    expect(screen.getByText(/⌘\+⇧\+Space still works/)).toBeInTheDocument();
    fireEvent.click(
      screen.getByRole("button", { name: "Allow Input Monitoring" }),
    );
    expect(onRequestInputMonitoring).toHaveBeenCalledOnce();
  });
});

describe("shortcut settings", () => {
  function renderShortcutSettings(
    onShortcutChange: (shortcut: string) => void,
    current: AppSettings = settings,
  ) {
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: current.hotkey === "⌥",
          optionListenerActive: current.hotkey === "⌥",
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={vi.fn()}
        onShortcutChange={onShortcutChange}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        settings={current}
        settingsSaving={false}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Dictation" }));
  }

  it("captures a valid custom shortcut", () => {
    const onShortcutChange = vi.fn();
    renderShortcutSettings(onShortcutChange);

    fireEvent.click(screen.getByRole("button", { name: "Custom" }));
    expect(
      screen.getByText("Press a shortcut. Escape cancels."),
    ).toBeInTheDocument();

    fireEvent.keyDown(window, {
      code: "KeyD",
      key: "D",
      metaKey: true,
      shiftKey: true,
    });

    expect(onShortcutChange).toHaveBeenCalledWith("Command+Shift+KeyD");
    expect(
      screen.queryByRole("button", { name: "Recording shortcut" }),
    ).not.toBeInTheDocument();
  });

  it("keeps recording after an invalid chord and cancels with Escape", () => {
    const onShortcutChange = vi.fn();
    renderShortcutSettings(onShortcutChange);

    fireEvent.click(
      screen.getByRole("button", { name: "Record a custom shortcut" }),
    );
    fireEvent.keyDown(window, {
      code: "KeyD",
      key: "D",
      shiftKey: true,
    });
    expect(
      screen.getByText(/Add Command, Control, or Option/),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Recording shortcut" }),
    ).toBeInTheDocument();

    fireEvent.keyDown(window, { code: "Escape", key: "Escape" });
    expect(onShortcutChange).not.toHaveBeenCalled();
    expect(screen.getByLabelText("Shortcut ⌥")).toBeInTheDocument();
  });

  it("switches a custom shortcut back to Option", () => {
    const onShortcutChange = vi.fn();
    renderShortcutSettings(onShortcutChange, {
      ...settings,
      hotkey: "⌘+⇧+D",
    });

    fireEvent.click(screen.getByRole("button", { name: "Option" }));
    expect(onShortcutChange).toHaveBeenCalledWith("Option");
  });
});

describe("privacy and local data", () => {
  it("separates aggregate usage from optional text history and confirms clears", async () => {
    const onClearLocalData = vi.fn(async () => ({
      scope: "transcriptHistory" as const,
      deletedUsageSessions: 0,
      deletedTranscripts: 2,
      deletedVocabularyEntries: 0,
      clearedLatestTranscript: true,
      clearedLatestSessionId: "session-2",
      storageCleanupComplete: true,
      storageCleanupWarning: null,
      memoryCleanupComplete: true,
      memoryCleanupWarning: null,
      clearedAtMs: 1,
    }));
    render(
      <SettingsView
        accessibilityPending={false}
        accessibilityStatus={{ state: "granted", canRequest: true }}
        clearError="first attempt failed"
        shortcutPending={false}
        shortcutStatus={{
          optionSelected: true,
          optionListenerActive: true,
          inputMonitoringGranted: true,
          fallbackShortcut: null,
        }}
        onChange={vi.fn()}
        onShortcutChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
        onRefreshShortcut={vi.fn()}
        onRequestInputMonitoring={vi.fn()}
        onRequestAccessibility={vi.fn()}
        onRestartOnboarding={vi.fn()}
        onClearLocalData={onClearLocalData}
        lastClearResult={{
          scope: "transcriptHistory",
          deletedUsageSessions: 0,
          deletedTranscripts: 2,
          deletedVocabularyEntries: 0,
          clearedLatestTranscript: true,
          clearedLatestSessionId: "session-2",
          storageCleanupComplete: false,
          storageCleanupWarning: "another SQLite reader is still open",
          memoryCleanupComplete: false,
          memoryCleanupWarning: "the recovery transcript lock is unavailable",
          clearedAtMs: 1,
        }}
        settings={settings}
        settingsSaving={false}
      />,
    );

    expect(screen.getByRole("alert")).toHaveTextContent("first attempt failed");
    fireEvent.click(screen.getByRole("button", { name: "Privacy & history" }));
    expect(
      screen.getByText(/Aggregate word counts, capture duration/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Turning this off leaves aggregate usage totals/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/first configured provider that supports its language/i),
    ).toHaveTextContent(/OpenAI, xAI, then Gemini.*next recording/i);
    expect(
      screen.getByText(/another SQLite reader is still open/),
    ).toHaveTextContent(/Quit and reopen Spick/i);
    expect(
      screen.getByText(/the recovery transcript lock is unavailable/),
    ).toHaveTextContent(/quit and reopen Spick/i);

    fireEvent.click(screen.getByRole("button", { name: "Delete transcripts" }));
    expect(onClearLocalData).not.toHaveBeenCalled();
    expect(screen.getByText(/cannot be undone/i)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Confirm delete" }));
    await waitFor(() =>
      expect(onClearLocalData).toHaveBeenCalledWith("transcriptHistory"),
    );
  });
});
