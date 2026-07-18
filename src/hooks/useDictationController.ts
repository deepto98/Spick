import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { HudState } from "../types";
import {
  getDictationSession,
  hasNativeRuntime,
  startDictationSession,
  stopDictationSession,
  subscribeToDictationState,
  toHudState,
  type NativeDictationStateEvent,
} from "../lib/nativeDictation";

export function useDictationController() {
  const native = useMemo(() => hasNativeRuntime(), []);
  const [state, setState] = useState<HudState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const pendingRef = useRef(false);
  const revisionRef = useRef(-1);

  const applyNativeState = useCallback((event: NativeDictationStateEvent) => {
    if (
      !Number.isSafeInteger(event.revision) ||
      event.revision < revisionRef.current
    ) {
      return;
    }
    revisionRef.current = event.revision;
    setState(toHudState(event.state));
    setError(
      event.state === "failed"
        ? (event.session?.error ?? "Recording failed")
        : null,
    );
  }, []);

  useEffect(() => {
    if (!native) return;

    let disposed = false;
    let unsubscribe: (() => void) | undefined;

    const connect = async () => {
      try {
        const stopListening = await subscribeToDictationState((event) => {
          if (!disposed) applyNativeState(event);
        });
        if (disposed) {
          stopListening();
          return;
        }
        unsubscribe = stopListening;
        const current = await getDictationSession();
        if (!disposed) applyNativeState(current);
      } catch (reason) {
        if (!disposed) {
          setError(`Couldn’t connect to the recorder: ${String(reason)}`);
        }
      }
    };

    void connect();
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [applyNativeState, native]);

  const transitionTo = useCallback(
    (next: HudState) => {
      setError(null);

      if (!native) {
        setState(next);
        return;
      }
      if (next === "idle") {
        setState("idle");
        return;
      }
      if (pendingRef.current) return;

      const transition = async () => {
        switch (next) {
          case "listening":
            return startDictationSession();
          case "processing":
            return stopDictationSession();
          case "success":
          case "error":
            return undefined;
        }
      };

      pendingRef.current = true;
      setPending(true);
      void transition()
        .then((event) => {
          if (event) applyNativeState(event);
        })
        .catch((reason) => {
          setError(`Couldn’t start or stop recording: ${String(reason)}`);
        })
        .finally(() => {
          pendingRef.current = false;
          setPending(false);
        });
    },
    [applyNativeState, native],
  );

  useEffect(() => {
    if (state !== "processing" || native) return;

    const timeout = window.setTimeout(() => {
      setState("success");
    }, 1150);

    return () => window.clearTimeout(timeout);
  }, [native, state]);

  useEffect(() => {
    if (state !== "success") return;
    const timeout = window.setTimeout(() => setState("idle"), 1250);
    return () => window.clearTimeout(timeout);
  }, [state]);

  return { error, native, pending, state, transitionTo };
}
