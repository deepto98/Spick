import { useEffect, useRef, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  AudioLines,
  Check,
  CheckCircle2,
  ChevronRight,
  Eraser,
  Info,
  Keyboard,
  Languages,
  LockKeyhole,
  Mic2,
  ShieldCheck,
} from "lucide-react";
import type { AccessibilityPermissionStatus } from "../lib/nativeAccessibility";
import type { NativeShortcutStatus } from "../lib/nativeShortcut";
import type { AppSettings, TranscriptionSource } from "../types";
import { DictationHud } from "./DictationHud";
import { SPEECH_LANGUAGE_OPTIONS } from "../lib/nativeSettings";
import { captureMacShortcut, matchesMacShortcut } from "../lib/shortcutCapture";
import { SelectField, ShortcutKeys, SpickLogo } from "./Ui";

interface OnboardingProps {
  accessibilityStatus: AccessibilityPermissionStatus | null;
  accessibilityPending: boolean;
  accessibilityError?: string;
  shortcutStatus: NativeShortcutStatus | null;
  shortcutPending: boolean;
  shortcutError?: string;
  settings: AppSettings;
  settingsError?: string;
  settingsReady: boolean;
  settingsSaving: boolean;
  transcriptionSource: TranscriptionSource;
  engineName?: string | null;
  engineReady: boolean;
  onRequestAccessibility: () => void;
  onRefreshAccessibility: () => void;
  onRefreshShortcut: () => void;
  onRequestInputMonitoring: () => void;
  onRetrySettings: () => void;
  onSettingsChange: (settings: AppSettings) => void;
  onComplete: () => void;
}

const totalSteps = 4;
const optionHoldThresholdMs = 280;

type ShortcutPracticeState =
  | "idle"
  | "optionArmed"
  | "optionDirtyIdle"
  | "optionHolding"
  | "optionToggleListening"
  | "optionToggleStopArmed"
  | "optionDirtyToggle"
  | "customHolding"
  | "customMismatch";

export function Onboarding({
  accessibilityStatus,
  accessibilityPending,
  accessibilityError,
  shortcutStatus,
  shortcutPending,
  shortcutError,
  settings,
  settingsError,
  settingsReady,
  settingsSaving,
  transcriptionSource,
  engineName,
  engineReady,
  onRequestAccessibility,
  onRefreshAccessibility,
  onRefreshShortcut,
  onRequestInputMonitoring,
  onRetrySettings,
  onSettingsChange,
  onComplete,
}: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [shortcutPractice, setShortcutPractice] =
    useState<ShortcutPracticeState>("idle");
  const shortcutPracticeRef = useRef<ShortcutPracticeState>("idle");

  const accessibilityReady =
    accessibilityStatus?.state === "granted" ||
    accessibilityStatus?.state === "unsupported";
  const usesOptionGesture = settings.hotkey === "⌥";
  const sourceCopy = onboardingSourceCopy(transcriptionSource, engineName);

  useEffect(() => {
    shortcutPracticeRef.current = "idle";
    if (step !== 3) return;

    let optionHoldTimer: number | null = null;
    let customMainKey: string | null = null;

    const updatePractice = (next: ShortcutPracticeState) => {
      shortcutPracticeRef.current = next;
      setShortcutPractice(next);
    };
    const clearOptionHoldTimer = () => {
      if (optionHoldTimer === null) return;
      window.clearTimeout(optionHoldTimer);
      optionHoldTimer = null;
    };
    const resetPractice = () => {
      clearOptionHoldTimer();
      customMainKey = null;
      updatePractice("idle");
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.repeat) return;

      if (!usesOptionGesture) {
        const captured = captureMacShortcut(event);
        if (captured.kind === "waiting") return;

        event.preventDefault();
        if (matchesMacShortcut(event, settings.hotkey)) {
          customMainKey = event.code;
          updatePractice("customHolding");
        } else {
          customMainKey = null;
          updatePractice("customMismatch");
        }
        return;
      }

      const isOption = event.key === "Alt";
      if (!isOption) {
        if (!event.altKey) return;
        clearOptionHoldTimer();
        if (shortcutPracticeRef.current === "optionArmed") {
          updatePractice("optionDirtyIdle");
        } else if (shortcutPracticeRef.current === "optionHolding") {
          updatePractice("idle");
        } else if (shortcutPracticeRef.current === "optionToggleStopArmed") {
          updatePractice("optionDirtyToggle");
        }
        return;
      }

      event.preventDefault();
      if (event.metaKey || event.ctrlKey || event.shiftKey) {
        updatePractice("optionDirtyIdle");
        return;
      }

      if (shortcutPracticeRef.current === "idle") {
        updatePractice("optionArmed");
        optionHoldTimer = window.setTimeout(() => {
          optionHoldTimer = null;
          if (shortcutPracticeRef.current === "optionArmed") {
            updatePractice("optionHolding");
          }
        }, optionHoldThresholdMs);
      } else if (shortcutPracticeRef.current === "optionToggleListening") {
        updatePractice("optionToggleStopArmed");
      }
    };

    const onKeyUp = (event: KeyboardEvent) => {
      if (!usesOptionGesture) {
        if (
          shortcutPracticeRef.current === "customHolding" &&
          event.code === customMainKey
        ) {
          customMainKey = null;
          updatePractice("idle");
        }
        return;
      }

      if (event.key !== "Alt") return;
      event.preventDefault();
      clearOptionHoldTimer();
      switch (shortcutPracticeRef.current) {
        case "optionArmed":
          updatePractice("optionToggleListening");
          break;
        case "optionHolding":
        case "optionToggleStopArmed":
        case "optionDirtyIdle":
          updatePractice("idle");
          break;
        case "optionDirtyToggle":
          updatePractice("optionToggleListening");
          break;
      }
    };

    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    window.addEventListener("blur", resetPractice);
    return () => {
      clearOptionHoldTimer();
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
      window.removeEventListener("blur", resetPractice);
    };
  }, [settings.hotkey, step, usesOptionGesture]);

  const shortcutRecording = [
    "optionHolding",
    "optionToggleListening",
    "optionToggleStopArmed",
    "optionDirtyToggle",
    "customHolding",
  ].includes(shortcutPractice);
  const shortcutPracticeTitle = shortcutRecording
    ? "Listening…"
    : shortcutPractice === "customMismatch"
      ? "That wasn’t your shortcut"
      : shortcutPractice === "optionArmed"
        ? "Option is down…"
        : "Try it here";
  const shortcutPracticeHelp = usesOptionGesture
    ? shortcutPractice === "optionHolding"
      ? "Release Option to finish"
      : [
            "optionToggleListening",
            "optionToggleStopArmed",
            "optionDirtyToggle",
          ].includes(shortcutPractice)
        ? "Tap Option again to finish"
        : shortcutPractice === "optionArmed"
          ? "Keep holding for push-to-talk, or release to start hands-free"
          : "Tap Option once to start and again to finish, or hold and release"
    : shortcutPractice === "customHolding"
      ? `Release the main key in ${settings.hotkey} to finish`
      : shortcutPractice === "customMismatch"
        ? `Press exactly ${settings.hotkey}; other chords stay inactive`
        : `Hold ${settings.hotkey} to start, then release the main key`;

  const resetShortcutPractice = () => {
    shortcutPracticeRef.current = "idle";
    setShortcutPractice("idle");
  };
  const next = () => {
    resetShortcutPractice();
    setStep((current) => Math.min(totalSteps - 1, current + 1));
  };
  const previous = () => {
    resetShortcutPractice();
    setStep((current) => Math.max(0, current - 1));
  };

  return (
    <main className="onboarding-shell">
      <header className="onboarding-topbar">
        <SpickLogo />
        <span className="prototype-badge">EARLY BUILD</span>
        {step > 0 && (
          <span className="onboarding-topbar__step">
            Step {step} of {totalSteps - 1}
          </span>
        )}
      </header>

      <div className="onboarding-progress" aria-hidden="true">
        <i
          style={{
            width: `${step === 0 ? 0 : (step / (totalSteps - 1)) * 100}%`,
          }}
        />
      </div>

      <div className={`onboarding-stage onboarding-stage--${step}`}>
        {step === 0 && (
          <section className="welcome-step">
            <div className="welcome-step__copy">
              <span className="onboarding-eyebrow">
                <AudioLines size={14} /> MEET SPICK
              </span>
              <h1>
                Talk it out.
                <br />
                <em>Catch the thought.</em>
              </h1>
              <p>
                {usesOptionGesture
                  ? "Tap or hold Option and speak. "
                  : "Hold your saved shortcut and speak. "}
                Spick listens on this Mac, turns your voice into text, and puts
                it where you started.
              </p>
              <button
                type="button"
                className="button button--primary button--large"
                onClick={next}
              >
                Let’s set it up <ArrowRight size={17} />
              </button>
              <span className="welcome-step__privacy">
                <ShieldCheck size={14} /> This walkthrough won’t turn on your
                mic
              </span>
            </div>
            <div className="welcome-demo" aria-label="Spick dictation example">
              <div className="welcome-demo__glow" />
              <div className="mock-window">
                <header>
                  <span />
                  <span />
                  <span />
                  <strong>New message</strong>
                </header>
                <div className="mock-window__body">
                  <div className="mock-recipient">
                    <span>To</span>
                    <strong>Product team</strong>
                  </div>
                  <div className="mock-text">
                    <p>
                      Could we move tomorrow’s design review to ten? I need one
                      more pass at the model notes.
                    </p>
                    <i />
                  </div>
                  <div className="mock-cleanup-badge">
                    <Eraser size={13} /> Example · light cleanup
                  </div>
                </div>
              </div>
              <div className="welcome-demo__hud">
                <DictationHud state="listening" shortcut={settings.hotkey} />
              </div>
              <div className="welcome-demo__note">
                <Check size={13} /> {sourceCopy.demo}
              </div>
            </div>
          </section>
        )}

        {step === 1 && (
          <section className="setup-step permission-step">
            <SetupHeading
              icon={<LockKeyhole size={21} />}
              eyebrow="BEFORE YOU START"
              title={
                usesOptionGesture
                  ? "Three small permissions."
                  : "Two small permissions."
              }
              description={
                usesOptionGesture
                  ? "Microphone hears you, Accessibility reaches the field, and Input Monitoring notices the Option key."
                  : "Microphone hears you, and Accessibility reaches the field where you started."
              }
            />
            <div className="permission-list">
              <PermissionCard
                number="01"
                icon={<Mic2 size={21} />}
                title="Microphone"
                description="Used only while you’re recording. macOS will ask the first time."
                ready={false}
                status="Asked on first use"
              />
              <PermissionCard
                number="02"
                icon={<LockKeyhole size={21} />}
                title="Accessibility"
                description="Keeps dictation tied to the field where you began and blocks protected controls."
                ready={accessibilityStatus?.state === "granted"}
                status={
                  accessibilityStatus?.state === "unsupported"
                    ? "Not needed in this preview"
                    : accessibilityStatus?.state === "granted"
                      ? "Allowed"
                      : undefined
                }
                button={
                  accessibilityStatus?.state === "missing"
                    ? accessibilityStatus.canRequest
                      ? "Allow in System Settings"
                      : "Check again"
                    : accessibilityStatus === null
                      ? accessibilityPending
                        ? "Checking…"
                        : "Check again"
                      : undefined
                }
                disabled={accessibilityPending}
                onGrant={
                  accessibilityStatus?.canRequest
                    ? onRequestAccessibility
                    : onRefreshAccessibility
                }
              />
              {usesOptionGesture && (
                <PermissionCard
                  number="03"
                  icon={<Keyboard size={21} />}
                  title="Input Monitoring"
                  description="Lets Spick notice a tap or hold of Option without taking focus from your app."
                  ready={shortcutStatus?.optionListenerActive === true}
                  status={
                    shortcutStatus?.optionListenerActive
                      ? "Ready"
                      : shortcutStatus?.inputMonitoringGranted
                        ? "Ready to activate"
                        : undefined
                  }
                  button={
                    shortcutStatus?.optionListenerActive
                      ? undefined
                      : shortcutStatus?.inputMonitoringGranted
                        ? "Activate Option"
                        : shortcutPending
                          ? "Opening…"
                          : "Allow in System Settings"
                  }
                  disabled={shortcutPending}
                  onGrant={
                    shortcutStatus?.inputMonitoringGranted
                      ? onRefreshShortcut
                      : onRequestInputMonitoring
                  }
                />
              )}
            </div>
            <div className="permission-note">
              <Info size={15} />
              <span>
                Spick checks the field before recording and again when your
                words are ready. If your cursor moves, the new field is left
                alone. Password fields are blocked before recording.
              </span>
            </div>
            {accessibilityError && (
              <div className="permission-error" role="alert">
                {accessibilityError}
              </div>
            )}
            {shortcutError && (
              <div className="permission-error" role="alert">
                {shortcutError}
              </div>
            )}
            <StepActions
              onBack={previous}
              onNext={next}
              nextDisabled={!accessibilityReady}
            />
          </section>
        )}

        {step === 2 && (
          <section className="setup-step personalize-step">
            <SetupHeading
              icon={<Languages size={21} />}
              eyebrow="A STARTING POINT"
              title="Make it sound like you."
              description="Choose what you speak and how much Spick should touch."
            />
            {settingsError && (
              <div
                className="permission-error setup-settings-error"
                role="alert"
              >
                <span>{settingsError}</span>
                {!settingsReady && (
                  <button
                    type="button"
                    className="button button--secondary button--small"
                    onClick={onRetrySettings}
                  >
                    Try again
                  </button>
                )}
              </div>
            )}
            <div className="personalize-grid">
              <div className="setup-field-group">
                <SelectField
                  label="Speech language"
                  value={settings.language}
                  options={[...SPEECH_LANGUAGE_OPTIONS]}
                  disabled={settingsSaving || !settingsReady}
                  onChange={(language) =>
                    onSettingsChange({ ...settings, language })
                  }
                  hint={
                    !settingsReady
                      ? settingsError
                        ? "Saved choices aren’t available yet."
                        : "Loading your saved choices…"
                      : settingsSaving
                        ? "Saving this choice…"
                        : "Auto travels best between models. Fixed choices are checked before recording; xAI has a shorter formatting-language list."
                  }
                />
              </div>
              <div className="setup-field-group">
                <span className="setup-field-group__label">Cleanup</span>
                <div className="processing-choice-list">
                  <button
                    type="button"
                    className={
                      settings.cleanupLevel === "Verbatim" ? "active" : ""
                    }
                    aria-pressed={settings.cleanupLevel === "Verbatim"}
                    disabled={settingsSaving || !settingsReady}
                    onClick={() =>
                      onSettingsChange({
                        ...settings,
                        cleanupLevel: "Verbatim",
                      })
                    }
                  >
                    <span className="choice-icon">
                      <AudioLines size={18} />
                    </span>
                    <div>
                      <strong>As transcribed</strong>
                      <small>Keep every word whisper.cpp returns</small>
                    </div>
                    {settings.cleanupLevel === "Verbatim" && (
                      <CheckCircle2 size={17} />
                    )}
                  </button>
                  <button
                    type="button"
                    className={
                      settings.cleanupLevel === "Clean" ? "active" : ""
                    }
                    aria-pressed={settings.cleanupLevel === "Clean"}
                    disabled={settingsSaving || !settingsReady}
                    onClick={() =>
                      onSettingsChange({ ...settings, cleanupLevel: "Clean" })
                    }
                  >
                    <span className="choice-icon">
                      <Eraser size={18} />
                    </span>
                    <div>
                      <strong>Trim obvious fillers</strong>
                      <small>Local list · keeps quoted and named uses</small>
                    </div>
                    {settings.cleanupLevel === "Clean" && (
                      <CheckCircle2 size={17} />
                    )}
                  </button>
                </div>
                <span className="setup-field-group__hint">
                  <ShieldCheck size={13} />
                  {sourceCopy.cleanup}
                </span>
              </div>
            </div>
            <StepActions
              onBack={previous}
              onNext={next}
              nextDisabled={settingsSaving || !settingsReady}
            />
          </section>
        )}

        {step === 3 && (
          <section className="setup-step shortcut-step">
            <SetupHeading
              icon={<Keyboard size={21} />}
              eyebrow="ONE LAST THING"
              title="Give the shortcut a try."
              description={
                usesOptionGesture
                  ? "Tap once to start and once to finish, or hold Option while you talk."
                  : "Hold your saved shortcut while you talk, then release to finish."
              }
            />
            <div
              className={`shortcut-practice ${shortcutRecording ? "shortcut-practice--pressed" : ""}`}
              aria-label="Shortcut practice"
            >
              <span className="shortcut-practice__label">
                {usesOptionGesture ? "TAP OR HOLD" : "HOLD TO RECORD"}
              </span>
              <div className="shortcut-practice__keys">
                <ShortcutKeys value={settings.hotkey} />
              </div>
              <div className="shortcut-practice__pulse">
                <Mic2 size={22} />
              </div>
              <div
                className="shortcut-practice__status"
                role="status"
                aria-live="polite"
                aria-atomic="true"
              >
                <strong>{shortcutPracticeTitle}</strong>
                <span>{shortcutPracticeHelp}</span>
              </div>
            </div>
            <div className="ready-summary">
              <span>
                <Check size={14} />
                <div>
                  <strong>Language</strong>
                  <small>{settings.language}</small>
                </div>
              </span>
              <span>
                <Check size={14} />
                <div>
                  <strong>Engine</strong>
                  <small>
                    {engineName ?? "Choose an engine"}
                    {!engineReady && " · finish in Engines"}
                  </small>
                </div>
              </span>
              <span>
                <Check size={14} />
                <div>
                  <strong>Cleanup</strong>
                  <small>
                    {settings.cleanupLevel === "Clean"
                      ? "Trim reviewed standalone fillers"
                      : "As transcribed"}
                  </small>
                </div>
              </span>
            </div>
            <div className="step-actions">
              <button
                type="button"
                className="button button--secondary"
                onClick={previous}
              >
                <ArrowLeft size={16} /> Back
              </button>
              <button
                type="button"
                className="button button--primary button--large"
                onClick={onComplete}
                disabled={settingsSaving || !settingsReady}
              >
                Finish setup <ArrowRight size={17} />
              </button>
            </div>
          </section>
        )}
      </div>

      <footer className="onboarding-footer">
        <span>Early macOS build</span>
        <span>{sourceCopy.footer}</span>
      </footer>
    </main>
  );
}

function onboardingSourceCopy(
  source: TranscriptionSource,
  engineName?: string | null,
) {
  switch (source) {
    case "cloud":
      return {
        demo: `Cloud transcription${engineName ? ` · ${engineName}` : ""}`,
        cleanup:
          "Audio leaves this Mac for transcription. Cleanup follows the selected provider; Spick’s local cleaner, when used, runs after the text returns.",
        footer: "Cloud transcription · careful field handoff",
      };
    case "localWithCloudFallback":
      return {
        demo: "Local first · cloud fallback is on",
        cleanup:
          "Local cleanup stays here. If fallback runs, provider handling applies to that upload.",
        footer: "Local first · cloud fallback on",
      };
    case "local":
      return {
        demo: "Local transcription · audio stays on this Mac",
        cleanup:
          "Both modes run on this Mac. Clean uses a short language-specific list; quoted uses and obvious word or code references stay untouched, with no sentence rewriting.",
        footer: "Local transcription · careful field handoff",
      };
    case "loading":
      return {
        demo: "Checking your saved transcription engine",
        cleanup: "Your saved cleanup and engine choices are still loading.",
        footer: "Loading saved engine · careful field handoff",
      };
    case "preview":
      return {
        demo: "Desktop preview · recording stays off",
        cleanup: "These choices take effect in the Tauri development app.",
        footer: "Browser preview · development app required",
      };
  }
}

function SetupHeading({
  icon,
  eyebrow,
  title,
  description,
}: {
  icon: React.ReactNode;
  eyebrow: string;
  title: string;
  description: string;
}) {
  return (
    <header className="setup-heading">
      <span className="setup-heading__icon">{icon}</span>
      <span className="onboarding-eyebrow">{eyebrow}</span>
      <h1>{title}</h1>
      <p>{description}</p>
    </header>
  );
}

interface PermissionCardProps {
  number: string;
  icon: React.ReactNode;
  title: string;
  description: string;
  ready: boolean;
  status?: string;
  button?: string;
  disabled?: boolean;
  onGrant?: () => void;
}

function PermissionCard({
  number,
  icon,
  title,
  description,
  ready,
  status,
  button,
  disabled,
  onGrant,
}: PermissionCardProps) {
  return (
    <article
      className={`permission-card ${ready ? "permission-card--ready" : ""}`}
    >
      <span className="permission-card__number">{number}</span>
      <span className="permission-card__icon">
        {ready ? <Check size={21} /> : icon}
      </span>
      <div>
        <strong>{title}</strong>
        <p>{description}</p>
      </div>
      {ready ? (
        <span className="permission-card__status permission-card__status--ready">
          <Check size={15} /> {status ?? "Done"}
        </span>
      ) : button && onGrant ? (
        <button
          type="button"
          className="button button--secondary"
          onClick={onGrant}
          disabled={disabled}
        >
          {button} <ChevronRight size={15} />
        </button>
      ) : (
        <span className="permission-card__status">{status}</span>
      )}
    </article>
  );
}

function StepActions({
  onBack,
  onNext,
  nextDisabled = false,
}: {
  onBack: () => void;
  onNext: () => void;
  nextDisabled?: boolean;
}) {
  return (
    <div className="step-actions">
      <button
        type="button"
        className="button button--secondary"
        onClick={onBack}
      >
        <ArrowLeft size={16} /> Back
      </button>
      <button
        type="button"
        className="button button--primary"
        onClick={onNext}
        disabled={nextDisabled}
      >
        Continue <ArrowRight size={16} />
      </button>
    </div>
  );
}
