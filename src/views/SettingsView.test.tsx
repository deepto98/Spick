import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "../types";
import { SettingsView } from "./SettingsView";

const settings: AppSettings = {
  hotkey: "⌘+⇧+Space",
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
        onChange={onChange}
        onRefreshAccessibility={vi.fn()}
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
        onChange={vi.fn()}
        onRefreshAccessibility={vi.fn()}
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
});
