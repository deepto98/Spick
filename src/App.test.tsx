import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";

describe("Spick product shell", () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.history.replaceState({}, "", "/");
  });

  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("explains real permissions without pretending to grant them", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));

    const continueButton = screen.getByRole("button", { name: "Continue" });
    expect(screen.getAllByText("Not needed in this preview")).toHaveLength(2);
    expect(screen.queryByText(/simulate/i)).not.toBeInTheDocument();
    expect(continueButton).toBeEnabled();
  });

  it("renders an honest empty dashboard and navigates to engine setup", () => {
    window.localStorage.setItem("spick-onboarding-complete", "true");
    render(<App />);

    expect(screen.getByRole("heading", { name: "Stats" })).toBeInTheDocument();
    expect(screen.getByText("No recordings yet")).toBeInTheDocument();
    expect(screen.queryByText("SAMPLE DATA")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Engines" }));

    expect(
      screen.getByRole("heading", { name: "Engines" }),
    ).toBeInTheDocument();
    expect(screen.getByText("None selected")).toBeInTheDocument();
  });

  it("finishes onboarding in Engines so setup can be completed", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));
    fireEvent.click(screen.getByRole("button", { name: "Continue" }));

    expect(
      screen.getByText(/Choose an engine · finish in Engines/),
    ).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "Choose an engine" }));

    expect(
      screen.getByRole("heading", { name: "Engines" }),
    ).toBeInTheDocument();
    expect(screen.getByText("Pick a working engine")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Choose an engine first" }),
    ).toBeDisabled();
    expect(window.localStorage.getItem("spick-onboarding-complete")).toBeNull();
    expect(
      screen.queryByRole("heading", { name: "Stats" }),
    ).not.toBeInTheDocument();
  });

  it("renders only the compact widget for the HUD window", () => {
    window.history.replaceState({}, "", "/?window=hud");
    render(<App />);

    expect(
      screen.getByRole("button", { name: "Start recording" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Let’s set it up" }),
    ).not.toBeInTheDocument();
  });

  it("runs the browser HUD through its preview lifecycle", () => {
    vi.useFakeTimers();
    window.history.replaceState({}, "", "/?window=hud");
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    expect(
      screen.getByRole("button", { name: "Finish recording" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Finish recording" }));
    expect(screen.getByText("Writing that down")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Cancel transcription" }),
    ).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1150));
    expect(screen.getByText("Got it")).toBeInTheDocument();

    act(() => vi.advanceTimersByTime(1250));
    expect(
      screen.getByRole("button", { name: "Start recording" }),
    ).toBeInTheDocument();
  });

  it("can leave processing without waiting for the preview", () => {
    window.history.replaceState({}, "", "/?window=hud");
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    fireEvent.click(screen.getByRole("button", { name: "Finish recording" }));
    fireEvent.click(
      screen.getByRole("button", { name: "Cancel transcription" }),
    );

    expect(
      screen.getByRole("button", { name: "Start recording" }),
    ).toBeInTheDocument();
  });
});
