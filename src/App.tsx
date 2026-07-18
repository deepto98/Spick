import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import "./App.css";
import { DictationHud } from "./components/DictationHud";
import { Onboarding } from "./components/Onboarding";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { initialEngines, initialVocabulary } from "./data/mockData";
import { useAudioLevel } from "./hooks/useAudioLevel";
import { useDictationController } from "./hooks/useDictationController";
import {
  activateLocalModel,
  cancelLocalModelInstall,
  formatModelBytes,
  installLocalModel,
  listLocalModels,
  modelStatus,
  removeLocalModel,
  subscribeToModelDownload,
  type ModelDownloadProgress,
} from "./lib/nativeModels";
import {
  getNativeSettings,
  languagePolicyBadge,
  languagePolicyForName,
  languagePolicyName,
  updateNativeSettings,
  type NativeAppSettings,
} from "./lib/nativeSettings";
import type { AppSettings, Engine, ViewId, VocabularyEntry } from "./types";
import { EnginesView } from "./views/EnginesView";
import { SettingsView } from "./views/SettingsView";
import { TodayView } from "./views/TodayView";
import { VocabularyView } from "./views/VocabularyView";

const defaultSettings: AppSettings = {
  hotkey: "⌘+⇧+Space",
  language: "Auto-detect",
  microphone: "System default microphone",
  launchAtLogin: false,
  playSounds: true,
  showWidget: true,
  keepHistory: false,
  cloudFallback: false,
  cleanupLevel: "Clean",
};

function App() {
  const hudOnly =
    new URLSearchParams(window.location.search).get("window") === "hud";
  const [activeView, setActiveView] = useState<ViewId>("today");
  const [engines, setEngines] = useState<Engine[]>(initialEngines);
  const [modelDownloads, setModelDownloads] = useState<
    Record<string, ModelDownloadProgress>
  >({});
  const [modelActionPending, setModelActionPending] = useState<string | null>(
    null,
  );
  const [cancellingModels, setCancellingModels] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const cancelledModelDownloads = useRef(new Set<string>());
  const [modelError, setModelError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [languageSaving, setLanguageSaving] = useState(false);
  const [nativeSettings, setNativeSettings] =
    useState<NativeAppSettings | null>(null);
  const nativeSettingsRef = useRef<NativeAppSettings | null>(null);
  const languageSaveRevision = useRef(0);
  const settingsSaveQueue = useRef<Promise<void>>(Promise.resolve());
  const [vocabulary, setVocabulary] =
    useState<VocabularyEntry[]>(initialVocabulary);
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const dictation = useDictationController(!hudOnly);
  const audioFrame = useAudioLevel(dictation.state === "listening");
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [onboardingComplete, setOnboardingComplete] = useState(
    () => window.localStorage.getItem("spick-onboarding-complete") === "true",
  );

  const acceptNativeSettings = useCallback((saved: NativeAppSettings) => {
    nativeSettingsRef.current = saved;
    setNativeSettings(saved);
    setSettings((current) => ({
      ...current,
      cloudFallback: saved.allowCloudFallback,
      keepHistory: saved.saveTranscriptHistory,
      language: languagePolicyName(saved.languagePolicy),
    }));
  }, []);

  useEffect(() => {
    if (!dictation.native || hudOnly) return;
    let disposed = false;
    void getNativeSettings()
      .then((saved) => {
        if (!disposed) acceptNativeSettings(saved);
      })
      .catch((reason) => {
        if (!disposed) {
          setSettingsError(`Couldn’t read saved language settings: ${reason}`);
        }
      });
    return () => {
      disposed = true;
    };
  }, [acceptNativeSettings, dictation.native, hudOnly]);

  const refreshLocalEngines = useCallback(async () => {
    if (!dictation.native) return;
    const catalog = await listLocalModels();
    setEngines((current) => {
      const templates = new Map(current.map((engine) => [engine.id, engine]));
      const local = catalog.map<Engine>((model) => {
        const template = templates.get(model.manifest.id);
        return {
          id: model.manifest.id,
          name: model.manifest.displayName,
          provider: "whisper.cpp",
          description:
            template?.description ?? "A verified local whisper.cpp model.",
          kind: "local",
          status: modelStatus(model),
          languageSupport:
            model.manifest.languages === "englishOnly"
              ? "English-only (.en)"
              : "Multilingual model",
          size: formatModelBytes(model.manifest.downloadBytes),
          performance:
            model.state === "installed"
              ? "Ready on this Mac"
              : "Benchmark pending",
          recommended: model.manifest.id === "whisper-small-multilingual-q5-1",
        };
      });
      return [...local, ...current.filter((engine) => engine.kind === "cloud")];
    });
  }, [dictation.native]);

  useEffect(() => {
    if (!dictation.native) return;
    const timeout = window.setTimeout(() => {
      void refreshLocalEngines().catch((reason) => {
        setModelError(`Couldn’t read local models: ${String(reason)}`);
      });
    }, 0);
    return () => window.clearTimeout(timeout);
  }, [dictation.native, refreshLocalEngines]);

  useEffect(() => {
    if (!dictation.native || dictation.state !== "error") return;
    const timeout = window.setTimeout(() => {
      void refreshLocalEngines().catch((reason) => {
        setModelError(`Couldn’t refresh local models: ${String(reason)}`);
      });
    }, 0);
    return () => window.clearTimeout(timeout);
  }, [dictation.native, dictation.state, refreshLocalEngines]);

  useEffect(() => {
    if (!dictation.native) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void subscribeToModelDownload((progress) => {
      if (disposed) return;
      setModelDownloads((current) => ({
        ...current,
        [progress.modelId]: progress,
      }));
    })
      .then((stopListening) => {
        if (disposed) stopListening();
        else unsubscribe = stopListening;
      })
      .catch((reason) => {
        if (!disposed) {
          setModelError(`Couldn’t watch model downloads: ${String(reason)}`);
        }
      });
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [dictation.native]);

  const navigate = (view: ViewId) => {
    setActiveView(view);
    setMobileNavOpen(false);
  };

  const activateEngine = (id: string) => {
    if (dictation.native) {
      setModelActionPending(id);
      setModelError(null);
      void activateLocalModel(id)
        .then((saved) => {
          acceptNativeSettings(saved);
          return refreshLocalEngines();
        })
        .catch((reason) => setModelError(`Couldn’t use that model: ${reason}`))
        .finally(() => setModelActionPending(null));
      return;
    }
    setEngines((current) =>
      current.map((engine) => {
        if (engine.id === id) return { ...engine, status: "active" };
        if (engine.status === "active") return { ...engine, status: "ready" };
        return engine;
      }),
    );
  };

  const changeSettings = (next: AppSettings) => {
    const previousLanguage = settings.language;
    setSettings(next);
    if (!dictation.native || next.language === previousLanguage) return;

    const languagePolicy = languagePolicyForName(next.language);
    if (!languagePolicy || !nativeSettingsRef.current) {
      setSettingsError("That language mode isn’t connected to dictation yet.");
      return;
    }

    const requestRevision = ++languageSaveRevision.current;
    setLanguageSaving(true);
    setSettingsError(null);
    settingsSaveQueue.current = settingsSaveQueue.current
      .catch(() => undefined)
      .then(async () => {
        const current = nativeSettingsRef.current;
        if (!current) throw new Error("saved settings are unavailable");
        const saved = await updateNativeSettings({
          ...current,
          languagePolicy,
        });
        nativeSettingsRef.current = saved;
        if (requestRevision === languageSaveRevision.current) {
          acceptNativeSettings(saved);
        }
      })
      .catch((reason) => {
        if (requestRevision === languageSaveRevision.current) {
          const current = nativeSettingsRef.current;
          if (current) acceptNativeSettings(current);
          setSettingsError(`Couldn’t save that language: ${reason}`);
        }
      })
      .finally(() => {
        if (requestRevision === languageSaveRevision.current) {
          setLanguageSaving(false);
        }
      });
  };

  const hudLanguage = nativeSettings
    ? languagePolicyBadge(nativeSettings.languagePolicy)
    : dictation.language;

  const installEngine = (id: string) => {
    if (dictation.native) {
      setModelActionPending(id);
      setModelError(null);
      void installLocalModel(id)
        .then(refreshLocalEngines)
        .catch((reason) => {
          if (!cancelledModelDownloads.current.has(id)) {
            setModelError(`Couldn’t download that model: ${reason}`);
          }
        })
        .finally(() => {
          cancelledModelDownloads.current.delete(id);
          setCancellingModels((current) => {
            const next = new Set(current);
            next.delete(id);
            return next;
          });
          setModelActionPending((current) => (current === id ? null : current));
          setModelDownloads((current) => {
            const next = { ...current };
            delete next[id];
            return next;
          });
        });
      return;
    }
    setEngines((current) =>
      current.map((engine) =>
        engine.id === id ? { ...engine, status: "ready" } : engine,
      ),
    );
  };

  const cancelEngineInstall = (id: string) => {
    cancelledModelDownloads.current.add(id);
    setCancellingModels((current) => new Set(current).add(id));
    void cancelLocalModelInstall(id)
      .then(() => {
        setModelActionPending((current) => (current === id ? null : current));
        setModelDownloads((current) => {
          const next = { ...current };
          delete next[id];
          return next;
        });
      })
      .catch((reason) => {
        cancelledModelDownloads.current.delete(id);
        setCancellingModels((current) => {
          const next = new Set(current);
          next.delete(id);
          return next;
        });
        setModelError(`Couldn’t stop that download: ${reason}`);
      });
  };

  const removeEngine = (id: string) => {
    if (dictation.native) {
      setModelActionPending(id);
      setModelError(null);
      void removeLocalModel(id)
        .then(refreshLocalEngines)
        .catch((reason) =>
          setModelError(`Couldn’t remove that model: ${reason}`),
        )
        .finally(() => setModelActionPending(null));
      return;
    }
    setEngines((current) =>
      current.map((engine) =>
        engine.id === id ? { ...engine, status: "available" } : engine,
      ),
    );
  };

  const completeOnboarding = () => {
    window.localStorage.setItem("spick-onboarding-complete", "true");
    setOnboardingComplete(true);
  };

  const restartOnboarding = () => {
    window.localStorage.removeItem("spick-onboarding-complete");
    setOnboardingComplete(false);
  };

  if (hudOnly) {
    return (
      <div className="hud-window-surface">
        <DictationHud
          autoAdvance={false}
          audioLevel={audioFrame?.level}
          disabled={dictation.pending}
          errorMessage={dictation.error ?? undefined}
          floating
          state={dictation.state}
          onStateChange={dictation.transitionTo}
          language={dictation.language}
        />
      </div>
    );
  }

  if (!onboardingComplete) {
    return (
      <Onboarding
        settings={settings}
        onSettingsChange={changeSettings}
        onComplete={completeOnboarding}
      />
    );
  }

  return (
    <div className={`app-shell ${mobileNavOpen ? "app-shell--nav-open" : ""}`}>
      <div
        className="mobile-nav-backdrop"
        onClick={() => setMobileNavOpen(false)}
        aria-hidden={!mobileNavOpen}
      />
      <div className="sidebar-wrap">
        <button
          type="button"
          className="mobile-nav-close icon-button"
          onClick={() => setMobileNavOpen(false)}
          aria-label="Close navigation"
        >
          <X size={18} />
        </button>
        <Sidebar activeView={activeView} onNavigate={navigate} />
      </div>
      <div className="app-main">
        <TopBar
          activeView={activeView}
          onOpenNav={() => setMobileNavOpen(true)}
        />
        <main className="content" id="main-content">
          {activeView === "today" && (
            <TodayView
              onOpenEngines={() => navigate("engines")}
              hudState={dictation.state}
              audioLevel={audioFrame?.level}
              dictationPending={dictation.pending}
              dictationError={dictation.error ?? undefined}
              lastTranscript={dictation.lastTranscript}
              language={hudLanguage}
              native={dictation.native}
              onHudStateChange={dictation.transitionTo}
            />
          )}
          {activeView === "engines" && (
            <EnginesView
              engines={engines}
              downloads={modelDownloads}
              native={dictation.native}
              cancellingModelIds={cancellingModels}
              pendingModelId={modelActionPending}
              error={modelError ?? undefined}
              onActivate={activateEngine}
              onCancelInstall={cancelEngineInstall}
              onInstall={installEngine}
              onRemove={removeEngine}
            />
          )}
          {activeView === "vocabulary" && (
            <VocabularyView
              vocabulary={vocabulary}
              onAdd={(entry) => setVocabulary((current) => [entry, ...current])}
              onRemove={(id) =>
                setVocabulary((current) =>
                  current.filter((entry) => entry.id !== id),
                )
              }
            />
          )}
          {activeView === "settings" && (
            <SettingsView
              settings={settings}
              languageSaving={languageSaving}
              nativeError={settingsError ?? undefined}
              onChange={changeSettings}
              onRestartOnboarding={restartOnboarding}
            />
          )}
        </main>
      </div>
      {settings.showWidget && activeView !== "today" && (
        <DictationHud
          autoAdvance={false}
          audioLevel={audioFrame?.level}
          disabled={dictation.pending}
          errorMessage={dictation.error ?? undefined}
          floating
          state={dictation.state}
          onStateChange={dictation.transitionTo}
          language={hudLanguage}
        />
      )}
      {dictation.error && (
        <div className="native-error-toast" role="alert">
          <strong>Dictation unavailable</strong>
          <span>{dictation.error}</span>
        </div>
      )}
    </div>
  );
}

export default App;
