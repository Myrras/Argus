/**
 * The settings tabs' shared controls: a switch row and a − value + stepper.
 * Both apply at once (the family's save model).
 */

import { AddIcon, MinusIcon } from "./icons";
import styles from "./SettingsModal.module.css";

/** Checkbox toggle row with label and optional helper text. */
export function SettingToggle({
  label,
  help,
  value,
  disabled,
  onChange,
}: {
  label: string;
  help?: string;
  value: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className={styles.row}>
      <span className={styles.rowLabel}>
        {label}
        {help && <small className={styles.rowHelp}>{help}</small>}
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={value}
        className={styles.toggle}
        data-on={value ? "true" : "false"}
        disabled={disabled}
        onClick={() => onChange(!value)}
        aria-label={label}
      />
    </div>
  );
}

/** A − value + control for a bounded whole number (`label` names it). */
export function Stepper({
  label,
  value,
  min,
  max,
  step = 1,
  show = String,
  onChange,
}: {
  label: string;
  value: number | null;
  min: number;
  max: number;
  /** How far one click moves (clamped to the bounds). */
  step?: number;
  show?: (value: number) => string;
  onChange: (value: number) => void;
}) {
  return (
    <div className={styles.stepper}>
      <button
        type="button"
        aria-label={`Decrease ${label.toLowerCase()}`}
        disabled={value === null || value <= min}
        onClick={() => value !== null && onChange(Math.max(min, value - step))}
      >
        <MinusIcon size={14} />
      </button>
      <span className={styles.stepperValue} aria-label={label}>
        {value === null ? "–" : show(value)}
      </span>
      <button
        type="button"
        aria-label={`Increase ${label.toLowerCase()}`}
        disabled={value === null || value >= max}
        onClick={() => value !== null && onChange(Math.min(max, value + step))}
      >
        <AddIcon size={14} />
      </button>
    </div>
  );
}
