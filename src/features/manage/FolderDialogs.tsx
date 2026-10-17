import { useQueryClient } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { TextField } from "../../components/Fields";
import { toast } from "../../components/toastStore";
import { errorMessage, folderCreate } from "../../ipc/commands";
import { folderLabel, visibleText } from "../../lib/keys";
import { qk, useActiveConnection } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import { useUiStore } from "../../app/uiStore";
import { validateName } from "./RenameFlow";
import { normalizeDest } from "./MoveToFlow";

export function NewFolderDialog() {
  const open = useUiStore((s) => s.newFolderOpen);
  const setOpen = useUiStore((s) => s.setNewFolderOpen);
  const loc = useNavStore((s) => s.loc);
  const connection = useActiveConnection();
  if (!open || !connection || loc.view !== "browser" || !loc.bucket) return null;
  return <NewFolderForm connectionId={connection.id} bucket={loc.bucket} prefix={loc.prefix} onClose={() => setOpen(false)} />;
}

function NewFolderForm({ connectionId, bucket, prefix, onClose }: { connectionId: string; bucket: string; prefix: string; onClose: () => void }) {
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);
  const error = submitted ? validateName(name) : null;

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
    if (validateName(name)) return;
    setBusy(true);
    try {
      await folderCreate(connectionId, bucket, prefix, name.trim());
      await client.invalidateQueries({ queryKey: qk.objects(connectionId, bucket, prefix) });
      toast.success(`Created ${visibleText(name.trim())}.`);
      onClose();
    } catch (err) {
      toast.danger(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o && !busy) onClose();
      }}
      title="New folder"
      description={`In ${folderLabel(bucket, prefix)}`}
      width="confirm"
    >
      <form className="stack" onSubmit={(e) => void submit(e)}>
        <TextField label="Name" value={name} onChange={(e) => setName(e.target.value)} error={error} autoFocus disabled={busy} />
        <p className="small muted">R2 has no real folders. This adds an empty marker object so the folder shows up.</p>
        <div className="dialog-footer">
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy}>
            Create
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function GoToPathDialog() {
  const open = useUiStore((s) => s.goToPathOpen);
  const setOpen = useUiStore((s) => s.setGoToPathOpen);
  const loc = useNavStore((s) => s.loc);
  const [value, setValue] = useState("");
  if (!open || loc.view !== "browser" || !loc.bucket) return null;
  const bucket = loc.bucket;
  const close = () => {
    setOpen(false);
    setValue("");
  };
  const submit = (e: FormEvent) => {
    e.preventDefault();
    useNavStore.getState().openBucket(bucket, normalizeDest(value));
    close();
  };
  return (
    <Dialog
      open
      onOpenChange={(o) => {
        if (!o) close();
      }}
      title="Go to path"
      description={`Open a key prefix in ${bucket}.`}
      width="confirm"
    >
      <form className="stack" onSubmit={submit}>
        <TextField label="Key prefix" mono placeholder="covers/2026/" value={value} onChange={(e) => setValue(e.target.value)} autoFocus />
        <div className="dialog-footer">
          <Button variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button type="submit" variant="primary">
            Go
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
