import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUp, Folder } from "lucide-react";
import { useState, type FormEvent } from "react";
import { Button, IconButton } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Skeleton } from "../../components/Controls";
import { Dialog } from "../../components/Dialog";
import { TextField } from "../../components/Fields";
import { toast } from "../../components/toastStore";
import { errorMessage, movePreview, objectList } from "../../ipc/commands";
import type { MovePreview } from "../../ipc/bindings/MovePreview";
import type { OpProgress } from "../../ipc/bindings/OpProgress";
import { EMPTY_SEGMENT, parentPrefix, visibleText } from "../../lib/keys";
import { plural } from "../../lib/format";
import { qk } from "../../app/queries";
import { OpResultDialog, type OpResult } from "./OpResultDialog";
import { moveConfirmText, moveProblems, moveProgressLabel, runMove } from "./moveRunner";
import { useOpsStore, type MoveToRequest } from "./opsStore";
import "./manage.css";

/** Normalize a typed destination to "" (bucket root) or a prefix ending in "/". */
export function normalizeDest(input: string): string {
  let v = input.trim();
  while (v.startsWith("/")) v = v.slice(1);
  if (v === "") return "";
  return v.endsWith("/") ? v : `${v}/`;
}

export function destError(dest: string, req: Pick<MoveToRequest, "prefixes" | "fromPrefix">): string | null {
  if (dest === req.fromPrefix) return "The items are already in this folder.";
  if (req.prefixes.some((p) => dest.startsWith(p))) return "A folder cannot be moved into itself.";
  if (dest.split("/").some((seg, i, arr) => seg === ".." || (seg === "." && i < arr.length - 1))) return "Use a plain folder path.";
  return null;
}

function FolderPicker({ req, browse, onBrowse }: { req: MoveToRequest; browse: string; onBrowse: (prefix: string) => void }) {
  const query = useQuery({
    queryKey: [...qk.objects(req.connectionId, req.bucket, browse), "picker"],
    queryFn: () => objectList(req.connectionId, req.bucket, browse, null),
    staleTime: 30_000,
  });
  const folders = (query.data?.folders ?? []).filter((f) => !req.prefixes.includes(f.prefix));
  return (
    <div className="folder-picker">
      <div className="picker-head">
        <IconButton label="Up one folder" size="sm" icon={<ArrowUp size={14} />} onClick={() => onBrowse(parentPrefix(browse))} disabled={browse === ""} />
        <span className="mono truncate">
          {req.bucket}/{visibleText(browse)}
        </span>
      </div>
      <ul className="picker-list" aria-label="Folders">
        {query.isLoading &&
          Array.from({ length: 4 }, (_, i) => (
            <li key={i} className="picker-item">
              <Skeleton width={`${40 + i * 12}%`} />
            </li>
          ))}
        {query.isError && <li className="picker-item muted">{errorMessage(query.error)}</li>}
        {query.data && folders.length === 0 && <li className="picker-item muted">No folders here</li>}
        {folders.map((f) => (
          <li key={f.prefix}>
            <button type="button" className="picker-item picker-button" onClick={() => onBrowse(f.prefix)}>
              <Folder size={16} className="tint-folder" aria-hidden="true" />
              <span className="truncate">{f.displayName || EMPTY_SEGMENT}</span>
            </button>
          </li>
        ))}
        {query.data?.nextToken && <li className="picker-item muted small">More folders exist. Type the path to choose one not shown.</li>}
      </ul>
    </div>
  );
}

function MoveToDialogs({ req }: { req: MoveToRequest }) {
  const close = useOpsStore((s) => s.closeMoveTo);
  const client = useQueryClient();
  const [browse, setBrowse] = useState(req.fromPrefix);
  const [destText, setDestText] = useState(req.fromPrefix);
  const [submitted, setSubmitted] = useState(false);
  const [preview, setPreview] = useState<MovePreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [progress, setProgress] = useState<OpProgress | null>(null);
  const [result, setResult] = useState<OpResult | null>(null);

  const dest = normalizeDest(destText);
  const error = submitted ? destError(dest, req) : null;
  const count = req.keys.length + req.prefixes.length;

  const onBrowse = (prefix: string) => {
    setBrowse(prefix);
    setDestText(prefix);
  };

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
    if (destError(dest, req)) return;
    setBusy(true);
    try {
      setPreview(await movePreview(req.connectionId, req.bucket, req.keys, req.prefixes, dest, null));
    } catch (err) {
      toast.danger(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const execute = async (p: MovePreview) => {
    setBusy(true);
    try {
      const res = await runMove(client, req.connectionId, req.bucket, p.token, setProgress);
      const problems = moveProblems(res);
      if (problems) setResult(problems);
      else {
        toast.success(`Moved ${plural(res.moved, "object")} to ${visibleText(p.destination) || req.bucket}.`);
        close();
      }
    } catch (err) {
      toast.danger(errorMessage(err));
      close();
    } finally {
      setBusy(false);
    }
  };

  if (result) return <OpResultDialog result={result} onClose={close} />;

  if (preview) {
    return (
      <ConfirmDialog
        open
        onOpenChange={(open) => {
          if (!open && !busy) close();
        }}
        title="Move objects?"
        body={<p>{moveConfirmText(preview.objects, visibleText(preview.destination) || `${req.bucket} (top level)`)}</p>}
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
      title={`Move ${plural(count, "item")} to...`}
      width="form"
    >
      <form className="stack" onSubmit={(e) => void submit(e)}>
        <FolderPicker req={req} browse={browse} onBrowse={onBrowse} />
        <TextField
          label="Destination folder"
          mono
          placeholder="covers/2026/"
          value={destText}
          onChange={(e) => setDestText(e.target.value)}
          error={error}
          helper="Leave empty for the top level of the bucket."
          disabled={busy}
        />
        <div className="dialog-footer">
          <Button variant="ghost" onClick={close} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy}>
            Continue
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

export function MoveToFlow() {
  const req = useOpsStore((s) => s.moveTo);
  if (!req) return null;
  return <MoveToDialogs key={`${req.bucket}:${req.keys.join("|")}:${req.prefixes.join("|")}`} req={req} />;
}
