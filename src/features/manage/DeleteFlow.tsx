import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { ConfirmDialog, TypedConfirmDialog } from "../../components/ConfirmDialog";
import { toast } from "../../components/toastStore";
import { deleteExecute, deletePreview, errorMessage } from "../../ipc/commands";
import type { DeletePreview } from "../../ipc/bindings/DeletePreview";
import type { OpProgress } from "../../ipc/bindings/OpProgress";
import { formatBytes, formatCount, plural } from "../../lib/format";
import { visibleText } from "../../lib/keys";
import { qk } from "../../app/queries";
import { useBrowserStore } from "../browser/browserStore";
import { OpResultDialog, type OpResult } from "./OpResultDialog";
import { useOpsStore, type DeleteRequest } from "./opsStore";

function progressLabel(p: OpProgress | null): { done: number; total: number; label: string } | null {
  if (!p) return null;
  return { done: p.done, total: p.total, label: `Deleted ${formatCount(p.done)} of ${formatCount(p.total)}` };
}

function DeleteDialogs({ req }: { req: DeleteRequest }) {
  const close = useOpsStore((s) => s.closeDelete);
  const client = useQueryClient();
  const [preview, setPreview] = useState<DeletePreview | null>(null);
  const [progress, setProgress] = useState<OpProgress | null>(null);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<OpResult | null>(null);

  useEffect(() => {
    let cancelled = false;
    deletePreview(req.connectionId, req.bucket, req.keys, req.prefixes)
      .then((p) => {
        if (!cancelled) setPreview(p);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        toast.danger(errorMessage(e));
        close();
      });
    return () => {
      cancelled = true;
    };
  }, [req, close]);

  const execute = async (typed: string | null) => {
    if (!preview) return;
    setRunning(true);
    try {
      const res = await deleteExecute(preview.token, typed, setProgress);
      void client.invalidateQueries({ queryKey: qk.objectsBucket(req.connectionId, req.bucket) });
      void client.invalidateQueries({ queryKey: ["search", req.connectionId, req.bucket] });
      void client.invalidateQueries({ queryKey: ["prefixSize", req.connectionId, req.bucket] });
      useBrowserStore.getState().clearSelection();
      if (res.failures.length > 0) {
        setResult({
          title: "Some objects were not deleted",
          summary: `Deleted ${plural(res.deleted, "object")}. ${formatCount(res.failures.length)} could not be deleted.`,
          failures: res.failures,
        });
      } else {
        toast.success(`Deleted ${plural(res.deleted, "object")}.`);
        close();
      }
    } catch (e) {
      toast.danger(errorMessage(e));
      close();
    } finally {
      setRunning(false);
    }
  };

  if (result) return <OpResultDialog result={result} onClose={close} />;
  if (!preview) return null;

  const singleFile = req.keys.length === 1 && req.prefixes.length === 0;
  const singleFolder = req.prefixes.length === 1 && req.keys.length === 0;
  const sample =
    preview.sample.length > 0 && !singleFile ? (
      <ul className="sample-list">
        {preview.sample.map((k) => (
          <li key={k} className="mono truncate">
            {visibleText(k)}
          </li>
        ))}
        {preview.objects > preview.sample.length && (
          <li className="muted small">and {formatCount(preview.objects - preview.sample.length)} more</li>
        )}
      </ul>
    ) : null;
  const body = (
    <>
      <p>This cannot be undone. R2 does not keep a trash.</p>
      {!singleFile && preview.bytes > 0 && <p className="secondary small num">{formatBytes(preview.bytes)} in total.</p>}
      {sample}
    </>
  );

  if (preview.objects === 0) {
    return (
      <ConfirmDialog
        open
        onOpenChange={(open) => {
          if (!open) close();
        }}
        title="Nothing to delete"
        body={<p>There are no objects here. Empty folders disappear once nothing is inside them.</p>}
        confirmLabel="OK"
        onConfirm={close}
      />
    );
  }

  if (preview.needsTyped) {
    const title = singleFolder
      ? `Delete folder "${req.label}" and ${plural(preview.objects, "object")} inside?`
      : `Delete ${plural(preview.objects, "object")}?`;
    return (
      <TypedConfirmDialog
        open
        onOpenChange={(open) => {
          if (!open && !running) close();
        }}
        title={title}
        body={body}
        phrase={preview.typedPhrase}
        confirmLabel="Delete"
        busy={running}
        progress={running ? progressLabel(progress) : null}
        onConfirm={(typed) => execute(typed)}
      />
    );
  }

  const title = singleFile ? `Delete ${req.label}?` : `Delete ${plural(preview.objects, "object")}?`;
  return (
    <ConfirmDialog
      open
      onOpenChange={(open) => {
        if (!open && !running) close();
      }}
      title={title}
      body={body}
      confirmLabel="Delete"
      danger
      busy={running}
      progress={running ? progressLabel(progress) : null}
      onConfirm={() => execute(null)}
    />
  );
}

export function DeleteFlow() {
  const req = useOpsStore((s) => s.del);
  if (!req) return null;
  return <DeleteDialogs key={`${req.bucket}:${req.keys.join("|")}:${req.prefixes.join("|")}`} req={req} />;
}
