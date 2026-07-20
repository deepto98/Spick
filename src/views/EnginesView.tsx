import { useMemo, useState } from "react";
import {
  Check,
  CircleDashed,
  Cloud,
  Cpu,
  Download,
  Eye,
  EyeOff,
  FlaskConical,
  Gauge,
  Globe2,
  HardDrive,
  KeyRound,
  Laptop,
  LockKeyhole,
  RefreshCw,
  ShieldCheck,
  Trash2,
  Wifi,
  X,
} from "lucide-react";
import type { Engine, EngineKind } from "../types";
import type { CloudProviderId, CloudProviderStatus } from "../lib/nativeCloud";
import type { CloudProviderPendingAction } from "../hooks/useCloudProviders";
import type { ModelDownloadProgress } from "../lib/nativeModels";
import { PageHeader } from "../components/Ui";

interface EnginesViewProps {
  engines: Engine[];
  downloads: Record<string, ModelDownloadProgress>;
  native: boolean;
  cancellingModelIds: ReadonlySet<string>;
  pendingModelId: string | null;
  error?: string;
  cloudProviders: CloudProviderStatus[];
  cloudLoading: boolean;
  cloudPending: CloudProviderPendingAction | null;
  cloudError?: string;
  cloudFallbackEnabled: boolean;
  onActivate: (id: string) => void;
  onCancelInstall: (id: string) => void;
  onInstall: (id: string) => void;
  onRemove: (id: string) => void;
  onCloudRefresh: () => void;
  onCloudConfigure: (
    provider: CloudProviderId,
    apiKey: string,
  ) => Promise<boolean>;
  onCloudDelete: (provider: CloudProviderId) => Promise<boolean>;
  onCloudActivate: (provider: CloudProviderId) => Promise<boolean>;
}

const providerMeta: Record<
  CloudProviderId,
  { initial: string; className: string }
> = {
  openAi: { initial: "O", className: "provider-logo--openai" },
  gemini: { initial: "G", className: "provider-logo--google" },
  xAi: { initial: "x", className: "provider-logo--xai" },
};

const browserCloudProviders: CloudProviderStatus[] = [
  {
    provider: "openAi",
    providerName: "OpenAI",
    engineId: "openai-gpt-4o-transcribe",
    modelName: "GPT-4o Transcribe",
    configured: false,
    selected: false,
    experimental: false,
    description:
      "Dedicated multilingual speech-to-text for completed recordings.",
    languageSupport: "Multilingual batch transcription",
    cleanupBehavior: "Spick cleanup runs after transcription",
  },
  {
    provider: "xAi",
    providerName: "xAI",
    engineId: "xai-grok-transcribe",
    modelName: "xAI Speech to Text",
    configured: false,
    selected: false,
    experimental: false,
    description: "Dedicated speech-to-text for completed recordings.",
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
    description:
      "Experimental general audio understanding, not a dedicated speech-to-text endpoint.",
    languageSupport: "Model-dependent multilingual audio",
    cleanupBehavior: "General audio response; cleanup is experimental",
  },
];

export function EnginesView({
  engines,
  downloads,
  native,
  cancellingModelIds,
  pendingModelId,
  error,
  cloudProviders,
  cloudLoading,
  cloudPending,
  cloudError,
  cloudFallbackEnabled,
  onActivate,
  onCancelInstall,
  onInstall,
  onRemove,
  onCloudRefresh,
  onCloudConfigure,
  onCloudDelete,
  onCloudActivate,
}: EnginesViewProps) {
  const [kind, setKind] = useState<EngineKind>("local");
  const [editingProvider, setEditingProvider] =
    useState<CloudProviderId | null>(null);
  const [credentialDraft, setCredentialDraft] = useState("");
  const [showCredential, setShowCredential] = useState(false);
  const [confirmDeleteProvider, setConfirmDeleteProvider] =
    useState<CloudProviderId | null>(null);

  const visibleEngines = useMemo(
    () => engines.filter((engine) => engine.kind === "local"),
    [engines],
  );
  const shownCloudProviders = native ? cloudProviders : browserCloudProviders;
  const activeEngine = engines.find((engine) => engine.status === "active");
  const activeCloudProvider = cloudProviders.find(
    (provider) => provider.selected,
  );
  const localCount = engines.filter((engine) => engine.kind === "local").length;
  const cloudCount = shownCloudProviders.length;
  const installedCount = engines.filter(
    (engine) =>
      engine.kind === "local" &&
      (engine.status === "active" || engine.status === "ready"),
  ).length;

  const clearCredentialDraft = () => {
    setCredentialDraft("");
    setShowCredential(false);
  };

  const beginCredentialEdit = (provider: CloudProviderId) => {
    clearCredentialDraft();
    setConfirmDeleteProvider(null);
    setEditingProvider(provider);
  };

  const cancelCredentialEdit = () => {
    clearCredentialDraft();
    setEditingProvider(null);
  };

  const saveCredential = async (provider: CloudProviderId) => {
    const submittedCredential = credentialDraft.trim();
    // Remove the key from the DOM and React state before beginning IPC. The
    // local invocation variable exists only until this one save settles.
    clearCredentialDraft();
    let saved: boolean;
    try {
      saved = await onCloudConfigure(provider, submittedCredential);
    } catch {
      return;
    }
    if (saved) setEditingProvider(null);
  };

  const deleteCredential = async (provider: CloudProviderId) => {
    if (confirmDeleteProvider !== provider) {
      clearCredentialDraft();
      setEditingProvider(null);
      setConfirmDeleteProvider(provider);
      return;
    }
    if (await onCloudDelete(provider)) setConfirmDeleteProvider(null);
  };

  return (
    <div className="view view--engines">
      <PageHeader
        eyebrow="TRANSCRIPTION"
        title="Engines"
        description="Choose where your recording is transcribed. Local models stay on this computer."
        actions={
          <div className="engine-active-pill">
            {activeEngine || activeCloudProvider ? (
              <span className="status-dot" />
            ) : (
              <CircleDashed size={16} />
            )}
            <div>
              <small>Selected here</small>
              <strong>
                {activeCloudProvider?.modelName ??
                  activeEngine?.name ??
                  "None selected"}
              </strong>
            </div>
          </div>
        }
      />

      {error && (
        <div className="engine-inline-error" role="alert">
          <strong>Model action didn’t finish</strong>
          <span>{error}</span>
        </div>
      )}

      <div
        className="engine-kind-tabs"
        role="tablist"
        aria-label="Engine source"
      >
        <button
          type="button"
          role="tab"
          aria-selected={kind === "local"}
          className={kind === "local" ? "active" : ""}
          onClick={() => setKind("local")}
        >
          <Laptop size={17} />
          <span>
            <strong>On this Mac</strong>
            <small>Offline once installed</small>
          </span>
          <span className="tab-count">{localCount}</span>
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={kind === "cloud"}
          className={kind === "cloud" ? "active" : ""}
          onClick={() => setKind("cloud")}
        >
          <Cloud size={17} />
          <span>
            <strong>Cloud providers</strong>
            <small>Needs internet</small>
          </span>
          <span className="tab-count">{cloudCount}</span>
        </button>
      </div>

      {kind === "local" ? (
        <>
          <section className="hardware-banner">
            <div className="hardware-banner__icon">
              <Cpu size={21} />
            </div>
            <div className="hardware-banner__copy">
              <span className="hardware-banner__eyebrow">THIS MAC</span>
              <strong>We haven’t checked this Mac yet</strong>
              <p>Spick will test model speed here before suggesting one.</p>
            </div>
            <div className="hardware-banner__stats">
              <span>
                <Gauge size={14} /> <strong>Speed test</strong> after install
              </span>
              <span>
                <HardDrive size={14} /> <strong>{installedCount}</strong>{" "}
                {installedCount === 1 ? "model" : "models"} installed
              </span>
            </div>
            <span className="compatibility-badge">
              <Gauge size={13} /> {native ? "Metal ready" : "Desktop only"}
            </span>
          </section>

          <div className="section-heading">
            <div>
              <h2>Local models</h2>
              <p>Every download is checked before Spick can load it.</p>
            </div>
            <span>
              <ShieldCheck size={14} />
              {cloudFallbackEnabled
                ? "Local first · fallback on"
                : "Stays on this Mac"}
            </span>
          </div>

          <div className="engine-list">
            {visibleEngines.map((engine) => (
              <EngineCard
                engine={engine}
                key={engine.id}
                download={downloads[engine.id]}
                actionsDisabled={
                  pendingModelId !== null || cloudPending !== null
                }
                cancelling={cancellingModelIds.has(engine.id)}
                pending={pendingModelId === engine.id}
                onActivate={() => onActivate(engine.id)}
                onCancelInstall={() => onCancelInstall(engine.id)}
                onInstall={() => onInstall(engine.id)}
                onRemove={() => onRemove(engine.id)}
              />
            ))}
          </div>

          <div className="engine-note">
            <LockKeyhole size={17} />
            <div>
              <strong>This model runs on this Mac</strong>
              <span>
                {cloudFallbackEnabled
                  ? "Cloud fallback is on. If local transcription cannot finish, that recording can be sent to your first configured, language-compatible cloud provider."
                  : "With cloud fallback off, recordings go from memory to whisper.cpp and are released when the session ends."}
              </span>
            </div>
          </div>
        </>
      ) : (
        <>
          <section className="cloud-intro">
            <div className="cloud-intro__icon">
              <Cloud size={21} />
            </div>
            <div>
              <strong>Your key, your provider</strong>
              <p>
                Audio leaves this Mac only when a cloud provider is selected as
                primary, or when cloud fallback is explicitly enabled in
                Settings. Keys are kept by the operating system credential
                store. Enabled vocabulary hints may accompany the recording.
                Once an upload begins, cancelling cannot recall audio already
                sent.
              </p>
            </div>
            <span>
              <Wifi size={14} /> Internet required
            </span>
          </section>

          <div className="section-heading">
            <div>
              <h2>Cloud providers</h2>
              <p>
                Completed recordings are sent as batch requests. Choose the
                tradeoff that fits your work.
              </p>
            </div>
            {native && (
              <button
                type="button"
                className="button button--secondary"
                onClick={onCloudRefresh}
                disabled={cloudLoading || cloudPending !== null}
              >
                <RefreshCw size={14} />
                {cloudLoading ? "Refreshing…" : "Refresh"}
              </button>
            )}
          </div>

          {cloudError && (
            <div className="engine-inline-error" role="alert">
              <strong>Cloud setup didn’t finish</strong>
              <span>{cloudError}</span>
              <button
                type="button"
                className="text-button"
                onClick={onCloudRefresh}
                disabled={cloudLoading || cloudPending !== null}
              >
                Try again
              </button>
            </div>
          )}

          {!native && (
            <div className="cloud-browser-note" role="status">
              <LockKeyhole size={15} />
              <span>
                Open the Tauri development app to save a key or select a cloud
                provider. This browser preview cannot access the credential
                store.
              </span>
            </div>
          )}

          {native && shownCloudProviders.length === 0 && (
            <div className="cloud-provider-empty" aria-busy={cloudLoading}>
              <Cloud size={20} />
              <strong>
                {cloudLoading
                  ? "Loading cloud providers…"
                  : "No cloud providers are available"}
              </strong>
              <span>
                {cloudLoading
                  ? "Reading configuration from the native app."
                  : "Refresh to ask the native app for provider status again."}
              </span>
            </div>
          )}

          <div className="cloud-provider-grid">
            {shownCloudProviders.map((provider) => {
              const meta = providerMeta[provider.provider];
              const providerPending =
                cloudPending?.provider === provider.provider;
              const actionsDisabled =
                !native ||
                cloudLoading ||
                cloudPending !== null ||
                pendingModelId !== null;
              const editing = editingProvider === provider.provider;
              const confirmingDelete =
                confirmDeleteProvider === provider.provider;
              return (
                <article
                  className={`cloud-provider-card ${provider.selected ? "cloud-provider-card--active" : ""}`}
                  key={provider.provider}
                >
                  <div className={`provider-logo ${meta.className}`}>
                    {meta.initial}
                  </div>
                  <div className="cloud-provider-card__title">
                    <div>
                      <strong>{provider.providerName}</strong>
                      <span>{provider.modelName}</span>
                    </div>
                    {provider.selected && (
                      <span className="active-badge">
                        <Check size={12} /> Active
                      </span>
                    )}
                    {provider.experimental && (
                      <span className="prototype-badge">
                        <FlaskConical size={11} /> Experimental
                      </span>
                    )}
                  </div>
                  <p>{provider.description}</p>
                  <div className="engine-capabilities">
                    <span>
                      <Globe2 size={13} /> {provider.languageSupport}
                    </span>
                    <span>
                      <Gauge size={13} /> {provider.cleanupBehavior}
                    </span>
                  </div>
                  <div className="cloud-credential-status">
                    <KeyRound size={14} />
                    <span>
                      {provider.configured
                        ? "Key saved in the OS credential store"
                        : "No API key saved"}
                    </span>
                  </div>

                  {editing && (
                    <div className="cloud-key-editor">
                      <label className="field">
                        <span className="field__label">
                          {provider.providerName} API key
                        </span>
                        <span className="cloud-key-input">
                          <KeyRound size={15} />
                          <input
                            autoFocus
                            aria-label={`${provider.providerName} API key`}
                            autoComplete="off"
                            autoCapitalize="none"
                            data-1p-ignore="true"
                            spellCheck={false}
                            type={showCredential ? "text" : "password"}
                            value={credentialDraft}
                            onChange={(event) =>
                              setCredentialDraft(event.currentTarget.value)
                            }
                            placeholder="Paste key for this save only"
                            disabled={cloudPending !== null}
                          />
                          <button
                            type="button"
                            className="icon-button icon-button--subtle"
                            aria-label={
                              showCredential ? "Hide API key" : "Show API key"
                            }
                            onClick={() => setShowCredential((shown) => !shown)}
                            disabled={cloudPending !== null}
                          >
                            {showCredential ? (
                              <EyeOff size={15} />
                            ) : (
                              <Eye size={15} />
                            )}
                          </button>
                        </span>
                      </label>
                      <div className="cloud-key-editor__actions">
                        <button
                          type="button"
                          className="button button--secondary"
                          onClick={cancelCredentialEdit}
                          disabled={cloudPending !== null}
                        >
                          Cancel
                        </button>
                        <button
                          type="button"
                          className="button button--primary"
                          onClick={() => void saveCredential(provider.provider)}
                          disabled={
                            !credentialDraft.trim() || cloudPending !== null
                          }
                        >
                          {providerPending &&
                          cloudPending?.action === "configure"
                            ? "Saving…"
                            : provider.configured
                              ? "Replace key"
                              : "Save key"}
                        </button>
                      </div>
                      <small>
                        Spick sends this value straight to the OS credential
                        store. The field is cleared as soon as you press save.
                      </small>
                    </div>
                  )}

                  {!editing && (
                    <div className="cloud-provider-actions">
                      <button
                        type="button"
                        className="button button--secondary"
                        onClick={() => beginCredentialEdit(provider.provider)}
                        disabled={actionsDisabled}
                      >
                        <KeyRound size={14} />
                        {provider.configured ? "Replace key" : "Add key"}
                      </button>
                      <button
                        type="button"
                        className="button button--primary"
                        onClick={() => void onCloudActivate(provider.provider)}
                        disabled={
                          actionsDisabled ||
                          !provider.configured ||
                          provider.selected
                        }
                      >
                        {providerPending && cloudPending?.action === "activate"
                          ? "Activating…"
                          : provider.selected
                            ? "In use"
                            : provider.configured
                              ? "Use provider"
                              : "Add key first"}
                      </button>
                      {provider.configured && (
                        <button
                          type="button"
                          className={`button button--secondary ${confirmingDelete ? "button--danger" : ""}`}
                          aria-label={
                            confirmingDelete
                              ? `Confirm remove ${provider.providerName} API key`
                              : `Remove ${provider.providerName} API key`
                          }
                          title={
                            provider.selected
                              ? "Select another engine before removing this key."
                              : undefined
                          }
                          onClick={() =>
                            void deleteCredential(provider.provider)
                          }
                          disabled={actionsDisabled || provider.selected}
                        >
                          <Trash2 size={14} />
                          {providerPending && cloudPending?.action === "delete"
                            ? "Removing…"
                            : confirmingDelete
                              ? "Confirm remove"
                              : "Remove key"}
                        </button>
                      )}
                    </div>
                  )}
                  {provider.selected && provider.configured && (
                    <small className="cloud-active-key-note">
                      Select another engine before removing this key.
                    </small>
                  )}
                </article>
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}

interface EngineCardProps {
  engine: Engine;
  download?: ModelDownloadProgress;
  actionsDisabled: boolean;
  cancelling: boolean;
  pending: boolean;
  onActivate: () => void;
  onCancelInstall: () => void;
  onInstall: () => void;
  onRemove: () => void;
}

function EngineCard({
  engine,
  download,
  actionsDisabled,
  cancelling,
  pending,
  onActivate,
  onCancelInstall,
  onInstall,
  onRemove,
}: EngineCardProps) {
  const progress = download
    ? Math.min(
        100,
        Math.round((download.downloadedBytes / download.totalBytes) * 100),
      )
    : null;
  const englishOnly = engine.languageSupport.startsWith("English-only");

  return (
    <article
      className={`engine-card ${engine.status === "active" ? "engine-card--active" : ""}`}
    >
      <div className="engine-card__radio" aria-hidden="true">
        {engine.status === "active" && <i />}
      </div>
      <div className="engine-card__body">
        <div className="engine-card__title">
          <strong>{engine.name}</strong>
          {engine.recommended && (
            <span className="recommended-badge">
              <Check size={11} /> Recommended
            </span>
          )}
          {engine.status === "active" && (
            <span className="active-badge">
              <Check size={12} /> Active
            </span>
          )}
        </div>
        <span className="engine-card__provider">{engine.provider}</span>
        <p>{engine.description}</p>
        <div className="engine-capabilities">
          <span>
            <Globe2 size={13} /> {engine.languageSupport}
          </span>
          <span>
            <Gauge size={13} /> {engine.performance}
          </span>
          {engine.size && (
            <span>
              <HardDrive size={13} /> Download {engine.size}
            </span>
          )}
          <span>
            <Cpu size={13} /> Runs on device
          </span>
        </div>
        {progress !== null && (
          <div
            className="download-progress"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={progress}
          >
            <div>
              <span>
                {download?.phase === "verifying"
                  ? "Checking download…"
                  : download?.phase === "installed"
                    ? "Ready"
                    : "Downloading model…"}
              </span>
              <strong>{progress}%</strong>
            </div>
            <i>
              <span style={{ width: `${progress}%` }} />
            </i>
          </div>
        )}
      </div>
      <div className="engine-card__actions">
        {engine.status === "active" && (
          <button type="button" className="button button--secondary" disabled>
            <Check size={15} /> In use
          </button>
        )}
        {engine.status === "ready" && (
          <button
            type="button"
            className="button button--primary"
            onClick={onActivate}
            disabled={actionsDisabled}
          >
            {pending
              ? "Checking…"
              : englishOnly
                ? "Use in English"
                : "Use model"}
          </button>
        )}
        {(engine.status === "available" || engine.status === "invalid") &&
          (pending ? (
            <button
              type="button"
              className="button button--secondary"
              onClick={onCancelInstall}
              disabled={cancelling}
            >
              <X size={15} /> {cancelling ? "Stopping…" : "Cancel download"}
            </button>
          ) : (
            <button
              type="button"
              className="button button--secondary"
              onClick={onInstall}
              disabled={actionsDisabled || progress !== null || cancelling}
            >
              {cancelling ? <X size={15} /> : <Download size={15} />}
              {cancelling
                ? "Stopping…"
                : engine.status === "invalid"
                  ? "Download again"
                  : "Download"}
            </button>
          ))}
        {engine.status === "ready" && (
          <button
            type="button"
            className="icon-button icon-button--subtle"
            onClick={onRemove}
            disabled={actionsDisabled}
            aria-label={`Remove ${engine.name}`}
          >
            <Trash2 size={16} />
          </button>
        )}
      </div>
    </article>
  );
}
