import { useCallback, useEffect, useMemo, useState } from "react";
import type { HudState } from "../types";
import {
  completeDictationSession,
  getDictationSession,
  hasNativeRuntime,
  startDictationSession,
  stopDictationSession,
  subscribeToDictationState,
  toHudState,
  type NativeDictationStateEvent,
} from "../lib/nativeDictation";

interface DictationControllerOptions {
  autoComplete: boolean;
}

export function useDictationController({
  autoComplete,
}: DictationControllerOptions) {
  const native = useMemo(() => hasNativeRuntime(), []);
  const [state, setState] = useState<HudState>("idle");
  const [error, setError] = useState<string | null>(null);

  const applyNativeState = useCallback((event: NativeDictationStateEvent) => {
    setState(toHudState(event.state));
    setError(
      event.state === "failed"
        ? (event.session?.error ?? "Dictation failed")
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
          setError(`Could not connect to native dictation: ${String(reason)}`);
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

      const transition = async () => {
        switch (next) {
          case "listening":
            return startDictationSession();
          case "processing":
            return stopDictationSession();
          case "success":
            return completeDictationSession();
          case "idle":
            setState("idle");
            return undefined;
        }
      };

      void transition()
        .then((event) => {
          if (event) applyNativeState(event);
        })
        .catch((reason) => {
          setState("idle");
          setError(
            `Native dictation could not change state: ${String(reason)}`,
          );
        });
    },
    [applyNativeState, native],
  );

  useEffect(() => {
    if (state !== "processing" || (!autoComplete && native)) return;

    const timeout = window.setTimeout(() => {
      if (native) {
        void completeDictationSession()
          .then(applyNativeState)
          .catch((reason) => {
            setState("idle");
            setError(`Native preview could not complete: ${String(reason)}`);
          });
      } else {
        setState("success");
      }
    }, 1150);

    return () => window.clearTimeout(timeout);
  }, [applyNativeState, autoComplete, native, state]);

  useEffect(() => {
    if (state !== "success") return;
    const timeout = window.setTimeout(() => setState("idle"), 1250);
    return () => window.clearTimeout(timeout);
  }, [state]);

  return { error, native, state, transitionTo };
}
