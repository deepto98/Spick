import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DictationHud } from "./DictationHud";

afterEach(cleanup);

describe("dictation delivery HUD", () => {
  it("does not claim to be listening while the microphone is starting", () => {
    render(<DictationHud autoAdvance={false} state="starting" />);

    expect(screen.getByText("Opening microphone")).toBeInTheDocument();
    expect(
      screen.getByText("Listening starts when it’s ready"),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("Microphone audio level")).toBeNull();
  });

  it("shows the insertion handoff", () => {
    render(<DictationHud autoAdvance={false} state="inserting" />);

    expect(screen.getByText("Preparing your text")).toBeInTheDocument();
    expect(screen.getByText("Checking where you started")).toBeInTheDocument();
  });

  it("distinguishes typed text from a copy recovery", () => {
    const { rerender } = render(
      <DictationHud
        autoAdvance={false}
        state="success"
        delivery={{
          status: "inserted",
          transcriptAvailable: true,
          targetApp: "Notes",
          caretRepositioned: true,
        }}
      />,
    );

    expect(screen.getByText("Typed")).toBeInTheDocument();
    expect(screen.getByText("Back in Notes")).toBeInTheDocument();

    rerender(
      <DictationHud
        autoAdvance={false}
        state="success"
        delivery={{
          status: "focusChanged",
          transcriptAvailable: true,
          targetApp: "Notes",
          caretRepositioned: null,
        }}
      />,
    );

    expect(screen.getByText("Text ready to copy")).toBeInTheDocument();
    expect(screen.getByText(/cursor moved/i)).toBeInTheDocument();
  });

  it("keeps the compact floating feedback draggable from its whole surface", () => {
    const onMove = vi.fn();
    render(
      <DictationHud
        audioLevel={0.8}
        floating
        onMovePointerDown={onMove}
        state="listening"
      />,
    );

    const widget = screen.getByRole("status", { name: "Spick is listening" });
    fireEvent.pointerDown(widget);
    expect(onMove).toHaveBeenCalledOnce();
  });

  it("offers language, model, and cleanup controls on hover", () => {
    const onHover = vi.fn();
    const onLanguage = vi.fn();
    const onMode = vi.fn();
    const onModels = vi.fn();
    render(
      <DictationHud
        autoAdvance={false}
        floating
        model="Whisper Tiny"
        onHoverChange={onHover}
        onLanguageChange={onLanguage}
        onModeChange={onMode}
        onOpenModels={onModels}
        state="idle"
      />,
    );

    const widget = screen.getByRole("status", { name: "Spick is ready" });
    fireEvent.mouseEnter(widget);
    fireEvent.change(screen.getByLabelText("Language"), {
      target: { value: "ES" },
    });
    fireEvent.change(screen.getByLabelText("Mode"), {
      target: { value: "Clean" },
    });
    fireEvent.click(screen.getByRole("button", { name: /ModelWhisper Tiny/ }));
    fireEvent.mouseLeave(widget);

    expect(onHover).toHaveBeenNthCalledWith(1, true);
    expect(onHover).toHaveBeenLastCalledWith(false);
    expect(onLanguage).toHaveBeenCalledWith("ES");
    expect(onMode).toHaveBeenCalledWith("Clean");
    expect(onModels).toHaveBeenCalledOnce();
  });
});
