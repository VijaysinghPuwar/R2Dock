import { useEffect, useId, useState } from "react";

export interface NumberSettingProps {
  label: string;
  value: number;
  min: number;
  max: number;
  /** Text after the input, for example "MB" or "days". */
  unit?: string;
  helper?: string;
  onCommit: (value: number) => void;
}

/** Number input that commits a clamped value on blur or Enter. */
export function NumberSetting({ label, value, min, max, unit, helper, onCommit }: NumberSettingProps) {
  const id = useId();
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  const commit = () => {
    const parsed = Number.parseInt(draft, 10);
    if (Number.isNaN(parsed)) {
      setDraft(String(value));
      return;
    }
    const clamped = Math.min(max, Math.max(min, parsed));
    setDraft(String(clamped));
    if (clamped !== value) onCommit(clamped);
  };

  return (
    <div className="setting-row">
      <div className="setting-text">
        <label htmlFor={id}>{label}</label>
        {helper && <span className="small muted">{helper}</span>}
      </div>
      <div className="setting-number">
        <input
          id={id}
          className="input num"
          type="number"
          inputMode="numeric"
          min={min}
          max={max}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commit();
            }
          }}
        />
        {unit && <span className="secondary">{unit}</span>}
      </div>
    </div>
  );
}
