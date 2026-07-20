import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { TopBar } from "./TopBar";

describe("TopBar", () => {
  afterEach(cleanup);

  it("shows only actions that are wired", () => {
    render(<TopBar activeView="today" hotkey="⌥" onOpenNav={vi.fn()} />);

    expect(
      screen.getByRole("button", { name: "Open navigation" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Help" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Notifications" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Account menu" })).toBeNull();
  });
});
