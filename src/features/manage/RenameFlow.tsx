import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { Button } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Dialog } from "../../components/Dialog";
import { TextField } from "../../components/Fields";
import { ProgressBar } from "../../components/ProgressBar";
import { toast } from "../../components/toastStore";
import { errorMessage, movePreview } from "../../ipc/commands";
import type { MovePreview } from "../../ipc/bindings/MovePreview";
import type { OpProgress } from "../../ipc/bindings/OpProgress";
import { parentPrefix, visibleText } from "../../lib/keys";
import { usePreferences, useUpdatePreferences } from "../../app/queries";
import { OpResultDialog, type OpResult } from "./OpResultDialog";
import { moveConfirmText, moveProblems, moveProgressLabel, runMove } from "./moveRunner";
import { useOpsStore, type RenameRequest } from "./opsStore";

type Step = "edit" | "notice" | "confirm" | "result";

export function validateName(name: string): string | null {
  const trimmed = name.trim();
  if (!trimmed) return "Enter a name.";
  if (trimmed.includes("/")) return "Names cannot contain /.";
  if (trimmed === "." || trimmed === "..") return "Choose a different name.";
  return null;
}

function RenameDialogs({ req }: { req: RenameRequest }) {
  const close = useOpsStore((s) => s.closeRename);
  const client = useQueryClient();
  const prefs = usePreferences();
  const updatePrefs = useUpdatePreferences();
  const target = req.target;
  const isFolder = target.kind === "folder";
  const [name, setName] = useState(target.name);
  const [submitted, setSubmitted] = useState(false);
  const [step, setStep] = useState<Step>("edit");
  const [dontShow, setDontShow] = useState(true);
  const [preview, setPreview] = useState<MovePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<OpProgress | null>(null);
  const [result, setResult] = useState<OpResult | null>(null);

  const error = submitted ? validateName(name) : null;
  const unchanged = name.trim() === target.name;
  const sourceKey = target.kind === "file" ? target.key : target.prefix;
  const dest = parentPrefix(sourceKey);

  const prepare = async (): Promise<MovePreview | null> => {
    try {
      const p = await movePreview(
        req.connectionId,
        req.bucket,
        target.kind === "file" ? [target.key] : [],
        target.kind === "folder" ? [target.prefix] : [],
        dest,
        name.trim(),
      );
      setPreview(p);
      return p;
    } catch (e) {
      toast.danger(errorMessage(e));
      return null;
    }
  };

  const execute = async (p: MovePreview) => {
    setBusy(true);
    try {
      const res = await runMove(client, req.connectionId, req.bucket, p.token, setProgress);
      const problems = moveProblems(res);
      if (problems) {
        setResult(problems);
        setStep("result");
      } else {
        toast.success(isFolder ? `Renamed folder to ${visibleText(name.trim())}.` : `Renamed to ${visibleText(name.trim())}.`);
        close();
      }
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
    if (validateName(name) || unchanged) {
      if (unchanged) close();
      return;
    }
    setBusy(true);
    const p = await prepare();
    setBusy(false);
    if (!p) return;
    if (isFolder) setStep("confirm");
    else if (!prefs.renameNoticeSeen) setStep("notice");
    else await execute(p);
  };

  if (step === "result" && result) return <OpResultDialog result={result} onClose={close} />;

  if (step === "notice" && preview) {
    return (
      <Dialog
        open
        onOpenChange={(open) => {
          if (!open && !busy) close();
        }}
        title={`Rename to ${visibleText(name.trim())}?`}
        width="confirm"
        locked={busy}
        footer={
          <>
            <Button variant="ghost" onClick={close} disabled={busy}>
              Cancel
            </Button>
            <Button
              variant="primary"
              loading={busy}
              onClick={() => {
                if (dontShow) updatePrefs({ renameNoticeSeen: true });
                void execute(preview);
              }}
            >
              Rename
            </Button>
          </>
        }
      >
        <p>Renaming copies the file to the new name, then deletes the original.</p>
        <label className="radio-option">
          <input type="checkbox" className="checkbox" checked={dontShow} onChange={(e) => setDontShow(e.target.checked)} />
          Do not show again
        </label>
        {busy && progress && (
          <ProgressBar value={progress.done} max={progress.total} label={moveProgressLabel(progress)?.label ?? ""} />
        )}
      </Dialog>
    );
  }

  if (step === "confirm" && preview) {
    return (
      <ConfirmDialog
        open
        onOpenChange={(open) => {
          if (!open && !busy) close();
        }}
        title={`Rename folder to ${visibleText(name.trim())}?`}
        body={<p>{moveConfirmText(preview.objects, visibleText(preview.destination))}</p>}
        confirmLabel="Move"
        busy={busy}
        progress={busy ? moveProgressLabel(progress) : null}
        onConfirm={() => execute(preview)}
      />
    );
  }

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) close();
      }}
      title={isFolder ? "Rename folder" : "Rename file"}
      width="confirm"
    >
      <form className="stack" onSubmit={(e) => void submit(e)}>
        <TextField
          label="New name"
          value={name}
          onChange={(e) => setName(e.target.value)}
          error={error}
          autoFocus
          onFocus={(e) => {
            const v = e.target.value;
            const dot = v.lastIndexOf(".");
            if (!isFolder && dot > 0) e.target.setSelectionRange(0, dot);
            else e.target.select();
          }}
          disabled={busy}
        />
        {isFolder && (
          <p className="small secondary">R2 has no real folders. Renaming copies every object inside, then deletes the originals.</p>
        )}
        <div className="dialog-footer">
          <Button variant="ghost" onClick={close} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy}>
            Rename
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function RenameFlow() {
  const req = useOpsStore((s) => s.rename);
  if (!req) return null;
  const id = req.target.kind === "file" ? req.target.key : req.target.prefix;
  return <RenameDialogs key={`${req.bucket}:${id}`} req={req} />;
}
