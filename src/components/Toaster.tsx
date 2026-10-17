import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { MAX_VISIBLE_TOASTS, TOAST_TIMEOUT_MS, useToastStore, type ToastItem } from "./toastStore";
import "./components.css";

function ToastView({ item }: { item: ToastItem }) {
  const dismiss = useToastStore((s) => s.dismiss);
  const [hovered, setHovered] = useState(false);
  const remaining = useRef(TOAST_TIMEOUT_MS);
  const autoDismiss = item.tone === "success" || item.tone === "neutral";

  useEffect(() => {
    if (!autoDismiss || hovered) return;
    const started = Date.now();
    const t = window.setTimeout(() => dismiss(item.id), remaining.current);
    return () => {
      window.clearTimeout(t);
      remaining.current = Math.max(1000, remaining.current - (Date.now() - started));
    };
  }, [autoDismiss, hovered, dismiss, item.id]);

  const Icon = item.tone === "success" ? CircleCheck : item.tone === "warning" ? TriangleAlert : item.tone === "danger" ? CircleAlert : Info;

  return (
    <div
      className={`toast toast-${item.tone}`}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      onFocus={() => setHovered(true)}
      onBlur={() => setHovered(false)}
    >
      <Icon size={16} className="toast-icon" aria-hidden="true" />
      <p className="toast-message">{item.message}</p>
      {item.action && (
        <button
          type="button"
          className="toast-action"
          onClick={() => {
            item.action?.run();
            dismiss(item.id);
          }}
        >
          {item.action.label}
        </button>
      )}
      <button type="button" className="toast-close" aria-label="Dismiss notification" onClick={() => dismiss(item.id)}>
        <X size={14} aria-hidden="true" />
      </button>
    </div>
  );
}

/** Visible toasts plus polite and assertive live regions. */
export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  const visible = toasts.slice(0, MAX_VISIBLE_TOASTS);
  const polite = visible.filter((t) => t.tone === "success" || t.tone === "neutral");
  const assertive = visible.filter((t) => t.tone === "warning" || t.tone === "danger");
  return (
    <>
      <div className="toaster" aria-label="Notifications">
        {visible.map((t) => (
          <ToastView key={t.id} item={t} />
        ))}
      </div>
      <div className="visually-hidden" role="status" aria-live="polite">
        {polite.map((t) => (
          <p key={t.id}>{t.message}</p>
        ))}
      </div>
      <div className="visually-hidden" role="alert" aria-live="assertive">
        {assertive.map((t) => (
          <p key={t.id}>{t.message}</p>
        ))}
      </div>
    </>
  );
}
