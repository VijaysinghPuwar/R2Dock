import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState, type FormEvent } from "react";
import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import { TextField } from "../../components/Fields";
import { toast } from "../../components/toastStore";
import { connectionSetPublicUrl, errorMessage } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import { qk, useConnections } from "../../app/queries";
import { useUiStore } from "../../app/uiStore";

function validUrl(value: string): boolean {
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:";
  } catch {
    return false;
  }
}

/** Per-bucket settings: the public base URL used for "Copy public URL". */
export function BucketSettingsDialog() {
  const target = useUiStore((s) => s.bucketSettings);
  const close = () => useUiStore.getState().setBucketSettings(null);
  const { data: connections } = useConnections();
  const client = useQueryClient();
  const connection = connections?.find((c) => c.id === target?.connectionId) ?? null;
  const current = target && connection ? (connection.publicBaseUrls[target.bucket] ?? "") : "";
  const [value, setValue] = useState(current);
  const [submitted, setSubmitted] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setValue(current);
    setSubmitted(false);
  }, [current, target]);

  if (!target || !connection) return null;
  const trimmed = value.trim();
  const error = submitted && trimmed && !validUrl(trimmed) ? "Enter a full URL, for example https://media.example.com" : null;

  const save = async (url: string | null) => {
    setBusy(true);
    try {
      const summary = await connectionSetPublicUrl(connection.id, target.bucket, url);
      client.setQueryData<ConnectionSummary[]>(qk.connections, (old) => old?.map((c) => (c.id === summary.id ? summary : c)));
      void client.invalidateQueries({ queryKey: ["head", connection.id, target.bucket] });
      toast.success(url ? "Public URL saved." : "Public URL removed.");
      close();
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    setSubmitted(true);
    if (trimmed && !validUrl(trimmed)) return;
    void save(trimmed || null);
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
      title={`Bucket settings: ${target.bucket}`}
      width="form"
    >
      <form className="stack" onSubmit={onSubmit}>
        <TextField
          label="Public base URL"
          placeholder="https://media.example.com"
          mono
          value={value}
          onChange={(e) => setValue(e.target.value)}
          error={error}
          helper="Used only to build Copy public URL. R2Dock does not change bucket access."
          autoFocus
          disabled={busy}
        />
        <div className="dialog-footer">
          {current && (
            <Button variant="ghost" onClick={() => void save(null)} disabled={busy} className="footer-start">
              Remove URL
            </Button>
          )}
          <Button variant="ghost" onClick={close} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={busy}>
            Save
          </Button>
        </div>
      </form>
    </Dialog>
  );
}
