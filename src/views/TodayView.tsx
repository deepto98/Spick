import { useMemo, useState } from "react";
import {
  ArrowDownRight,
  ArrowUpRight,
  CheckCircle2,
  ChevronRight,
  Clock3,
  Copy,
  Cpu,
  Gauge,
  Languages,
  Mic2,
  RefreshCw,
  Timer,
} from "lucide-react";
import { DictationHud } from "../components/DictationHud";
import { PageHeader } from "../components/Ui";
import { percentageChange } from "../lib/localDataPresentation";
import type {
  TranscriptHistoryItem,
  UsageDashboard,
  UsageLanguage,
} from "../lib/nativeLocalData";
import type {
  NativeDeliveryOutcome,
  NativeDeliveryStatus,
  NativeDictationLatencyEvent,
  NativeDictationTranscript,
} from "../lib/nativeDictation";
import type { HudState } from "../types";

interface TodayViewProps {
  onOpenEngines: () => void;
  hudState: HudState;
  audioLevel?: number;
  dictationPending?: boolean;
  dictationError?: string;
  delivery: NativeDeliveryOutcome | null;
  lastTranscript: NativeDictationTranscript | null;
  lastLatency?: NativeDictationLatencyEvent | null;
  hiddenEphemeralSessionId?: string | null;
  language: string;
  native: boolean;
  dashboard: UsageDashboard | null;
  dashboardLoading: boolean;
  dashboardError?: string | null;
  subscriptionError?: string | null;
  history: TranscriptHistoryItem[];
  historyLoading: boolean;
  historyLoadingMore: boolean;
  historyError?: string | null;
  hasOlderHistory: boolean;
  saveTranscriptHistory: boolean;
  onRefreshLocalData: () => void;
  onLoadOlderHistory: () => void;
  onHudStateChange: (state: HudState) => void;
}

const languageColors = ["#b35432", "#c6974c", "#78907a", "#80658f", "#577d89"];

function formatDuration(milliseconds: number) {
  if (milliseconds <= 0) return "0m";
  const minutes = Math.round(milliseconds / 60_000);
  if (minutes < 60) return `${Math.max(1, minutes)}m`;
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return remainder ? `${hours}h ${remainder}m` : `${hours}h`;
}

function formatDay(localDate: string) {
  const parsed = new Date(`${localDate}T00:00:00`);
  return Number.isNaN(parsed.getTime())
    ? localDate
    : new Intl.DateTimeFormat(undefined, { weekday: "short" }).format(parsed);
}

function formatRelativeTime(timestamp: number) {
  const elapsed = Math.max(0, Date.now() - timestamp);
  if (elapsed < 60_000) return "Just now";
  const minutes = Math.floor(elapsed / 60_000);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
  }).format(new Date(timestamp));
}

function formatLatency(milliseconds: number) {
  if (milliseconds < 1_000) return `${milliseconds} ms`;
  const seconds = (milliseconds / 1_000).toFixed(milliseconds < 10_000 ? 2 : 1);
  return `${Number(seconds)} s`;
}

function languageCode(languageTag: string | null | undefined) {
  return languageTag?.split("-", 1)[0]?.toUpperCase() || "AUTO";
}

function languageName(languageTag: string) {
  try {
    return (
      new Intl.DisplayNames(undefined, { type: "language" }).of(languageTag) ??
      languageTag
    );
  } catch {
    return languageTag;
  }
}

function deliveryFromHistory(
  item: TranscriptHistoryItem,
): NativeDeliveryOutcome {
  const allowed: NativeDeliveryStatus[] = [
    "inserted",
    "focusChanged",
    "secureField",
    "accessibilityMissing",
    "unsupported",
    "failed",
    "indeterminate",
  ];
  const status = allowed.includes(item.deliveryStatus as NativeDeliveryStatus)
    ? (item.deliveryStatus as NativeDeliveryStatus)
    : "failed";
  return {
    status,
    transcriptAvailable: true,
    targetApp: item.targetApp,
    caretRepositioned: null,
  };
}

type TranscriptRow = {
  id: string;
  application: string;
  text: string;
  completedAtMs: number | null;
  words: number;
  languageTag: string | null;
  delivery: NativeDeliveryOutcome;
  ephemeral: boolean;
};

export function TodayView({
  onOpenEngines,
  hudState,
  audioLevel,
  dictationPending,
  dictationError,
  delivery,
  lastLatency = null,
  lastTranscript,
  hiddenEphemeralSessionId,
  language,
  native,
  dashboard,
  dashboardLoading,
  dashboardError,
  subscriptionError,
  history,
  historyLoading,
  historyLoadingMore,
  historyError,
  hasOlderHistory,
  saveTranscriptHistory,
  onRefreshLocalData,
  onLoadOlderHistory,
  onHudStateChange,
}: TodayViewProps) {
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [checkedFieldId, setCheckedFieldId] = useState<string | null>(null);
  const latestDelivery = lastTranscript?.delivery ?? delivery;
  const lifetimeWpm = dashboard?.lifetime.averageWpm;
  const days = dashboard?.days ?? [];
  const maxWords = Math.max(0, ...days.map((item) => item.words));
  const sparkDays = days.length
    ? days.map((item) => ({ key: item.localDate, words: item.words }))
    : Array.from({ length: 7 }, (_, index) => ({
        key: `empty-${index}`,
        words: 0,
      }));
  const trend = percentageChange(
    dashboard?.period.words ?? 0,
    dashboard?.previousPeriod?.words ?? null,
  );
  const languages = (dashboard?.languages ?? []).filter(
    (item) => item.sessions > 0 || item.words > 0,
  );
  const transcriptRows = useMemo<TranscriptRow[]>(() => {
    const persisted = history.map((item) => ({
      id: item.sessionId,
      application: item.targetApp ?? "Spick",
      text: item.text,
      completedAtMs: item.completedAtMs,
      words: item.wordCount,
      languageTag: item.languageTag,
      delivery: deliveryFromHistory(item),
      ephemeral: false,
    }));
    if (
      !lastTranscript ||
      lastTranscript.sessionId === hiddenEphemeralSessionId ||
      persisted.some((item) => item.id === lastTranscript.sessionId)
    ) {
      return persisted;
    }
    return [
      {
        id: lastTranscript.sessionId,
        application: lastTranscript.delivery.targetApp ?? "Spick",
        text: lastTranscript.transcript.text,
        completedAtMs: null,
        words: lastTranscript.transcript.text.trim()
          ? lastTranscript.transcript.text.trim().split(/\s+/u).length
          : 0,
        languageTag: lastTranscript.transcript.detectedLanguage,
        delivery: lastTranscript.delivery,
        ephemeral: true,
      },
      ...persisted,
    ];
  }, [hiddenEphemeralSessionId, history, lastTranscript]);

  const copyText = async (id: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopiedId(id);
      window.setTimeout(() => setCopiedId(null), 1200);
    } catch {
      setCopiedId(null);
    }
  };

  return (
    <div className="view view--today">
      <PageHeader
        eyebrow="YOUR DICTATION"
        title="Today"
        description="Word counts and recording pace come from dictations on this Mac. Audio is never kept."
        actions={
          <button
            type="button"
            className="button button--secondary"
            onClick={onOpenEngines}
          >
            <Cpu size={16} />
            Choose an engine
            <ChevronRight size={15} />
          </button>
        }
      />

      {dashboardError && (
        <div className="engine-inline-error" role="alert">
          <strong>Couldn’t load usage</strong>
          <span>{dashboardError}</span>
          <button
            type="button"
            className="text-button"
            onClick={onRefreshLocalData}
          >
            Try again
          </button>
        </div>
      )}

      {subscriptionError && !dashboardError && (
        <div className="engine-inline-error" role="alert">
          <strong>Live updates are paused</strong>
          <span>{subscriptionError}</span>
          <button
            type="button"
            className="text-button"
            onClick={onRefreshLocalData}
          >
            Refresh now
          </button>
        </div>
      )}

      <section
        className={`stat-grid ${dashboardLoading ? "data-section--loading" : ""}`}
        aria-label="Dictation statistics"
        aria-busy={dashboardLoading}
      >
        <article className="stat-card stat-card--primary">
          <span className="stat-card__icon">
            <Mic2 size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>{dashboard?.today.words.toLocaleString() ?? "—"}</strong>
            <span>words today</span>
          </div>
          <small>
            {dashboard
              ? `${dashboard.lifetime.words.toLocaleString()} words all time`
              : "All-time total"}
          </small>
          <div className="stat-card__sparkline" aria-hidden="true">
            {sparkDays.map((item) => (
              <i
                key={item.key}
                style={{
                  height: `${maxWords > 0 ? Math.max(5, (item.words / maxWords) * 100) : 5}%`,
                }}
              />
            ))}
          </div>
        </article>
        <article className="stat-card">
          <span className="stat-card__icon">
            <Gauge size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>{dashboard?.today.averageWpm ?? "—"}</strong>
            <span>recording words/min</span>
          </div>
          <small>
            {lifetimeWpm == null
              ? "Full capture time, including pauses"
              : `${lifetimeWpm} WPM all time · includes pauses`}
          </small>
        </article>
        <article className="stat-card">
          <span className="stat-card__icon">
            <Timer size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>
              {dashboard
                ? formatDuration(dashboard.today.estimatedTimeSavedMs)
                : "—"}
            </strong>
            <span>estimated time saved</span>
          </div>
          <small>Compared with typing at 40 words/min</small>
        </article>
        <article className="stat-card">
          <span className="stat-card__icon">
            <Languages size={18} />
          </span>
          <div className="stat-card__metric">
            <strong>{dashboard ? languages.length : "—"}</strong>
            <span>languages used</span>
          </div>
          <span
            className="language-stack"
            aria-label={
              languages
                .map((item) => languageName(item.languageTag))
                .join(", ") || "No languages yet"
            }
          >
            {languages.slice(0, 3).map((item) => (
              <i key={item.languageTag}>{languageCode(item.languageTag)}</i>
            ))}
          </span>
          <small>Across the last {dashboard?.daysRequested ?? 7} days</small>
        </article>
      </section>

      <div className="dashboard-grid">
        <section className="panel activity-panel">
          <header className="panel__header">
            <div>
              <h2>Words by day</h2>
              <p>Completed recordings · local time</p>
            </div>
            <span className="auto-badge">
              Last {dashboard?.daysRequested ?? 7} days
            </span>
          </header>
          <div className="chart-summary">
            <strong>{dashboard?.period.words.toLocaleString() ?? "—"}</strong>
            {dashboard?.previousPeriod && trend !== null && (
              <span className={trend < 0 ? "trend-copy--down" : ""}>
                {trend < 0 ? (
                  <ArrowDownRight size={13} />
                ) : (
                  <ArrowUpRight size={13} />
                )}{" "}
                {Math.abs(trend)}% from the prior period
              </span>
            )}
            {dashboard?.previousPeriod && trend === null && (
              <span>
                {dashboard.period.words > 0
                  ? "First activity in this comparison"
                  : "No change from the prior period"}
              </span>
            )}
          </div>
          {days.length > 0 ? (
            <div
              className="bar-chart"
              aria-label="Words dictated by day bar chart"
              style={{ gridTemplateColumns: `repeat(${days.length}, 1fr)` }}
            >
              <div className="bar-chart__grid" aria-hidden="true">
                <i />
                <i />
                <i />
              </div>
              {days.map((item, index) => (
                <div className="bar-chart__item" key={item.localDate}>
                  <span className="bar-chart__value">
                    {item.words.toLocaleString()}
                  </span>
                  <div className="bar-chart__track">
                    <i
                      className={
                        index === days.length - 1
                          ? "bar-chart__bar--active"
                          : ""
                      }
                      style={{
                        height:
                          item.words === 0 || maxWords === 0
                            ? "0%"
                            : `${Math.max(7, (item.words / maxWords) * 100)}%`,
                      }}
                    />
                  </div>
                  <span>{formatDay(item.localDate)}</span>
                </div>
              ))}
            </div>
          ) : (
            <DataEmpty
              title={
                dashboardLoading ? "Loading your week…" : "No recordings yet"
              }
              detail="Your first completed dictation will start this chart."
            />
          )}
        </section>

        <LanguagePanel languages={languages} loading={dashboardLoading} />
      </div>

      <div className="dashboard-grid dashboard-grid--bottom">
        <section className="panel recent-panel">
          <header className="panel__header">
            <div>
              <h2>Recent dictations</h2>
              <p>
                {saveTranscriptHistory
                  ? "Transcript history is saved on this Mac"
                  : "New transcripts stay memory-only; older saved history remains until deleted"}
              </p>
            </div>
            <button
              type="button"
              className="icon-button icon-button--subtle"
              aria-label="Refresh local history"
              onClick={onRefreshLocalData}
              disabled={historyLoading}
            >
              <RefreshCw size={15} />
            </button>
          </header>
          {historyError && (
            <div className="inline-data-error" role="alert">
              <strong>History didn’t load.</strong> {historyError}
            </div>
          )}
          <div className="dictation-list" aria-busy={historyLoading}>
            {transcriptRows.map((item) => (
              <TranscriptRow
                checked={checkedFieldId === item.id}
                copied={copiedId === item.id}
                item={item}
                key={item.id}
                onCopy={() => void copyText(item.id, item.text)}
                onConfirm={() => setCheckedFieldId(item.id)}
              />
            ))}
            {transcriptRows.length === 0 && (
              <DataEmpty
                title={
                  historyLoading
                    ? "Loading recent dictations…"
                    : "Nothing here yet"
                }
                detail={
                  saveTranscriptHistory
                    ? "Saved transcript text will appear after your next dictation."
                    : "Turn on transcript history in Privacy if you want text to remain here."
                }
              />
            )}
          </div>
          {hasOlderHistory && (
            <button
              type="button"
              className="button button--secondary history-load-more"
              onClick={onLoadOlderHistory}
              disabled={historyLoadingMore}
            >
              {historyLoadingMore ? "Loading…" : "Load older dictations"}
            </button>
          )}
        </section>

        <section className="panel try-panel">
          <div className="try-panel__glow" />
          <span className="try-panel__eyebrow">
            <i /> {native ? "DESKTOP DICTATION" : "BROWSER PREVIEW"}
          </span>
          <h2>Try the shortcut</h2>
          <p>
            {native
              ? "Use your shortcut and speak. Spick will type into the field where you started when it can do so safely."
              : "Recording is available in the Tauri development app."}
          </p>
          <div className="try-panel__hud">
            <DictationHud
              autoAdvance={false}
              audioLevel={audioLevel}
              disabled={dictationPending}
              errorMessage={dictationError}
              delivery={latestDelivery}
              language={language}
              state={hudState}
              onStateChange={onHudStateChange}
            />
          </div>
          <div className="try-panel__footer">
            <span>
              <Clock3 size={14} /> Status
            </span>
            <strong>
              {native
                ? latestDelivery
                  ? latestDelivery.status === "inserted"
                    ? "Typed where you started"
                    : latestDelivery.transcriptAvailable
                      ? "Ready to copy"
                      : "Field left alone"
                  : "Waiting for your shortcut"
                : "Development app required"}
            </strong>
          </div>
          {native && lastLatency && (
            <details className="latency-details">
              <summary>
                <span>
                  <Timer size={13} />
                  {lastLatency.outcome === "completed"
                    ? "Last handoff"
                    : "Last attempt stopped"}
                </span>
                <strong>{formatLatency(lastLatency.processingTotalMs)}</strong>
              </summary>
              <dl>
                <LatencyValue
                  label="Processing signal"
                  value={lastLatency.stopToProcessingMs}
                />
                <LatencyValue
                  label="Mic handoff"
                  value={lastLatency.captureFinalizeMs}
                />
                <LatencyValue
                  label="Transcription"
                  value={lastLatency.transcriptionMs}
                />
                <LatencyValue
                  label="Text handoff"
                  value={lastLatency.deliveryMs}
                />
              </dl>
              <small>
                Elapsed times only, kept in memory until Spick quits. No
                recording, dictated text, or app name is included.
              </small>
            </details>
          )}
        </section>
      </div>
    </div>
  );
}

function LatencyValue({
  label,
  value,
}: {
  label: string;
  value: number | null;
}) {
  if (value === null) return null;
  return (
    <div>
      <dt>{label}</dt>
      <dd>{formatLatency(value)}</dd>
    </div>
  );
}

function LanguagePanel({
  languages,
  loading,
}: {
  languages: UsageLanguage[];
  loading: boolean;
}) {
  const totalWords = languages.reduce((total, item) => total + item.words, 0);
  const slices = languages.map((item, index) => {
    const priorWords = languages
      .slice(0, index)
      .reduce((total, prior) => total + prior.words, 0);
    const start = totalWords > 0 ? (priorWords / totalWords) * 100 : 0;
    const end =
      totalWords > 0 ? ((priorWords + item.words) / totalWords) * 100 : 0;
    return `${languageColors[index % languageColors.length]} ${start}% ${end}%`;
  });
  const donutBackground = slices.length
    ? `conic-gradient(${slices.join(", ")})`
    : "#ece9e3";

  return (
    <section className="panel language-panel">
      <header className="panel__header">
        <div>
          <h2>Languages</h2>
          <p>Words in the current period</p>
        </div>
        <span className="auto-badge">
          <i /> Detected
        </span>
      </header>
      {languages.length > 0 ? (
        <>
          <div className="language-donut-wrap">
            <div
              className="language-donut"
              aria-label={`${languages.length} languages detected`}
              style={{ background: donutBackground }}
            >
              <div>
                <strong>{languages.length}</strong>
                <span>languages</span>
              </div>
            </div>
          </div>
          <div className="language-legend">
            {languages.map((item, index) => {
              const percentage =
                totalWords > 0
                  ? Math.round((item.words / totalWords) * 100)
                  : 0;
              return (
                <div key={item.languageTag}>
                  <i
                    style={{
                      backgroundColor:
                        languageColors[index % languageColors.length],
                    }}
                  />
                  <span>{languageName(item.languageTag)}</span>
                  <strong>{percentage}%</strong>
                </div>
              );
            })}
          </div>
        </>
      ) : (
        <DataEmpty
          title={loading ? "Checking languages…" : "No languages yet"}
          detail="Detected languages appear after a completed dictation."
        />
      )}
    </section>
  );
}

function TranscriptRow({
  item,
  copied,
  checked,
  onCopy,
  onConfirm,
}: {
  item: TranscriptRow;
  copied: boolean;
  checked: boolean;
  onCopy: () => void;
  onConfirm: () => void;
}) {
  const needsConfirmation =
    item.delivery.status === "indeterminate" && !checked;
  return (
    <article
      className={`dictation-row ${item.ephemeral ? "dictation-row--real" : ""}`}
    >
      <span className="app-tile app-tile--spick">
        {item.application[0]?.toUpperCase() ?? "S"}
      </span>
      <div className="dictation-row__body">
        <div className="dictation-row__meta">
          <strong>{item.application}</strong>
          <span>
            {item.ephemeral || item.completedAtMs === null
              ? "Just now"
              : formatRelativeTime(item.completedAtMs)}
          </span>
          {item.ephemeral && <em className="memory-badge">MEMORY ONLY</em>}
        </div>
        <p>{item.text}</p>
        {item.ephemeral && <DeliveryNote delivery={item.delivery} />}
        <div className="dictation-row__details">
          <span>{languageCode(item.languageTag)}</span>
          <span>{item.words} words</span>
        </div>
      </div>
      <button
        type="button"
        className={`button button--secondary dictation-copy-button ${
          item.delivery.status === "inserted"
            ? ""
            : "dictation-copy-button--recovery"
        }`}
        onClick={needsConfirmation ? onConfirm : onCopy}
        aria-label={
          needsConfirmation
            ? "Confirm field checked before copy"
            : item.ephemeral
              ? "Copy latest transcript"
              : `Copy dictation from ${item.application}`
        }
        disabled={!item.delivery.transcriptAvailable}
      >
        {copied ? (
          <>
            <CheckCircle2 size={14} /> Copied
          </>
        ) : needsConfirmation ? (
          <>
            <CheckCircle2 size={14} /> I checked the field
          </>
        ) : (
          <>
            <Copy size={14} /> Copy text
          </>
        )}
      </button>
    </article>
  );
}

function DataEmpty({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="empty-state data-empty-state">
      <Mic2 size={21} />
      <strong>{title}</strong>
      <span>{detail}</span>
    </div>
  );
}

function DeliveryNote({ delivery }: { delivery: NativeDeliveryOutcome }) {
  const presentation = describeDelivery(delivery);
  return (
    <div
      className={`delivery-note delivery-note--${presentation.tone}`}
      role="status"
    >
      {delivery.status === "inserted" ? (
        <CheckCircle2 size={14} />
      ) : (
        <Copy size={14} />
      )}
      <span>
        <strong>{presentation.title}</strong>
        <small>{presentation.detail}</small>
      </span>
    </div>
  );
}

function describeDelivery(delivery: NativeDeliveryOutcome) {
  const app = delivery.targetApp;
  switch (delivery.status) {
    case "inserted":
      return {
        tone: "inserted",
        title: app ? `Typed into ${app}` : "Typed where you started",
        detail:
          delivery.caretRepositioned === false
            ? "The text arrived, but the final caret position wasn’t confirmed."
            : "The field was still yours, so Spick put the words back.",
      };
    case "focusChanged":
      return {
        tone: "recovery",
        title: "Not typed—the cursor moved",
        detail:
          "Spick left the new field alone. Copy the text when you’re ready.",
      };
    case "secureField":
      return {
        tone: "recovery",
        title: "Not typed into a secure field",
        detail: "Password and private fields are always left alone.",
      };
    case "accessibilityMissing":
      return {
        tone: "recovery",
        title: "Accessibility access is off",
        detail: "Allow it in Settings, or copy this text yourself.",
      };
    case "unsupported":
      return {
        tone: "recovery",
        title: "This field needs a paste",
        detail: "Spick kept the transcript here instead of guessing.",
      };
    case "failed":
      return {
        tone: "recovery",
        title: "The field wouldn’t take the text",
        detail: "Nothing else was changed. You can copy the transcript below.",
      };
    case "indeterminate":
      return {
        tone: "recovery",
        title: "Spick couldn’t confirm the field",
        detail:
          "Check the field first; copy only if the words aren’t already there.",
      };
  }
}
