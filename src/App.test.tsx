import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import App from "./App";

describe("Spick product shell", () => {
  beforeEach(() => {
    window.localStorage.clear();
    window.history.replaceState({}, "", "/");
  });

  afterEach(() => {
    cleanup();
  });

  it("gates onboarding progress on both simulated permissions", () => {
    render(<App />);

    fireEvent.click(screen.getByRole("button", { name: "Set up Spick" }));

    const continueButton = screen.getByRole("button", { name: "Continue" });
    expect(continueButton).toBeDisabled();

    fireEvent.click(
      screen.getByRole("button", { name: "Simulate microphone access" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Simulate accessibility access" }),
    );

    expect(continueButton).toBeEnabled();
  });

  it("renders sample dashboard data and navigates to engine setup", () => {
    window.localStorage.setItem("spick-onboarding-complete", "true");
    render(<App />);

    expect(
      screen.getByRole("heading", { name: "Your dictation workspace" }),
    ).toBeInTheDocument();
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
      screen.getByRole("button", { name: "Start dictation" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Set up Spick" }),
    ).not.toBeInTheDocument();
  });
});
