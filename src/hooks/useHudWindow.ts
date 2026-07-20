import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import {
  getHudSettings,
  markHudRendererReady,
  setHudPresentation,
  startHudDrag,
} from "../lib/nativeHud";
import type { NativeHudSettings } from "../lib/nativeSettings";

export function useHudWindow(enabled: boolean) {
  const [settings, setSettings] = useState<NativeHudSettings | null>(null);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const rendererReadySent = useRef(false);
  const rendererReadyAttempt = useRef(0);

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

  const hydrated = settings !== null;
  useLayoutEffect(() => {
    if (!enabled || !hydrated || rendererReadySent.current) return;

    // Layout effects run after React has committed the persisted compact or
    // expanded surface, but before the browser paints it. The native panel
    // stays hidden until this explicit acknowledgement reaches Rust.
    const generation = ++rendererReadyAttempt.current;
    let retryTimer: number | null = null;

    const acknowledge = (attempt: number) => {
      void markHudRendererReady()
        .then(() => {
          if (rendererReadyAttempt.current !== generation) return;
          rendererReadySent.current = true;
          setError(null);
        })
        .catch((reason) => {
          if (rendererReadyAttempt.current !== generation) return;
          if (attempt < 2) {
            retryTimer = window.setTimeout(
              () => acknowledge(attempt + 1),
              100 * 2 ** attempt,
            );
            return;
          }
          setError(`Couldn’t show the HUD: ${String(reason)}`);
        });
    };
    acknowledge(0);

    return () => {
      if (rendererReadyAttempt.current === generation) {
        rendererReadyAttempt.current += 1;
      }
      if (retryTimer !== null) window.clearTimeout(retryTimer);
    };
  }, [enabled, hydrated]);

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
