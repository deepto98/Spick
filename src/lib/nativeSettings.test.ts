import { describe, expect, it } from "vitest";
import {
  cleanupEngineForLevel,
  cleanupLevelForEngine,
  languagePolicyBadge,
  languagePolicyForName,
  languagePolicyName,
} from "./nativeSettings";

describe("native language settings", () => {
  it("maps the supported settings labels to native policies", () => {
    expect(languagePolicyForName("Auto-detect")).toEqual({ mode: "auto" });
    expect(languagePolicyForName("Hindi")).toEqual({
      mode: "fixed",
      language: "hi",
    });
    expect(languagePolicyForName("Hinglish")).toBeNull();
  });

  it("renders effective fixed and detected policies without guessing", () => {
    expect(languagePolicyName({ mode: "fixed", language: "en-IN" })).toBe(
      "English",
    );
    expect(languagePolicyBadge({ mode: "fixed", language: "bn-BD" })).toBe(
      "BN",
    );
    expect(languagePolicyBadge({ mode: "auto" })).toBe("AUTO");
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
