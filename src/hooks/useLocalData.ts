import { useCallback, useEffect, useRef, useState } from "react";
import {
  clearLocalData,
  createVocabularyEntry,
  deleteVocabularyEntry,
  getUsageDashboard,
  listTranscriptHistory,
  listVocabulary,
  subscribeToLocalDataChanges,
  updateVocabularyEntry,
  type ClearLocalDataResult,
  type ClearLocalDataScope,
  type HistoryCursor,
  type TranscriptHistoryItem,
  type UsageDashboard,
  type VocabularyEntryDto,
  type VocabularyInput,
} from "../lib/nativeLocalData";

const HISTORY_PAGE_SIZE = 20;

function errorMessage(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}

function domainChanged(domains: readonly string[], ...needles: string[]) {
  if (domains.length === 0) return true;
  return domains.some((domain) => {
    const normalized = domain.toLowerCase();
    return needles.some((needle) => normalized.includes(needle));
  });
}

function insertAt<T>(items: readonly T[], item: T, index: number) {
  const next = [...items];
  next.splice(Math.min(index, next.length), 0, item);
  return next;
}

function sqliteLower(value: string) {
  return value.replace(/[A-Z]/g, (character) => character.toLowerCase());
}

function sortVocabulary(entries: readonly VocabularyEntryDto[]) {
  return [...entries].sort((left, right) => {
    if (left.enabled !== right.enabled) return left.enabled ? -1 : 1;
    const leftLower = sqliteLower(left.phrase);
    const rightLower = sqliteLower(right.phrase);
    if (leftLower !== rightLower) return leftLower < rightLower ? -1 : 1;
    if (left.phrase !== right.phrase)
      return left.phrase < right.phrase ? -1 : 1;
    return left.id < right.id ? -1 : left.id === right.id ? 0 : 1;
  });
}

export interface LocalDataController {
  dashboard: UsageDashboard | null;
  dashboardError: string | null;
  dashboardLoading: boolean;
  subscriptionError: string | null;
  history: TranscriptHistoryItem[];
  historyError: string | null;
  historyLoading: boolean;
  historyLoadingMore: boolean;
  historyNextCursor: HistoryCursor | null;
  vocabulary: VocabularyEntryDto[];
  vocabularyError: string | null;
  vocabularyLoading: boolean;
  vocabularyPendingIds: ReadonlySet<string>;
  clearError: string | null;
  clearPendingScope: ClearLocalDataScope | null;
  lastClearResult: ClearLocalDataResult | null;
  refreshDashboardAndHistory: () => Promise<void>;
  loadOlderHistory: () => Promise<void>;
  refreshVocabulary: () => Promise<void>;
  createVocabulary: (input: VocabularyInput) => Promise<boolean>;
  updateVocabulary: (id: string, input: VocabularyInput) => Promise<boolean>;
  deleteVocabulary: (id: string) => Promise<boolean>;
  clearData: (
    scope: ClearLocalDataScope,
  ) => Promise<ClearLocalDataResult | null>;
}

export function useLocalData(enabled: boolean): LocalDataController {
  const [dashboard, setDashboard] = useState<UsageDashboard | null>(null);
  const [dashboardError, setDashboardError] = useState<string | null>(null);
  const [dashboardLoading, setDashboardLoading] = useState(enabled);
  const [subscriptionError, setSubscriptionError] = useState<string | null>(
    null,
  );
  const [history, setHistory] = useState<TranscriptHistoryItem[]>([]);
  const [historyError, setHistoryError] = useState<string | null>(null);
  const [historyLoading, setHistoryLoading] = useState(enabled);
  const [historyLoadingMore, setHistoryLoadingMore] = useState(false);
  const [historyNextCursor, setHistoryNextCursor] =
    useState<HistoryCursor | null>(null);
  const [vocabulary, setVocabulary] = useState<VocabularyEntryDto[]>([]);
  const [vocabularyError, setVocabularyError] = useState<string | null>(null);
  const [vocabularyLoading, setVocabularyLoading] = useState(enabled);
  const [vocabularyPendingIds, setVocabularyPendingIds] = useState<
    ReadonlySet<string>
  >(new Set());
  const [clearError, setClearError] = useState<string | null>(null);
  const [clearPendingScope, setClearPendingScope] =
    useState<ClearLocalDataScope | null>(null);
  const [lastClearResult, setLastClearResult] =
    useState<ClearLocalDataResult | null>(null);
  const aliveRef = useRef(true);
  const dashboardRequestRef = useRef(0);
  const historyRequestRef = useRef(0);
  const historyLoadMoreRequestRef = useRef(0);
  const historyLoadingMoreRef = useRef(false);
  const vocabularyPendingRef = useRef<ReadonlySet<string>>(new Set());
  const clearPendingRef = useRef<ClearLocalDataScope | null>(null);
  const vocabularyRequestRef = useRef(0);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const refreshDashboard = useCallback(async () => {
    if (!enabled) return;
    const request = ++dashboardRequestRef.current;
    setDashboardLoading(true);
    setDashboardError(null);
    try {
      const next = await getUsageDashboard(7);
      if (aliveRef.current && request === dashboardRequestRef.current) {
        setDashboard(next);
      }
    } catch (reason) {
      if (aliveRef.current && request === dashboardRequestRef.current) {
        setDashboardError(errorMessage(reason));
      }
    } finally {
      if (aliveRef.current && request === dashboardRequestRef.current) {
        setDashboardLoading(false);
      }
    }
  }, [enabled]);

  const refreshHistory = useCallback(async () => {
    if (!enabled) return;
    const request = ++historyRequestRef.current;
    ++historyLoadMoreRequestRef.current;
    historyLoadingMoreRef.current = false;
    setHistoryLoadingMore(false);
    setHistoryLoading(true);
    setHistoryError(null);
    try {
      const page = await listTranscriptHistory(null, HISTORY_PAGE_SIZE);
      if (aliveRef.current && request === historyRequestRef.current) {
        setHistory(page.items);
        setHistoryNextCursor(page.nextCursor);
      }
    } catch (reason) {
      if (aliveRef.current && request === historyRequestRef.current) {
        setHistoryError(errorMessage(reason));
      }
    } finally {
      if (aliveRef.current && request === historyRequestRef.current) {
        setHistoryLoading(false);
      }
    }
  }, [enabled]);

  const refreshDashboardAndHistory = useCallback(async () => {
    await Promise.all([refreshDashboard(), refreshHistory()]);
  }, [refreshDashboard, refreshHistory]);

  const refreshVocabulary = useCallback(async () => {
    if (!enabled) return;
    const request = ++vocabularyRequestRef.current;
    setVocabularyLoading(true);
    setVocabularyError(null);
    try {
      const entries = await listVocabulary();
      if (aliveRef.current && request === vocabularyRequestRef.current) {
        setVocabulary(entries);
      }
    } catch (reason) {
      if (aliveRef.current && request === vocabularyRequestRef.current) {
        setVocabularyError(errorMessage(reason));
      }
    } finally {
      if (aliveRef.current && request === vocabularyRequestRef.current) {
        setVocabularyLoading(false);
      }
    }
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;

    void subscribeToLocalDataChanges((change) => {
      if (disposed || !Number.isSafeInteger(change.revision)) {
        return;
      }
      // Revisions are useful metadata, but events can be delivered out of
      // allocation order when different native worker threads finish. Process
      // every valid event so one domain cannot accidentally mask another.
      const domains = Array.isArray(change.domains) ? change.domains : [];
      const historyChanged = domainChanged(domains, "history", "transcript");
      if (domainChanged(domains, "usage") || historyChanged)
        void refreshDashboard();
      if (historyChanged) void refreshHistory();
      if (domainChanged(domains, "vocabulary")) void refreshVocabulary();
    })
      .then((stopListening) => {
        if (disposed) stopListening();
        else {
          unsubscribe = stopListening;
          setSubscriptionError(null);
          // Refresh after the listener is attached so a mutation cannot land
          // in a gap between the initial reads and event subscription.
          void refreshDashboardAndHistory();
          void refreshVocabulary();
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setSubscriptionError(errorMessage(reason));
          // A listener failure should not make the stored data unreadable.
          void refreshDashboardAndHistory();
          void refreshVocabulary();
        }
      });

    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [
    enabled,
    refreshDashboard,
    refreshDashboardAndHistory,
    refreshHistory,
    refreshVocabulary,
  ]);

  const loadOlderHistory = useCallback(async () => {
    if (!enabled || !historyNextCursor || historyLoadingMoreRef.current) return;
    const cursor = historyNextCursor;
    const historyGeneration = historyRequestRef.current;
    const request = ++historyLoadMoreRequestRef.current;
    historyLoadingMoreRef.current = true;
    setHistoryLoadingMore(true);
    setHistoryError(null);
    try {
      const page = await listTranscriptHistory(cursor, HISTORY_PAGE_SIZE);
      if (
        !aliveRef.current ||
        request !== historyLoadMoreRequestRef.current ||
        historyGeneration !== historyRequestRef.current
      ) {
        return;
      }
      setHistory((current) => {
        const seen = new Set(current.map((item) => item.sessionId));
        return [
          ...current,
          ...page.items.filter((item) => !seen.has(item.sessionId)),
        ];
      });
      setHistoryNextCursor(page.nextCursor);
    } catch (reason) {
      if (
        aliveRef.current &&
        request === historyLoadMoreRequestRef.current &&
        historyGeneration === historyRequestRef.current
      ) {
        setHistoryError(errorMessage(reason));
      }
    } finally {
      if (aliveRef.current && request === historyLoadMoreRequestRef.current) {
        historyLoadingMoreRef.current = false;
        setHistoryLoadingMore(false);
      }
    }
  }, [enabled, historyNextCursor]);

  const markVocabularyPending = useCallback((id: string, pending: boolean) => {
    const next = new Set(vocabularyPendingRef.current);
    if (pending) next.add(id);
    else next.delete(id);
    vocabularyPendingRef.current = next;
    setVocabularyPendingIds(next);
  }, []);

  const createVocabulary = useCallback(
    async (input: VocabularyInput) => {
      if (!enabled || vocabularyPendingRef.current.has("create")) return false;
      if (clearPendingRef.current) {
        setVocabularyError("Wait for the local-data reset to finish.");
        return false;
      }
      markVocabularyPending("create", true);
      setVocabularyError(null);
      try {
        const saved = await createVocabularyEntry(input);
        if (!aliveRef.current) return false;
        setVocabulary((current) => [
          ...sortVocabulary([
            saved,
            ...current.filter((entry) => entry.id !== saved.id),
          ]),
        ]);
        return true;
      } catch (reason) {
        if (aliveRef.current) setVocabularyError(errorMessage(reason));
        return false;
      } finally {
        if (aliveRef.current) markVocabularyPending("create", false);
      }
    },
    [enabled, markVocabularyPending],
  );

  const updateVocabulary = useCallback(
    async (id: string, input: VocabularyInput) => {
      if (!enabled || vocabularyPendingRef.current.has(id)) return false;
      if (clearPendingRef.current) {
        setVocabularyError("Wait for the local-data reset to finish.");
        return false;
      }
      const original = vocabulary.find((entry) => entry.id === id);
      if (!original) return false;
      markVocabularyPending(id, true);
      setVocabularyError(null);
      setVocabulary((current) =>
        sortVocabulary(
          current.map((entry) =>
            entry.id === id ? { ...entry, ...input } : entry,
          ),
        ),
      );
      try {
        const saved = await updateVocabularyEntry(id, input);
        if (!aliveRef.current) return false;
        setVocabulary((current) =>
          sortVocabulary(
            current.map((entry) => (entry.id === id ? saved : entry)),
          ),
        );
        return true;
      } catch (reason) {
        if (aliveRef.current) {
          setVocabulary((current) =>
            sortVocabulary(
              current.map((entry) => (entry.id === id ? original : entry)),
            ),
          );
          setVocabularyError(errorMessage(reason));
        }
        return false;
      } finally {
        if (aliveRef.current) markVocabularyPending(id, false);
      }
    },
    [enabled, markVocabularyPending, vocabulary],
  );

  const deleteVocabulary = useCallback(
    async (id: string) => {
      if (!enabled || vocabularyPendingRef.current.has(id)) return false;
      if (clearPendingRef.current) {
        setVocabularyError("Wait for the local-data reset to finish.");
        return false;
      }
      const index = vocabulary.findIndex((entry) => entry.id === id);
      if (index < 0) return false;
      const original = vocabulary[index];
      markVocabularyPending(id, true);
      setVocabularyError(null);
      setVocabulary((current) => current.filter((entry) => entry.id !== id));
      try {
        const result = await deleteVocabularyEntry(id);
        if (!result.deleted)
          throw new Error("That phrase was already removed.");
        return true;
      } catch (reason) {
        if (aliveRef.current && original) {
          setVocabulary((current) =>
            current.some((entry) => entry.id === id)
              ? current
              : insertAt(current, original, index),
          );
          setVocabularyError(errorMessage(reason));
        }
        return false;
      } finally {
        if (aliveRef.current) markVocabularyPending(id, false);
      }
    },
    [enabled, markVocabularyPending, vocabulary],
  );

  const clearData = useCallback(
    async (scope: ClearLocalDataScope) => {
      if (!enabled || clearPendingRef.current) return null;
      if (
        (scope === "vocabulary" || scope === "all") &&
        vocabularyPendingRef.current.size > 0
      ) {
        setClearError(
          "Wait for the vocabulary change to finish, then try again.",
        );
        return null;
      }
      clearPendingRef.current = scope;
      if (scope !== "vocabulary") {
        ++historyRequestRef.current;
        ++historyLoadMoreRequestRef.current;
        historyLoadingMoreRef.current = false;
        setHistoryLoading(false);
        setHistoryLoadingMore(false);
      }
      setClearPendingScope(scope);
      setClearError(null);
      setLastClearResult(null);
      try {
        const result = await clearLocalData(scope);
        if (!aliveRef.current) return null;
        setLastClearResult(result);
        if (scope === "transcriptHistory") {
          setHistory([]);
          setHistoryNextCursor(null);
        } else if (scope === "usageAndHistory" || scope === "all") {
          setDashboard(null);
          setHistory([]);
          setHistoryNextCursor(null);
        }
        if (scope === "vocabulary" || scope === "all") setVocabulary([]);
        await Promise.all([
          scope === "vocabulary" ? Promise.resolve() : refreshDashboard(),
          scope === "vocabulary" ? Promise.resolve() : refreshHistory(),
          scope === "vocabulary" || scope === "all"
            ? refreshVocabulary()
            : Promise.resolve(),
        ]);
        return result;
      } catch (reason) {
        if (aliveRef.current) setClearError(errorMessage(reason));
        return null;
      } finally {
        clearPendingRef.current = null;
        if (aliveRef.current) setClearPendingScope(null);
      }
    },
    [enabled, refreshDashboard, refreshHistory, refreshVocabulary],
  );

  return {
    dashboard,
    dashboardError,
    dashboardLoading,
    subscriptionError,
    history,
    historyError,
    historyLoading,
    historyLoadingMore,
    historyNextCursor,
    vocabulary,
    vocabularyError,
    vocabularyLoading,
    vocabularyPendingIds,
    clearError,
    clearPendingScope,
    lastClearResult,
    refreshDashboardAndHistory,
    loadOlderHistory,
    refreshVocabulary,
    createVocabulary,
    updateVocabulary,
    deleteVocabulary,
    clearData,
  };
}
