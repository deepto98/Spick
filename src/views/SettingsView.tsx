import { useState } from "react";
import {
  AppWindow,
  BellRing,
  BookOpenText,
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
  Sparkles,
  Volume2,
} from "lucide-react";
import type { AppSettings } from "../types";
import {
  PageHeader,
  SelectField,
  SettingRow,
  ShortcutKeys,
  Toggle,
} from "../components/Ui";

interface SettingsViewProps {
  settings: AppSettings;
  onChange: (next: AppSettings) => void;
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

export function SettingsView({
  settings,
  onChange,
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
        eyebrow="Preferences"
        title="Settings"
        description="Tune Spick to fit how, where, and what you dictate."
        actions={
          <span className="settings-saved">
            <Check size={14} /> Preview changes · not persisted
          </span>
        }
      />

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
                description="How Spick behaves on your computer."
              />
              <section className="settings-card">
                <SettingRow
                  icon={<MonitorUp size={17} />}
                  title="Launch Spick at login"
                  description="Keep dictation one shortcut away after restarting your Mac."
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
                  description="Keep the compact microphone control above other windows."
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
                  description="Play quiet cues when listening starts and text is inserted."
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
                  icon={<Sparkles size={17} />}
                  title="Run welcome setup again"
                  description="Review permissions, language, and your dictation shortcut."
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
                description="Your shortcut, microphone, and interaction preferences."
              />
              <section className="settings-card">
                <div className="setting-block">
                  <div className="setting-block__heading">
                    <span>
                      <Keyboard size={17} />
                    </span>
                    <div>
                      <strong>Global shortcut</strong>
                      <p>Hold this shortcut in any text field to dictate.</p>
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
                      <p>The input used when you start dictating.</p>
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
                  description="Shortcut state is wired; audio, cleanup, and insertion are next."
                  control={
                    <span className="fixed-value">
                      <Check size={14} /> State wired
                    </span>
                  }
                />
              </section>
              <div className="settings-callout">
                <Gauge size={17} />
                <div>
                  <strong>Latency benchmark pending</strong>
                  <span>
                    Stage timing begins with the native audio pipeline.
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
                description="Control detection, code-switching, and how speech becomes prose."
              />
              <section className="settings-card settings-card--form">
                <SelectField
                  label="Speech language"
                  value={settings.language}
                  onChange={(value) => update("language", value)}
                  options={[
                    "Auto-detect",
                    "English",
                    "Hindi",
                    "Bengali",
                    "Spanish",
                    "French",
                    "Hinglish",
                  ]}
                  hint="Auto-detect will route language hints according to the selected engine's verified capabilities."
                />
                <div className="cleanup-setting">
                  <span className="field__label">Cleanup style</span>
                  <div className="cleanup-options">
                    {(["Verbatim", "Clean", "Polished"] as const).map(
                      (level) => (
                        <button
                          type="button"
                          key={level}
                          className={
                            settings.cleanupLevel === level ? "active" : ""
                          }
                          onClick={() => update("cleanupLevel", level)}
                        >
                          <span>{level}</span>
                          <small>
                            {level === "Verbatim"
                              ? "Only punctuation"
                              : level === "Clean"
                                ? "Remove fillers & repeats"
                                : "Rewrite for clarity"}
                          </small>
                          {settings.cleanupLevel === level && (
                            <Check size={14} />
                          )}
                        </button>
                      ),
                    )}
                  </div>
                </div>
                <SettingRow
                  icon={<BookOpenText size={17} />}
                  title="Preserve specialist language"
                  description="Planned protection for saved vocabulary and code terms during cleanup."
                  control={<span className="fixed-value">Planned</span>}
                />
              </section>
            </>
          )}

          {section === "privacy" && (
            <>
              <SettingsSectionHeader
                icon={<ShieldCheck size={18} />}
                title="Privacy & history"
                description="Choose what leaves your Mac and what Spick remembers."
              />
              <section className="privacy-hero">
                <span>
                  <LockKeyhole size={22} />
                </span>
                <div>
                  <strong>Privacy-first local mode is the target</strong>
                  <p>
                    The native pipeline will process local audio in memory and
                    discard recordings after each session.
                  </p>
                </div>
                <span className="privacy-grade">LOCAL</span>
              </section>
              <section className="settings-card">
                <SettingRow
                  icon={<History size={17} />}
                  title="Keep transcript history"
                  description="Save polished text and usage statistics on this Mac."
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
                  description="Retry low-confidence phrases with your selected cloud engine."
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
                  description="Secure-field detection is planned for the native insertion layer."
                  control={<span className="fixed-value">Planned</span>}
                />
              </section>
              <section className="danger-card">
                <div>
                  <strong>Delete local history</strong>
                  <span>
                    Remove saved transcripts and statistics from this Mac.
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
