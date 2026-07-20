import { useCallback, useEffect, useRef, useState } from "react";
import { X } from "lucide-react";
import "./App.css";
import { DictationHud } from "./components/DictationHud";
import { Onboarding } from "./components/Onboarding";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { initialEngines } from "./data/mockData";
import { useAudioLevel } from "./hooks/useAudioLevel";
import { useAccessibilityPermission } from "./hooks/useAccessibilityPermission";
import { useDictationController } from "./hooks/useDictationController";
import { useHudWindow } from "./hooks/useHudWindow";
import { useLocalData } from "./hooks/useLocalData";
import { useShortcutStatus } from "./hooks/useShortcutStatus";
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
  cleanupEngineForLevel,
  cleanupLevelForEngine,
  getNativeSettings,
  languagePolicyBadge,
  languagePolicyForName,
  languagePolicyName,
  shortcutDisplayName,
  updateNativeSettings,
  type NativeAppSettings,
  type NativeLanguagePolicy,
} from "./lib/nativeSettings";
import type { ClearLocalDataScope } from "./lib/nativeLocalData";
import type { AppSettings, Engine, ViewId } from "./types";
import { EnginesView } from "./views/EnginesView";
import { SettingsView } from "./views/SettingsView";
import { TodayView } from "./views/TodayView";
import { VocabularyView } from "./views/VocabularyView";

const defaultSettings: AppSettings = {
  hotkey: "⌥",
  language: "Auto-detect",
  microphone: "System default microphone",
  launchAtLogin: false,
  playSounds: true,
  showWidget: true,
  keepHistory: false,
  cloudFallback: false,
  cleanupLevel: "Verbatim",
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
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [settingsLoadRevision, setSettingsLoadRevision] = useState(0);
  const [nativeSettings, setNativeSettings] =
    useState<NativeAppSettings | null>(null);
  const nativeSettingsRef = useRef<NativeAppSettings | null>(null);
  const settingsIntentRef = useRef<{
    languagePolicy: NativeLanguagePolicy;
    cleanupLevel: AppSettings["cleanupLevel"];
    allowCloudFallback: boolean;
    saveTranscriptHistory: boolean;
  }>({
    languagePolicy: { mode: "auto" },
    cleanupLevel: defaultSettings.cleanupLevel,
    allowCloudFallback: defaultSettings.cloudFallback,
    saveTranscriptHistory: defaultSettings.keepHistory,
  });
  const settingsSaveRevision = useRef(0);
  const settingsSaveQueue = useRef<Promise<void>>(Promise.resolve());
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const dictation = useDictationController(!hudOnly);
  const localData = useLocalData(dictation.native && !hudOnly);
  const [hiddenEphemeralSessionId, setHiddenEphemeralSessionId] = useState<
    string | null
  >(null);
  const accessibility = useAccessibilityPermission(
    dictation.native && !hudOnly,
  );
  const shortcut = useShortcutStatus(dictation.native && !hudOnly);
  const hudWindow = useHudWindow(hudOnly && dictation.native);
  const audioFrame = useAudioLevel(dictation.state === "listening");
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [onboardingComplete, setOnboardingComplete] = useState(
    () => window.localStorage.getItem("spick-onboarding-complete") === "true",
  );

  const acceptNativeSettings = useCallback(
    (saved: NativeAppSettings, syncIntent = true) => {
      const language = languagePolicyName(saved.languagePolicy);
      const cleanupLevel =
        cleanupLevelForEngine(saved.cleanupEngine) ?? "Verbatim";
      nativeSettingsRef.current = saved;
      if (syncIntent) {
        settingsIntentRef.current = {
          languagePolicy: saved.languagePolicy,
          cleanupLevel,
          allowCloudFallback: saved.allowCloudFallback,
          saveTranscriptHistory: saved.saveTranscriptHistory,
        };
      }
      setNativeSettings(saved);
      setSettings((current) => ({
        ...current,
        hotkey: shortcutDisplayName(saved.pushToTalkShortcut),
        cloudFallback: saved.allowCloudFallback,
        keepHistory: saved.saveTranscriptHistory,
        language,
        cleanupLevel,
      }));
    },
    [],
  );

  useEffect(() => {
    if (!dictation.native || hudOnly) return;
    let disposed = false;
    void getNativeSettings()
      .then((saved) => {
        if (!disposed) acceptNativeSettings(saved);
      })
      .catch((reason) => {
        if (!disposed) {
          setSettingsError(`Couldn’t read saved settings: ${reason}`);
        }
      });
    return () => {
      disposed = true;
    };
  }, [acceptNativeSettings, dictation.native, hudOnly, settingsLoadRevision]);

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
    if (!dictation.native) {
      setSettings(next);
      return;
    }

    const languageChanged = next.language !== settings.language;
    const cleanupChanged = next.cleanupLevel !== settings.cleanupLevel;
    const cloudFallbackChanged = next.cloudFallback !== settings.cloudFallback;
    const transcriptHistoryChanged = next.keepHistory !== settings.keepHistory;
    if (
      !languageChanged &&
      !cleanupChanged &&
      !cloudFallbackChanged &&
      !transcriptHistoryChanged
    ) {
      setSettings(next);
      return;
    }

    const intent = settingsIntentRef.current;
    const languagePolicy = languageChanged
      ? languagePolicyForName(next.language)
      : intent.languagePolicy;
    const desiredCleanupLevel = cleanupChanged
      ? next.cleanupLevel
      : intent.cleanupLevel;
    const allowCloudFallback = cloudFallbackChanged
      ? next.cloudFallback
      : intent.allowCloudFallback;
    const saveTranscriptHistory = transcriptHistoryChanged
      ? next.keepHistory
      : intent.saveTranscriptHistory;
    if (!languagePolicy || !nativeSettingsRef.current) {
      setSettingsError("That setting isn’t connected to dictation yet.");
      return;
    }
    const cleanupEngine = cleanupEngineForLevel(desiredCleanupLevel);

    // Native-backed choices stay on their last acknowledged values while the
    // write is in flight. Preview-only choices can still update immediately.
    setSettings((current) => ({
      ...next,
      language: current.language,
      cleanupLevel: current.cleanupLevel,
      cloudFallback: current.cloudFallback,
      keepHistory: current.keepHistory,
    }));
    settingsIntentRef.current = {
      languagePolicy,
      cleanupLevel: desiredCleanupLevel,
      allowCloudFallback,
      saveTranscriptHistory,
    };

    const requestRevision = ++settingsSaveRevision.current;
    setSettingsSaving(true);
    setSettingsError(null);
    settingsSaveQueue.current = settingsSaveQueue.current
      .catch(() => undefined)
      .then(async () => {
        const current = nativeSettingsRef.current;
        if (!current) throw new Error("saved settings are unavailable");
        const saved = await updateNativeSettings({
          ...current,
          languagePolicy,
          cleanupEngine,
          allowCloudFallback,
          saveTranscriptHistory,
        });
        acceptNativeSettings(
          saved,
          requestRevision === settingsSaveRevision.current,
        );
      })
      .catch((reason) => {
        if (requestRevision === settingsSaveRevision.current) {
          const current = nativeSettingsRef.current;
          if (current) acceptNativeSettings(current);
          setSettingsError(`Couldn’t save that setting: ${reason}`);
        }
      })
      .finally(() => {
        if (requestRevision === settingsSaveRevision.current) {
          setSettingsSaving(false);
        }
      });
  };

  const changeShortcut = (pushToTalkShortcut: string) => {
    const displayedShortcut = shortcutDisplayName(pushToTalkShortcut);
    if (!dictation.native) {
      setSettings((current) => ({
        ...current,
        hotkey: displayedShortcut,
      }));
      return;
    }

    const acknowledged = nativeSettingsRef.current;
    if (!acknowledged) {
      setSettingsError("Saved settings are still loading.");
      return;
    }
    if (acknowledged.pushToTalkShortcut === pushToTalkShortcut) return;

    const requestRevision = ++settingsSaveRevision.current;
    setSettingsSaving(true);
    setSettingsError(null);
    settingsSaveQueue.current = settingsSaveQueue.current
      .catch(() => undefined)
      .then(async () => {
        const current = nativeSettingsRef.current;
        if (!current) throw new Error("saved settings are unavailable");
        const saved = await updateNativeSettings({
          ...current,
          pushToTalkShortcut,
        });
        acceptNativeSettings(
          saved,
          requestRevision === settingsSaveRevision.current,
        );
        if (requestRevision === settingsSaveRevision.current) {
          void shortcut.refresh();
        }
      })
      .catch((reason) => {
        if (requestRevision === settingsSaveRevision.current) {
          const current = nativeSettingsRef.current;
          if (current) acceptNativeSettings(current);
          setSettingsError(`Couldn’t save that setting: ${reason}`);
        }
      })
      .finally(() => {
        if (requestRevision === settingsSaveRevision.current) {
          setSettingsSaving(false);
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

  const clearSavedLocalData = async (scope: ClearLocalDataScope) => {
    const result = await localData.clearData(scope);
    if (result?.clearedLatestSessionId) {
      setHiddenEphemeralSessionId(result.clearedLatestSessionId);
    }
    return result;
  };

  if (hudOnly) {
    return (
      <div className="hud-window-surface">
        <DictationHud
          autoAdvance={false}
          audioLevel={audioFrame?.level}
          disabled={dictation.pending}
          errorMessage={dictation.error ?? hudWindow.error ?? undefined}
          delivery={dictation.delivery}
          floating
          state={dictation.state}
          onStateChange={dictation.transitionTo}
          language={dictation.language}
          compact={hudWindow.compact}
          compactPending={hudWindow.pending}
          onMovePointerDown={hudWindow.beginDrag}
          onToggleCompact={() => void hudWindow.togglePresentation()}
        />
      </div>
    );
  }

  if (!onboardingComplete) {
    const settingsReady = !dictation.native || nativeSettings !== null;
    return (
      <Onboarding
        accessibilityError={accessibility.error ?? undefined}
        accessibilityPending={accessibility.pending}
        accessibilityStatus={accessibility.status}
        shortcutError={shortcut.error ?? undefined}
        shortcutPending={shortcut.pending}
        shortcutStatus={shortcut.status}
        settings={settings}
        settingsError={settingsError ?? undefined}
        settingsReady={settingsReady}
        settingsSaving={settingsSaving}
        onRefreshAccessibility={() => void accessibility.refresh()}
        onRequestAccessibility={() => void accessibility.request()}
        onRefreshShortcut={() => void shortcut.refresh()}
        onRequestInputMonitoring={() => void shortcut.request()}
        onRetrySettings={() => {
          setSettingsError(null);
          setSettingsLoadRevision((revision) => revision + 1);
        }}
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
        <Sidebar
          activeView={activeView}
          hotkey={settings.hotkey}
          onNavigate={navigate}
        />
      </div>
      <div className="app-main">
        <TopBar
          activeView={activeView}
          hotkey={settings.hotkey}
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
              delivery={dictation.delivery}
              lastTranscript={dictation.lastTranscript}
              hiddenEphemeralSessionId={hiddenEphemeralSessionId}
              language={hudLanguage}
              native={dictation.native}
              dashboard={localData.dashboard}
              dashboardLoading={localData.dashboardLoading}
              dashboardError={localData.dashboardError}
              subscriptionError={localData.subscriptionError}
              history={localData.history}
              historyLoading={localData.historyLoading}
              historyLoadingMore={localData.historyLoadingMore}
              historyError={localData.historyError}
              hasOlderHistory={localData.historyNextCursor !== null}
              saveTranscriptHistory={settings.keepHistory}
              onRefreshLocalData={() =>
                void localData.refreshDashboardAndHistory()
              }
              onLoadOlderHistory={() => void localData.loadOlderHistory()}
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
              vocabulary={localData.vocabulary}
              loading={localData.vocabularyLoading}
              error={localData.vocabularyError}
              pendingIds={localData.vocabularyPendingIds}
              native={dictation.native}
              onRefresh={() => void localData.refreshVocabulary()}
              onAdd={localData.createVocabulary}
              onUpdate={localData.updateVocabulary}
              onRemove={localData.deleteVocabulary}
            />
          )}
          {activeView === "settings" && (
            <SettingsView
              settings={settings}
              accessibilityError={accessibility.error ?? undefined}
              accessibilityPending={accessibility.pending}
              accessibilityStatus={accessibility.status}
              shortcutError={shortcut.error ?? undefined}
              shortcutPending={shortcut.pending}
              shortcutStatus={shortcut.status}
              settingsSaving={settingsSaving}
              nativeError={settingsError ?? undefined}
              clearError={localData.clearError ?? undefined}
              clearPendingScope={localData.clearPendingScope}
              lastClearResult={localData.lastClearResult}
              onChange={changeSettings}
              onShortcutChange={changeShortcut}
              onRefreshAccessibility={() => void accessibility.refresh()}
              onRequestAccessibility={() => void accessibility.request()}
              onRefreshShortcut={() => void shortcut.refresh()}
              onRequestInputMonitoring={() => void shortcut.request()}
              onRestartOnboarding={restartOnboarding}
              onClearLocalData={clearSavedLocalData}
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
          delivery={dictation.delivery}
          floating
          state={dictation.state}
          onStateChange={dictation.transitionTo}
          language={hudLanguage}
          shortcut={settings.hotkey}
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
