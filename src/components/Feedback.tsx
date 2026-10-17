import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { clipboardWrite, errorMessage, openActivityFolder } from "../ipc/commands";
import { Check, Copy } from "lucide-react";
import { Button, IconButton } from "./Button";
import { toast } from "./toastStore";
import "./components.css";

/* Empty and error states */

export interface EmptyStateProps {
  icon: ReactNode;
  title: ReactNode;
  body?: ReactNode;
  actions?: ReactNode;
  compact?: boolean;
}

export function EmptyState({ icon, title, body, actions, compact }: EmptyStateProps) {
  return (
    <div className={compact ? "empty-state is-compact" : "empty-state"}>
      <span className="empty-icon" aria-hidden="true">
        {icon}
      </span>
      <h2 className="empty-title">{title}</h2>
      {body && <p className="empty-body">{body}</p>}
      {actions && <div className="empty-actions">{actions}</div>}
    </div>
  );
}

/* Banner */

export type BannerTone = "info" | "warning" | "danger" | "success";

export function Banner({
  tone,
  children,
  actions,
  onDismiss,
}: {
  tone: BannerTone;
  children: ReactNode;
  actions?: ReactNode;
  onDismiss?: () => void;
}) {
  const Icon = tone === "info" ? Info : tone === "success" ? CircleCheck : tone === "warning" ? TriangleAlert : CircleAlert;
  return (
    <div className={`banner banner-${tone}`} role={tone === "danger" || tone === "warning" ? "alert" : "status"}>
      <Icon size={16} className="banner-icon" aria-hidden="true" />
      <div className="banner-text">{children}</div>
      {actions && <div className="banner-actions">{actions}</div>}
      {onDismiss && <IconButton label="Dismiss" size="sm" icon={<X size={14} />} onClick={onDismiss} />}
    </div>
  );
}

/* Inline result line, for connection tests and similar. */

export function InlineResult({
  tone,
  children,
  detail,
}: {
  tone: "success" | "warning" | "danger" | "info";
  children: ReactNode;
  detail?: string | null;
}) {
  const [open, setOpen] = useState(false);
  const Icon = tone === "success" ? CircleCheck : tone === "info" ? Info : tone === "warning" ? TriangleAlert : CircleAlert;
  return (
    <div className={`inline-result inline-result-${tone}`} role={tone === "danger" ? "alert" : "status"}>
      <Icon size={16} aria-hidden="true" className="inline-result-icon" />
      <div className="inline-result-body">
        <p>{children}</p>
        {detail && (
          <>
            <button type="button" className="link-button small" aria-expanded={open} onClick={() => setOpen((v) => !v)}>
              {open ? "Hide technical details" : "Show technical details"}
            </button>
            {open && (
              <>
                <pre className="tech-detail mono selectable">{detail}</pre>
                <OpenActivityLogButton />
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

/** Opens the activity log folder, so the user can find the row for an error. */
export function OpenActivityLogButton() {
  const open = () => {
    openActivityFolder().catch((e: unknown) => toast.danger(errorMessage(e)));
  };
  return (
    <button type="button" className="link-button small open-activity-log" onClick={open}>
      Open activity log
    </button>
  );
}

/* Copy button */

export function CopyButton({ text, label = "Copy", size = "sm" }: { text: string; label?: string; size?: "sm" | "md" }) {
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(t);
  }, [copied]);
  const copy = async () => {
    try {
      await clipboardWrite(text);
      setCopied(true);
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };
  return (
    <>
      <IconButton
        label={copied ? "Copied" : label}
        size={size}
        icon={copied ? <Check size={14} /> : <Copy size={14} />}
        onClick={() => void copy()}
      />
      <span className="visually-hidden" aria-live="polite">
        {copied ? "Copied" : ""}
      </span>
    </>
  );
}

/** Retry button used in error states. */
export function RetryButton({ onRetry, label = "Try again" }: { onRetry: () => void; label?: string }) {
  return <Button onClick={onRetry}>{label}</Button>;
}
