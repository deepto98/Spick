import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { getHudSettings, setHudPresentation, startHudDrag } from "./nativeHud";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

describe("native HUD controls", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("reads and changes the persisted presentation", async () => {
    const settings = {
      position: "bottomCenter" as const,
      presentation: "compact" as const,
      customPosition: { x: 120, y: 80 },
    };
    vi.mocked(invoke).mockResolvedValue(settings);

    await expect(getHudSettings()).resolves.toEqual(settings);
    await expect(setHudPresentation("compact")).resolves.toEqual(settings);
    expect(invoke).toHaveBeenNthCalledWith(1, "get_hud_settings");
    expect(invoke).toHaveBeenNthCalledWith(2, "set_hud_presentation", {
      presentation: "compact",
    });
  });

  it("asks only the HUD window to begin a native drag", async () => {
    vi.mocked(invoke).mockResolvedValue(undefined);

    await startHudDrag();

    expect(invoke).toHaveBeenCalledWith("start_hud_drag");
  });
});
