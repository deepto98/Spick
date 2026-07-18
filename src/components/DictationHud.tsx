import { useCallback, useEffect, useMemo, useState } from "react";
import { AlertTriangle, Check, LoaderCircle, Mic, X } from "lucide-react";
import type { HudState } from "../types";

interface DictationHudProps {
  state?: HudState;
  onStateChange?: (state: HudState) => void;
  floating?: boolean;
  language?: string;
  autoAdvance?: boolean;
  audioLevel?: number;
  disabled?: boolean;
  errorMessage?: string;
}

const sampleBars = [
  9, 15, 22, 12, 27, 18, 31, 14, 24, 18, 11, 20, 28, 16, 10, 23,
];

export function DictationHud({
  state: controlledState,
  onStateChange,
  floating = false,
  language = "EN",
  autoAdvance = true,
  audioLevel,
  disabled = false,
  errorMessage,
}: DictationHudProps) {
  const [internalState, setInternalState] = useState<HudState>("idle");
  const [elapsed, setElapsed] = useState(0);
  const state = controlledState ?? internalState;

  const transitionTo = useCallback(
    (next: HudState) => {
      if (next === "listening") setElapsed(0);
      setInternalState(next);
      onStateChange?.(next);
    },
    [onStateChange],
  );

  useEffect(() => {
    if (state !== "listening") return;

    const startedAt = Date.now();
    const interval = window.setInterval(() => {
      setElapsed(Math.floor((Date.now() - startedAt) / 1000));
    }, 250);
    return () => window.clearInterval(interval);
  }, [state]);

  useEffect(() => {
    if (!autoAdvance || state !== "processing") return;
    const timeout = window.setTimeout(() => transitionTo("success"), 1150);
    return () => window.clearTimeout(timeout);
  }, [autoAdvance, state, transitionTo]);

  useEffect(() => {
    if (!autoAdvance || state !== "success") return;
    const timeout = window.setTimeout(() => transitionTo("idle"), 1250);
    return () => window.clearTimeout(timeout);
  }, [autoAdvance, state, transitionTo]);

  const time = useMemo(() => {
    const minutes = Math.floor(elapsed / 60);
    const seconds = elapsed % 60;
    return `${minutes}:${seconds.toString().padStart(2, "0")}`;
  }, [elapsed]);

  if (state === "idle") {
    return (
      <button
        type="button"
        className={`dictation-hud dictation-hud--idle ${floating ? "dictation-hud--floating" : ""}`}
        onClick={() => transitionTo("listening")}
        aria-label="Start recording"
        disabled={disabled}
      >
        <span className="hud-orb">
          <Mic size={17} />
        </span>
        <span className="hud-idle-copy">
          <strong>Hold to record</strong>
          <small>⌘ ⇧ Space</small>
        </span>
      </button>
    );
  }

  return (
    <div
      className={`dictation-hud dictation-hud--${state} ${floating ? "dictation-hud--floating" : ""}`}
      role="status"
      aria-live="polite"
      aria-busy={disabled}
    >
      <span className="hud-orb" aria-hidden="true">
        {state === "listening" && <Mic size={17} />}
        {state === "processing" && <LoaderCircle size={17} />}
        {state === "success" && <Check size={18} />}
        {state === "error" && <AlertTriangle size={17} />}
      </span>

      {state === "listening" && (
        <>
          <div
            className={`hud-waveform ${audioLevel === undefined ? "" : "hud-waveform--live"}`}
            aria-label="Microphone audio level"
          >
            {sampleBars.map((height, index) => (
              <i
                key={index}
                style={
                  {
                    "--bar-height": `${
                      audioLevel === undefined
                        ? height
                        : Math.max(3, height * (0.22 + audioLevel * 0.78))
                    }px`,
                    "--bar-delay": `${index * -45}ms`,
                  } as React.CSSProperties
                }
              />
            ))}
          </div>
          <span className="hud-time">{time}</span>
          <span className="hud-language">{language}</span>
          <button
            type="button"
            className="hud-stop"
            onClick={() => transitionTo("processing")}
            aria-label="Finish recording"
            disabled={disabled}
          >
            <X size={15} />
          </button>
        </>
      )}

      {state === "processing" && (
        <div className="hud-status-copy">
          <strong>Finishing recording</strong>
          <span className="hud-loading">
            <i />
            <i />
            <i />
          </span>
        </div>
      )}

      {state === "success" && (
        <div className="hud-status-copy">
          <strong>Recording finished</strong>
          <span>Transcription isn’t connected yet</span>
        </div>
      )}

      {state === "error" && (
        <>
          <div className="hud-status-copy">
            <strong>Microphone unavailable</strong>
            <span>{errorMessage ?? "Check access and try again"}</span>
          </div>
          <button
            type="button"
            className="hud-stop"
            onClick={() => transitionTo("idle")}
            aria-label="Dismiss recording error"
          >
            <X size={15} />
          </button>
        </>
      )}
    </div>
  );
}
