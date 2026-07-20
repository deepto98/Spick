import { useCallback, useEffect, useState } from "react";
import {
  getHudSettings,
  setHudPresentation,
  startHudDrag,
} from "../lib/nativeHud";
import type { NativeHudSettings } from "../lib/nativeSettings";

export function useHudWindow(enabled: boolean) {
  const [settings, setSettings] = useState<NativeHudSettings | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    void getHudSettings()
      .then((next) => {
        if (!disposed) setSettings(next);
      })
      .catch((reason) => {
        if (!disposed)
          setError(`Couldn’t read HUD settings: ${String(reason)}`);
      });
    return () => {
      disposed = true;
    };
  }, [enabled]);

  const togglePresentation = useCallback(async () => {
    if (!enabled || pending) return;
    const next = settings?.presentation === "compact" ? "expanded" : "compact";
    setPending(true);
    setError(null);
    try {
      setSettings(await setHudPresentation(next));
    } catch (reason) {
      setError(`Couldn’t resize the HUD: ${String(reason)}`);
    } finally {
      setPending(false);
    }
  }, [enabled, pending, settings?.presentation]);

  const beginDrag = useCallback(() => {
    if (!enabled) return;
    setError(null);
    void startHudDrag().catch((reason) => {
      setError(`Couldn’t move the HUD: ${String(reason)}`);
    });
  }, [enabled]);

  return {
    beginDrag,
    compact: settings?.presentation === "compact",
    error,
    pending,
    settings,
    togglePresentation,
  };
}
