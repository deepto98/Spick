import { useState } from "react";
import {
  AppWindow,
  BellRing,
  Check,
  ChevronRight,
  Cloud,
  Gauge,
  Headphones,
  History,
  Keyboard,
  Languages,
  LockKeyhole,
  Mic2,
  MonitorUp,
  RotateCcw,
  ShieldCheck,
  SlidersHorizontal,
  Volume2,
} from "lucide-react";
import type { AppSettings } from "../types";
import type { AccessibilityPermissionStatus } from "../lib/nativeAccessibility";
import { SPEECH_LANGUAGE_OPTIONS } from "../lib/nativeSettings";
import {
  PageHeader,
  SelectField,
  SettingRow,
  ShortcutKeys,
  Toggle,
} from "../components/Ui";

interface SettingsViewProps {
  settings: AppSettings;
  accessibilityStatus: AccessibilityPermissionStatus | null;
  accessibilityPending: boolean;
  accessibilityError?: string;
  settingsSaving: boolean;
  nativeError?: string;
  onChange: (next: AppSettings) => void;
  onRequestAccessibility: () => void;
  onRefreshAccessibility: () => void;
  onRestartOnboarding: () => void;
}

type SettingsSection = "general" | "dictation" | "language" | "privacy";

const sectionItems: Array<{
  id: SettingsSection;
  label: string;
  icon: typeof SlidersHorizontal;
}> = [
  { id: "general", label: "General", icon: SlidersHorizontal },
  { id: "dictation", label: "Dictation", icon: Mic2 },
  { id: "language", label: "Language & cleanup", icon: Languages },
  { id: "privacy", label: "Privacy & history", icon: ShieldCheck },
];

function accessibilityPermissionDescription(
  status: AccessibilityPermissionStatus | null,
) {
  switch (status?.state) {
    case "granted":
      return "Spick can remember and re-check the field where recording began.";
    case "missing":
      return "Allow Accessibility so the shortcut can verify fields in other apps and avoid protected controls.";
    case "unsupported":
      return "Field tracking currently ships in the macOS desktop build.";
    default:
      return "Checking whether Spick can reach fields in other apps.";
  }
}

export function SettingsView({
  settings,
  accessibilityStatus,
  accessibilityPending,
  accessibilityError,
  settingsSaving,
  nativeError,
  onChange,
  onRequestAccessibility,
  onRefreshAccessibility,
  onRestartOnboarding,
}: SettingsViewProps) {
  const [section, setSection] = useState<SettingsSection>("general");
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const update = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onChange({ ...settings, [key]: value });

  const recordShortcut = () => {
    setRecordingShortcut(true);
    window.setTimeout(() => {
      update("hotkey", settings.hotkey === "⌘+⇧+Space" ? "⌘+⇧+D" : "⌘+⇧+Space");
      setRecordingShortcut(false);
    }, 900);
  };

  return (
    <div className="view view--settings">
      <PageHeader
        eyebrow="PREFERENCES"
        title="Settings"
        description="Language, light cleanup, and Accessibility are live. Preview-only controls are marked."
        actions={
          <span className="settings-saved">
            <Check size={14} />
            {settingsSaving ? "Saving…" : "Saved on this Mac"}
          </span>
        }
      />

      {nativeError && (
        <div className="engine-inline-error" role="alert">
          <strong>Couldn’t save that change</strong>
          <span>{nativeError}</span>
        </div>
      )}

      {accessibilityError && (
        <div className="engine-inline-error" role="alert">
          <strong>Couldn’t check Accessibility</strong>
          <span>{accessibilityError}</span>
        </div>
      )}

      <div className="settings-layout">
        <nav className="settings-nav" aria-label="Settings sections">
          {sectionItems.map((item) => {
            const Icon = item.icon;
            return (
              <button
                type="button"
                key={item.id}
                className={section === item.id ? "active" : ""}
                onClick={() => setSection(item.id)}
              >
                <Icon size={16} />
                <span>{item.label}</span>
                <ChevronRight size={14} />
              </button>
            );
          })}
        </nav>

        <div className="settings-content">
          {section === "general" && (
            <>
              <SettingsSectionHeader
                icon={<SlidersHorizontal size={18} />}
                title="General"
                description="Startup, the floating widget, and sounds."
              />
              <section className="settings-card">
                <SettingRow
                  icon={<MonitorUp size={17} />}
                  title="Open Spick at login"
                  description="Start Spick when you sign in to your Mac."
                  control={
                    <Toggle
                      label="Launch at login"
                      checked={settings.launchAtLogin}
                      onChange={(value) => update("launchAtLogin", value)}
                    />
                  }
                />
                <SettingRow
                  icon={<AppWindow size={17} />}
                  title="Show floating widget"
                  description="Keep the microphone control above other windows."
                  control={
                    <Toggle
                      label="Show floating widget"
                      checked={settings.showWidget}
                      onChange={(value) => update("showWidget", value)}
                    />
                  }
                />
                <SettingRow
                  icon={<Volume2 size={17} />}
                  title="Interface sounds"
                  description="Play a cue when recording starts and stops."
                  control={
                    <Toggle
                      label="Interface sounds"
                      checked={settings.playSounds}
                      onChange={(value) => update("playSounds", value)}
                    />
                  }
                />
              </section>
              <section className="settings-card settings-card--standalone">
                <SettingRow
                  icon={<RotateCcw size={17} />}
                  title="Run setup again"
                  description="Go through the short welcome tour again."
                  control={
                    <button
                      type="button"
                      className="button button--secondary"
                      onClick={onRestartOnboarding}
                    >
                      <RotateCcw size={14} /> Restart setup
                    </button>
                  }
                />
              </section>
            </>
          )}

          {section === "dictation" && (
            <>
              <SettingsSectionHeader
                icon={<Mic2 size={18} />}
                title="Dictation"
                description="Your shortcut, mic, and recording behavior."
              />
              <section className="settings-card">
                <div className="setting-block">
                  <div className="setting-block__heading">
                    <span>
                      <Keyboard size={17} />
                    </span>
                    <div>
                      <strong>Global shortcut</strong>
                      <p>The shortcut Spick listens for across your Mac.</p>
                    </div>
                  </div>
                  <button
                    type="button"
                    className={`shortcut-recorder ${recordingShortcut ? "recording" : ""}`}
                    onClick={recordShortcut}
                  >
                    {recordingShortcut ? (
                      <span>
                        <i /> Press your shortcut…
                      </span>
                    ) : (
                      <>
                        <ShortcutKeys value={settings.hotkey} />
                        <small>Click to change</small>
                      </>
                    )}
                  </button>
                </div>
                <div className="setting-block">
                  <div className="setting-block__heading">
                    <span>
                      <Headphones size={17} />
                    </span>
                    <div>
                      <strong>Microphone</strong>
                      <p>The input Spick will record from.</p>
                    </div>
                  </div>
                  <SelectField
                    label=""
                    value={settings.microphone}
                    onChange={(value) => update("microphone", value)}
                    options={[
                      "System default microphone",
                      "Connected wireless microphone",
                      "External USB microphone",
                    ]}
                  />
                </div>
                <SettingRow
                  icon={<BellRing size={17} />}
                  title="Hold to speak"
                  description="Spick records while the shortcut is held, then keeps the latest text ready on Today."
                  control={
                    <span className="fixed-value">
                      <Check size={14} /> Ready
                    </span>
                  }
                />
                <SettingRow
                  icon={<LockKeyhole size={17} />}
                  title="Track the starting field"
                  description={accessibilityPermissionDescription(
                    accessibilityStatus,
                  )}
                  control={
                    accessibilityStatus?.state === "granted" ? (
                      <span className="fixed-value permission-value--granted">
                        <Check size={14} /> Allowed
                      </span>
                    ) : accessibilityStatus?.state === "unsupported" ? (
                      <span className="fixed-value">macOS only</span>
                    ) : (
                      <button
                        type="button"
                        className="button button--secondary"
                        onClick={
                          accessibilityStatus?.canRequest
                            ? onRequestAccessibility
                            : onRefreshAccessibility
                        }
                        disabled={accessibilityPending}
                      >
                        {accessibilityPending
                          ? "Checking…"
                          : accessibilityStatus?.canRequest
                            ? "Allow access"
                            : "Check again"}
                      </button>
                    )
                  }
                />
              </section>
              <div className="settings-callout">
                <Gauge size={17} />
                <div>
                  <strong>A careful handoff</strong>
                  <span>
                    This checkpoint never pastes automatically. Your latest
                    words stay in Spick for an explicit copy.
                  </span>
                </div>
              </div>
            </>
          )}

          {section === "language" && (
            <>
              <SettingsSectionHeader
                icon={<Languages size={18} />}
                title="Language & cleanup"
                description="Choose what you’re speaking. Spick can also trim a few obvious English fillers."
              />
              <section className="settings-card settings-card--form">
                <SelectField
                  label="Speech language"
                  value={settings.language}
                  disabled={settingsSaving}
                  onChange={(value) => update("language", value)}
                  options={[...SPEECH_LANGUAGE_OPTIONS]}
                  hint="Auto picks one language per recording. English-only models pin this setting to English."
                />
                <div className="cleanup-setting">
                  <span className="field__label">Cleanup style</span>
                  <div className="cleanup-options">
                    {(["Verbatim", "Clean"] as const).map((level) => (
                      <button
                        type="button"
                        key={level}
                        className={
                          settings.cleanupLevel === level ? "active" : ""
                        }
                        disabled={settingsSaving}
                        onClick={() => update("cleanupLevel", level)}
                      >
                        <span>
                          {level === "Verbatim"
                            ? "As transcribed"
                            : "Trim obvious fillers"}
                        </span>
                        <small>
                          {level === "Verbatim"
                            ? "Leave the transcript alone"
                            : "Remove pause-marked “um”, “uh”, and “erm”"}
                        </small>
                        {settings.cleanupLevel === level && <Check size={14} />}
                      </button>
                    ))}
                  </div>
                  <p className="cleanup-note">
                    English only for now. Bare words and other languages stay as
                    transcribed.
                  </p>
                </div>
              </section>
            </>
          )}

          {section === "privacy" && (
            <>
              <SettingsSectionHeader
                icon={<ShieldCheck size={18} />}
                title="Privacy & history"
                description="Control uploads and saved transcripts."
              />
              <section className="privacy-hero">
                <span>
                  <LockKeyhole size={22} />
                </span>
                <div>
                  <strong>Recordings stay in memory</strong>
                  <p>
                    Spick discards audio after each session. Local models keep
                    transcription on this computer.
                  </p>
                </div>
                <span className="privacy-grade">ON DEVICE</span>
              </section>
              <section className="settings-card">
                <SettingRow
                  icon={<History size={17} />}
                  title="Keep transcript history"
                  description="Save transcripts and usage numbers on this Mac."
                  control={
                    <Toggle
                      label="Keep transcript history"
                      checked={settings.keepHistory}
                      onChange={(value) => update("keepHistory", value)}
                    />
                  }
                />
                <SettingRow
                  icon={<Cloud size={17} />}
                  title="Allow cloud fallback"
                  description="Retry uncertain phrases with your cloud provider."
                  control={
                    <Toggle
                      label="Allow cloud fallback"
                      checked={settings.cloudFallback}
                      onChange={(value) => update("cloudFallback", value)}
                    />
                  }
                />
                <SettingRow
                  icon={<LockKeyhole size={17} />}
                  title="Secure fields"
                  description="Spick refuses to record from password and other protected fields."
                  control={<span className="fixed-value">Always blocked</span>}
                />
              </section>
              <section className="danger-card">
                <div>
                  <strong>Delete local history</strong>
                  <span>
                    Remove transcripts and usage numbers saved on this Mac.
                  </span>
                </div>
                <button
                  type="button"
                  className="button button--danger"
                  disabled
                >
                  No history yet
                </button>
              </section>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function SettingsSectionHeader({
  icon,
  title,
  description,
}: {
  icon: React.ReactNode;
  title: string;
  description: string;
}) {
  return (
    <div className="settings-section-header">
      <span>{icon}</span>
      <div>
        <h2>{title}</h2>
        <p>{description}</p>
      </div>
    </div>
  );
}
