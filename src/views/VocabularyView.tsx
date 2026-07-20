import { useMemo, useState } from "react";
import {
  BookOpenText,
  Check,
  Info,
  Languages,
  Lightbulb,
  PenLine,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { PageHeader, SelectField, Toggle } from "../components/Ui";
import type {
  VocabularyCategory,
  VocabularyEntryDto,
  VocabularyInput,
} from "../lib/nativeLocalData";

interface VocabularyViewProps {
  vocabulary: VocabularyEntryDto[];
  loading: boolean;
  error?: string | null;
  pendingIds: ReadonlySet<string>;
  native: boolean;
  onRefresh: () => void;
  onAdd: (input: VocabularyInput) => Promise<boolean>;
  onUpdate: (id: string, input: VocabularyInput) => Promise<boolean>;
  onRemove: (id: string) => Promise<boolean>;
}

const filters = [
  "All",
  "Names",
  "Technical",
  "Companies",
  "Replacements",
] as const;

const categoryLabels: Record<VocabularyCategory, string> = {
  name: "Name",
  technical: "Technical",
  company: "Company",
  replacement: "Replacement",
};

const filterCategories: Partial<
  Record<(typeof filters)[number], VocabularyCategory>
> = {
  Names: "name",
  Technical: "technical",
  Companies: "company",
  Replacements: "replacement",
};

const languageOptions = [
  { label: "Any language", tag: null },
  { label: "English", tag: "en" },
  { label: "Hindi", tag: "hi" },
  { label: "Bengali", tag: "bn" },
  { label: "Spanish", tag: "es" },
  { label: "French", tag: "fr" },
] as const;

function languageLabel(tag: string | null) {
  if (!tag) return "Any language";
  const known = languageOptions.find((item) => item.tag === tag);
  if (known) return known.label;
  try {
    return (
      new Intl.DisplayNames(undefined, { type: "language" }).of(tag) ?? tag
    );
  } catch {
    return tag;
  }
}

export function VocabularyView({
  vocabulary,
  loading,
  error,
  pendingIds,
  native,
  onRefresh,
  onAdd,
  onUpdate,
  onRemove,
}: VocabularyViewProps) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<(typeof filters)[number]>("All");
  const [dialogEntry, setDialogEntry] = useState<
    VocabularyEntryDto | "new" | null
  >(null);
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const filteredEntries = useMemo(() => {
    const category = filterCategories[filter];
    const normalizedQuery = query.trim().toLocaleLowerCase();
    return vocabulary.filter((entry) => {
      const matchesQuery = `${entry.phrase} ${entry.spokenForm ?? ""}`
        .toLocaleLowerCase()
        .includes(normalizedQuery);
      return matchesQuery && (!category || entry.category === category);
    });
  }, [filter, query, vocabulary]);
  const enabledCount = vocabulary.filter((entry) => entry.enabled).length;
  const languageCount = new Set(
    vocabulary.map((entry) => entry.languageTag).filter(Boolean),
  ).size;

  const remove = async (id: string) => {
    if (confirmDeleteId !== id) {
      setConfirmDeleteId(id);
      return;
    }
    setConfirmDeleteId(null);
    await onRemove(id);
  };

  return (
    <div className="view view--vocabulary">
      <PageHeader
        eyebrow="NAMES & TERMS"
        title="Vocabulary"
        description="The local prompt holds up to 64 enabled phrases and 1 KB. Pause a phrase to free space; notes and categories are for organization."
        actions={
          <button
            type="button"
            className="button button--primary"
            onClick={() => setDialogEntry("new")}
            disabled={!native || loading || pendingIds.has("create")}
          >
            <Plus size={16} /> Add phrase
          </button>
        }
      />

      {error && (
        <div className="engine-inline-error" role="alert">
          <strong>Vocabulary wasn’t saved</strong>
          <span>{error}</span>
          <button type="button" className="text-button" onClick={onRefresh}>
            Try again
          </button>
        </div>
      )}

      {!native && (
        <div className="engine-inline-error" role="status">
          <strong>Development app required</strong>
          <span>
            Vocabulary is stored by the Tauri app, not this browser preview.
          </span>
        </div>
      )}

      <section className="vocabulary-summary" aria-busy={loading}>
        <div className="vocabulary-summary__icon">
          <BookOpenText size={22} />
        </div>
        <div>
          <strong>
            {loading && vocabulary.length === 0
              ? "Loading your phrases…"
              : `${vocabulary.length} saved ${vocabulary.length === 1 ? "phrase" : "phrases"}`}
          </strong>
          <span>Every enabled written phrase is supplied as a local hint</span>
        </div>
        <div className="vocabulary-summary__metric">
          <strong>{enabledCount}/64</strong>
          <span>enabled prompt phrases</span>
        </div>
        <div className="vocabulary-summary__metric">
          <strong>{languageCount || "Any"}</strong>
          <span>{languageCount === 1 ? "language" : "languages"}</span>
        </div>
        <span className="sync-badge">
          <Check size={13} /> On this Mac
        </span>
      </section>

      <div className="vocabulary-toolbar">
        <label className="search-field">
          <Search size={16} />
          <input
            value={query}
            onChange={(event) => setQuery(event.currentTarget.value)}
            placeholder="Search your vocabulary…"
          />
          {query && (
            <button
              type="button"
              onClick={() => setQuery("")}
              aria-label="Clear search"
            >
              <X size={14} />
            </button>
          )}
        </label>
        <div
          className="filter-tabs"
          role="tablist"
          aria-label="Vocabulary categories"
        >
          {filters.map((item) => (
            <button
              type="button"
              role="tab"
              aria-selected={filter === item}
              className={filter === item ? "active" : ""}
              key={item}
              onClick={() => setFilter(item)}
            >
              {item}
            </button>
          ))}
        </div>
        <button
          type="button"
          className="icon-button"
          aria-label="Refresh vocabulary"
          onClick={onRefresh}
          disabled={!native || loading}
        >
          <RefreshCw size={15} />
        </button>
      </div>

      <section className="panel vocabulary-table-wrap">
        <div
          className="vocabulary-table"
          role="table"
          aria-label="Saved vocabulary"
          aria-busy={loading}
        >
          <div className="vocabulary-table__header" role="row">
            <span role="columnheader">Written as</span>
            <span role="columnheader">Pronunciation note</span>
            <span role="columnheader">Category</span>
            <span role="columnheader">Language</span>
            <span role="columnheader">
              <span className="sr-only">Actions</span>
            </span>
          </div>
          {filteredEntries.map((entry) => {
            const pending = pendingIds.has(entry.id);
            const deleting = confirmDeleteId === entry.id;
            return (
              <div
                className={`vocabulary-table__row ${entry.enabled ? "" : "vocabulary-table__row--disabled"}`}
                role="row"
                key={entry.id}
                aria-busy={pending}
              >
                <div role="cell">
                  <span className="phrase-avatar">
                    {entry.phrase[0]?.toUpperCase()}
                  </span>
                  <span className="phrase-title">
                    <strong>{entry.phrase}</strong>
                    {!entry.enabled && <small>Paused</small>}
                  </span>
                </div>
                <span role="cell" className="said-phrase">
                  {entry.spokenForm
                    ? `“${entry.spokenForm}”`
                    : "Same as written"}
                </span>
                <span role="cell">
                  <span
                    className={`category-badge category-badge--${entry.category}`}
                  >
                    {categoryLabels[entry.category]}
                  </span>
                </span>
                <span role="cell" className="language-cell">
                  <Languages size={14} /> {languageLabel(entry.languageTag)}
                </span>
                <div role="cell" className="table-actions">
                  <button
                    type="button"
                    className="icon-button icon-button--subtle"
                    aria-label={`Edit ${entry.phrase}`}
                    onClick={() => setDialogEntry(entry)}
                    disabled={pending}
                  >
                    <Pencil size={15} />
                  </button>
                  <button
                    type="button"
                    className="icon-button icon-button--subtle icon-button--danger"
                    onClick={() => void remove(entry.id)}
                    aria-label={
                      deleting
                        ? `Confirm delete ${entry.phrase}`
                        : `Delete ${entry.phrase}`
                    }
                    disabled={pending}
                  >
                    {deleting ? <Check size={15} /> : <Trash2 size={15} />}
                  </button>
                </div>
              </div>
            );
          })}
        </div>
        {filteredEntries.length === 0 && (
          <div className="empty-state">
            <Search size={22} />
            <strong>
              {loading
                ? "Loading vocabulary…"
                : vocabulary.length === 0
                  ? "No saved phrases yet"
                  : "No phrases found"}
            </strong>
            <span>
              {vocabulary.length === 0
                ? "Add a name or term you want Spick to spell correctly."
                : "Try another search or category."}
            </span>
          </div>
        )}
      </section>

      <section className="vocabulary-tip">
        <div>
          <Lightbulb size={18} />
        </div>
        <p>
          <strong>Keep hints short and specific.</strong>
          <span>
            The current local engine uses only the written phrase as a prompt
            hint. Pronunciation notes, language, and categories are stored for
            future adapters and organization.
          </span>
        </p>
      </section>

      {dialogEntry && (
        <PhraseDialog
          entry={dialogEntry === "new" ? null : dialogEntry}
          pending={
            dialogEntry === "new"
              ? pendingIds.has("create")
              : pendingIds.has(dialogEntry.id)
          }
          onClose={() => setDialogEntry(null)}
          onSave={async (input) => {
            const saved =
              dialogEntry === "new"
                ? await onAdd(input)
                : await onUpdate(dialogEntry.id, input);
            if (saved) setDialogEntry(null);
          }}
        />
      )}
    </div>
  );
}

interface PhraseDialogProps {
  entry: VocabularyEntryDto | null;
  pending: boolean;
  onClose: () => void;
  onSave: (input: VocabularyInput) => Promise<void>;
}

function PhraseDialog({ entry, pending, onClose, onSave }: PhraseDialogProps) {
  const customLanguageOption =
    entry?.languageTag &&
    !languageOptions.some((item) => item.tag === entry.languageTag)
      ? {
          label: `${languageLabel(entry.languageTag)} (${entry.languageTag})`,
          tag: entry.languageTag,
        }
      : null;
  const selectableLanguages = customLanguageOption
    ? [...languageOptions, customLanguageOption]
    : [...languageOptions];
  const [phrase, setPhrase] = useState(entry?.phrase ?? "");
  const [spokenForm, setSpokenForm] = useState(entry?.spokenForm ?? "");
  const [category, setCategory] = useState<VocabularyCategory>(
    entry?.category ?? "technical",
  );
  const [language, setLanguage] = useState(
    customLanguageOption?.label ?? languageLabel(entry?.languageTag ?? null),
  );
  const [enabled, setEnabled] = useState(entry?.enabled ?? true);

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!phrase.trim() || pending) return;
    const languageTag =
      selectableLanguages.find((item) => item.label === language)?.tag ?? null;
    void onSave({
      phrase: phrase.trim(),
      spokenForm: spokenForm.trim() || null,
      category,
      languageTag,
      enabled,
    });
  };

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !pending) onClose();
      }}
    >
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="phrase-dialog-title"
      >
        <header className="dialog__header">
          <span className="dialog__icon">
            <PenLine size={19} />
          </span>
          <div>
            <h2 id="phrase-dialog-title">
              {entry ? "Edit phrase" : "Add a phrase"}
            </h2>
            <p>The written phrase becomes a bounded local prompt hint.</p>
          </div>
          <button
            type="button"
            className="icon-button"
            onClick={onClose}
            aria-label="Close"
            disabled={pending}
          >
            <X size={18} />
          </button>
        </header>
        <form onSubmit={submit}>
          <label className="field">
            <span className="field__label">Write it like this</span>
            <input
              autoFocus
              value={phrase}
              onChange={(event) => setPhrase(event.currentTarget.value)}
              placeholder="e.g. WebRTC"
              disabled={pending}
            />
          </label>
          <label className="field">
            <span className="field__label">
              Pronunciation note (saved for future adapters)
            </span>
            <input
              value={spokenForm}
              onChange={(event) => setSpokenForm(event.currentTarget.value)}
              placeholder="e.g. web R T C"
              disabled={pending}
            />
            <span className="field__hint">
              Optional reference note. It does not rewrite transcripts yet.
            </span>
          </label>
          <div className="dialog__field-grid">
            <SelectField
              label="Category"
              value={categoryLabels[category]}
              disabled={pending}
              onChange={(value) => {
                const match = Object.entries(categoryLabels).find(
                  ([, label]) => label === value,
                );
                if (match) setCategory(match[0] as VocabularyCategory);
              }}
              options={Object.values(categoryLabels)}
            />
            <SelectField
              label="Language"
              value={language}
              disabled={pending}
              onChange={setLanguage}
              options={selectableLanguages.map((item) => item.label)}
            />
          </div>
          <div className="dialog__preview vocabulary-enabled-row">
            <Info size={15} />
            <span>
              {enabled
                ? "Uses the 64 phrase / 1 KB local prompt capacity"
                : "Saved, but not sent to the engine"}
            </span>
            <Toggle
              label="Use this hint"
              checked={enabled}
              onChange={setEnabled}
            />
          </div>
          <footer className="dialog__footer">
            <button
              type="button"
              className="button button--secondary"
              onClick={onClose}
              disabled={pending}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="button button--primary"
              disabled={!phrase.trim() || pending}
            >
              {pending ? (
                "Saving…"
              ) : (
                <>
                  {entry ? <Check size={15} /> : <Plus size={15} />}
                  {entry ? "Save changes" : "Add phrase"}
                </>
              )}
            </button>
          </footer>
        </form>
      </div>
    </div>
  );
}
