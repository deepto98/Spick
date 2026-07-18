import { useCallback, useEffect, useMemo, useState } from "react";
import { Check, Mic, Sparkles, X } from "lucide-react";
import type { HudState } from "../types";

interface DictationHudProps {
  state?: HudState;
  onStateChange?: (state: HudState) => void;
  floating?: boolean;
  language?: string;
  autoAdvance?: boolean;
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
        aria-label="Start dictation"
      >
        <span className="hud-orb">
          <Mic size={17} />
        </span>
        <span className="hud-idle-copy">
          <strong>Hold to speak</strong>
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
    >
      <span className="hud-orb" aria-hidden="true">
        {state === "listening" && <Mic size={17} />}
        {state === "processing" && <Sparkles size={17} />}
        {state === "success" && <Check size={18} />}
      </span>

      {state === "listening" && (
        <>
          <div className="hud-waveform" aria-label="Microphone audio level">
            {sampleBars.map((height, index) => (
              <i
                key={index}
                style={
                  {
                    "--bar-height": `${height}px`,
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
            aria-label="Finish dictation"
          >
            <X size={15} />
          </button>
        </>
      )}

      {state === "processing" && (
        <div className="hud-status-copy">
          <strong>Polishing your words</strong>
          <span className="hud-loading">
            <i />
            <i />
            <i />
          </span>
        </div>
      )}

      {state === "success" && (
        <div className="hud-status-copy">
          <strong>Preview complete</strong>
          <span>Audio and insertion are not connected yet</span>
        </div>
      )}
    </div>
  );
}
