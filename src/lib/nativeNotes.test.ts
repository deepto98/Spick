import { beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import {
  createNote,
  deleteNote,
  exportNote,
  listNotes,
  updateNote,
} from "./nativeNotes";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => vi.mocked(invoke).mockReset());

it("keeps note persistence and export behind native commands", async () => {
  vi.mocked(invoke).mockResolvedValue(undefined);

  await listNotes();
  await createNote({ title: "Idea", body: "# Draft" });
  await updateNote("note-id", { title: "Idea", body: "Ready" });
  await exportNote("note-id", "md");
  await deleteNote("note-id");

  expect(invoke).toHaveBeenNthCalledWith(1, "list_notes");
  expect(invoke).toHaveBeenNthCalledWith(2, "create_note", {
    input: { title: "Idea", body: "# Draft" },
  });
  expect(invoke).toHaveBeenNthCalledWith(3, "update_note", {
    id: "note-id",
    input: { title: "Idea", body: "Ready" },
  });
  expect(invoke).toHaveBeenNthCalledWith(4, "export_note", {
    id: "note-id",
    format: "md",
  });
  expect(invoke).toHaveBeenNthCalledWith(5, "delete_note", { id: "note-id" });
});
