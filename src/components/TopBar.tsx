import { Bell, CircleHelp, Command, PanelLeft } from "lucide-react";
import type { ViewId } from "../types";
import { ShortcutKeys } from "./Ui";

const viewLabels: Record<ViewId, string> = {
  today: "Today",
  engines: "Engines",
  vocabulary: "Vocabulary",
  settings: "Settings",
};

interface TopBarProps {
  activeView: ViewId;
  onOpenNav: () => void;
}

export function TopBar({ activeView, onOpenNav }: TopBarProps) {
  return (
    <header className="topbar">
      <button
        type="button"
        className="icon-button topbar__menu"
        onClick={onOpenNav}
        aria-label="Open navigation"
      >
        <PanelLeft size={18} />
      </button>
      <div className="topbar__crumb">
        <span>Spick</span>
        <i>/</i>
        <strong>{viewLabels[activeView]}</strong>
      </div>
      <div className="topbar__actions">
        <div className="topbar__hint">
          <Command size={14} />
          <span>Interaction preview</span>
          <ShortcutKeys value="⌘+⇧+Space" />
        </div>
        <button type="button" className="icon-button" aria-label="Help">
          <CircleHelp size={18} />
        </button>
        <button
          type="button"
          className="icon-button icon-button--notification"
          aria-label="Notifications"
        >
          <Bell size={18} />
          <span />
        </button>
        <button type="button" className="avatar" aria-label="Account menu">
          D
        </button>
      </div>
    </header>
  );
}
