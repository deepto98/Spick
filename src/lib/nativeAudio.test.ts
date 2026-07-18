import { describe, expect, it } from "vitest";
import { clampAudioLevel } from "./nativeAudio";

describe("audio-level normalization", () => {
  it.each([
    [-1, 0],
    [0, 0],
    [0.42, 0.42],
    [1, 1],
    [4, 1],
    [Number.NaN, 0],
    [Number.POSITIVE_INFINITY, 0],
  ])("clamps %s to %s", (input, expected) => {
    expect(clampAudioLevel(input)).toBe(expected);
  });
});
