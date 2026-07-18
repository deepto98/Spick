import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { NativeDictationTranscript } from "../lib/nativeDictation";
import { TodayView } from "./TodayView";

const lastTranscript: NativeDictationTranscript = {
  sessionId: "session-1",
  engineId: "whisper-tiny",
  transcript: {
    text: "Move the review to ten tomorrow.",
    segments: [],
    detectedLanguage: "en",
    confidence: null,
    isFinal: true,
  },
  delivery: {
    status: "focusChanged",
    transcriptAvailable: true,
    targetApp: "Mail",
    caretRepositioned: null,
  },
};

describe("latest dictation delivery", () => {
  const writeText = vi.fn();

  beforeEach(() => {
    writeText.mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("keeps a focus-change transcript visible with an explicit copy action", async () => {
    render(
      <TodayView
        delivery={lastTranscript.delivery}
        hudState="idle"
        language="EN"
        lastTranscript={lastTranscript}
        native
        onHudStateChange={vi.fn()}
        onOpenEngines={vi.fn()}
      />,
    );

    expect(screen.getByText("Not typed—the cursor moved")).toBeInTheDocument();
    expect(screen.getByText("Mail")).toBeInTheDocument();
    expect(writeText).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: "Copy latest transcript" }),
    );

    expect(writeText).toHaveBeenCalledWith(lastTranscript.transcript.text);
    expect(await screen.findByText("Copied")).toBeInTheDocument();
  });

  it("makes an indeterminate write a two-step copy", () => {
    const indeterminateTranscript: NativeDictationTranscript = {
      ...lastTranscript,
      sessionId: "session-indeterminate",
      delivery: {
        ...lastTranscript.delivery,
        status: "indeterminate",
      },
    };
    render(
      <TodayView
        delivery={indeterminateTranscript.delivery}
        hudState="idle"
        language="EN"
        lastTranscript={indeterminateTranscript}
        native
        onHudStateChange={vi.fn()}
        onOpenEngines={vi.fn()}
      />,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "Confirm field checked before copy",
      }),
    );
    expect(writeText).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: "Copy latest transcript" }),
    );
    expect(writeText).toHaveBeenCalledWith(
      indeterminateTranscript.transcript.text,
    );
  });
});
