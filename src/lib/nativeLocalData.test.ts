import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  clearLocalData,
  createVocabularyEntry,
  deleteVocabularyEntry,
  getUsageDashboard,
  listTranscriptHistory,
  listVocabulary,
  updateVocabularyEntry,
  type VocabularyInput,
} from "./nativeLocalData";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const input: VocabularyInput = {
  phrase: "WebRTC",
  spokenForm: "web R T C",
  category: "technical",
  languageTag: "en",
  enabled: true,
};

describe("native local-data commands", () => {
  beforeEach(() => vi.mocked(invoke).mockReset());

  it("uses the dashboard and cursor contracts verbatim", async () => {
    vi.mocked(invoke).mockResolvedValue({});
    const cursor = { completedAtMs: 12, sessionId: "session-1" };

    await getUsageDashboard(14);
    await listTranscriptHistory(cursor, 25);

    expect(invoke).toHaveBeenNthCalledWith(1, "get_usage_dashboard", {
      days: 14,
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "list_transcript_history", {
      cursor,
      limit: 25,
    });
  });

  it("sends acknowledged vocabulary mutations with camel-case DTOs", async () => {
    vi.mocked(invoke).mockResolvedValue({});

    await listVocabulary();
    await createVocabularyEntry(input);
    await updateVocabularyEntry("vocab-1", input);
    await deleteVocabularyEntry("vocab-1");

    expect(invoke).toHaveBeenNthCalledWith(1, "list_vocabulary");
    expect(invoke).toHaveBeenNthCalledWith(2, "create_vocabulary_entry", {
      input,
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "update_vocabulary_entry", {
      id: "vocab-1",
      input,
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "delete_vocabulary_entry", {
      id: "vocab-1",
    });
  });

  it("passes the selected clear scope", async () => {
    vi.mocked(invoke).mockResolvedValue({});

    await clearLocalData("usageAndHistory");

    expect(invoke).toHaveBeenCalledWith("clear_local_data", {
      scope: "usageAndHistory",
    });
  });
});
