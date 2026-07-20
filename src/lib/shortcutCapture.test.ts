import { describe, expect, it } from "vitest";
import { captureMacShortcut, matchesMacShortcut } from "./shortcutCapture";

function keyEvent(
  overrides: Partial<Parameters<typeof captureMacShortcut>[0]> = {},
) {
  return {
    altKey: false,
    code: "KeyD",
    ctrlKey: false,
    key: "d",
    metaKey: false,
    shiftKey: false,
    ...overrides,
  };
}

describe("Mac shortcut capture", () => {
  it("maps a Mac key chord to Tauri accelerator syntax", () => {
    expect(
      captureMacShortcut(
        keyEvent({ code: "KeyD", key: "D", metaKey: true, shiftKey: true }),
      ),
    ).toEqual({
      kind: "complete",
      shortcut: "Command+Shift+KeyD",
    });
  });

  it("waits on modifier-only events and lets Escape cancel", () => {
    expect(
      captureMacShortcut(
        keyEvent({ code: "MetaLeft", key: "Meta", metaKey: true }),
      ),
    ).toEqual({ kind: "waiting" });
    expect(
      captureMacShortcut(keyEvent({ code: "Escape", key: "Escape" })),
    ).toEqual({ kind: "cancelled" });
  });

  it("rejects typing keys without a safe global modifier", () => {
    expect(
      captureMacShortcut(keyEvent({ shiftKey: true, key: "D" })),
    ).toMatchObject({
      kind: "invalid",
      message: expect.stringContaining("Command, Control, or Option"),
    });
  });

  it("rejects keys the native shortcut parser does not support", () => {
    expect(
      captureMacShortcut(keyEvent({ code: "IntlRo", key: "_", ctrlKey: true })),
    ).toMatchObject({
      kind: "invalid",
      message: expect.stringContaining("can’t be used"),
    });
  });

  it("matches the exact saved chord without depending on token aliases or order", () => {
    const commandShiftD = keyEvent({
      code: "KeyD",
      key: "D",
      metaKey: true,
      shiftKey: true,
    });

    expect(matchesMacShortcut(commandShiftD, "Shift+CommandOrControl+D")).toBe(
      true,
    );
    expect(matchesMacShortcut(commandShiftD, "⌘+⇧+D")).toBe(true);
    expect(matchesMacShortcut(commandShiftD, "Command+KeyD")).toBe(false);
    expect(
      matchesMacShortcut(
        keyEvent({
          code: "KeyD",
          key: "D",
          altKey: true,
          metaKey: true,
          shiftKey: true,
        }),
        "Command+Shift+KeyD",
      ),
    ).toBe(false);
  });
});
