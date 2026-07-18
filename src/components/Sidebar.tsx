import {
  BarChart3,
  BookOpenText,
  Boxes,
  ChevronRight,
  Command,
  Settings2,
  ShieldCheck,
} from "lucide-react";
import type { ViewId } from "../types";
import { ShortcutKeys, SpickLogo } from "./Ui";

interface SidebarProps {
  activeView: ViewId;
  onNavigate: (view: ViewId) => void;
}

const navItems: Array<{ id: ViewId; label: string; icon: typeof BarChart3 }> = [
  { id: "today", label: "Today", icon: BarChart3 },
  { id: "engines", label: "Engines", icon: Boxes },
  { id: "vocabulary", label: "Vocabulary", icon: BookOpenText },
  { id: "settings", label: "Settings", icon: Settings2 },
];

export function Sidebar({ activeView, onNavigate }: SidebarProps) {
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
          <Command size={16} />
        </div>
        <div>
          <span>Dictation shortcut</span>
          <ShortcutKeys value="⌘+⇧+Space" />
        </div>
      </div>

      <div className="sidebar__footer">
        <div className="privacy-status">
          <ShieldCheck size={16} />
          <div>
            <strong>Local transcription</strong>
            <span>Automatic typing is next</span>
          </div>
          <span className="status-dot" aria-label="Early build" />
        </div>
      </div>
    </aside>
  );
}
