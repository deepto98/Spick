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

  it("gates onboarding progress on both simulated permissions", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Let’s set it up" }));

    const continueButton = screen.getByRole("button", { name: "Continue" });
    expect(continueButton).toBeDisabled();

    fireEvent.click(
      screen.getByRole("button", { name: "Simulate mic approval" }),
    );
    fireEvent.click(
      screen.getByRole("button", {
        name: "Simulate Accessibility approval",
      }),
    );

    expect(continueButton).toBeEnabled();
  });

  it("renders sample dashboard data and navigates to engine setup", () => {
    window.localStorage.setItem("spick-onboarding-complete", "true");
    render(<App />);

    expect(screen.getByRole("heading", { name: "Today" })).toBeInTheDocument();
    expect(screen.getByText("SAMPLE DATA")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Engines" }));

    expect(
      screen.getByRole("heading", { name: "Engines" }),
    ).toBeInTheDocument();
    expect(screen.getByText("None selected")).toBeInTheDocument();
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
    expect(screen.getByText("Transcript ready")).toBeInTheDocument();

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
