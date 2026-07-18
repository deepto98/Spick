import { useEffect, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  AudioLines,
  Check,
  CheckCircle2,
  ChevronRight,
  Cloud,
  Eraser,
  Globe2,
  Info,
  Keyboard,
  Languages,
  LockKeyhole,
  Mic2,
  ShieldCheck,
} from "lucide-react";
import type { AccessibilityPermissionStatus } from "../lib/nativeAccessibility";
import type { AppSettings } from "../types";
import { DictationHud } from "./DictationHud";
import { ShortcutKeys, SpickLogo } from "./Ui";

interface OnboardingProps {
  accessibilityStatus: AccessibilityPermissionStatus | null;
  accessibilityPending: boolean;
  accessibilityError?: string;
  settings: AppSettings;
  onRequestAccessibility: () => void;
  onRefreshAccessibility: () => void;
  onSettingsChange: (settings: AppSettings) => void;
  onComplete: () => void;
}

const totalSteps = 4;

export function Onboarding({
  accessibilityStatus,
  accessibilityPending,
  accessibilityError,
  settings,
  onRequestAccessibility,
  onRefreshAccessibility,
  onSettingsChange,
  onComplete,
}: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [shortcutPressed, setShortcutPressed] = useState(false);

  const accessibilityReady =
    accessibilityStatus?.state === "granted" ||
    accessibilityStatus?.state === "unsupported";

  useEffect(() => {
    if (step !== 3) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code === "Space") setShortcutPressed(true);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (event.code === "Space") setShortcutPressed(false);
    };
    window.addEventListener("keydown", onKeyDown);
    window.addEventListener("keyup", onKeyUp);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      window.removeEventListener("keyup", onKeyUp);
    };
  }, [step]);

  const next = () =>
    setStep((current) => Math.min(totalSteps - 1, current + 1));
  const previous = () => setStep((current) => Math.max(0, current - 1));

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
                Hold a shortcut and speak. Spick listens on this Mac, turns your
                voice into text, and keeps it ready for you.
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
                    <Eraser size={13} /> Example · cleanup comes later
                  </div>
                </div>
              </div>
              <div className="welcome-demo__hud">
                <DictationHud state="listening" />
              </div>
              <div className="welcome-demo__note">
                <Check size={13} /> Local dictation, without sending audio away
              </div>
            </div>
          </section>
        )}

        {step === 1 && (
          <section className="setup-step permission-step">
            <SetupHeading
              icon={<LockKeyhole size={21} />}
              eyebrow="BEFORE YOU START"
              title="Two small permissions."
              description="Microphone lets Spick hear you. Accessibility lets it remember where you started and stay out of private fields."
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
                icon={<Keyboard size={21} />}
                title="Accessibility"
                description="Keeps the shortcut tied to the field where you began. Automatic paste is still being hardened."
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
              title="Pick your language."
              description="Choose a default. You can change it later."
            />
            <div className="personalize-grid">
              <div className="setup-field-group">
                <span className="setup-field-group__label">
                  Speech language
                </span>
                <div className="language-choice-grid">
                  {["Auto-detect", "English", "Hindi", "Bengali"].map(
                    (language) => (
                      <button
                        type="button"
                        key={language}
                        className={
                          settings.language === language ? "active" : ""
                        }
                        onClick={() =>
                          onSettingsChange({ ...settings, language })
                        }
                      >
                        <span>
                          {language === "Auto-detect" ? (
                            <Globe2 size={18} />
                          ) : (
                            language.slice(0, 2).toUpperCase()
                          )}
                        </span>
                        <strong>{language}</strong>
                        {settings.language === language && (
                          <CheckCircle2 size={16} />
                        )}
                      </button>
                    ),
                  )}
                </div>
                <span className="setup-field-group__hint">
                  <Globe2 size={13} /> Auto lets the model choose one language
                  for each recording.
                </span>
              </div>
              <div className="setup-field-group">
                <span className="setup-field-group__label">
                  Where transcription runs
                </span>
                <div className="processing-choice-list">
                  <button type="button" className="active">
                    <span className="choice-icon">
                      <ShieldCheck size={18} />
                    </span>
                    <div>
                      <strong>On this Mac</strong>
                      <small>Works offline after you download a model</small>
                    </div>
                    <span className="recommended-label">SUGGESTED</span>
                    <CheckCircle2 size={17} />
                  </button>
                  <button type="button">
                    <span className="choice-icon">
                      <Cloud size={18} />
                    </span>
                    <div>
                      <strong>Cloud provider (planned)</strong>
                      <small>Bring an API key when adapters are ready</small>
                    </div>
                    <ChevronRight size={17} />
                  </button>
                </div>
                <span className="setup-field-group__hint">
                  You’ll choose the actual model from Engines after setup.
                </span>
              </div>
            </div>
            <StepActions onBack={previous} onNext={next} />
          </section>
        )}

        {step === 3 && (
          <section className="setup-step shortcut-step">
            <SetupHeading
              icon={<Keyboard size={21} />}
              eyebrow="ONE LAST THING"
              title="Give the shortcut a try."
              description="Spick records and transcribes locally, then keeps the words ready to copy from Today."
            />
            <div
              className={`shortcut-practice ${shortcutPressed ? "shortcut-practice--pressed" : ""}`}
            >
              <span className="shortcut-practice__label">HOLD TO RECORD</span>
              <div className="shortcut-practice__keys">
                <ShortcutKeys value={settings.hotkey} />
              </div>
              <div className="shortcut-practice__pulse">
                <Mic2 size={22} />
              </div>
              <strong>{shortcutPressed ? "Recording…" : "Try it here"}</strong>
              <span>
                {shortcutPressed
                  ? "Say something, then let go"
                  : "Hold Space to test the animation"}
              </span>
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
                  <small>whisper.cpp · choose a model next</small>
                </div>
              </span>
              <span>
                <Check size={14} />
                <div>
                  <strong>Cleanup</strong>
                  <small>Clean mode · planned</small>
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
              >
                Finish setup <ArrowRight size={17} />
              </button>
            </div>
          </section>
        )}
      </div>

      <footer className="onboarding-footer">
        <span>Early macOS build</span>
        <span>Local transcription · careful field handoff</span>
      </footer>
    </main>
  );
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
