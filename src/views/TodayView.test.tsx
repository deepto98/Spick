import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  TranscriptHistoryItem,
  UsageDashboard,
  UsageMetrics,
} from "../lib/nativeLocalData";
import type { NativeDictationTranscript } from "../lib/nativeDictation";
import { percentageChange } from "../lib/localDataPresentation";
import { TodayView } from "./TodayView";

const emptyMetrics: UsageMetrics = {
  sessions: 0,
  words: 0,
  speechDurationMs: 0,
  averageWpm: null,
  estimatedTimeSavedMs: 0,
};

const dashboard: UsageDashboard = {
  generatedAtMs: 1,
  daysRequested: 7,
  durationBasis: "capture",
  typingBaselineWpm: 40,
  today: {
    sessions: 2,
    words: 120,
    speechDurationMs: 60_000,
    averageWpm: 120,
    estimatedTimeSavedMs: 120_000,
  },
  period: {
    sessions: 5,
    words: 500,
    speechDurationMs: 300_000,
    averageWpm: 100,
    estimatedTimeSavedMs: 450_000,
  },
  previousPeriod: {
    sessions: 4,
    words: 400,
    speechDurationMs: 280_000,
    averageWpm: 86,
    estimatedTimeSavedMs: 320_000,
  },
  lifetime: {
    sessions: 5,
    words: 500,
    speechDurationMs: 300_000,
    averageWpm: 100,
    estimatedTimeSavedMs: 450_000,
  },
  days: [
    { ...emptyMetrics, localDate: "2026-07-14" },
    { ...emptyMetrics, localDate: "2026-07-15", sessions: 1, words: 80 },
    { ...emptyMetrics, localDate: "2026-07-16" },
    { ...emptyMetrics, localDate: "2026-07-17", sessions: 1, words: 120 },
    { ...emptyMetrics, localDate: "2026-07-18", sessions: 1, words: 100 },
    { ...emptyMetrics, localDate: "2026-07-19" },
    { ...emptyMetrics, localDate: "2026-07-20", sessions: 2, words: 200 },
  ],
  languages: [
    {
      ...emptyMetrics,
      languageTag: "en",
      sessions: 4,
      words: 420,
    },
    { ...emptyMetrics, languageTag: "hi", sessions: 1, words: 80 },
  ],
  savedTranscriptCount: 1,
};

const lastTranscript: NativeDictationTranscript = {
  sessionId: "session-1",
  engineId: "whisper-tiny",
  transcript: {
    text: "Move the review to ten tomorrow.",
    segments: [],
    detectedLanguage: "en",
    confidence: null,
    isFinal: true,
  },
  delivery: {
    status: "focusChanged",
    transcriptAvailable: true,
    targetApp: "Mail",
    caretRepositioned: null,
  },
};

const baseProps = {
  dashboard,
  dashboardLoading: false,
  delivery: lastTranscript.delivery,
  hasOlderHistory: false,
  history: [] as TranscriptHistoryItem[],
  historyLoading: false,
  historyLoadingMore: false,
  hudState: "idle" as const,
  language: "EN",
  lastTranscript,
  native: true,
  onHudStateChange: vi.fn(),
  onLoadOlderHistory: vi.fn(),
  onOpenEngines: vi.fn(),
  onRefreshLocalData: vi.fn(),
  saveTranscriptHistory: false,
};

describe("local usage and transcript history", () => {
  const writeText = vi.fn();

  beforeEach(() => {
    writeText.mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
  });

  afterEach(() => {
    cleanup();
    vi.clearAllMocks();
  });

  it("renders real usage with capture-duration wording and a safe trend", () => {
    render(<TodayView {...baseProps} />);

    expect(screen.getAllByText("120").length).toBeGreaterThan(0);
    expect(screen.getByText("recording words/min")).toBeInTheDocument();
    expect(
      screen.getByText("Full capture time, including pauses"),
    ).toBeInTheDocument();
    expect(screen.getByText("25% from the prior period")).toBeInTheDocument();
    expect(screen.queryByText(/sample data/i)).not.toBeInTheDocument();
  });

  it("shows loading, empty, and retryable error states without sample rows", () => {
    const onRefreshLocalData = vi.fn();
    render(
      <TodayView
        {...baseProps}
        dashboard={null}
        dashboardError="database busy"
        dashboardLoading
        history={[]}
        historyError="history unavailable"
        historyLoading
        lastTranscript={null}
        onRefreshLocalData={onRefreshLocalData}
      />,
    );

    expect(screen.getByText("Couldn’t load usage")).toBeInTheDocument();
    expect(screen.getByText("Loading your week…")).toBeInTheDocument();
    expect(screen.getByText("Loading recent dictations…")).toBeInTheDocument();
    expect(screen.queryByText("Notion")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onRefreshLocalData).toHaveBeenCalledOnce();
  });

  it("renders zero days without inventing activity or dividing by zero", () => {
    render(
      <TodayView
        {...baseProps}
        dashboard={{
          ...dashboard,
          today: emptyMetrics,
          period: emptyMetrics,
          previousPeriod: emptyMetrics,
          days: dashboard.days.map((day) => ({
            ...emptyMetrics,
            localDate: day.localDate,
          })),
          languages: [],
        }}
        lastTranscript={null}
      />,
    );

    expect(
      screen.getByText("No change from the prior period"),
    ).toBeInTheDocument();
    expect(screen.getByText("No languages yet")).toBeInTheDocument();
    expect(screen.queryByText(/Infinity|NaN/)).not.toBeInTheDocument();
    expect(percentageChange(0, 0)).toBeNull();
  });

  it("deduplicates the ephemeral recovery transcript after it is persisted", () => {
    const persisted: TranscriptHistoryItem = {
      sessionId: lastTranscript.sessionId,
      startedAtMs: 1,
      completedAtMs: Date.now(),
      text: lastTranscript.transcript.text,
      wordCount: 6,
      speechDurationMs: 4_000,
      languageTag: "en",
      engineId: lastTranscript.engineId,
      targetApp: "Mail",
      deliveryStatus: "focusChanged",
    };
    render(
      <TodayView {...baseProps} history={[persisted]} saveTranscriptHistory />,
    );

    expect(screen.getAllByText(lastTranscript.transcript.text)).toHaveLength(1);
    expect(screen.queryByText("MEMORY ONLY")).not.toBeInTheDocument();
  });

  it("explains that disabling future saves does not hide older history", () => {
    render(
      <TodayView
        {...baseProps}
        history={[
          {
            sessionId: "older-session",
            startedAtMs: 1,
            completedAtMs: 2,
            text: "An older saved transcript.",
            wordCount: 4,
            speechDurationMs: 1_000,
            languageTag: "en",
            engineId: "local",
            targetApp: "Notes",
            deliveryStatus: "inserted",
          },
        ]}
        lastTranscript={null}
        saveTranscriptHistory={false}
      />,
    );

    expect(
      screen.getByText(
        "New transcripts stay memory-only; older saved history remains until deleted",
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("An older saved transcript.")).toBeInTheDocument();
  });

  it("keeps a focus-change transcript visible with an explicit copy action", async () => {
    render(<TodayView {...baseProps} />);

    expect(screen.getByText("Not typed—the cursor moved")).toBeInTheDocument();
    expect(screen.getByText("Mail")).toBeInTheDocument();
    expect(writeText).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: "Copy latest transcript" }),
    );

    expect(writeText).toHaveBeenCalledWith(lastTranscript.transcript.text);
    expect(await screen.findByText("Copied")).toBeInTheDocument();
  });

  it("makes an indeterminate write a two-step copy", () => {
    const indeterminateTranscript: NativeDictationTranscript = {
      ...lastTranscript,
      sessionId: "session-indeterminate",
      delivery: {
        ...lastTranscript.delivery,
        status: "indeterminate",
      },
    };
    render(
      <TodayView {...baseProps} lastTranscript={indeterminateTranscript} />,
    );

    fireEvent.click(
      screen.getByRole("button", {
        name: "Confirm field checked before copy",
      }),
    );
    expect(writeText).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: "Copy latest transcript" }),
    );
    expect(writeText).toHaveBeenCalledWith(
      indeterminateTranscript.transcript.text,
    );
  });
});
