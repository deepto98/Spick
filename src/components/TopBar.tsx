import { Keyboard, PanelLeft } from "lucide-react";
import type { ViewId } from "../types";
import { ShortcutKeys } from "./Ui";

const viewLabels: Record<ViewId, string> = {
  today: "Stats",
  notes: "Notes",
  engines: "Engines",
  vocabulary: "Vocabulary",
  settings: "Settings",
};

interface TopBarProps {
  activeView: ViewId;
  hotkey: string;
  onOpenNav: () => void;
}

export function TopBar({ activeView, hotkey, onOpenNav }: TopBarProps) {
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
          <Keyboard size={14} />
          <span>{hotkey === "⌥" ? "Tap or hold" : "Hold to speak"}</span>
          <ShortcutKeys value={hotkey} />
        </div>
      </div>
    </header>
  );
}
