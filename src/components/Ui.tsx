import type { ReactNode } from "react";
import { AudioWaveform, ChevronDown } from "lucide-react";

export function SpickLogo({ compact = false }: { compact?: boolean }) {
  return (
    <div
      className={`brand ${compact ? "brand--compact" : ""}`}
      aria-label="Spick"
    >
      <span className="brand__mark" aria-hidden="true">
        <AudioWaveform size={18} strokeWidth={2.4} />
      </span>
      {!compact && <span className="brand__name">Spick</span>}
    </div>
  );
}

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  disabled?: boolean;
}

export function Toggle({
  checked,
  onChange,
  label,
  disabled = false,
}: ToggleProps) {
  return (
    <button
      type="button"
      className={`toggle ${checked ? "toggle--on" : ""}`}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
    >
      <span className="toggle__thumb" />
    </button>
  );
}

interface PageHeaderProps {
  eyebrow?: string;
  title: string;
  description: string;
  actions?: ReactNode;
}

export function PageHeader({
  eyebrow,
  title,
  description,
  actions,
}: PageHeaderProps) {
  return (
    <header className="page-header">
      <div>
        {eyebrow && <span className="page-header__eyebrow">{eyebrow}</span>}
        <h1>{title}</h1>
        <p>{description}</p>
      </div>
      {actions && <div className="page-header__actions">{actions}</div>}
    </header>
  );
}

interface SelectFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: string[];
  hint?: string;
  disabled?: boolean;
}

export function SelectField({
  label,
  value,
  onChange,
  options,
  hint,
  disabled = false,
}: SelectFieldProps) {
  return (
    <label className="field">
      <span className="field__label">{label}</span>
      <span className="select-wrap">
        <select
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.currentTarget.value)}
        >
          {options.map((option) => (
            <option key={option} value={option}>
              {option}
            </option>
          ))}
        </select>
        <ChevronDown size={15} aria-hidden="true" />
      </span>
      {hint && <span className="field__hint">{hint}</span>}
    </label>
  );
}

interface SettingRowProps {
  icon: ReactNode;
  title: string;
  description: string;
  control: ReactNode;
}

export function SettingRow({
  icon,
  title,
  description,
  control,
}: SettingRowProps) {
  return (
    <div className="setting-row">
      <span className="setting-row__icon" aria-hidden="true">
        {icon}
      </span>
      <div className="setting-row__copy">
        <strong>{title}</strong>
        <span>{description}</span>
      </div>
      <div className="setting-row__control">{control}</div>
    </div>
  );
}

export function ShortcutKeys({ value }: { value: string }) {
  const keys = value.split("+");
  return (
    <span className="shortcut-keys" aria-label={`Shortcut ${value}`}>
      {keys.map((key) => (
        <kbd key={key}>{key}</kbd>
      ))}
    </span>
  );
}
