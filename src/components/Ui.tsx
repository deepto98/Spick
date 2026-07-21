import { useId, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";

export function SpickLogo({ compact = false }: { compact?: boolean }) {
  return (
    <div
      className={`brand ${compact ? "brand--compact" : ""}`}
      aria-label="Spick"
    >
      <span className="brand__mark" aria-hidden="true">
        <img src="/spick-mark.png" alt="" />
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

export type SelectFieldOption =
  string | { value: string; label: string; disabled?: boolean };

interface SelectFieldProps {
  label: string;
  value: string;
  onChange: (value: string) => void;
  options: SelectFieldOption[];
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
  const selectId = useId();
  const hintId = hint ? `${selectId}-hint` : undefined;

  return (
    <div className="field">
      <label className="field__label" htmlFor={selectId}>
        {label}
      </label>
      <span className="select-wrap">
        <select
          id={selectId}
          value={value}
          disabled={disabled}
          aria-describedby={hintId}
          onChange={(event) => onChange(event.currentTarget.value)}
        >
          {options.map((option) => {
            const value = typeof option === "string" ? option : option.value;
            const label = typeof option === "string" ? option : option.label;
            const optionDisabled =
              typeof option === "string" ? false : option.disabled;
            return (
              <option key={value} value={value} disabled={optionDisabled}>
                {label}
              </option>
            );
          })}
        </select>
        <ChevronDown size={15} aria-hidden="true" />
      </span>
      {hint && (
        <span className="field__hint" id={hintId}>
          {hint}
        </span>
      )}
    </div>
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
