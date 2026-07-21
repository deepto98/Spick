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
  setHudHovered,
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
  const dragging = useRef(false);
  const hovered = useRef(false);
  const hoverTimer = useRef<number | null>(null);

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

    // Layout effects run after React has committed the persisted compact
    // surface, but before the browser paints it. The native panel
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

  const resizeForHover = useCallback((next: boolean) => {
    void setHudHovered(next).catch((reason) => {
      setError(`Couldn’t resize the HUD controls: ${String(reason)}`);
    });
  }, []);

  useEffect(
    () => () => {
      if (hoverTimer.current !== null) {
        window.clearTimeout(hoverTimer.current);
      }
    },
    [],
  );

  const beginDrag = useCallback(() => {
    if (!enabled) return;
    if (hoverTimer.current !== null) {
      window.clearTimeout(hoverTimer.current);
      hoverTimer.current = null;
    }
    dragging.current = true;
    setError(null);
    void startHudDrag()
      .catch((reason) => {
        setError(`Couldn’t move the HUD: ${String(reason)}`);
      })
      .finally(() => {
        dragging.current = false;
        resizeForHover(hovered.current);
      });
  }, [enabled, resizeForHover]);

  const setHovered = useCallback(
    (nextHovered: boolean) => {
      if (!enabled) return;
      hovered.current = nextHovered;
      if (hoverTimer.current !== null) {
        window.clearTimeout(hoverTimer.current);
        hoverTimer.current = null;
      }
      // A pointer leave commonly fires while AppKit owns the synchronous drag
      // loop. Remember it, but never resize or reposition the panel mid-drag.
      // That race was the source of the HUD jumping away from the pointer.
      if (dragging.current) return;

      if (!nextHovered) {
        resizeForHover(false);
        return;
      }

      // Give a quick drag first refusal before expanding the native window.
      hoverTimer.current = window.setTimeout(() => {
        hoverTimer.current = null;
        if (!dragging.current && hovered.current) resizeForHover(true);
      }, 160);
    },
    [enabled, resizeForHover],
  );

  return {
    beginDrag,
    compact: settings?.presentation === "compact",
    error,
    pending,
    settings,
    setHovered,
    togglePresentation,
  };
}
