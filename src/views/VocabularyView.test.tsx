import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  VocabularyEntryDto,
  VocabularyInput,
} from "../lib/nativeLocalData";
import { VocabularyView } from "./VocabularyView";

const entry: VocabularyEntryDto = {
  id: "vocab-1",
  phrase: "WebRTC",
  spokenForm: "web R T C",
  category: "technical",
  languageTag: "en",
  enabled: true,
  createdAtMs: 1,
  updatedAtMs: 1,
};

const baseProps = {
  error: null,
  loading: false,
  native: true,
  onAdd: vi.fn(async () => true),
  onRefresh: vi.fn(),
  onRemove: vi.fn(async () => true),
  onUpdate: vi.fn(async () => true),
  pendingIds: new Set<string>(),
  vocabulary: [entry],
};

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("VocabularyView", () => {
  it("discloses the native prompt capacity while treating saved enabled entries as active", () => {
    render(<VocabularyView {...baseProps} />);

    expect(screen.getByText("1/64")).toBeInTheDocument();
    expect(
      screen.getByText(/Every enabled written phrase is supplied/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/up to 64 enabled phrases and 1 KB/),
    ).toBeInTheDocument();
  });

  it("shows honest loading, empty, and error states", () => {
    render(
      <VocabularyView
        {...baseProps}
        error="database is busy"
        loading
        vocabulary={[]}
      />,
    );

    expect(screen.getByText("Loading your phrases…")).toBeInTheDocument();
    expect(screen.getByText("Loading vocabulary…")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("database is busy");
    expect(screen.queryByText(/example phrase/i)).not.toBeInTheDocument();
  });

  it("keeps the add dialog open until native creation acknowledges", async () => {
    const onAdd = vi.fn(async () => false);
    render(<VocabularyView {...baseProps} onAdd={onAdd} vocabulary={[]} />);

    fireEvent.click(screen.getByRole("button", { name: "Add phrase" }));
    fireEvent.change(screen.getByLabelText("Write it like this"), {
      target: { value: "Tauri" },
    });
    fireEvent.change(screen.getByLabelText(/^Pronunciation note/), {
      target: { value: "tow ree" },
    });
    fireEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Add phrase",
      }),
    );

    await waitFor(() => expect(onAdd).toHaveBeenCalledOnce());
    expect(onAdd).toHaveBeenCalledWith({
      phrase: "Tauri",
      spokenForm: "tow ree",
      category: "technical",
      languageTag: null,
      enabled: true,
    });
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });

  it("edits a phrase through the acknowledged update callback", async () => {
    const onUpdate = vi.fn(async () => true);
    render(<VocabularyView {...baseProps} onUpdate={onUpdate} />);

    fireEvent.click(screen.getByRole("button", { name: "Edit WebRTC" }));
    fireEvent.change(screen.getByLabelText("Write it like this"), {
      target: { value: "Web RTC" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(onUpdate).toHaveBeenCalledOnce());
    expect(onUpdate).toHaveBeenCalledWith(
      entry.id,
      expect.objectContaining({ phrase: "Web RTC", enabled: true }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
  });

  it("preserves a stored language tag that is outside the shortcut list", async () => {
    const onUpdate = vi
      .fn<(id: string, input: VocabularyInput) => Promise<boolean>>()
      .mockResolvedValue(true);
    render(
      <VocabularyView
        {...baseProps}
        onUpdate={onUpdate}
        vocabulary={[{ ...entry, languageTag: "ga-IE" }]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Edit WebRTC" }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(onUpdate).toHaveBeenCalledOnce());
    expect(onUpdate.mock.calls[0]?.[1]).toMatchObject({ languageTag: "ga-IE" });
  });

  it("requires a second click before deleting a phrase", async () => {
    const onRemove = vi.fn(async () => true);
    render(<VocabularyView {...baseProps} onRemove={onRemove} />);

    fireEvent.click(screen.getByRole("button", { name: "Delete WebRTC" }));
    expect(onRemove).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("button", { name: "Confirm delete WebRTC" }),
    );

    await waitFor(() => expect(onRemove).toHaveBeenCalledWith(entry.id));
  });
});
