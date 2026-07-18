import { useState } from "react";
import { X } from "lucide-react";
import "./App.css";
import { DictationHud } from "./components/DictationHud";
import { Onboarding } from "./components/Onboarding";
import { Sidebar } from "./components/Sidebar";
import { TopBar } from "./components/TopBar";
import { initialEngines, initialVocabulary } from "./data/mockData";
import { useDictationController } from "./hooks/useDictationController";
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
  const [vocabulary, setVocabulary] =
    useState<VocabularyEntry[]>(initialVocabulary);
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const dictation = useDictationController({
    autoComplete: !hudOnly,
  });
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [onboardingComplete, setOnboardingComplete] = useState(
    () => window.localStorage.getItem("spick-onboarding-complete") === "true",
  );

  const navigate = (view: ViewId) => {
    setActiveView(view);
    setMobileNavOpen(false);
  };

  const activateEngine = (id: string) => {
    setEngines((current) =>
      current.map((engine) => {
        if (engine.id === id) return { ...engine, status: "active" };
        if (engine.status === "active") return { ...engine, status: "ready" };
        return engine;
      }),
    );
  };

  const installEngine = (id: string) => {
    setEngines((current) =>
      current.map((engine) =>
        engine.id === id ? { ...engine, status: "ready" } : engine,
      ),
    );
  };

  const removeEngine = (id: string) => {
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
          floating
          state={dictation.state}
          onStateChange={dictation.transitionTo}
          language="AUTO"
        />
      </div>
    );
  }

  if (!onboardingComplete) {
    return (
      <Onboarding
        settings={settings}
        onSettingsChange={setSettings}
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
              onHudStateChange={dictation.transitionTo}
            />
          )}
          {activeView === "engines" && (
            <EnginesView
              engines={engines}
              onActivate={activateEngine}
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
              onChange={setSettings}
              onRestartOnboarding={restartOnboarding}
            />
          )}
        </main>
      </div>
      {settings.showWidget && activeView !== "today" && (
        <DictationHud
          autoAdvance={false}
          floating
          state={dictation.state}
          onStateChange={dictation.transitionTo}
          language="AUTO"
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
