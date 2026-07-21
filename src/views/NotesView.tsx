import { useEffect, useMemo, useRef, useState } from "react";
import {
  Download,
  Eye,
  FilePlus2,
  Mic2,
  PencilLine,
  Save,
  Trash2,
} from "lucide-react";
import { PageHeader } from "../components/Ui";
import type { NativeDictationTranscript } from "../lib/nativeDictation";
import {
  createNote,
  deleteNote,
  exportNote,
  listNotes,
  updateNote,
  type Note,
} from "../lib/nativeNotes";
import type { HudState } from "../types";

interface NotesViewProps {
  native: boolean;
  dictationState: HudState;
  transcript: NativeDictationTranscript | null;
  shortcut: string;
}

export function NotesView({
  native,
  dictationState,
  transcript,
  shortcut,
}: NotesViewProps) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [preview, setPreview] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seenTranscript = useRef<string | null>(transcript?.sessionId ?? null);
  const selected = notes.find((note) => note.id === selectedId) ?? null;

  useEffect(() => {
    if (!native) return;
    void listNotes()
      .then((items) => {
        setNotes(items);
        const first = items[0];
        if (first) {
          setSelectedId(first.id);
          setTitle(first.title);
          setBody(first.body);
        }
      })
      .catch((reason) => setError(`Couldn’t open notes: ${String(reason)}`));
  }, [native]);

  useEffect(() => {
    if (!transcript || transcript.sessionId === seenTranscript.current) return;
    seenTranscript.current = transcript.sessionId;
    setBody((current) => {
      const separator = current.trim() && !current.endsWith("\n") ? " " : "";
      return `${current}${separator}${transcript.transcript.text}`;
    });
  }, [transcript]);

  useEffect(() => {
    if (!native || !selectedId || !selected) return;
    if (title === selected.title && body === selected.body) return;
    const timeout = window.setTimeout(() => {
      setSaving(true);
      void updateNote(selectedId, { title, body })
        .then((updated) => {
          setNotes((current) =>
            current
              .map((note) => (note.id === updated.id ? updated : note))
              .sort((left, right) => right.updatedAtMs - left.updatedAtMs),
          );
          setError(null);
        })
        .catch((reason) =>
          setError(`Couldn’t save the note: ${String(reason)}`),
        )
        .finally(() => setSaving(false));
    }, 450);
    return () => window.clearTimeout(timeout);
  }, [body, native, selected, selectedId, title]);

  const status = useMemo(() => {
    if (dictationState === "listening") return "Listening — keep talking";
    if (dictationState === "processing") return "Writing that down…";
    if (saving) return "Saving…";
    return "Saved on this Mac";
  }, [dictationState, saving]);

  const makeNote = async () => {
    if (!native) return;
    try {
      const note = await createNote({ title: "Untitled note", body: "" });
      setNotes((current) => [note, ...current]);
      setSelectedId(note.id);
      setTitle(note.title);
      setBody(note.body);
      setPreview(false);
    } catch (reason) {
      setError(`Couldn’t create a note: ${String(reason)}`);
    }
  };

  const removeSelected = async () => {
    if (!selectedId || !window.confirm("Delete this note?")) return;
    try {
      await deleteNote(selectedId);
      const remaining = notes.filter((note) => note.id !== selectedId);
      setNotes(remaining);
      const next = remaining[0] ?? null;
      setSelectedId(next?.id ?? null);
      setTitle(next?.title ?? "");
      setBody(next?.body ?? "");
    } catch (reason) {
      setError(`Couldn’t delete the note: ${String(reason)}`);
    }
  };

  const selectNote = (note: Note) => {
    setSelectedId(note.id);
    setTitle(note.title);
    setBody(note.body);
  };

  return (
    <div className="view view--notes">
      <PageHeader
        eyebrow="VOICE-FIRST WRITING"
        title="Notes"
        description="Capture a thought with Option, shape it in Markdown, and export it when it’s ready."
        actions={
          <button
            type="button"
            className="button button--primary"
            onClick={() => void makeNote()}
          >
            <FilePlus2 size={15} /> New note
          </button>
        }
      />
      {error && (
        <div className="engine-inline-error" role="alert">
          {error}
        </div>
      )}
      <div className="notes-workspace">
        <aside className="notes-list" aria-label="Notes">
          {notes.length === 0 ? (
            <div className="notes-empty">
              <PencilLine size={22} />
              <strong>No notes yet</strong>
              <span>Start with a sentence. Structure can come later.</span>
            </div>
          ) : (
            notes.map((note) => (
              <button
                type="button"
                key={note.id}
                className={
                  selectedId === note.id
                    ? "notes-list__item notes-list__item--active"
                    : "notes-list__item"
                }
                onClick={() => selectNote(note)}
              >
                <strong>{note.title}</strong>
                <span>{note.body.trim().slice(0, 82) || "Empty note"}</span>
              </button>
            ))
          )}
        </aside>
        <section className="note-editor">
          {selected ? (
            <>
              <header className="note-editor__toolbar">
                <div className="note-editor__voice">
                  <Mic2 size={14} />
                  <span>{status}</span>
                  <kbd>{shortcut}</kbd>
                </div>
                <div>
                  <button
                    type="button"
                    className="icon-button"
                    onClick={() => setPreview((value) => !value)}
                    aria-label={preview ? "Edit note" : "Preview Markdown"}
                  >
                    {preview ? <PencilLine size={15} /> : <Eye size={15} />}
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    onClick={() => void exportNote(selected.id, "md")}
                    aria-label="Export Markdown"
                  >
                    <Download size={15} />
                    <small>.md</small>
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    onClick={() => void exportNote(selected.id, "txt")}
                    aria-label="Export text"
                  >
                    <Save size={15} />
                    <small>.txt</small>
                  </button>
                  <button
                    type="button"
                    className="icon-button"
                    onClick={() => void removeSelected()}
                    aria-label="Delete note"
                  >
                    <Trash2 size={15} />
                  </button>
                </div>
              </header>
              <input
                className="note-title"
                value={title}
                onChange={(event) => setTitle(event.target.value)}
                aria-label="Note title"
              />
              {preview ? (
                <MarkdownPreview markdown={body} />
              ) : (
                <textarea
                  className="note-body"
                  value={body}
                  onChange={(event) => setBody(event.target.value)}
                  placeholder="Speak or write here… Markdown is welcome."
                  aria-label="Note body"
                />
              )}
            </>
          ) : (
            <div className="note-editor__blank">
              <PencilLine size={28} />
              <strong>Make room for a thought</strong>
              <button
                type="button"
                className="button button--primary"
                onClick={() => void makeNote()}
              >
                Create a note
              </button>
            </div>
          )}
        </section>
      </div>
    </div>
  );
}

function MarkdownPreview({ markdown }: { markdown: string }) {
  return (
    <article className="markdown-preview">
      {markdown.split("\n").map((line, index) => {
        if (line.startsWith("### "))
          return <h3 key={index}>{line.slice(4)}</h3>;
        if (line.startsWith("## ")) return <h2 key={index}>{line.slice(3)}</h2>;
        if (line.startsWith("# ")) return <h1 key={index}>{line.slice(2)}</h1>;
        if (line.startsWith("- ")) return <li key={index}>{line.slice(2)}</li>;
        if (line.startsWith("> "))
          return <blockquote key={index}>{line.slice(2)}</blockquote>;
        return line ? <p key={index}>{line}</p> : <br key={index} />;
      })}
    </article>
  );
}
