import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Sidebar } from "./Sidebar";

describe("Sidebar transcription status", () => {
  afterEach(cleanup);

  it("names the active cloud engine instead of claiming transcription is local", () => {
    render(
      <Sidebar
        activeView="today"
        hotkey="⌥"
        transcriptionSource="cloud"
        engineName="GPT-4o Transcribe"
        onNavigate={vi.fn()}
      />,
    );

    expect(screen.getByText("Cloud transcription")).toBeVisible();
    expect(screen.getByText("GPT-4o Transcribe")).toBeVisible();
    expect(screen.queryByText("Local transcription")).toBeNull();
  });

  it("qualifies local transcription when cloud fallback is enabled", () => {
    render(
      <Sidebar
        activeView="engines"
        hotkey="⌥"
        transcriptionSource="localWithCloudFallback"
        engineName="Whisper Small"
        onNavigate={vi.fn()}
      />,
    );

    expect(screen.getByText("Local first")).toBeVisible();
    expect(screen.getByText("Whisper Small · fallback on")).toBeVisible();
  });
});
