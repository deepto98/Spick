import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  HistoryPage,
  LocalDataChangedEvent,
  UsageDashboard,
  VocabularyEntryDto,
} from "../lib/nativeLocalData";
import { useLocalData } from "./useLocalData";

const mocks = vi.hoisted(() => ({
  clear: vi.fn(),
  create: vi.fn(),
  delete: vi.fn(),
  dashboard: vi.fn(),
  history: vi.fn(),
  listVocabulary: vi.fn(),
  subscribe: vi.fn(),
  update: vi.fn(),
}));

vi.mock("../lib/nativeLocalData", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("../lib/nativeLocalData")>();
  return {
    ...actual,
    clearLocalData: mocks.clear,
    createVocabularyEntry: mocks.create,
    deleteVocabularyEntry: mocks.delete,
    getUsageDashboard: mocks.dashboard,
    listTranscriptHistory: mocks.history,
    listVocabulary: mocks.listVocabulary,
    subscribeToLocalDataChanges: mocks.subscribe,
    updateVocabularyEntry: mocks.update,
  };
});

const emptyMetrics = {
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
  today: emptyMetrics,
  period: emptyMetrics,
  previousPeriod: null,
  lifetime: emptyMetrics,
  days: [],
  languages: [],
  savedTranscriptCount: 0,
};

const vocabularyEntry: VocabularyEntryDto = {
  id: "vocab-1",
  phrase: "Spick",
  spokenForm: "speak",
  category: "company",
  languageTag: "en",
  enabled: true,
  createdAtMs: 1,
  updatedAtMs: 1,
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((onResolve, onReject) => {
    resolve = onResolve;
    reject = onReject;
  });
  return { promise, reject, resolve };
}

describe("useLocalData", () => {
  let listener: ((event: LocalDataChangedEvent) => void) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    listener = undefined;
    mocks.subscribe.mockImplementation(
      async (next: (event: LocalDataChangedEvent) => void) => {
        listener = next;
        return vi.fn();
      },
    );
    mocks.dashboard.mockResolvedValue(dashboard);
    mocks.history.mockResolvedValue({
      items: [],
      nextCursor: null,
    } satisfies HistoryPage);
    mocks.listVocabulary.mockResolvedValue([]);
  });

  afterEach(cleanup);

  it("attaches the metadata listener before its initial reads", async () => {
    const subscription = deferred<() => void>();
    mocks.subscribe.mockReturnValue(subscription.promise);
    const { result } = renderHook(() => useLocalData(true));

    expect(mocks.dashboard).not.toHaveBeenCalled();
    expect(mocks.history).not.toHaveBeenCalled();
    expect(result.current.dashboard).toBeNull();

    await act(async () => subscription.resolve(vi.fn()));

    await waitFor(() => expect(mocks.dashboard).toHaveBeenCalledOnce());
    expect(mocks.history).toHaveBeenCalledOnce();
    expect(mocks.listVocabulary).toHaveBeenCalledOnce();
  });

  it("does not subscribe from a disabled HUD/window controller", () => {
    renderHook(() => useLocalData(false));
    expect(mocks.subscribe).not.toHaveBeenCalled();
    expect(mocks.dashboard).not.toHaveBeenCalled();
  });

  it("keeps a subscription failure visible after the fallback reads succeed", async () => {
    mocks.subscribe.mockRejectedValue(new Error("event bridge unavailable"));
    const { result } = renderHook(() => useLocalData(true));

    await waitFor(() => expect(result.current.dashboard).toEqual(dashboard));
    expect(result.current.dashboardError).toBeNull();
    expect(result.current.subscriptionError).toBe("event bridge unavailable");
  });

  it("discards an older-page response after an event refresh", async () => {
    const older = deferred<HistoryPage>();
    const cursor = { completedAtMs: 10, sessionId: "page-1" };
    mocks.history
      .mockResolvedValueOnce({ items: [], nextCursor: cursor })
      .mockReturnValueOnce(older.promise)
      .mockResolvedValueOnce({ items: [], nextCursor: null });
    const { result } = renderHook(() => useLocalData(true));

    await waitFor(() =>
      expect(result.current.historyNextCursor).toEqual(cursor),
    );
    let olderRequest!: Promise<void>;
    act(() => {
      olderRequest = result.current.loadOlderHistory();
    });
    await waitFor(() => expect(mocks.history).toHaveBeenCalledTimes(2));

    act(() => listener?.({ revision: 2, domains: ["transcriptHistory"] }));
    await waitFor(() => expect(mocks.history).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(mocks.dashboard).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(result.current.historyNextCursor).toBeNull());

    await act(async () => {
      older.resolve({
        items: [
          {
            sessionId: "stale-session",
            startedAtMs: 1,
            completedAtMs: 2,
            text: "This should not return",
            wordCount: 4,
            speechDurationMs: 1_000,
            languageTag: "en",
            engineId: "local",
            targetApp: null,
            deliveryStatus: "inserted",
          },
        ],
        nextCursor: null,
      });
      await olderRequest;
    });

    expect(result.current.history).toEqual([]);
  });

  it("rolls an optimistic vocabulary deletion back when native storage fails", async () => {
    const deletion = deferred<{ deleted: boolean; id: string }>();
    mocks.listVocabulary.mockResolvedValue([vocabularyEntry]);
    mocks.delete.mockReturnValue(deletion.promise);
    const { result } = renderHook(() => useLocalData(true));

    await waitFor(() => expect(result.current.vocabulary).toHaveLength(1));
    let request!: Promise<boolean>;
    act(() => {
      request = result.current.deleteVocabulary(vocabularyEntry.id);
    });
    expect(result.current.vocabulary).toEqual([]);

    await act(async () => {
      deletion.reject(new Error("database is read-only"));
      await request;
    });

    expect(result.current.vocabulary).toEqual([vocabularyEntry]);
    expect(result.current.vocabularyError).toBe("database is read-only");
  });

  it("will not clear vocabulary while a mutation acknowledgement is pending", async () => {
    const creation = deferred<VocabularyEntryDto>();
    mocks.create.mockReturnValue(creation.promise);
    const { result } = renderHook(() => useLocalData(true));

    await waitFor(() => expect(result.current.vocabularyLoading).toBe(false));
    let createRequest!: Promise<boolean>;
    act(() => {
      createRequest = result.current.createVocabulary({
        phrase: "Tauri",
        spokenForm: null,
        category: "technical",
        languageTag: null,
        enabled: true,
      });
    });

    let clearResult: unknown;
    await act(async () => {
      clearResult = await result.current.clearData("all");
    });
    expect(clearResult).toBeNull();
    expect(mocks.clear).not.toHaveBeenCalled();
    expect(result.current.clearError).toMatch(/vocabulary change/i);

    await act(async () => {
      creation.resolve({
        ...vocabularyEntry,
        id: "vocab-2",
        phrase: "Tauri",
        spokenForm: null,
        languageTag: null,
      });
      await createRequest;
    });
  });
});
