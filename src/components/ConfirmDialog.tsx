import { useId, useState, type ReactNode } from "react";
import { Button } from "./Button";
import { Dialog } from "./Dialog";
import { ProgressBar } from "./ProgressBar";

export interface ConfirmDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  title: ReactNode;
  body?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void | Promise<void>;
  /** Shown while the confirmed operation runs. */
  progress?: { done: number; total: number; label: string } | null;
  busy?: boolean;
}

export function ConfirmDialog({
  open,
  onOpenChange,
  title,
  body,
  confirmLabel,
  cancelLabel = "Cancel",
  danger = false,
  onConfirm,
  progress,
  busy = false,
}: ConfirmDialogProps) {
  const [running, setRunning] = useState(false);
  const working = busy || running;

  const confirm = async () => {
    setRunning(true);
    try {
      await onConfirm();
    } finally {
      setRunning(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      title={title}
      width="confirm"
      locked={working}
      footer={
        <>
          <Button variant="ghost" onClick={() => onOpenChange(false)} disabled={working}>
            {cancelLabel}
          </Button>
          <Button variant={danger ? "danger" : "primary"} onClick={() => void confirm()} loading={working}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      {body}
      {progress && (
        <div className="confirm-progress">
          <ProgressBar value={progress.done} max={progress.total} label={progress.label} />
          <span className="small secondary num">{progress.label}</span>
        </div>
      )}
    </Dialog>
  );
}

export interface TypedConfirmDialogProps extends Omit<ConfirmDialogProps, "onConfirm"> {
  /** The exact text the user must type. */
  phrase: string;
  onConfirm: (typed: string) => void | Promise<void>;
}

/** Destructive confirmation that stays disabled until the phrase matches exactly. */
export function TypedConfirmDialog({
  open,
  onOpenChange,
  title,
  body,
  confirmLabel,
  cancelLabel = "Cancel",
  phrase,
  onConfirm,
  progress,
  busy = false,
}: TypedConfirmDialogProps) {
  const [typed, setTyped] = useState("");
  const [running, setRunning] = useState(false);
  const inputId = useId();
  const working = busy || running;
  const matches = typed === phrase;

  const close = (next: boolean) => {
    if (!next) setTyped("");
    onOpenChange(next);
  };

  const confirm = async () => {
    if (!matches) return;
    setRunning(true);
    try {
      await onConfirm(typed);
    } finally {
      setRunning(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={close}
      title={title}
      width="confirm"
      locked={working}
      footer={
        <>
          <Button variant="ghost" onClick={() => close(false)} disabled={working}>
            {cancelLabel}
          </Button>
          <Button variant="danger" onClick={() => void confirm()} disabled={!matches} loading={working}>
            {confirmLabel}
          </Button>
        </>
      }
    >
      {body}
      <form
        className="field"
        onSubmit={(e) => {
          e.preventDefault();
          void confirm();
        }}
      >
        <label htmlFor={inputId} className="field-label">
          Type <span className="mono typed-phrase">{phrase}</span> to confirm
        </label>
        <input
          id={inputId}
          className="input mono"
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          disabled={working}
        />
      </form>
      {progress && (
        <div className="confirm-progress">
          <ProgressBar value={progress.done} max={progress.total} label={progress.label} />
          <span className="small secondary num">{progress.label}</span>
        </div>
      )}
    </Dialog>
  );
}
