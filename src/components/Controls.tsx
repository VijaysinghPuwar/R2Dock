import { useId, useRef, type KeyboardEvent, type ReactNode } from "react";
import "./components.css";

/* Tabs */

export interface TabItem<T extends string> {
  value: T;
  label: string;
  count?: number;
}

export interface TabsProps<T extends string> {
  value: T;
  onChange: (value: T) => void;
  items: Array<TabItem<T>>;
  label: string;
  idPrefix: string;
  className?: string;
}

/** Tab list with arrow-key navigation. Panels use id `${idPrefix}-panel-${value}`. */
export function Tabs<T extends string>({ value, onChange, items, label, idPrefix, className }: TabsProps<T>) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const idx = items.findIndex((i) => i.value === value);
    let next = -1;
    if (e.key === "ArrowRight") next = (idx + 1) % items.length;
    else if (e.key === "ArrowLeft") next = (idx - 1 + items.length) % items.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = items.length - 1;
    if (next < 0) return;
    e.preventDefault();
    const item = items[next];
    if (item) {
      onChange(item.value);
      refs.current[next]?.focus();
    }
  };
  return (
    <div role="tablist" aria-label={label} className={["tabs", className].filter(Boolean).join(" ")} onKeyDown={onKeyDown}>
      {items.map((item, i) => {
        const selected = item.value === value;
        return (
          <button
            key={item.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="tab"
            id={`${idPrefix}-tab-${item.value}`}
            aria-selected={selected}
            aria-controls={`${idPrefix}-panel-${item.value}`}
            tabIndex={selected ? 0 : -1}
            className={selected ? "tab is-selected" : "tab"}
            onClick={() => onChange(item.value)}
          >
            {item.label}
            {item.count !== undefined && <span className="tab-count num">{item.count.toLocaleString("en-US")}</span>}
          </button>
        );
      })}
    </div>
  );
}

/* Segmented control */

export interface SegmentOption<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
  /** Render icon only; the label becomes the accessible name. */
  iconOnly?: boolean;
}

export function SegmentedControl<T extends string>({
  value,
  onChange,
  options,
  label,
  size = "md",
}: {
  value: T;
  onChange: (value: T) => void;
  options: Array<SegmentOption<T>>;
  label: string;
  size?: "sm" | "md";
}) {
  const refs = useRef<Array<HTMLButtonElement | null>>([]);
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const idx = options.findIndex((o) => o.value === value);
    let next = -1;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") next = (idx + 1) % options.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") next = (idx - 1 + options.length) % options.length;
    if (next < 0) return;
    e.preventDefault();
    const opt = options[next];
    if (opt) {
      onChange(opt.value);
      refs.current[next]?.focus();
    }
  };
  return (
    <div role="radiogroup" aria-label={label} className={`segmented segmented-${size}`} onKeyDown={onKeyDown}>
      {options.map((o, i) => {
        const checked = o.value === value;
        return (
          <button
            key={o.value}
            ref={(el) => {
              refs.current[i] = el;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            aria-label={o.iconOnly ? o.label : undefined}
            title={o.iconOnly ? o.label : undefined}
            tabIndex={checked ? 0 : -1}
            className={checked ? "segment is-checked" : "segment"}
            onClick={() => onChange(o.value)}
          >
            {o.icon}
            {!o.iconOnly && <span>{o.label}</span>}
          </button>
        );
      })}
    </div>
  );
}

/* Switch */

export function Switch({
  checked,
  onChange,
  label,
  description,
  disabled,
}: {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <div className={disabled ? "switch-row is-disabled" : "switch-row"}>
      <span className="switch-text">
        <label id={`${id}-label`} htmlFor={id}>
          {label}
        </label>
        {description && (
          <span id={`${id}-desc`} className="small muted">
            {description}
          </span>
        )}
      </span>
      <button
        id={id}
        type="button"
        role="switch"
        aria-checked={checked}
        aria-labelledby={`${id}-label`}
        aria-describedby={description ? `${id}-desc` : undefined}
        className={checked ? "switch is-on" : "switch"}
        onClick={() => onChange(!checked)}
        disabled={disabled}
      >
        <span className="switch-thumb" />
      </button>
    </div>
  );
}

/* Badge and Kbd */

export type BadgeTone = "neutral" | "accent" | "success" | "warning" | "danger" | "info";

export function Badge({ children, tone = "neutral", title }: { children: ReactNode; tone?: BadgeTone; title?: string }) {
  return (
    <span className={`badge badge-${tone}`} title={title}>
      {children}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="kbd">{children}</kbd>;
}

/* Skeleton */

export function Skeleton({ width, height = 12 }: { width?: number | string; height?: number }) {
  return <span className="skeleton" style={{ width: width ?? "100%", height }} aria-hidden="true" />;
}

/* Status dot */

export function StatusDot({ tone, label }: { tone: "success" | "warning" | "muted" | "danger"; label?: string }) {
  return (
    <span className={`status-dot status-dot-${tone}`} role={label ? "img" : undefined} aria-label={label} aria-hidden={label ? undefined : true} />
  );
}
