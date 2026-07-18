import { useMemo, useState } from "react";
import {
  Check,
  ChevronRight,
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
  Sparkles,
  Trash2,
  Wifi,
  Zap,
} from "lucide-react";
import type { Engine, EngineKind } from "../types";
import { PageHeader } from "../components/Ui";

interface EnginesViewProps {
  engines: Engine[];
  onActivate: (id: string) => void;
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
  onActivate,
  onInstall,
  onRemove,
}: EnginesViewProps) {
  const [kind, setKind] = useState<EngineKind>("local");

  const visibleEngines = useMemo(
    () => engines.filter((engine) => engine.kind === kind),
    [engines, kind],
  );
  const activeEngine = engines.find((engine) => engine.status === "active");

  return (
    <div className="view view--engines">
      <PageHeader
        eyebrow="Speech intelligence"
        title="Engines"
        description="Preview local and cloud engine configuration. Runtime adapters and model downloads are not connected yet."
        actions={
          <div className="engine-active-pill">
            {activeEngine ? (
              <span className="status-dot" />
            ) : (
              <Sparkles size={16} />
            )}
            <div>
              <small>Preview selection</small>
              <strong>{activeEngine?.name ?? "None selected"}</strong>
            </div>
          </div>
        }
      />

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
            <small>Private & offline</small>
          </span>
          <span className="tab-count">3</span>
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
            <small>Frontier accuracy</small>
          </span>
          <span className="tab-count">3</span>
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
              <strong>Native hardware scan pending</strong>
              <p>
                Compatibility and acceleration will be measured on this device
                before a model is recommended.
              </p>
            </div>
            <div className="hardware-banner__stats">
              <span>
                <Zap size={14} /> <strong>Benchmark</strong> after install
              </span>
              <span>
                <HardDrive size={14} /> <strong>No models</strong> installed
              </span>
            </div>
            <span className="compatibility-badge">
              <Sparkles size={13} /> Preview profile
            </span>
          </section>

          <div className="section-heading">
            <div>
              <h2>Local models</h2>
              <p>Downloads and activation are simulated in this milestone.</p>
            </div>
            <span>
              <ShieldCheck size={14} /> Local-first plan
            </span>
          </div>

          <div className="engine-list">
            {visibleEngines.map((engine) => (
              <EngineCard
                engine={engine}
                key={engine.id}
                onActivate={() => onActivate(engine.id)}
                onInstall={() => onInstall(engine.id)}
                onRemove={() => onRemove(engine.id)}
              />
            ))}
          </div>

          <div className="engine-note">
            <LockKeyhole size={17} />
            <div>
              <strong>Local means local</strong>
              <span>
                Once connected, local models will run without sending audio to a
                provider. Raw audio will remain ephemeral by default.
              </span>
            </div>
            <button type="button" className="text-button">
              Privacy details <ChevronRight size={14} />
            </button>
          </div>
        </>
      ) : (
        <>
          <section className="cloud-intro">
            <div className="cloud-intro__icon">
              <Sparkles size={21} />
            </div>
            <div>
              <strong>Frontier speech, when you need it</strong>
              <p>
                Planned adapters will expose only the capabilities each provider
                and model actually supports.
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
                Provider credentials stay disabled until native credential
                storage is implemented.
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
                <p>Credential storage is shown as a setup preview.</p>
              </div>
              <span className="prototype-badge">NOT CONNECTED</span>
            </div>
            <div className="api-key-input">
              <KeyRound size={16} />
              <input
                value=""
                type="password"
                aria-label="OpenAI API key"
                placeholder="Keychain integration required"
                disabled
                readOnly
              />
              <button
                type="button"
                className="button button--primary button--small"
                disabled
              >
                Unavailable in preview
              </button>
            </div>
            <span className="api-key-hint">
              <LockKeyhole size={13} /> Do not paste credentials into this
              preview. Native builds will use the OS credential store.
            </span>
          </section>
        </>
      )}
    </div>
  );
}

interface EngineCardProps {
  engine: Engine;
  onActivate: () => void;
  onInstall: () => void;
  onRemove: () => void;
}

function EngineCard({
  engine,
  onActivate,
  onInstall,
  onRemove,
}: EngineCardProps) {
  const [progress, setProgress] = useState<number | null>(null);

  const install = () => {
    setProgress(12);
    const progressSteps = [34, 61, 84, 100];
    progressSteps.forEach((value, index) => {
      window.setTimeout(
        () => {
          setProgress(value);
          if (value === 100) {
            window.setTimeout(() => {
              setProgress(null);
              onInstall();
            }, 280);
          }
        },
        280 * (index + 1),
      );
    });
  };

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
              <Sparkles size={11} /> Recommended
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
              <HardDrive size={13} /> Example {engine.size}
            </span>
          )}
          <span>
            <Zap size={13} /> Runtime planned
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
              <span>Downloading model…</span>
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
          >
            Use model
          </button>
        )}
        {engine.status === "available" && (
          <button
            type="button"
            className="button button--secondary"
            onClick={install}
            disabled={progress !== null}
          >
            <Download size={15} /> Preview download
          </button>
        )}
        {engine.status === "ready" && (
          <button
            type="button"
            className="icon-button icon-button--subtle"
            onClick={onRemove}
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
