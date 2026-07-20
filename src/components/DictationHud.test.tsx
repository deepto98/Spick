import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DictationHud } from "./DictationHud";

afterEach(cleanup);

describe("dictation delivery HUD", () => {
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

  it("keeps compact listening feedback movable and expandable", () => {
    const onMove = vi.fn();
    const onExpand = vi.fn();
    render(
      <DictationHud
        audioLevel={0.8}
        compact
        onMovePointerDown={onMove}
        onToggleCompact={onExpand}
        state="listening"
      />,
    );

    expect(
      screen.getByRole("status", { name: "Spick is listening" }),
    ).toBeInTheDocument();
    fireEvent.pointerDown(
      screen.getByRole("button", { name: "Move dictation widget" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Expand dictation widget" }),
    );
    expect(onMove).toHaveBeenCalledOnce();
    expect(onExpand).toHaveBeenCalledOnce();
  });
});
