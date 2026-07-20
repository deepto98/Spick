import { useCallback, useEffect, useMemo, useState } from "react";
import type { CSSProperties, ReactNode } from "react";
import {
  AlertTriangle,
  Check,
  Copy,
  GripVertical,
  LoaderCircle,
  Maximize2,
  Mic,
  Minimize2,
  X,
} from "lucide-react";
import type { NativeDeliveryOutcome } from "../lib/nativeDictation";
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
  delivery?: NativeDeliveryOutcome | null;
  shortcut?: string;
  compact?: boolean;
  compactPending?: boolean;
  onToggleCompact?: () => void;
  onMovePointerDown?: () => void;
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
  delivery,
  shortcut = "⌥",
  compact = false,
  compactPending = false,
  onToggleCompact,
  onMovePointerDown,
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
  const delivered = delivery?.status === "inserted";
  const deliveryCopy = describeHudDelivery(delivery);

  const frame = (content: ReactNode) => (
    <div
      className={`dictation-hud-frame ${floating ? "dictation-hud-frame--floating" : ""}`}
    >
      {content}
      {(onMovePointerDown || onToggleCompact) && (
        <div className="hud-window-controls">
          {onMovePointerDown && (
            <button
              type="button"
              className="hud-window-control hud-window-control--move"
              aria-label="Move dictation widget"
              onPointerDown={(event) => {
                event.preventDefault();
                onMovePointerDown();
              }}
            >
              <GripVertical size={13} />
            </button>
          )}
          {onToggleCompact && (
            <button
              type="button"
              className="hud-window-control"
              aria-label="Minimize dictation widget"
              onClick={onToggleCompact}
              disabled={compactPending}
            >
              <Minimize2 size={12} />
            </button>
          )}
        </div>
      )}
    </div>
  );

  if (compact) {
    const compactBars = sampleBars.slice(4, 11);
    return (
      <div
        className={`dictation-hud-compact dictation-hud-compact--${state}`}
        role="status"
        aria-label={compactStatusLabel(state)}
        aria-live="polite"
      >
        <button
          type="button"
          className="hud-compact-grip"
          aria-label="Move dictation widget"
          onPointerDown={(event) => {
            event.preventDefault();
            onMovePointerDown?.();
          }}
        >
          <GripVertical size={13} />
        </button>
        <div
          className={`hud-compact-wave ${state === "listening" ? "hud-compact-wave--live" : ""}`}
          aria-hidden="true"
        >
          {compactBars.map((height, index) => (
            <i
              key={index}
              style={
                {
                  "--bar-height": `${
                    state === "listening"
                      ? Math.max(
                          4,
                          height * (0.22 + (audioLevel ?? 0.55) * 0.78),
                        )
                      : 4
                  }px`,
                  "--bar-delay": `${index * -55}ms`,
                } as CSSProperties
              }
            />
          ))}
        </div>
        <button
          type="button"
          className="hud-compact-expand"
          aria-label="Expand dictation widget"
          onClick={onToggleCompact}
          disabled={compactPending || !onToggleCompact}
        >
          <Maximize2 size={11} />
        </button>
      </div>
    );
  }

  if (state === "idle") {
    return frame(
      <button
        type="button"
        className="dictation-hud dictation-hud--idle"
        onClick={() => transitionTo("listening")}
        aria-label="Start recording"
        disabled={disabled}
      >
        <span className="hud-orb">
          <Mic size={17} />
        </span>
        <span className="hud-idle-copy">
          <strong>Tap or hold to talk</strong>
          <small>{shortcut}</small>
        </span>
      </button>,
    );
  }

  return frame(
    <div
      className={`dictation-hud dictation-hud--${state} ${
        state === "success" && delivery && !delivered
          ? "dictation-hud--recovery"
          : ""
      }`}
      role="status"
      aria-live="polite"
      aria-busy={disabled || state === "starting"}
    >
      <span className="hud-orb" aria-hidden="true">
        {state === "starting" && <LoaderCircle size={17} />}
        {state === "listening" && <Mic size={17} />}
        {state === "processing" && <LoaderCircle size={17} />}
        {state === "inserting" && <LoaderCircle size={17} />}
        {state === "success" &&
          (delivery && !delivered ? <Copy size={17} /> : <Check size={18} />)}
        {state === "error" && <AlertTriangle size={17} />}
      </span>

      {state === "starting" && (
        <>
          <div className="hud-status-copy">
            <strong>Opening microphone</strong>
            <span>Listening starts when it’s ready</span>
          </div>
          <button
            type="button"
            className="hud-stop"
            onClick={() => transitionTo("idle")}
            aria-label="Cancel microphone startup"
            disabled={disabled}
          >
            <X size={15} />
          </button>
        </>
      )}

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
                  } as CSSProperties
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
        <>
          <div className="hud-status-copy">
            <strong>Writing that down</strong>
            <span className="hud-loading">
              <i />
              <i />
              <i />
            </span>
          </div>
          <button
            type="button"
            className="hud-stop"
            onClick={() => transitionTo("idle")}
            aria-label="Cancel transcription"
            disabled={disabled}
          >
            <X size={15} />
          </button>
        </>
      )}

      {state === "inserting" && (
        <div className="hud-status-copy">
          <strong>Preparing your text</strong>
          <span>Checking where you started</span>
        </div>
      )}

      {state === "success" && (
        <div className="hud-status-copy">
          <strong>{deliveryCopy.title}</strong>
          <span>{deliveryCopy.detail}</span>
        </div>
      )}

      {state === "error" && (
        <>
          <div className="hud-status-copy">
            <strong>Couldn’t finish</strong>
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
    </div>,
  );
}

function compactStatusLabel(state: HudState) {
  switch (state) {
    case "idle":
      return "Spick is ready";
    case "starting":
      return "Spick is opening the microphone";
    case "listening":
      return "Spick is listening";
    case "processing":
      return "Spick is transcribing";
    case "inserting":
      return "Spick is typing";
    case "success":
      return "Dictation complete";
    case "error":
      return "Dictation failed";
  }
}

function describeHudDelivery(delivery?: NativeDeliveryOutcome | null) {
  if (!delivery) {
    return {
      title: "Got it",
      detail: "Ready when you need it",
    };
  }

  if (delivery.status === "inserted") {
    return {
      title: "Typed",
      detail: delivery.targetApp
        ? `Back in ${delivery.targetApp}`
        : "Back where you started",
    };
  }

  const detail = (() => {
    switch (delivery.status) {
      case "focusChanged":
        return "Your cursor moved—copy from Spick";
      case "secureField":
        return "Password fields are always left alone";
      case "accessibilityMissing":
        return "Allow Accessibility, or copy from Spick";
      case "unsupported":
        return "This field needs a manual paste";
      case "failed":
        return "The field refused it—copy from Spick";
      case "indeterminate":
        return "Check the field before copying";
    }
  })();

  return {
    title: delivery.transcriptAvailable ? "Text ready to copy" : "Not typed",
    detail,
  };
}
