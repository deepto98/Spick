import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
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
});
