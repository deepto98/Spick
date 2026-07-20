import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export const LOCAL_DATA_CHANGED_EVENT = "local-data://changed";

export interface UsageMetrics {
  sessions: number;
  words: number;
  speechDurationMs: number;
  averageWpm: number | null;
  estimatedTimeSavedMs: number;
}

export interface UsageDay extends UsageMetrics {
  localDate: string;
}

export interface UsageLanguage extends UsageMetrics {
  languageTag: string;
}

export interface UsageDashboard {
  generatedAtMs: number;
  daysRequested: number;
  durationBasis: "capture";
  typingBaselineWpm: 40;
  today: UsageMetrics;
  period: UsageMetrics;
  previousPeriod: UsageMetrics | null;
  lifetime: UsageMetrics;
  days: UsageDay[];
  languages: UsageLanguage[];
  savedTranscriptCount: number;
}

export interface HistoryCursor {
  completedAtMs: number;
  sessionId: string;
}

export interface TranscriptHistoryItem {
  sessionId: string;
  startedAtMs: number;
  completedAtMs: number;
  text: string;
  wordCount: number;
  speechDurationMs: number;
  languageTag: string;
  engineId: string;
  targetApp: string | null;
  deliveryStatus: string;
}

export interface HistoryPage {
  items: TranscriptHistoryItem[];
  nextCursor: HistoryCursor | null;
}

export type VocabularyCategory =
  "name" | "technical" | "company" | "replacement";

export interface VocabularyInput {
  phrase: string;
  spokenForm: string | null;
  category: VocabularyCategory;
  languageTag: string | null;
  enabled: boolean;
}

export interface VocabularyEntryDto extends VocabularyInput {
  id: string;
  createdAtMs: number;
  updatedAtMs: number;
}

export type ClearLocalDataScope =
  "transcriptHistory" | "usageAndHistory" | "vocabulary" | "all";

export interface DeleteVocabularyResult {
  deleted: boolean;
  id: string;
}

export interface ClearLocalDataResult {
  scope: ClearLocalDataScope;
  deletedUsageSessions: number;
  deletedTranscripts: number;
  deletedVocabularyEntries: number;
  clearedLatestTranscript: boolean;
  clearedLatestSessionId: string | null;
  storageCleanupComplete: boolean;
  storageCleanupWarning: string | null;
  memoryCleanupComplete: boolean;
  memoryCleanupWarning: string | null;
  clearedAtMs: number;
}

export interface LocalDataChangedEvent {
  revision: number;
  domains: string[];
}

export function getUsageDashboard(days = 7) {
  return invoke<UsageDashboard>("get_usage_dashboard", { days });
}

export function listTranscriptHistory(
  cursor: HistoryCursor | null = null,
  limit: number | null = null,
) {
  return invoke<HistoryPage>("list_transcript_history", { cursor, limit });
}

export function listVocabulary() {
  return invoke<VocabularyEntryDto[]>("list_vocabulary");
}

export function createVocabularyEntry(input: VocabularyInput) {
  return invoke<VocabularyEntryDto>("create_vocabulary_entry", { input });
}

export function updateVocabularyEntry(id: string, input: VocabularyInput) {
  return invoke<VocabularyEntryDto>("update_vocabulary_entry", { id, input });
}

export function deleteVocabularyEntry(id: string) {
  return invoke<DeleteVocabularyResult>("delete_vocabulary_entry", { id });
}

export function clearLocalData(scope: ClearLocalDataScope) {
  return invoke<ClearLocalDataResult>("clear_local_data", { scope });
}

export function subscribeToLocalDataChanges(
  handler: (change: LocalDataChangedEvent) => void,
): Promise<UnlistenFn> {
  return listen<LocalDataChangedEvent>(LOCAL_DATA_CHANGED_EVENT, (event) => {
    handler(event.payload);
  });
}
