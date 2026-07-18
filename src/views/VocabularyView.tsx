import { useMemo, useState } from "react";
import {
  BookOpenText,
  Check,
  Languages,
  Lightbulb,
  MoreHorizontal,
  Plus,
  Search,
  Sparkles,
  Trash2,
  Upload,
  WandSparkles,
  X,
} from "lucide-react";
import type { VocabularyEntry } from "../types";
import { PageHeader, SelectField } from "../components/Ui";

interface VocabularyViewProps {
  vocabulary: VocabularyEntry[];
  onAdd: (entry: VocabularyEntry) => void;
  onRemove: (id: string) => void;
}

const filters = [
  "All",
  "Names",
  "Technical",
  "Companies",
  "Replacements",
] as const;

export function VocabularyView({
  vocabulary,
  onAdd,
  onRemove,
}: VocabularyViewProps) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<(typeof filters)[number]>("All");
  const [showAdd, setShowAdd] = useState(false);

  const filteredEntries = useMemo(() => {
    const category =
      filter === "Companies"
        ? "Company"
        : filter === "Names"
          ? "Name"
          : filter === "Replacements"
            ? "Replacement"
            : filter;
    return vocabulary.filter((entry) => {
      const matchesQuery = `${entry.phrase} ${entry.soundsLike ?? ""}`
        .toLowerCase()
        .includes(query.toLowerCase());
      const matchesFilter =
        filter === "All" || entry.category === category.replace(/s$/, "");
      return matchesQuery && matchesFilter;
    });
  }, [filter, query, vocabulary]);

  return (
    <div className="view view--vocabulary">
      <PageHeader
        eyebrow="Personalization"
        title="Vocabulary"
        description="Preview a shared vocabulary for names and technical terms. Engine adapters are not connected yet."
        actions={
          <button
            type="button"
            className="button button--primary"
            onClick={() => setShowAdd(true)}
          >
            <Plus size={16} /> Add phrase
          </button>
        }
      />

      <section className="vocabulary-summary">
        <div className="vocabulary-summary__icon">
          <BookOpenText size={22} />
        </div>
        <div>
          <strong>{vocabulary.length} sample phrases</strong>
          <span>Editable in this preview; not persisted or applied yet</span>
        </div>
        <div className="vocabulary-summary__metric">
          <strong>Searchable</strong>
          <span>sample library</span>
        </div>
        <div className="vocabulary-summary__metric">
          <strong>Local</strong>
          <span>preview state</span>
        </div>
        <span className="sync-badge">
          <Check size={13} /> Preview data
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
        <button type="button" className="button button--secondary" disabled>
          <Upload size={15} /> Import planned
        </button>
      </div>

      <section className="panel vocabulary-table-wrap">
        <div
          className="vocabulary-table"
          role="table"
          aria-label="Saved vocabulary"
        >
          <div className="vocabulary-table__header" role="row">
            <span role="columnheader">Written as</span>
            <span role="columnheader">When you say</span>
            <span role="columnheader">Category</span>
            <span role="columnheader">Language</span>
            <span role="columnheader">
              <span className="sr-only">Actions</span>
            </span>
          </div>
          {filteredEntries.map((entry) => (
            <div className="vocabulary-table__row" role="row" key={entry.id}>
              <div role="cell">
                <span className="phrase-avatar">
                  {entry.phrase[0]?.toUpperCase()}
                </span>
                <strong>{entry.phrase}</strong>
              </div>
              <span role="cell" className="said-phrase">
                {entry.soundsLike ? `“${entry.soundsLike}”` : "Same as written"}
              </span>
              <span role="cell">
                <span
                  className={`category-badge category-badge--${entry.category.toLowerCase()}`}
                >
                  {entry.category}
                </span>
              </span>
              <span role="cell" className="language-cell">
                <Languages size={14} /> {entry.language}
              </span>
              <div role="cell" className="table-actions">
                <button
                  type="button"
                  className="icon-button icon-button--subtle"
                  aria-label={`More options for ${entry.phrase}`}
                >
                  <MoreHorizontal size={16} />
                </button>
                <button
                  type="button"
                  className="icon-button icon-button--subtle icon-button--danger"
                  onClick={() => onRemove(entry.id)}
                  aria-label={`Delete ${entry.phrase}`}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            </div>
          ))}
        </div>
        {filteredEntries.length === 0 && (
          <div className="empty-state">
            <Search size={22} />
            <strong>No phrases found</strong>
            <span>Try another search or add a new phrase.</span>
          </div>
        )}
      </section>

      <section className="vocabulary-tip">
        <div>
          <Lightbulb size={18} />
        </div>
        <p>
          <strong>Make recognition even better</strong>
          <span>
            Add pronunciation hints for unusual names, acronyms, and product
            terms. Spick applies them before cleanup.
          </span>
        </p>
        <button type="button" className="text-button">
          See tips
        </button>
      </section>

      {showAdd && (
        <AddPhraseDialog
          onClose={() => setShowAdd(false)}
          onAdd={(entry) => {
            onAdd(entry);
            setShowAdd(false);
          }}
        />
      )}
    </div>
  );
}

interface AddPhraseDialogProps {
  onClose: () => void;
  onAdd: (entry: VocabularyEntry) => void;
}

function AddPhraseDialog({ onClose, onAdd }: AddPhraseDialogProps) {
  const [phrase, setPhrase] = useState("");
  const [soundsLike, setSoundsLike] = useState("");
  const [category, setCategory] =
    useState<VocabularyEntry["category"]>("Technical");
  const [language, setLanguage] = useState("English");

  const submit = (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (!phrase.trim()) return;
    onAdd({
      id: crypto.randomUUID(),
      phrase: phrase.trim(),
      soundsLike: soundsLike.trim() || undefined,
      category,
      language,
    });
  };

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="add-phrase-title"
      >
        <header className="dialog__header">
          <span className="dialog__icon">
            <WandSparkles size={19} />
          </span>
          <div>
            <h2 id="add-phrase-title">Add a phrase</h2>
            <p>Help Spick write it exactly the way you want.</p>
          </div>
          <button
            type="button"
            className="icon-button"
            onClick={onClose}
            aria-label="Close"
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
            />
          </label>
          <label className="field">
            <span className="field__label">When I say</span>
            <input
              value={soundsLike}
              onChange={(event) => setSoundsLike(event.currentTarget.value)}
              placeholder="e.g. web R T C"
            />
            <span className="field__hint">
              Optional pronunciation or replacement phrase
            </span>
          </label>
          <div className="dialog__field-grid">
            <SelectField
              label="Category"
              value={category}
              onChange={(value) =>
                setCategory(value as VocabularyEntry["category"])
              }
              options={["Name", "Technical", "Company", "Replacement"]}
            />
            <SelectField
              label="Language"
              value={language}
              onChange={setLanguage}
              options={["English", "Hindi", "Bengali", "Spanish", "French"]}
            />
          </div>
          <div className="dialog__preview">
            <Sparkles size={15} />
            <span>
              Spick will apply this phrase locally before inserting your text.
            </span>
          </div>
          <footer className="dialog__footer">
            <button
              type="button"
              className="button button--secondary"
              onClick={onClose}
            >
              Cancel
            </button>
            <button
              type="submit"
              className="button button--primary"
              disabled={!phrase.trim()}
            >
              <Plus size={15} /> Add phrase
            </button>
          </footer>
        </form>
      </div>
    </div>
  );
}
