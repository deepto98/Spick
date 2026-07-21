import { invoke } from "@tauri-apps/api/core";

export interface Note {
  id: string;
  title: string;
  body: string;
  createdAtMs: number;
  updatedAtMs: number;
}

export interface NoteInput {
  title: string;
  body: string;
}

export const listNotes = () => invoke<Note[]>("list_notes");
export const createNote = (input: NoteInput) =>
  invoke<Note>("create_note", { input });
export const updateNote = (id: string, input: NoteInput) =>
  invoke<Note>("update_note", { id, input });
export const deleteNote = (id: string) =>
  invoke<boolean>("delete_note", { id });
export const exportNote = (id: string, format: "md" | "txt") =>
  invoke<boolean>("export_note", { id, format });
