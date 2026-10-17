import { useQueryClient } from "@tanstack/react-query";
import { CircleAlert, KeyRound, SearchX, ShieldAlert, WifiOff } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "../../components/Button";
import { EmptyState } from "../../components/Feedback";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { toast } from "../../components/toastStore";
import { connectionRemove, errorMessage } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import { asAppError } from "../../lib/errors";
import { qk } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import { useUiStore } from "../../app/uiStore";

/** True after `delayMs` of `active` being true. Avoids flashing skeletons. */
export function useDelayed(active: boolean, delayMs = 150): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!active) {
      setShown(false);
      return;
    }
    const t = window.setTimeout(() => setShown(true), delayMs);
    return () => window.clearTimeout(t);
  }, [active, delayMs]);
  return shown;
}

export interface ErrorViewProps {
  error: unknown;
  connection: ConnectionSummary;
  /** Bucket being listed, for access-denied copy. */
  bucket?: string | null;
  onRetry: () => void;
}

/** Screen 8 error states mapped from AppError codes. */
export function ErrorView({ error, connection, bucket, onRetry }: ErrorViewProps) {
  const openConnect = useUiStore((s) => s.openConnect);
  const navigate = useNavStore((s) => s.navigate);
  const client = useQueryClient();
  const [confirmRemove, setConfirmRemove] = useState(false);
  const appError = asAppError(error);
  const code = appError?.code ?? null;
  const edit = (replaceSecret: boolean) => openConnect({ mode: "edit", id: connection.id, replaceSecret, showHelp: false });

  switch (code) {
    case "network":
    case "timeout":
      return (
        <EmptyState
          icon={<WifiOff size={32} />}
          title="You are offline"
          body="R2Dock will reconnect automatically. Transfers will continue when the connection returns."
          actions={<Button onClick={onRetry}>Try again</Button>}
        />
      );
    case "accessDenied":
      return (
        <EmptyState
          icon={<ShieldAlert size={32} />}
          title="You do not have access to this"
          body={
            bucket
              ? `This key cannot list "${bucket}". Use a token with access to this bucket.`
              : "This key cannot list buckets. If your token is limited to specific buckets, add a bucket by name."
          }
          actions={
            <>
              <Button onClick={() => edit(false)}>Edit connection</Button>
              {bucket && <Button variant="ghost" onClick={() => navigate({ view: "buckets" })}>Go to buckets</Button>}
            </>
          }
        />
      );
    case "invalidAccessKey":
    case "signatureMismatch":
      return (
        <EmptyState
          icon={<KeyRound size={32} />}
          title="These credentials no longer work"
          body="The key may have been deleted or rotated in Cloudflare."
          actions={
            <>
              <Button variant="primary" onClick={() => edit(true)}>
                Replace secret
              </Button>
              <Button onClick={() => edit(false)}>Edit connection</Button>
            </>
          }
        />
      );
    case "credentialMissing":
      return (
        <>
          <EmptyState
            icon={<KeyRound size={32} />}
            title="Keys not found in keychain"
            body={`The saved secret for "${connection.name}" was removed outside R2Dock.`}
            actions={
              <>
                <Button variant="primary" onClick={() => edit(true)}>
                  Enter secret again
                </Button>
                <Button onClick={() => setConfirmRemove(true)}>Remove connection</Button>
              </>
            }
          />
          <ConfirmDialog
            open={confirmRemove}
            onOpenChange={setConfirmRemove}
            title={`Remove ${connection.name}?`}
            body={<p>Its keys will be deleted from your system keychain. Your files in R2 are not affected.</p>}
            confirmLabel="Remove"
            danger
            onConfirm={async () => {
              try {
                await connectionRemove(connection.id);
                setConfirmRemove(false);
                await client.invalidateQueries({ queryKey: qk.connections });
              } catch (e) {
                toast.danger(errorMessage(e));
              }
            }}
          />
        </>
      );
    case "noSuchBucket":
      return (
        <EmptyState
          icon={<SearchX size={32} />}
          title="Bucket not found"
          body={appError?.message}
          actions={<Button onClick={() => navigate({ view: "buckets" })}>Go to buckets</Button>}
        />
      );
    default:
      return (
        <EmptyState
          icon={<CircleAlert size={32} />}
          title="Could not load this"
          body={errorMessage(error)}
          actions={<Button onClick={onRetry}>Try again</Button>}
        />
      );
  }
}
