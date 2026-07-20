import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { CloudProviderStatus } from "../lib/nativeCloud";
import type { Engine } from "../types";
import { EnginesView } from "./EnginesView";

const localEngine: Engine = {
  id: "whisper-small",
  name: "Whisper Small",
  provider: "whisper.cpp",
  description: "A local multilingual model.",
  kind: "local",
  status: "ready",
  languageSupport: "Multilingual model",
  size: "190 MB",
  performance: "Ready on this Mac",
  origin: "curated",
};

const cloudProviders: CloudProviderStatus[] = [
  {
    provider: "openAi",
    providerName: "OpenAI",
    engineId: "openai-gpt-4o-transcribe",
    modelName: "GPT-4o Transcribe",
    configured: true,
    selected: false,
    experimental: false,
    description: "Dedicated multilingual speech-to-text.",
    languageSupport: "Multilingual batch transcription",
    cleanupBehavior: "Spick cleanup runs after transcription",
  },
  {
    provider: "xAi",
    providerName: "xAI",
    engineId: "xai-speech-to-text",
    modelName: "xAI Speech to Text",
    configured: false,
    selected: false,
    experimental: false,
    description: "Dedicated speech-to-text.",
    languageSupport: "Multilingual batch transcription",
    cleanupBehavior: "Filler handling follows your cleanup setting",
  },
  {
    provider: "gemini",
    providerName: "Google",
    engineId: "gemini-3-5-flash",
    modelName: "Gemini 3.5 Flash",
    configured: false,
    selected: false,
    experimental: true,
    description: "General audio understanding.",
    languageSupport: "Model-dependent multilingual audio",
    cleanupBehavior: "General audio response",
  },
];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((onResolve) => {
    resolve = onResolve;
  });
  return { promise, resolve };
}

function props(
  overrides: Partial<React.ComponentProps<typeof EnginesView>> = {},
): React.ComponentProps<typeof EnginesView> {
  return {
    engines: [localEngine],
    downloads: {},
    native: true,
    cancellingModelIds: new Set<string>(),
    pendingModelId: null,
    importPending: false,
    localLoading: false,
    cloudProviders,
    cloudLoading: false,
    cloudPending: null,
    cloudFallbackEnabled: false,
    onActivate: vi.fn(),
    onCancelInstall: vi.fn(),
    onInstall: vi.fn(),
    onImport: vi.fn(),
    onRemove: vi.fn(),
    onLocalRefresh: vi.fn(),
    onCloudRefresh: vi.fn(),
    onCloudConfigure: vi.fn(async () => true),
    onCloudDelete: vi.fn(async () => true),
    onCloudActivate: vi.fn(async () => true),
    ...overrides,
  };
}

function openCloudTab() {
  fireEvent.click(screen.getByRole("tab", { name: /Cloud providers/i }));
}

function providerCard(name: string) {
  const heading = screen.getByText(name, { selector: "strong" });
  const card = heading.closest("article");
  if (!card) throw new Error(`Provider card for ${name} was not rendered`);
  return within(card);
}

describe("EnginesView cloud providers", () => {
  afterEach(cleanup);

  it("finishes first-run setup only after the selected engine is ready", () => {
    const onFinishSetup = vi.fn();
    const { rerender } = render(
      <EnginesView
        {...props({
          setupRequired: true,
          setupReady: false,
          onFinishSetup,
        })}
      />,
    );

    expect(
      screen.getByRole("button", { name: "Choose an engine first" }),
    ).toBeDisabled();

    rerender(
      <EnginesView
        {...props({
          setupRequired: true,
          setupReady: true,
          onFinishSetup,
        })}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Finish setup" }));
    expect(onFinishSetup).toHaveBeenCalledOnce();
  });

  it("shows an honest, disabled cloud preview in a browser", () => {
    render(
      <EnginesView
        {...props({ native: false, engines: [], cloudProviders: [] })}
      />,
    );
    openCloudTab();

    expect(screen.getByText("GPT-4o Transcribe")).toBeInTheDocument();
    expect(screen.getByText("xAI Speech to Text")).toBeInTheDocument();
    expect(screen.getByText("Gemini 3.5 Flash")).toBeInTheDocument();
    expect(screen.getByText(/browser preview cannot access/i)).toBeVisible();
    expect(screen.queryByRole("textbox", { name: /API key/i })).toBeNull();
    expect(
      providerCard("OpenAI").getByRole("button", { name: /^Add key$/i }),
    ).toBeDisabled();
  });

  it("shows native loading and retry states without placeholder providers", () => {
    const onCloudRefresh = vi.fn();
    const { rerender } = render(
      <EnginesView
        {...props({
          cloudProviders: [],
          cloudLoading: true,
          onCloudRefresh,
        })}
      />,
    );
    openCloudTab();
    expect(screen.getByText("Loading cloud providers…")).toBeVisible();

    rerender(
      <EnginesView
        {...props({
          cloudProviders: [],
          cloudError: "Native status unavailable",
          onCloudRefresh,
        })}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onCloudRefresh).toHaveBeenCalledOnce();
  });

  it("clears a credential from the field before its save settles", async () => {
    const save = deferred<boolean>();
    const onCloudConfigure = vi.fn(() => save.promise);
    render(<EnginesView {...props({ onCloudConfigure })} />);
    openCloudTab();
    const openAi = providerCard("OpenAI");

    fireEvent.click(openAi.getByRole("button", { name: "Replace key" }));
    const input = openAi.getByLabelText("OpenAI API key");
    expect(input).toHaveAttribute("type", "password");
    fireEvent.change(input, { target: { value: "  short-lived-value  " } });
    fireEvent.click(openAi.getByRole("button", { name: "Replace key" }));

    expect(onCloudConfigure).toHaveBeenCalledWith(
      "openAi",
      "short-lived-value",
    );
    expect(openAi.getByLabelText("OpenAI API key")).toHaveValue("");
    expect(screen.queryByDisplayValue("short-lived-value")).toBeNull();
    expect(document.body.textContent).not.toContain("short-lived-value");

    await act(async () => {
      save.resolve(true);
      await save.promise;
    });
    await waitFor(() =>
      expect(openAi.queryByLabelText("OpenAI API key")).toBeNull(),
    );
  });

  it("keeps a failed credential editor open and blank", async () => {
    const onCloudConfigure = vi.fn(async () => false);
    render(<EnginesView {...props({ onCloudConfigure })} />);
    openCloudTab();
    const xAi = providerCard("xAI");

    fireEvent.click(xAi.getByRole("button", { name: "Add key" }));
    fireEvent.change(xAi.getByLabelText("xAI API key"), {
      target: { value: "never-render-after-save" },
    });
    fireEvent.click(xAi.getByRole("button", { name: "Save key" }));

    await waitFor(() => expect(onCloudConfigure).toHaveBeenCalledOnce());
    expect(xAi.getByLabelText("xAI API key")).toHaveValue("");
    expect(document.body.textContent).not.toContain("never-render-after-save");
  });

  it("requires delete confirmation and prevents deleting the active key", async () => {
    const onCloudDelete = vi.fn(async () => true);
    const { rerender } = render(<EnginesView {...props({ onCloudDelete })} />);
    openCloudTab();
    const openAi = providerCard("OpenAI");

    fireEvent.click(
      openAi.getByRole("button", { name: "Remove OpenAI API key" }),
    );
    expect(onCloudDelete).not.toHaveBeenCalled();
    fireEvent.click(
      openAi.getByRole("button", {
        name: "Confirm remove OpenAI API key",
      }),
    );
    await waitFor(() => expect(onCloudDelete).toHaveBeenCalledWith("openAi"));

    rerender(
      <EnginesView
        {...props({
          cloudProviders: [
            { ...cloudProviders[0]!, selected: true },
            ...cloudProviders.slice(1),
          ],
          onCloudDelete,
        })}
      />,
    );
    expect(
      providerCard("OpenAI").getByRole("button", {
        name: "Remove OpenAI API key",
      }),
    ).toBeDisabled();
    expect(
      screen.getByText("Select another engine before removing this key."),
    ).toBeVisible();
  });

  it("activates configured providers only and reports cloud/local selection", () => {
    const onCloudActivate = vi.fn(async () => true);
    const { rerender } = render(
      <EnginesView {...props({ onCloudActivate })} />,
    );
    openCloudTab();

    fireEvent.click(
      providerCard("OpenAI").getByRole("button", { name: "Use provider" }),
    );
    expect(onCloudActivate).toHaveBeenCalledWith("openAi");
    expect(
      providerCard("xAI").getByRole("button", { name: "Add key first" }),
    ).toBeDisabled();

    rerender(
      <EnginesView
        {...props({
          engines: [{ ...localEngine, status: "active" }],
          cloudProviders: [],
          onCloudActivate,
        })}
      />,
    );
    expect(
      screen.getByText("Whisper Small", { selector: "strong" }),
    ).toBeVisible();
  });

  it("locks local and cloud engine actions across in-flight selections", () => {
    const { rerender } = render(
      <EnginesView {...props({ pendingModelId: localEngine.id })} />,
    );
    openCloudTab();
    expect(
      providerCard("OpenAI").getByRole("button", { name: "Use provider" }),
    ).toBeDisabled();

    rerender(
      <EnginesView
        {...props({
          cloudPending: { provider: "openAi", action: "activate" },
        })}
      />,
    );
    fireEvent.click(screen.getByRole("tab", { name: /On this Mac/i }));
    expect(screen.getByRole("button", { name: "Use model" })).toBeDisabled();
  });

  it("qualifies local privacy copy when cloud fallback is enabled", () => {
    render(<EnginesView {...props({ cloudFallbackEnabled: true })} />);

    expect(screen.getByText("Local first · fallback on")).toBeVisible();
    expect(screen.queryByText("Stays on this Mac")).toBeNull();
    expect(
      screen.getByText(/first configured, language-compatible cloud provider/i),
    ).toBeVisible();
  });

  it("loads an empty native catalog honestly and lets the user retry", () => {
    const onLocalRefresh = vi.fn();
    const { rerender } = render(
      <EnginesView
        {...props({
          engines: [],
          localLoading: true,
          onLocalRefresh,
        })}
      />,
    );

    expect(screen.getByText("Loading local models…")).toBeVisible();
    expect(screen.queryByText(/Metal ready/i)).toBeNull();
    expect(screen.queryByText(/haven’t checked this Mac/i)).toBeNull();

    rerender(
      <EnginesView
        {...props({
          engines: [],
          error: "catalog unavailable",
          onLocalRefresh,
        })}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(onLocalRefresh).toHaveBeenCalledOnce();
  });

  it("imports from the native picker and removes an invalid imported model", () => {
    const onImport = vi.fn();
    const onRemove = vi.fn();
    const imported = {
      ...localEngine,
      id: "whisper-imported-digest",
      name: "My meeting model",
      origin: "imported" as const,
      provider: "whisper.cpp · imported",
      status: "invalid" as const,
    };
    const missingImported = {
      ...imported,
      id: "whisper-imported-missing",
      name: "Missing imported model",
      status: "available" as const,
    };
    render(
      <EnginesView
        {...props({
          engines: [imported, missingImported],
          onImport,
          onRemove,
        })}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Import model" }));
    expect(onImport).toHaveBeenCalledOnce();
    fireEvent.click(
      screen.getByRole("button", { name: "Remove broken model" }),
    );
    expect(onRemove).toHaveBeenCalledWith(imported.id);
    fireEvent.click(screen.getByRole("button", { name: "Remove model" }));
    expect(onRemove).toHaveBeenCalledWith(missingImported.id);
    expect(
      screen.queryByRole("button", { name: /Download again/i }),
    ).toBeNull();
    expect(screen.queryByRole("button", { name: /^Download$/i })).toBeNull();
    expect(screen.getByText(/GGML \.bin file you trust/i)).toBeVisible();
    expect(screen.getByText(/not its safety or license/i)).toBeVisible();
    expect(screen.getByText(/GGUF and general LLM files/i)).toBeVisible();
  });
});
