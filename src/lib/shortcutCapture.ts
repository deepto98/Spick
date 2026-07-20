import { shortcutDisplayName } from "./nativeSettings";

export type MacShortcutCapture =
  | { kind: "cancelled" }
  | { kind: "waiting" }
  | { kind: "invalid"; message: string }
  | { kind: "complete"; shortcut: string };

type ShortcutKeyboardEvent = Pick<
  KeyboardEvent,
  "altKey" | "code" | "ctrlKey" | "key" | "metaKey" | "shiftKey"
>;

const MODIFIER_CODES = new Set([
  "AltLeft",
  "AltRight",
  "ControlLeft",
  "ControlRight",
  "MetaLeft",
  "MetaRight",
  "ShiftLeft",
  "ShiftRight",
]);

const NAMED_KEY_CODES = new Set([
  "ArrowDown",
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "Backquote",
  "Backslash",
  "Backspace",
  "BracketLeft",
  "BracketRight",
  "Comma",
  "Delete",
  "End",
  "Enter",
  "Equal",
  "Home",
  "Minus",
  "PageDown",
  "PageUp",
  "Period",
  "Quote",
  "Semicolon",
  "Slash",
  "Space",
  "Tab",
]);

function supportedKey(code: string) {
  return (
    /^Key[A-Z]$/.test(code) ||
    /^Digit[0-9]$/.test(code) ||
    /^F(?:[1-9]|1[0-9]|2[0-4])$/.test(code) ||
    NAMED_KEY_CODES.has(code)
  );
}

/**
 * Convert one macOS keyboard event into the accelerator syntax accepted by
 * Tauri's global-shortcut parser. Option by itself is selected separately in
 * the UI because it uses Spick's tap/hold gesture listener, not an accelerator.
 */
export function captureMacShortcut(
  event: ShortcutKeyboardEvent,
): MacShortcutCapture {
  if (event.code === "Escape" || event.key === "Escape") {
    return { kind: "cancelled" };
  }

  if (MODIFIER_CODES.has(event.code)) return { kind: "waiting" };

  // Shift-only shortcuts interfere with ordinary capital-letter typing. A
  // custom global shortcut must include one of the less ambiguous modifiers.
  if (!event.metaKey && !event.ctrlKey && !event.altKey) {
    return {
      kind: "invalid",
      message: "Add Command, Control, or Option, then press another key.",
    };
  }

  if (!supportedKey(event.code)) {
    return {
      kind: "invalid",
      message:
        "That key can’t be used here. Try a letter, number, arrow, or function key.",
    };
  }

  const modifiers: string[] = [];
  if (event.metaKey) modifiers.push("Command");
  if (event.ctrlKey) modifiers.push("Control");
  if (event.altKey) modifiers.push("Option");
  if (event.shiftKey) modifiers.push("Shift");

  return {
    kind: "complete",
    shortcut: [...modifiers, event.code].join("+"),
  };
}

function semanticShortcut(shortcut: string) {
  return shortcutDisplayName(shortcut)
    .split("+")
    .map((part) => part.trim())
    .filter(Boolean)
    .sort()
    .join("+");
}

/**
 * Match the complete chord, including the absence of extra modifiers. The
 * saved shortcut may use either native accelerator syntax or its display
 * form; comparing semantic display tokens also keeps legacy aliases such as
 * `CommandOrControl` and `D` compatible with newly captured `Command` and
 * `KeyD` tokens.
 */
export function matchesMacShortcut(
  event: ShortcutKeyboardEvent,
  shortcut: string,
) {
  const captured = captureMacShortcut(event);
  return (
    captured.kind === "complete" &&
    semanticShortcut(captured.shortcut) === semanticShortcut(shortcut)
  );
}
