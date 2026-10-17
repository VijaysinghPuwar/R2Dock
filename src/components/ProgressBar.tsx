import "./components.css";

export type ProgressTone = "accent" | "warning" | "danger" | "success";

export interface ProgressBarProps {
  value: number;
  max: number;
  /** Text equivalent for screen readers, for example "42%". */
  label: string;
  tone?: ProgressTone;
  thin?: boolean;
  indeterminate?: boolean;
  className?: string;
}

export function ProgressBar({ value, max, label, tone = "accent", thin = false, indeterminate = false, className }: ProgressBarProps) {
  const pct = max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0;
  return (
    <div
      className={["progress", thin ? "progress-thin" : "", `progress-${tone}`, indeterminate ? "is-indeterminate" : "", className]
        .filter(Boolean)
        .join(" ")}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indeterminate ? undefined : Math.floor(pct)}
      aria-valuetext={label}
    >
      <div className="progress-fill" style={indeterminate ? undefined : { transform: `scaleX(${pct / 100})` }} />
    </div>
  );
}
