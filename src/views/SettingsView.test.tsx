import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../types";
import { SettingsView } from "./SettingsView";

const settings: AppSettings = {
  hotkey: "⌥",
  language: "English",
  microphone: "System default microphone",
  launchAtLogin: false,
  playSounds: true,
  showWidget: true,
  keepHistory: false,
  cloudFallback: false,
  cleanupLevel: "Clean",
};

afterEach(cleanup);

describe("cleanup settings", () => {
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
      screen.getByRole("button", { name: /As transcribed/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Trim obvious fillers/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(
        "English only for now. Bare words and other languages stay as transcribed.",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Polished")).not.toBeInTheDocument();
    expect(screen.queryByText(/repeats|rewrite/i)).not.toBeInTheDocument();

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
