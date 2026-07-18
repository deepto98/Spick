import { describe, expect, it } from "vitest";
import {
  formatModelBytes,
  modelStatus,
  type LocalModelSummary,
} from "./nativeModels";

function summary(
  state: LocalModelSummary["state"],
  active = false,
): LocalModelSummary {
  return {
    active,
    installedBytes: 0,
    state,
    manifest: {
      id: "whisper-tiny",
      displayName: "Whisper Tiny",
      fileName: "ggml-tiny.bin",
      family: "tiny",
      languages: "multilingual",
      quantization: "f16",
      downloadBytes: 77_691_713,
      sha256: "0".repeat(64),
      sourceUrl: "https://example.com/model.bin",
    },
  };
}

describe("native local models", () => {
  it("only labels an installed selected model as active", () => {
    expect(modelStatus(summary("installed", true))).toBe("active");
    expect(modelStatus(summary("installed"))).toBe("ready");
    expect(modelStatus(summary("needsVerification", true))).toBe("ready");
    expect(modelStatus(summary("notInstalled", true))).toBe("available");
    expect(modelStatus(summary("invalid"))).toBe("invalid");
  });

  it("formats decimal download sizes", () => {
    expect(formatModelBytes(77_691_713)).toBe("77.7 MB");
    expect(formatModelBytes(Number.NaN)).toBe("0 MB");
  });
});
