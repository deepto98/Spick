import { useMemo, useState } from "react";
import {
  Check,
  ChevronRight,
  CircleDashed,
  Cloud,
  Cpu,
  Download,
  Gauge,
  Globe2,
  HardDrive,
  KeyRound,
  Laptop,
  LockKeyhole,
  MoreHorizontal,
  ShieldCheck,
  Trash2,
  Wifi,
  X,
} from "lucide-react";
import type { Engine, EngineKind } from "../types";
import type { ModelDownloadProgress } from "../lib/nativeModels";
import { PageHeader } from "../components/Ui";

interface EnginesViewProps {
  engines: Engine[];
  downloads: Record<string, ModelDownloadProgress>;
  native: boolean;
  cancellingModelIds: ReadonlySet<string>;
  pendingModelId: string | null;
  error?: string;
  onActivate: (id: string) => void;
  onCancelInstall: (id: string) => void;
  onInstall: (id: string) => void;
  onRemove: (id: string) => void;
}

const providerMeta: Record<string, { initial: string; className: string }> = {
  OpenAI: { initial: "O", className: "provider-logo--openai" },
  Google: { initial: "G", className: "provider-logo--google" },
  xAI: { initial: "x", className: "provider-logo--xai" },
};

export function EnginesView({
  engines,
  downloads,
  native,
  cancellingModelIds,
  pendingModelId,
  error,
  onActivate,
  onCancelInstall,
  onInstall,
  onRemove,
}: EnginesViewProps) {
  const [kind, setKind] = useState<EngineKind>("local");

  const visibleEngines = useMemo(
    () => engines.filter((engine) => engine.kind === kind),
    [engines, kind],
  );
  const activeEngine = engines.find((engine) => engine.status === "active");
  const localCount = engines.filter((engine) => engine.kind === "local").length;
  const cloudCount = engines.filter((engine) => engine.kind === "cloud").length;
  const installedCount = engines.filter(
    (engine) =>
      engine.kind === "local" &&
      (engine.status === "active" || engine.status === "ready"),
  ).length;

  return (
    <div className="view view--engines">
      <PageHeader
        eyebrow="TRANSCRIPTION"
        title="Engines"
        description="Choose where your recording is transcribed. Local models stay on this computer."
        actions={
          <div className="engine-active-pill">
            {activeEngine ? (
              <span className="status-dot" />
            ) : (
              <CircleDashed size={16} />
            )}
            <div>
              <small>Selected here</small>
              <strong>{activeEngine?.name ?? "None selected"}</strong>
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
              <ShieldCheck size={14} /> Stays on this Mac
            </span>
          </div>

          <div className="engine-list">
            {visibleEngines.map((engine) => (
              <EngineCard
                engine={engine}
                key={engine.id}
                download={downloads[engine.id]}
                actionsDisabled={pendingModelId !== null}
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
              <strong>Audio stays on this Mac</strong>
              <span>
                Recordings go from memory to whisper.cpp and are released when
                the session ends.
              </span>
            </div>
            <button type="button" className="text-button">
              How it works <ChevronRight size={14} />
            </button>
          </div>
        </>
      ) : (
        <>
          <section className="cloud-intro">
            <div className="cloud-intro__icon">
              <Cloud size={21} />
            </div>
            <div>
              <strong>Use your own provider</strong>
              <p>
                Cloud adapters aren’t connected yet. When they are, each one
                will show exactly what its model supports.
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
                API keys stay disabled until OS credential storage is ready.
              </p>
            </div>
          </div>

          <div className="cloud-provider-grid">
            {visibleEngines.map((engine) => {
              const meta = providerMeta[engine.provider];
              return (
                <article
                  className={`cloud-provider-card ${engine.status === "active" ? "cloud-provider-card--active" : ""}`}
                  key={engine.id}
                >
                  <div className={`provider-logo ${meta?.className ?? ""}`}>
                    {meta?.initial ?? engine.provider[0]}
                  </div>
                  <div className="cloud-provider-card__title">
                    <div>
                      <strong>{engine.provider}</strong>
                      <span>{engine.name}</span>
                    </div>
                    {engine.status === "active" && (
                      <span className="active-badge">
                        <Check size={12} /> Active
                      </span>
                    )}
                  </div>
                  <p>{engine.description}</p>
                  <div className="engine-capabilities">
                    <span>
                      <Globe2 size={13} /> {engine.languageSupport}
                    </span>
                    <span>
                      <Gauge size={13} /> {engine.performance}
                    </span>
                  </div>
                  <button
                    type="button"
                    className="button button--secondary button--full"
                    disabled
                  >
                    <KeyRound size={15} /> Adapter planned
                  </button>
                </article>
              );
            })}
          </div>

          <section className="api-key-card">
            <div className="api-key-card__heading">
              <div className="provider-logo provider-logo--openai">O</div>
              <div>
                <h2>OpenAI API key</h2>
                <p>This field is here to show the planned setup.</p>
              </div>
              <span className="prototype-badge">NOT CONNECTED</span>
            </div>
            <div className="api-key-input">
              <KeyRound size={16} />
              <input
                value=""
                type="password"
                aria-label="OpenAI API key"
                placeholder="Keychain support is not ready"
                disabled
                readOnly
              />
              <button
                type="button"
                className="button button--primary button--small"
                disabled
              >
                Not ready
              </button>
            </div>
            <span className="api-key-hint">
              <LockKeyhole size={13} /> Do not paste credentials into this
              field. Spick will use the OS credential store when this is ready.
            </span>
          </section>
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
        {engine.status === "active" && (
          <button
            type="button"
            className="icon-button icon-button--subtle"
            aria-label={`More options for ${engine.name}`}
          >
            <MoreHorizontal size={17} />
          </button>
        )}
      </div>
    </article>
  );
}
