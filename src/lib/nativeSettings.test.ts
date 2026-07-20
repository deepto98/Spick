import { describe, expect, it } from "vitest";
import {
  cleanupEngineForLevel,
  cleanupLevelForEngine,
  languagePolicyBadge,
  languagePolicyForName,
  languagePolicyName,
  shortcutDisplayName,
  SPEECH_LANGUAGE_OPTIONS,
} from "./nativeSettings";

describe("native language settings", () => {
  it("maps the supported settings labels to native policies", () => {
    expect(languagePolicyForName("Auto-detect")).toEqual({ mode: "auto" });
    expect(languagePolicyForName("Hindi")).toEqual({
      mode: "fixed",
      language: "hi",
    });
    expect(languagePolicyForName("Japanese")).toEqual({
      mode: "fixed",
      language: "ja",
    });
    expect(languagePolicyForName("Tagalog")).toEqual({
      mode: "fixed",
      language: "fil",
    });
    expect(languagePolicyForName("Yoruba")).toEqual({
      mode: "fixed",
      language: "yo",
    });
    expect(languagePolicyForName("Hinglish")).toBeNull();
  });

  it("offers Auto plus the full multilingual whisper.cpp language set", () => {
    expect(SPEECH_LANGUAGE_OPTIONS).toHaveLength(100);
    expect(SPEECH_LANGUAGE_OPTIONS[0]).toBe("Auto-detect");
    expect(SPEECH_LANGUAGE_OPTIONS).toEqual(
      expect.arrayContaining([
        "Arabic",
        "Chinese",
        "Japanese",
        "Tamil",
        "Ukrainian",
        "Yoruba",
      ]),
    );
    expect(new Set(SPEECH_LANGUAGE_OPTIONS).size).toBe(
      SPEECH_LANGUAGE_OPTIONS.length,
    );
  });

  it("renders effective fixed and detected policies without guessing", () => {
    expect(languagePolicyName({ mode: "fixed", language: "en-IN" })).toBe(
      "English",
    );
    expect(languagePolicyBadge({ mode: "fixed", language: "bn-BD" })).toBe(
      "BN",
    );
    expect(languagePolicyBadge({ mode: "auto" })).toBe("AUTO");
    expect(languagePolicyName({ mode: "fixed", language: "jv-ID" })).toBe(
      "Javanese",
    );
    expect(languagePolicyName({ mode: "fixed", language: "tl-PH" })).toBe(
      "Tagalog",
    );
  });
});

describe("native shortcut settings", () => {
  it("renders native accelerator names as compact Mac keys", () => {
    expect(shortcutDisplayName("Option")).toBe("⌥");
    expect(shortcutDisplayName("CommandOrControl+Shift+Space")).toBe(
      "⌘+⇧+Space",
    );
    expect(shortcutDisplayName("Command+Option+KeyD")).toBe("⌘+⌥+D");
    expect(shortcutDisplayName("Control+ArrowUp")).toBe("⌃+↑");
  });
});

describe("native cleanup settings", () => {
  it("maps as-spoken and deterministic cleanup modes", () => {
    expect(cleanupEngineForLevel("Verbatim")).toBeNull();
    expect(cleanupEngineForLevel("Clean")).toEqual({
      provider: "builtIn",
      model: "readable-v1",
      location: "local",
    });
  });

  it("renders the active native cleanup engine honestly", () => {
    expect(cleanupLevelForEngine(null)).toBe("Verbatim");
    expect(
      cleanupLevelForEngine({
        provider: "builtIn",
        model: "readable-v1",
        location: "local",
      }),
    ).toBe("Clean");
    expect(
      cleanupLevelForEngine({
        provider: "openAi",
        model: "future-cleanup",
        location: "cloud",
      }),
    ).toBeNull();
  });
});
