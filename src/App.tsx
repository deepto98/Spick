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
import { useCloudProviders } from "./hooks/useCloudProviders";
import { useDictationController } from "./hooks/useDictationController";
import { useHudWindow } from "./hooks/useHudWindow";
import { useLocalData } from "./hooks/useLocalData";
import { useMicrophonePermission } from "./hooks/useMicrophonePermission";
import { useShortcutStatus } from "./hooks/useShortcutStatus";
import {
  listNativeAudioInputDevices,
  type NativeAudioInputDevice,
} from "./lib/nativeAudio";
import {
  activateLocalModel,
  cancelLocalModelInstall,
  formatModelBytes,
  importLocalModel,
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
import type { CloudProviderId } from "./lib/nativeCloud";
import type { AppSettings, Engine, TranscriptionSource, ViewId } from "./types";
import { EnginesView } from "./views/EnginesView";
import { SettingsView } from "./views/SettingsView";
import { TodayView } from "./views/TodayView";
import { VocabularyView } from "./views/VocabularyView";

const defaultSettings: AppSettings = {
  hotkey: "⌥",
  language: "Auto-detect",
  microphone: "System default microphone",
  showWidget: true,
  keepHistory: false,
  cloudFallback: false,
  cleanupLevel: "Verbatim",
};

const SYSTEM_DEFAULT_MICROPHONE = "System default microphone";

function App() {
  const hudOnly =
    new URLSearchParams(window.location.search).get("window") === "hud";
  const dictation = useDictationController(!hudOnly);
  const [activeView, setActiveView] = useState<ViewId>("today");
  const [engines, setEngines] = useState<Engine[]>(() =>
    dictation.native ? [] : initialEngines,
  );
  const [localModelsLoading, setLocalModelsLoading] = useState(
    dictation.native && !hudOnly,
  );
  const [modelDownloads, setModelDownloads] = useState<
    Record<string, ModelDownloadProgress>
  >({});
  const [modelActionPending, setModelActionPending] = useState<string | null>(
    null,
  );
  const [modelImportPending, setModelImportPending] = useState(false);
  const [cancellingModels, setCancellingModels] = useState<ReadonlySet<string>>(
    new Set(),
  );
  const cancelledModelDownloads = useRef(new Set<string>());
  const [modelError, setModelError] = useState<string | null>(null);
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [settingsLoadError, setSettingsLoadError] = useState<string | null>(
    null,
  );
  const [settingsLoading, setSettingsLoading] = useState(
    dictation.native && !hudOnly,
  );
  const [settingsSaving, setSettingsSaving] = useState(false);
  const [settingsLoadRevision, setSettingsLoadRevision] = useState(0);
  const [nativeSettings, setNativeSettings] =
    useState<NativeAppSettings | null>(null);
  const nativeSettingsRef = useRef<NativeAppSettings | null>(null);
  const settingsIntentRef = useRef<{
    languagePolicy: NativeLanguagePolicy;
    cleanupLevel: AppSettings["cleanupLevel"];
    inputDeviceName: string | null;
    hudVisible: boolean;
    allowCloudFallback: boolean;
    saveTranscriptHistory: boolean;
  }>({
    languagePolicy: { mode: "auto" },
    cleanupLevel: defaultSettings.cleanupLevel,
    inputDeviceName: null,
    hudVisible: true,
    allowCloudFallback: defaultSettings.cloudFallback,
    saveTranscriptHistory: defaultSettings.keepHistory,
  });
  const settingsSaveRevision = useRef(0);
  const settingsSaveQueue = useRef<Promise<void>>(Promise.resolve());
  const engineSelectionRevision = useRef(0);
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [audioInputDevices, setAudioInputDevices] = useState<
    NativeAudioInputDevice[]
  >([]);
  const [audioInputDevicesLoading, setAudioInputDevicesLoading] =
    useState(false);
  const [audioInputDevicesLoaded, setAudioInputDevicesLoaded] = useState(false);
  const [audioInputDevicesError, setAudioInputDevicesError] = useState<
    string | null
  >(null);
  const audioInputDevicesRequest = useRef(0);
  const localData = useLocalData(dictation.native && !hudOnly);
  const [cloudManagementEnabled, setCloudManagementEnabled] = useState(false);
  const cloudProviders = useCloudProviders(
    dictation.native && !hudOnly && cloudManagementEnabled,
  );
  const [hiddenEphemeralSessionId, setHiddenEphemeralSessionId] = useState<
    string | null
  >(null);
  const accessibility = useAccessibilityPermission(
    dictation.native && !hudOnly,
  );
  const microphonePermission = useMicrophonePermission(
    dictation.native && !hudOnly,
  );
  const shortcut = useShortcutStatus(dictation.native && !hudOnly);
  const hudWindow = useHudWindow(hudOnly && dictation.native);
  const audioFrame = useAudioLevel(dictation.state === "listening");
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [onboardingComplete, setOnboardingComplete] = useState(
    () => window.localStorage.getItem("spick-onboarding-complete") === "true",
  );
  const [engineSetupRequired, setEngineSetupRequired] = useState(false);

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
          inputDeviceName: saved.inputDeviceName,
          hudVisible: saved.hud.visible,
          allowCloudFallback: saved.allowCloudFallback,
          saveTranscriptHistory: saved.saveTranscriptHistory,
        };
      }
      setNativeSettings(saved);
      setSettings((current) => ({
        ...current,
        hotkey: shortcutDisplayName(saved.pushToTalkShortcut),
        microphone: saved.inputDeviceName ?? SYSTEM_DEFAULT_MICROPHONE,
        showWidget: saved.hud.visible,
        cloudFallback: saved.allowCloudFallback,
        keepHistory: saved.saveTranscriptHistory,
        language,
        cleanupLevel,
      }));
    },
    [],
  );

  useEffect(() => {
    if (!dictation.native) return;
    let disposed = false;
    void getNativeSettings()
      .then((saved) => {
        if (!disposed) {
          acceptNativeSettings(saved);
          setSettingsLoadError(null);
        }
      })
      .catch((reason) => {
        if (!disposed) {
          setSettingsLoadError(`Couldn’t read saved settings: ${reason}`);
        }
      })
      .finally(() => {
        if (!disposed && !hudOnly) setSettingsLoading(false);
      });
    return () => {
      disposed = true;
    };
  }, [acceptNativeSettings, dictation.native, hudOnly, settingsLoadRevision]);

  const refreshAudioInputDevices = useCallback(async () => {
    if (!dictation.native || hudOnly) return;
    const request = ++audioInputDevicesRequest.current;
    setAudioInputDevicesLoading(true);
    setAudioInputDevicesError(null);
    try {
      const devices = await listNativeAudioInputDevices();
      if (request !== audioInputDevicesRequest.current) return;
      setAudioInputDevices(devices);
      setAudioInputDevicesLoaded(true);
    } catch (reason) {
      if (request !== audioInputDevicesRequest.current) return;
      setAudioInputDevicesError(`Couldn’t list microphones: ${String(reason)}`);
    } finally {
      if (request === audioInputDevicesRequest.current) {
        setAudioInputDevicesLoading(false);
      }
    }
  }, [dictation.native, hudOnly]);

  useEffect(() => {
    if (activeView !== "settings" || !dictation.native || hudOnly) return;
    const timeout = window.setTimeout(() => {
      void refreshAudioInputDevices();
    }, 0);
    return () => window.clearTimeout(timeout);
  }, [activeView, dictation.native, hudOnly, refreshAudioInputDevices]);

  const refreshLocalEngines = useCallback(async () => {
    if (!dictation.native || hudOnly) return;
    setLocalModelsLoading(true);
    try {
      const catalog = await listLocalModels();
      const templates = new Map(
        initialEngines.map((engine) => [engine.id, engine]),
      );
      const local = catalog.map<Engine>((model) => {
        const template = templates.get(model.manifest.id);
        return {
          id: model.manifest.id,
          name: model.manifest.displayName,
          provider:
            model.manifest.origin === "imported"
              ? "whisper.cpp · imported"
              : "whisper.cpp",
          description:
            template?.description ??
            (model.manifest.origin === "imported"
              ? "Imported from a file you selected on this Mac."
              : "A verified local whisper.cpp model."),
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
              : model.state === "needsVerification"
                ? "Checks before the next use"
                : model.state === "invalid"
                  ? "Model file needs repair"
                  : model.manifest.origin === "imported"
                    ? "Model file is missing"
                    : "Not downloaded yet",
          usable:
            model.state === "installed" || model.state === "needsVerification",
          recommended: model.manifest.id === "whisper-small-multilingual-q5-1",
          origin: model.manifest.origin,
        };
      });
      setEngines(local);
    } finally {
      setLocalModelsLoading(false);
    }
  }, [dictation.native, hudOnly]);

  useEffect(() => {
    if (!dictation.native || hudOnly) return;
    const timeout = window.setTimeout(() => {
      void refreshLocalEngines().catch((reason) => {
        setModelError(`Couldn’t read local models: ${String(reason)}`);
      });
    }, 0);
    return () => window.clearTimeout(timeout);
  }, [dictation.native, hudOnly, refreshLocalEngines]);

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
      const selectionRevision = ++engineSelectionRevision.current;
      setModelActionPending(id);
      setModelError(null);
      void activateLocalModel(id)
        .then((saved) => {
          if (selectionRevision === engineSelectionRevision.current) {
            acceptNativeSettings(saved);
            cloudProviders.clearSelectedProvider();
          }
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

  const activateCloudEngine = async (provider: CloudProviderId) => {
    setModelError(null);
    const selectionRevision = ++engineSelectionRevision.current;
    const saved = await cloudProviders.activate(provider);
    if (!saved || selectionRevision !== engineSelectionRevision.current)
      return false;
    acceptNativeSettings(saved);
    try {
      await refreshLocalEngines();
    } catch (reason) {
      setModelError(
        `Cloud provider is active, but local models didn’t refresh: ${reason}`,
      );
    }
    return true;
  };

  const changeSettings = (next: AppSettings) => {
    if (!dictation.native) {
      setSettings(next);
      return;
    }

    const languageChanged = next.language !== settings.language;
    const cleanupChanged = next.cleanupLevel !== settings.cleanupLevel;
    const microphoneChanged = next.microphone !== settings.microphone;
    const hudVisibilityChanged = next.showWidget !== settings.showWidget;
    const cloudFallbackChanged = next.cloudFallback !== settings.cloudFallback;
    const transcriptHistoryChanged = next.keepHistory !== settings.keepHistory;
    if (
      !languageChanged &&
      !cleanupChanged &&
      !microphoneChanged &&
      !hudVisibilityChanged &&
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
    const inputDeviceName = microphoneChanged
      ? next.microphone === SYSTEM_DEFAULT_MICROPHONE
        ? null
        : next.microphone
      : intent.inputDeviceName;
    const hudVisible = hudVisibilityChanged
      ? next.showWidget
      : intent.hudVisible;
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
    // write is in flight, so a failed save never masquerades as success.
    setSettings((current) => ({
      ...next,
      language: current.language,
      cleanupLevel: current.cleanupLevel,
      microphone: current.microphone,
      showWidget: current.showWidget,
      cloudFallback: current.cloudFallback,
      keepHistory: current.keepHistory,
    }));
    settingsIntentRef.current = {
      languagePolicy,
      cleanupLevel: desiredCleanupLevel,
      inputDeviceName,
      hudVisible,
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
          inputDeviceName,
          hud: {
            ...current.hud,
            visible: hudVisible,
          },
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
  const selectedCloudProvider = cloudProviders.providers.find(
    (provider) =>
      provider.provider === nativeSettings?.transcriptionEngine.provider,
  );
  const selectedLocalEngine = engines.find(
    (engine) => engine.id === nativeSettings?.transcriptionEngine.model,
  );
  const selectedEngineName = nativeSettings
    ? (selectedCloudProvider?.modelName ??
      selectedLocalEngine?.name ??
      nativeSettings.transcriptionEngine.model)
    : null;
  const selectedEngineReady = nativeSettings
    ? nativeSettings.transcriptionEngine.location === "cloud"
      ? selectedCloudProvider?.configured === true
      : selectedLocalEngine?.usable === true
    : false;
  const selectedEngineChecking =
    dictation.native &&
    (settingsLoading ||
      nativeSettings === null ||
      (nativeSettings.transcriptionEngine.location === "cloud"
        ? cloudProviders.loading
        : localModelsLoading));
  const transcriptionSource: TranscriptionSource = !dictation.native
    ? "preview"
    : !nativeSettings
      ? "loading"
      : nativeSettings.transcriptionEngine.location === "cloud"
        ? "cloud"
        : settings.cloudFallback
          ? "localWithCloudFallback"
          : "local";

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

  const importEngine = () => {
    if (!dictation.native || modelImportPending) return;
    setModelImportPending(true);
    setModelError(null);
    void (async () => {
      let imported;
      try {
        imported = await importLocalModel();
      } catch (reason) {
        const importError = `Couldn’t import that model: ${String(reason)}`;
        setModelError(importError);
        try {
          // Import is content-addressed and registry persistence is its commit
          // point. Refresh even after an error in case only later bookkeeping
          // failed and the model is already present.
          await refreshLocalEngines();
        } catch (refreshReason) {
          setModelError(
            `${importError} The list also didn’t refresh: ${String(refreshReason)}`,
          );
        }
        return;
      }
      if (!imported) return;
      try {
        await refreshLocalEngines();
      } catch (reason) {
        setModelError(
          `Model imported, but the list didn’t refresh: ${String(reason)}`,
        );
      }
    })().finally(() => setModelImportPending(false));
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
      void (async () => {
        try {
          await removeLocalModel(id);
        } catch (reason) {
          setModelError(`Model removal needs attention: ${String(reason)}`);
          try {
            await refreshLocalEngines();
          } catch (refreshReason) {
            setModelError(
              `Model removal needs attention: ${String(reason)} The list also didn’t refresh: ${String(refreshReason)}`,
            );
          }
          return;
        }
        try {
          await refreshLocalEngines();
        } catch (reason) {
          setModelError(
            `Model removed, but the list didn’t refresh: ${String(reason)}`,
          );
        }
      })().finally(() => setModelActionPending(null));
      return;
    }
    setEngines((current) =>
      current.map((engine) =>
        engine.id === id ? { ...engine, status: "available" } : engine,
      ),
    );
  };

  const completeOnboarding = () => {
    if (selectedEngineChecking) return;
    if (selectedEngineReady) {
      window.localStorage.setItem("spick-onboarding-complete", "true");
      setEngineSetupRequired(false);
      setActiveView("today");
    } else {
      window.localStorage.removeItem("spick-onboarding-complete");
      setEngineSetupRequired(true);
      setActiveView("engines");
    }
    setOnboardingComplete(true);
  };

  const finishEngineSetup = () => {
    if (selectedEngineChecking || !selectedEngineReady) return;
    window.localStorage.setItem("spick-onboarding-complete", "true");
    setEngineSetupRequired(false);
    setActiveView("today");
  };

  const retrySettingsLoad = () => {
    setSettingsError(null);
    setSettingsLoadError(null);
    setSettingsLoading(true);
    setSettingsLoadRevision((revision) => revision + 1);
  };

  const retryLocalModels = () => {
    setModelError(null);
    void refreshLocalEngines().catch((reason) => {
      setModelError(`Couldn’t read local models: ${String(reason)}`);
    });
  };

  const restartOnboarding = () => {
    window.localStorage.removeItem("spick-onboarding-complete");
    setEngineSetupRequired(false);
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
          shortcut={settings.hotkey}
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
        microphoneError={microphonePermission.error ?? undefined}
        microphonePending={microphonePermission.pending}
        microphoneStatus={microphonePermission.status}
        shortcutError={shortcut.error ?? undefined}
        shortcutPending={shortcut.pending}
        shortcutStatus={shortcut.status}
        settings={settings}
        settingsError={settingsLoadError ?? settingsError ?? undefined}
        settingsReady={settingsReady}
        settingsSaving={settingsSaving}
        transcriptionSource={transcriptionSource}
        engineName={selectedEngineName}
        engineReady={selectedEngineReady}
        engineChecking={selectedEngineChecking}
        onRefreshAccessibility={() => void accessibility.refresh()}
        onRequestAccessibility={() => void accessibility.request()}
        onRefreshMicrophone={() => void microphonePermission.refresh()}
        onRequestMicrophone={() => void microphonePermission.request()}
        onRefreshShortcut={() => void shortcut.refresh()}
        onRequestInputMonitoring={() => void shortcut.request()}
        onRetrySettings={retrySettingsLoad}
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
          transcriptionSource={transcriptionSource}
          engineName={selectedEngineName}
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
              lastLatency={dictation.lastLatency}
              lastTranscript={dictation.lastTranscript}
              hiddenEphemeralSessionId={hiddenEphemeralSessionId}
              language={hudLanguage}
              native={dictation.native}
              shortcut={settings.hotkey}
              shortcutStatus={shortcut.status}
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
              importPending={modelImportPending}
              localLoading={localModelsLoading}
              error={modelError ?? undefined}
              cloudProviders={cloudProviders.providers}
              cloudLoading={cloudProviders.loading}
              cloudPending={cloudProviders.pending}
              cloudError={cloudProviders.error ?? undefined}
              cloudFallbackEnabled={settings.cloudFallback}
              setupRequired={engineSetupRequired}
              setupReady={selectedEngineReady}
              setupChecking={selectedEngineChecking}
              onActivate={activateEngine}
              onCancelInstall={cancelEngineInstall}
              onInstall={installEngine}
              onImport={importEngine}
              onRemove={removeEngine}
              onLocalRefresh={retryLocalModels}
              onCloudRefresh={() => void cloudProviders.refresh()}
              onCloudOpen={() => setCloudManagementEnabled(true)}
              onCloudConfigure={cloudProviders.configure}
              onCloudDelete={cloudProviders.removeCredential}
              onCloudActivate={activateCloudEngine}
              onFinishSetup={finishEngineSetup}
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
              native={dictation.native}
              audioInputDevices={audioInputDevices}
              audioInputDevicesLoaded={audioInputDevicesLoaded}
              audioInputDevicesLoading={audioInputDevicesLoading}
              audioInputDevicesError={audioInputDevicesError ?? undefined}
              accessibilityError={accessibility.error ?? undefined}
              accessibilityPending={accessibility.pending}
              accessibilityStatus={accessibility.status}
              microphonePermissionError={
                microphonePermission.error ?? undefined
              }
              microphonePermissionPending={microphonePermission.pending}
              microphonePermissionStatus={microphonePermission.status}
              shortcutError={shortcut.error ?? undefined}
              shortcutPending={shortcut.pending}
              shortcutStatus={shortcut.status}
              settingsAcknowledged={
                !dictation.native || nativeSettings !== null
              }
              settingsLoading={settingsLoading}
              settingsSaving={settingsSaving}
              nativeError={settingsLoadError ?? settingsError ?? undefined}
              nativeErrorTitle={
                settingsLoadError
                  ? "Couldn’t load saved settings"
                  : "Couldn’t save that change"
              }
              clearError={localData.clearError ?? undefined}
              clearPendingScope={localData.clearPendingScope}
              lastClearResult={localData.lastClearResult}
              onChange={changeSettings}
              onShortcutChange={changeShortcut}
              onRefreshAccessibility={() => void accessibility.refresh()}
              onRequestAccessibility={() => void accessibility.request()}
              onRequestMicrophone={() => void microphonePermission.request()}
              onRefreshShortcut={() => void shortcut.refresh()}
              onRequestInputMonitoring={() => void shortcut.request()}
              onRestartOnboarding={restartOnboarding}
              onRetryNativeSettings={
                settingsLoadError ? retrySettingsLoad : undefined
              }
              onRefreshAudioInputDevices={() => void refreshAudioInputDevices()}
              onClearLocalData={clearSavedLocalData}
            />
          )}
        </main>
      </div>
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
