import {
  BarChart3,
  BookOpenText,
  Boxes,
  ChevronRight,
  Keyboard,
  Settings2,
  ShieldCheck,
} from "lucide-react";
import type { TranscriptionSource, ViewId } from "../types";
import { ShortcutKeys, SpickLogo } from "./Ui";

interface SidebarProps {
  activeView: ViewId;
  hotkey: string;
  transcriptionSource: TranscriptionSource;
  engineName?: string | null;
  onNavigate: (view: ViewId) => void;
}

const navItems: Array<{ id: ViewId; label: string; icon: typeof BarChart3 }> = [
  { id: "today", label: "Stats", icon: BarChart3 },
  { id: "engines", label: "Engines", icon: Boxes },
  { id: "vocabulary", label: "Vocabulary", icon: BookOpenText },
  { id: "settings", label: "Settings", icon: Settings2 },
];

function transcriptionStatus(
  source: TranscriptionSource,
  engineName?: string | null,
) {
  switch (source) {
    case "cloud":
      return {
        title: "Cloud transcription",
        detail: engineName ?? "Selected cloud provider",
      };
    case "localWithCloudFallback":
      return {
        title: "Local first",
        detail: `${engineName ?? "Local model"} · fallback on`,
      };
    case "local":
      return {
        title: "Local transcription",
        detail: engineName ?? "Blocks protected fields",
      };
    case "loading":
      return {
        title: "Checking engine",
        detail: "Loading saved settings",
      };
    case "preview":
      return {
        title: "Browser preview",
        detail: "Development app required",
      };
  }
}

export function Sidebar({
  activeView,
  hotkey,
  transcriptionSource,
  engineName,
  onNavigate,
}: SidebarProps) {
  const status = transcriptionStatus(transcriptionSource, engineName);

  return (
    <aside className="sidebar">
      <div className="sidebar__top">
        <SpickLogo />
        <span className="sidebar__version">EARLY BUILD</span>
      </div>

      <nav className="sidebar__nav" aria-label="Main navigation">
        <span className="sidebar__label">Go to</span>
        {navItems.map((item) => {
          const Icon = item.icon;
          return (
            <button
              type="button"
              key={item.id}
              className={`nav-item ${activeView === item.id ? "nav-item--active" : ""}`}
              aria-current={activeView === item.id ? "page" : undefined}
              onClick={() => onNavigate(item.id)}
            >
              <Icon size={17} strokeWidth={1.9} />
              <span>{item.label}</span>
              {activeView === item.id && (
                <ChevronRight className="nav-item__chevron" size={14} />
              )}
            </button>
          );
        })}
      </nav>

      <div className="sidebar__spacer" />

      <div className="shortcut-card">
        <div className="shortcut-card__icon">
          <Keyboard size={16} />
        </div>
        <div>
          <span>Dictation shortcut</span>
          <ShortcutKeys value={hotkey} />
        </div>
      </div>

      <div className="sidebar__footer">
        <div className="privacy-status">
          <ShieldCheck size={16} />
          <div>
            <strong>{status.title}</strong>
            <span>{status.detail}</span>
          </div>
          <span className="status-dot" aria-label="Early build" />
        </div>
      </div>
    </aside>
  );
}
