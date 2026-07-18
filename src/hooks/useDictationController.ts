import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { HudState } from "../types";
import { languagePolicyBadge } from "../lib/nativeSettings";
import {
  cancelDictationSession,
  getDictationSession,
  getLastTranscript,
  hasNativeRuntime,
  startDictationSession,
  stopDictationSession,
  subscribeToDictationState,
  subscribeToDictationTranscript,
  toHudState,
  type NativeDeliveryOutcome,
  type NativeDictationStateEvent,
  type NativeDictationTranscript,
} from "../lib/nativeDictation";

export function useDictationController(includeTranscripts = true) {
  const native = useMemo(() => hasNativeRuntime(), []);
  const [state, setState] = useState<HudState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [lastTranscript, setLastTranscript] =
    useState<NativeDictationTranscript | null>(null);
  const [delivery, setDelivery] = useState<NativeDeliveryOutcome | null>(null);
  const [language, setLanguage] = useState("AUTO");
  const pendingRef = useRef(false);
  const revisionRef = useRef(-1);
  const transcriptEventSeenRef = useRef(false);

  const applyNativeState = useCallback((event: NativeDictationStateEvent) => {
    if (
      !Number.isSafeInteger(event.revision) ||
      event.revision < revisionRef.current
    ) {
      return;
    }
    revisionRef.current = event.revision;
    setState(toHudState(event.state));
    if (event.session)
      setLanguage(languagePolicyBadge(event.session.languagePolicy));
    if (event.state === "listening") {
      setLastTranscript(null);
      setDelivery(null);
    } else if (event.session?.delivery) {
      setDelivery(event.session.delivery);
    }
    setError(
      event.state === "failed"
        ? (event.session?.error ?? "Recording failed")
        : null,
    );
  }, []);

  useEffect(() => {
    if (!native) return;

    let disposed = false;
    let unsubscribeState: (() => void) | undefined;
    let unsubscribeTranscript: (() => void) | undefined;
    transcriptEventSeenRef.current = false;

    const connect = async () => {
      try {
        const stopListening = await subscribeToDictationState((event) => {
          if (!disposed) applyNativeState(event);
        });
        if (disposed) {
          stopListening();
          return;
        }
        unsubscribeState = stopListening;
        if (includeTranscripts) {
          const stopTranscripts = await subscribeToDictationTranscript(
            (transcript) => {
              if (!disposed) {
                transcriptEventSeenRef.current = true;
                setLastTranscript(transcript);
                setDelivery(transcript.delivery);
              }
            },
          );
          if (disposed) {
            stopTranscripts();
            return;
          }
          unsubscribeTranscript = stopTranscripts;
        }
        const [current, transcript] = await Promise.all([
          getDictationSession(),
          includeTranscripts ? getLastTranscript() : Promise.resolve(null),
        ]);
        if (!disposed) {
          applyNativeState(current);
          if (!transcriptEventSeenRef.current) {
            setLastTranscript(transcript);
            if (transcript) setDelivery(transcript.delivery);
          }
        }
      } catch (reason) {
        if (!disposed) {
          setError(`Couldn’t connect to the recorder: ${String(reason)}`);
        }
      }
    };

    void connect();
    return () => {
      disposed = true;
      unsubscribeState?.();
      unsubscribeTranscript?.();
    };
  }, [applyNativeState, includeTranscripts, native]);

  const transitionTo = useCallback(
    (next: HudState) => {
      setError(null);

      if (!native) {
        setState(next);
        return;
      }
      if (
        next === "idle" &&
        state !== "listening" &&
        state !== "processing" &&
        state !== "inserting"
      ) {
        setState("idle");
        return;
      }
      if (pendingRef.current) return;

      const transition = async () => {
        switch (next) {
          case "idle":
            return cancelDictationSession();
          case "listening":
            return startDictationSession();
          case "processing":
            return stopDictationSession();
          case "inserting":
            return undefined;
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
          setError(`Couldn’t change the recording: ${String(reason)}`);
        })
        .finally(() => {
          pendingRef.current = false;
          setPending(false);
        });
    },
    [applyNativeState, native, state],
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

  return {
    error,
    delivery,
    language,
    lastTranscript,
    native,
    pending,
    state,
    transitionTo,
  };
}
