import { useEffect, useState } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  CheckCircle2,
  ChevronRight,
  Globe2,
  Keyboard,
  Languages,
  LockKeyhole,
  Mic2,
  ShieldCheck,
  Sparkles,
  WandSparkles,
} from "lucide-react";
import type { AppSettings } from "../types";
import { DictationHud } from "./DictationHud";
import { ShortcutKeys, SpickLogo } from "./Ui";

interface OnboardingProps {
  settings: AppSettings;
  onSettingsChange: (settings: AppSettings) => void;
  onComplete: () => void;
}

const totalSteps = 4;

export function Onboarding({
  settings,
  onSettingsChange,
  onComplete,
}: OnboardingProps) {
  const [step, setStep] = useState(0);
  const [microphoneReady, setMicrophoneReady] = useState(false);
  const [accessibilityReady, setAccessibilityReady] = useState(false);
  const [shortcutPressed, setShortcutPressed] = useState(false);

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
        <span className="prototype-badge">INTERACTIVE PREVIEW</span>
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
                <Sparkles size={14} /> YOUR VOICE, EVERYWHERE
              </span>
              <h1>
                Speak naturally.
                <br />
                <em>Type beautifully.</em>
              </h1>
              <p>
                Spick is being built to turn your voice into clear, polished
                text in every app—with a shortcut that’s always within reach.
              </p>
              <button
                type="button"
                className="button button--primary button--large"
                onClick={next}
              >
                Set up Spick <ArrowRight size={17} />
              </button>
              <span className="welcome-step__privacy">
                <ShieldCheck size={14} /> This preview does not access your
                microphone
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
                      Let’s keep the new onboarding focused on one clear
                      promise: speak naturally, then get polished text anywhere.
                    </p>
                    <i />
                  </div>
                  <div className="mock-cleanup-badge">
                    <WandSparkles size={13} /> 3 fillers removed
                  </div>
                </div>
              </div>
              <div className="welcome-demo__hud">
                <DictationHud state="listening" />
              </div>
              <div className="welcome-demo__note">
                <Check size={13} /> Designed for browsers, editors & desktop
                apps
              </div>
            </div>
          </section>
        )}

        {step === 1 && (
          <section className="setup-step permission-step">
            <SetupHeading
              icon={<LockKeyhole size={21} />}
              eyebrow="ONE-TIME SETUP"
              title="Two permissions. That’s it."
              description="Spick needs to hear you and place text at your cursor. You remain in control."
            />
            <div className="permission-list">
              <PermissionCard
                number="01"
                icon={<Mic2 size={21} />}
                title="Microphone"
                description="Capture your voice only while you hold the shortcut."
                ready={microphoneReady}
                button="Simulate microphone access"
                onGrant={() => setMicrophoneReady(true)}
              />
              <PermissionCard
                number="02"
                icon={<Keyboard size={21} />}
                title="Accessibility"
                description="Insert polished text into the focused input in any app."
                ready={accessibilityReady}
                button="Simulate accessibility access"
                onGrant={() => setAccessibilityReady(true)}
              />
            </div>
            <div className="simulation-note">
              <Sparkles size={15} />
              <span>
                This preview simulates macOS permissions. The native app will
                open the relevant System Settings panels.
              </span>
            </div>
            <StepActions
              onBack={previous}
              onNext={next}
              nextDisabled={!microphoneReady || !accessibilityReady}
            />
          </section>
        )}

        {step === 2 && (
          <section className="setup-step personalize-step">
            <SetupHeading
              icon={<Languages size={21} />}
              eyebrow="MAKE IT YOURS"
              title="How do you speak?"
              description="Choose a starting point. You can switch languages and engines at any time."
            />
            <div className="personalize-grid">
              <div className="setup-field-group">
                <span className="setup-field-group__label">
                  Speech language
                </span>
                <div className="language-choice-grid">
                  {["Auto-detect", "English", "Hindi", "Hinglish"].map(
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
                  <Sparkles size={13} /> Auto-detect can preserve language
                  changes between phrases.
                </span>
              </div>
              <div className="setup-field-group">
                <span className="setup-field-group__label">
                  Processing preference
                </span>
                <div className="processing-choice-list">
                  <button type="button" className="active">
                    <span className="choice-icon">
                      <ShieldCheck size={18} />
                    </span>
                    <div>
                      <strong>On this Mac</strong>
                      <small>Private, fast, and works offline</small>
                    </div>
                    <span className="recommended-label">RECOMMENDED</span>
                    <CheckCircle2 size={17} />
                  </button>
                  <button type="button">
                    <span className="choice-icon">
                      <Sparkles size={18} />
                    </span>
                    <div>
                      <strong>Cloud engine</strong>
                      <small>Connect a provider after setup</small>
                    </div>
                    <ChevronRight size={17} />
                  </button>
                </div>
                <span className="setup-field-group__hint">
                  Model availability shown here is representative until native
                  hardware detection runs.
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
              eyebrow="YOUR NEW SHORTCUT"
              title="Ready when you are."
              description="Preview the hold-to-speak interaction now. Audio transcription and text insertion are the next native milestone."
            />
            <div
              className={`shortcut-practice ${shortcutPressed ? "shortcut-practice--pressed" : ""}`}
            >
              <span className="shortcut-practice__label">PRESS & HOLD</span>
              <div className="shortcut-practice__keys">
                <ShortcutKeys value={settings.hotkey} />
              </div>
              <div className="shortcut-practice__pulse">
                <Mic2 size={22} />
              </div>
              <strong>{shortcutPressed ? "Listening…" : "Try it now"}</strong>
              <span>
                {shortcutPressed
                  ? "Say a few words, then release Space"
                  : "Hold Space to preview the interaction"}
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
                  <small>whisper.cpp · planned local runtime</small>
                </div>
              </span>
              <span>
                <Check size={14} />
                <div>
                  <strong>Cleanup</strong>
                  <small>Clean mode</small>
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
                Start speaking <ArrowRight size={17} />
              </button>
            </div>
          </section>
        )}
      </div>

      <footer className="onboarding-footer">
        <span>Spick preview · macOS</span>
        <span>Privacy-first architecture</span>
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
  button: string;
  onGrant: () => void;
}

function PermissionCard({
  number,
  icon,
  title,
  description,
  ready,
  button,
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
      <button
        type="button"
        className={`button ${ready ? "button--success" : "button--secondary"}`}
        onClick={onGrant}
        disabled={ready}
      >
        {ready ? (
          <>
            <Check size={15} /> Preview ready
          </>
        ) : (
          <>
            {button} <ChevronRight size={15} />
          </>
        )}
      </button>
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
