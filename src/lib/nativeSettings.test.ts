import { describe, expect, it } from "vitest";
import {
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
