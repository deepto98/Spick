import { useEffect, useState } from "react";
import {
  AppWindow,
  AlertTriangle,
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
  Trash2,
  Volume2,
} from "lucide-react";
import type { AppSettings } from "../types";
import type {
  ClearLocalDataResult,
  ClearLocalDataScope,
} from "../lib/nativeLocalData";
import type { AccessibilityPermissionStatus } from "../lib/nativeAccessibility";
import type { NativeShortcutStatus } from "../lib/nativeShortcut";
import { captureMacShortcut } from "../lib/shortcutCapture";
import {
  shortcutDisplayName,
  SPEECH_LANGUAGE_OPTIONS,
} from "../lib/nativeSettings";
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
  shortcutStatus: NativeShortcutStatus | null;
  shortcutPending: boolean;
  shortcutError?: string;
  settingsSaving: boolean;
  settingsAcknowledged?: boolean;
  settingsLoading?: boolean;
  nativeError?: string;
  nativeErrorTitle?: string;
  clearError?: string;
  clearPendingScope?: ClearLocalDataScope | null;
  lastClearResult?: ClearLocalDataResult | null;
  onChange: (next: AppSettings) => void;
  onShortcutChange: (shortcut: string) => void;
  onRequestAccessibility: () => void;
  onRefreshAccessibility: () => void;
  onRefreshShortcut: () => void;
  onRequestInputMonitoring: () => void;
  onRestartOnboarding: () => void;
  onRetryNativeSettings?: () => void;
  onClearLocalData?: (
    scope: ClearLocalDataScope,
  ) => Promise<ClearLocalDataResult | null>;
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

function optionShortcutDescription(status: NativeShortcutStatus | null) {
  if (status?.optionListenerActive) {
    return "Tap Option to start and stop, or hold it while you speak.";
  }
  if (status?.inputMonitoringGranted) {
    return "Input Monitoring is allowed. Activate Option to replace the temporary fallback.";
  }
  if (status?.fallbackShortcut) {
    return `Allow Input Monitoring for Option. Until then, ${shortcutDisplayName(status.fallbackShortcut)} still works.`;
  }
  return "Tap Option to start and stop, or hold it while you speak.";
}

export function SettingsView({
  settings,
  accessibilityStatus,
  accessibilityPending,
  accessibilityError,
  shortcutStatus,
  shortcutPending,
  shortcutError,
  settingsSaving,
  settingsAcknowledged = true,
  settingsLoading = false,
  nativeError,
  nativeErrorTitle = "Couldn’t save that change",
  clearError,
  clearPendingScope = null,
  lastClearResult = null,
  onChange,
  onShortcutChange,
  onRequestAccessibility,
  onRefreshAccessibility,
  onRefreshShortcut,
  onRequestInputMonitoring,
  onRestartOnboarding,
  onRetryNativeSettings,
  onClearLocalData,
}: SettingsViewProps) {
  const [section, setSection] = useState<SettingsSection>("general");
  const [recordingShortcut, setRecordingShortcut] = useState(false);
  const [shortcutCaptureError, setShortcutCaptureError] = useState<
    string | null
  >(null);
  const [confirmClearScope, setConfirmClearScope] =
    useState<ClearLocalDataScope | null>(null);
  const usesOptionGesture = settings.hotkey === "⌥";
  const settingsControlsDisabled = settingsSaving || !settingsAcknowledged;
  const shortcutControlsDisabled = settingsControlsDisabled || shortcutPending;
  const update = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onChange({ ...settings, [key]: value });

  useEffect(() => {
    if (!recordingShortcut) return;

    const capture = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      const result = captureMacShortcut(event);
      if (result.kind === "waiting") return;
      if (result.kind === "cancelled") {
        setRecordingShortcut(false);
        setShortcutCaptureError(null);
        return;
      }
      if (result.kind === "invalid") {
        setShortcutCaptureError(result.message);
        return;
      }

      setRecordingShortcut(false);
      setShortcutCaptureError(null);
      onShortcutChange(result.shortcut);
    };

    window.addEventListener("keydown", capture, true);
    return () => window.removeEventListener("keydown", capture, true);
  }, [onShortcutChange, recordingShortcut]);

  const beginShortcutCapture = () => {
    if (shortcutControlsDisabled) return;
    setShortcutCaptureError(null);
    setRecordingShortcut(true);
  };

  const useOptionShortcut = () => {
    setRecordingShortcut(false);
    setShortcutCaptureError(null);
    if (!usesOptionGesture) onShortcutChange("Option");
  };

  const changeSection = (next: SettingsSection) => {
    setRecordingShortcut(false);
    setShortcutCaptureError(null);
    setSection(next);
  };

  const requestClear = async (scope: ClearLocalDataScope) => {
    if (!onClearLocalData || clearPendingScope) return;
    if (confirmClearScope !== scope) {
      setConfirmClearScope(scope);
      return;
    }
    const result = await onClearLocalData(scope);
    if (result) setConfirmClearScope(null);
  };

  return (
    <div className="view view--settings">
      <PageHeader
        eyebrow="PREFERENCES"
        title="Settings"
        description="Shortcut, language, cleanup, and privacy choices are saved here. Unconnected controls are marked."
        actions={
          <span
            className={`settings-saved ${settingsAcknowledged ? "" : "settings-saved--unavailable"}`}
            role="status"
            aria-busy={settingsLoading || settingsSaving}
          >
            {settingsAcknowledged && <Check size={14} />}
            {settingsLoading
              ? "Loading saved settings…"
              : settingsSaving
                ? "Saving…"
                : settingsAcknowledged
                  ? "Saved on this Mac"
                  : "Settings not loaded"}
          </span>
        }
      />

      {nativeError && (
        <div className="engine-inline-error" role="alert">
          <strong>{nativeErrorTitle}</strong>
          <span>{nativeError}</span>
          {onRetryNativeSettings && (
            <button
              type="button"
              className="text-button"
              onClick={onRetryNativeSettings}
              disabled={settingsLoading}
            >
              {settingsLoading ? "Trying again…" : "Try again"}
            </button>
          )}
        </div>
      )}

      {accessibilityError && (
        <div className="engine-inline-error" role="alert">
          <strong>Couldn’t check Accessibility</strong>
          <span>{accessibilityError}</span>
        </div>
      )}

      {shortcutError && (
        <div className="engine-inline-error" role="alert">
          <strong>Couldn’t check the Option shortcut</strong>
          <span>{shortcutError}</span>
        </div>
      )}

      {clearError && (
        <div className="engine-inline-error" role="alert">
          <strong>Couldn’t clear local data</strong>
          <span>{clearError}</span>
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
                onClick={() => changeSection(item.id)}
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
                  description="Not connected in this development build yet."
                  control={<span className="fixed-value">Coming later</span>}
                />
                <SettingRow
                  icon={<AppWindow size={17} />}
                  title="Show floating widget"
                  description="Show the microphone control above other windows during this run."
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
                  description="Audio cues are not connected in this development build yet."
                  control={<span className="fixed-value">Coming later</span>}
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
                  <div className="shortcut-control">
                    <div
                      className="shortcut-mode"
                      role="group"
                      aria-label="Shortcut type"
                    >
                      <button
                        type="button"
                        className={usesOptionGesture ? "active" : ""}
                        aria-pressed={usesOptionGesture}
                        disabled={shortcutControlsDisabled}
                        onClick={useOptionShortcut}
                      >
                        Option
                      </button>
                      <button
                        type="button"
                        className={!usesOptionGesture ? "active" : ""}
                        aria-pressed={!usesOptionGesture}
                        disabled={shortcutControlsDisabled}
                        onClick={beginShortcutCapture}
                      >
                        Custom
                      </button>
                    </div>
                    <button
                      type="button"
                      className={`shortcut-recorder ${recordingShortcut ? "recording" : ""}`}
                      aria-label={
                        recordingShortcut
                          ? "Recording shortcut"
                          : "Record a custom shortcut"
                      }
                      disabled={shortcutControlsDisabled}
                      onClick={beginShortcutCapture}
                    >
                      {recordingShortcut ? (
                        <span>
                          <i /> Press your shortcut…
                        </span>
                      ) : (
                        <>
                          <ShortcutKeys value={settings.hotkey} />
                          <small>
                            {settingsSaving
                              ? "Saving…"
                              : usesOptionGesture
                                ? "Tap once or hold"
                                : "Click to change"}
                          </small>
                        </>
                      )}
                    </button>
                    {(recordingShortcut || shortcutCaptureError) && (
                      <small
                        className={`shortcut-capture-note ${shortcutCaptureError ? "shortcut-capture-note--error" : ""}`}
                        role="status"
                      >
                        {shortcutCaptureError ??
                          "Press a shortcut. Escape cancels."}
                      </small>
                    )}
                  </div>
                </div>
                <div className="setting-block">
                  <div className="setting-block__heading">
                    <span>
                      <Headphones size={17} />
                    </span>
                    <div>
                      <strong>Microphone</strong>
                      <p>
                        Device selection is not connected yet; Spick uses the
                        system default.
                      </p>
                    </div>
                  </div>
                  <SelectField
                    label=""
                    value={settings.microphone}
                    disabled
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
                  title={
                    usesOptionGesture ? "Option-key gesture" : "Custom shortcut"
                  }
                  description={
                    usesOptionGesture
                      ? optionShortcutDescription(shortcutStatus)
                      : "Your saved shortcut records while held and finishes when released."
                  }
                  control={
                    !usesOptionGesture ? (
                      <span className="fixed-value permission-value--granted">
                        <Check size={14} /> Ready
                      </span>
                    ) : shortcutStatus?.optionListenerActive ? (
                      <span className="fixed-value permission-value--granted">
                        <Check size={14} /> Ready
                      </span>
                    ) : shortcutStatus?.inputMonitoringGranted ? (
                      <button
                        type="button"
                        className="button button--secondary"
                        onClick={onRefreshShortcut}
                        disabled={shortcutPending}
                      >
                        {shortcutPending ? "Activating…" : "Activate Option"}
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="button button--secondary"
                        onClick={onRequestInputMonitoring}
                        disabled={shortcutPending}
                      >
                        {shortcutPending
                          ? "Opening…"
                          : "Allow Input Monitoring"}
                      </button>
                    )
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
                  <strong>Two ways to speak</strong>
                  <span>
                    {usesOptionGesture
                      ? "Tap Option once to start and once more to finish. Or hold it down, speak, and release."
                      : "Hold your shortcut, speak, and release when you’re done."}
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
                  disabled={settingsControlsDisabled}
                  onChange={(value) => update("language", value)}
                  options={[...SPEECH_LANGUAGE_OPTIONS]}
                  hint="Auto works with the widest range of models. Fixed choices are checked against the model or provider; xAI only accepts its shorter formatting-language list."
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
                        disabled={settingsControlsDisabled}
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
                  <strong>Audio is discarded after every dictation</strong>
                  <p>
                    Aggregate word counts, capture duration, language, engine,
                    and delivery result stay on this Mac for stats. They never
                    include transcript text.
                  </p>
                </div>
                <span className="privacy-grade">ON DEVICE</span>
              </section>
              <section className="settings-card">
                <SettingRow
                  icon={<History size={17} />}
                  title="Keep transcript history"
                  description="Save transcript text on this Mac. Turning this off leaves aggregate usage totals in place and does not delete older text."
                  control={
                    <Toggle
                      label="Keep transcript history"
                      checked={settings.keepHistory}
                      disabled={settingsControlsDisabled}
                      onChange={(value) => update("keepHistory", value)}
                    />
                  }
                />
                <SettingRow
                  icon={<Cloud size={17} />}
                  title="Allow cloud fallback"
                  description="If local transcription cannot finish, send that recording to the first configured provider that supports its language: OpenAI, xAI, then Gemini. Audio leaves this Mac only when fallback runs. Changes apply to your next recording."
                  control={
                    <Toggle
                      label="Allow cloud fallback"
                      checked={settings.cloudFallback}
                      disabled={settingsControlsDisabled}
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
                  <strong>Delete saved transcript text</strong>
                  <span>
                    Keep aggregate usage stats, vocabulary, and preferences.
                  </span>
                </div>
                <button
                  type="button"
                  className="button button--danger"
                  disabled={!onClearLocalData || clearPendingScope !== null}
                  onClick={() => void requestClear("transcriptHistory")}
                >
                  {clearPendingScope === "transcriptHistory"
                    ? "Deleting…"
                    : confirmClearScope === "transcriptHistory"
                      ? "Confirm delete"
                      : "Delete transcripts"}
                </button>
              </section>
              <section className="danger-card">
                <div>
                  <strong>Reset all local data</strong>
                  <span>
                    Delete usage totals, transcript text, and vocabulary. App
                    preferences stay in place.
                  </span>
                </div>
                <button
                  type="button"
                  className="button button--danger"
                  disabled={!onClearLocalData || clearPendingScope !== null}
                  onClick={() => void requestClear("all")}
                >
                  {clearPendingScope === "all"
                    ? "Resetting…"
                    : confirmClearScope === "all"
                      ? "Confirm reset"
                      : "Reset local data"}
                </button>
              </section>
              {confirmClearScope && !clearPendingScope && (
                <div className="clear-confirmation" role="status">
                  <Trash2 size={15} />
                  <span>
                    This cannot be undone. Press the same button once more to
                    confirm.
                  </span>
                  <button
                    type="button"
                    className="text-button"
                    onClick={() => setConfirmClearScope(null)}
                  >
                    Cancel
                  </button>
                </div>
              )}
              {lastClearResult && (
                <div
                  className={`clear-result ${lastClearResult.storageCleanupComplete && lastClearResult.memoryCleanupComplete ? "" : "clear-result--warning"}`}
                  role="status"
                >
                  {lastClearResult.storageCleanupComplete &&
                  lastClearResult.memoryCleanupComplete ? (
                    <Check size={15} />
                  ) : (
                    <AlertTriangle size={15} />
                  )}
                  <span>
                    <strong>
                      Cleared {lastClearResult.deletedTranscripts} saved{" "}
                      {lastClearResult.deletedTranscripts === 1
                        ? "transcript"
                        : "transcripts"}
                      , {lastClearResult.deletedUsageSessions} usage{" "}
                      {lastClearResult.deletedUsageSessions === 1
                        ? "session"
                        : "sessions"}
                      , and {lastClearResult.deletedVocabularyEntries}{" "}
                      vocabulary{" "}
                      {lastClearResult.deletedVocabularyEntries === 1
                        ? "entry"
                        : "entries"}
                      .
                    </strong>
                    {!lastClearResult.storageCleanupComplete && (
                      <small>
                        {lastClearResult.storageCleanupWarning ??
                          "The SQLite storage cleanup could not finish."}{" "}
                        Quit and reopen Spick, then run the same clear action
                        again.
                      </small>
                    )}
                    {!lastClearResult.memoryCleanupComplete && (
                      <small>
                        {lastClearResult.memoryCleanupWarning ??
                          "The latest in-memory recovery transcript could not be cleared."}{" "}
                        Quit and reopen Spick to discard that process memory.
                      </small>
                    )}
                  </span>
                </div>
              )}
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
