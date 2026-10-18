import { useQueryClient } from "@tanstack/react-query";
import { Link2, Pencil, Plus, KeyRound, Plug, Trash2 } from "lucide-react";
import { useState } from "react";
import { qk, useActiveConnection, useBuckets, useConnections } from "../../app/queries";
import { useUiStore } from "../../app/uiStore";
import { Button } from "../../components/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Badge, Skeleton, StatusDot } from "../../components/Controls";
import { EmptyState } from "../../components/Feedback";
import { toast } from "../../components/toastStore";
import { connectionRemove, connectionTest, errorMessage } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { ConnectionStatus } from "../../ipc/bindings/ConnectionStatus";
import type { Jurisdiction } from "../../ipc/bindings/Jurisdiction";
import { formatAgo } from "../../lib/format";

const STATUS: Record<ConnectionStatus, { tone: "success" | "warning" | "muted"; label: string }> = {
  online: { tone: "success", label: "Online" },
  needsAttention: { tone: "warning", label: "Needs attention" },
  offline: { tone: "muted", label: "Offline" },
  unknown: { tone: "muted", label: "Not tested" },
};

const JURISDICTION: Record<Jurisdiction, string> = { default: "Default", eu: "EU", fedramp: "FedRAMP" };

function SecretNote({ storage }: { storage: ConnectionSummary["secretStorage"] }) {
  if (storage === "keychain") return <span className="small secondary">Stored in keychain</span>;
  if (storage === "session") return <span className="small secondary">Session only</span>;
  return <span className="small danger-text">Secret missing</span>;
}

function BucketUrls({ connection, extra }: { connection: ConnectionSummary; extra: string[] }) {
  const setBucketSettings = useUiStore((s) => s.setBucketSettings);
  const names = new Set<string>([
    ...connection.manualBuckets,
    ...connection.favoriteBuckets,
    ...Object.keys(connection.publicBaseUrls),
    ...extra,
  ]);
  const sorted = [...names].sort((a, b) => a.localeCompare(b));
  if (sorted.length === 0) return null;
  return (
    <div className="bucket-urls">
      <h4 className="small secondary">Public URLs</h4>
      <ul>
        {sorted.map((name) => {
          const url = connection.publicBaseUrls[name];
          return (
            <li key={name} className="bucket-url-row">
              <span className="truncate">{name}</span>
              <span className={url ? "mono truncate bucket-url-value" : "small muted bucket-url-value"}>{url ?? "Not set"}</span>
              <Button
                size="sm"
                variant="ghost"
                icon={<Link2 size={14} />}
                onClick={() => setBucketSettings({ connectionId: connection.id, bucket: name })}
              >
                Public URL
              </Button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function ConnectionCard({ connection, isActive }: { connection: ConnectionSummary; isActive: boolean }) {
  const client = useQueryClient();
  const openConnect = useUiStore((s) => s.openConnect);
  const buckets = useBuckets(isActive ? connection : null);
  const [testing, setTesting] = useState(false);
  const [removeOpen, setRemoveOpen] = useState(false);
  const status = STATUS[connection.status];

  const test = async () => {
    setTesting(true);
    try {
      const result = await connectionTest({
        id: connection.id,
        name: connection.name,
        accountId: connection.accountId,
        accessKeyId: "",
        secret: null,
        bucket: null,
        jurisdiction: connection.jurisdiction,
        apiToken: null,
        removeApiToken: false,
      });
      toast.success(result.message);
      if (result.warning) toast.warning(result.warning);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setTesting(false);
      void client.invalidateQueries({ queryKey: qk.connections });
    }
  };

  const remove = async () => {
    try {
      await connectionRemove(connection.id);
      setRemoveOpen(false);
      toast.success(`Removed ${connection.name}.`);
      await client.invalidateQueries({ queryKey: qk.connections });
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };

  return (
    <article className="conn-card" aria-label={connection.name}>
      <div className="conn-head">
        <div className="conn-title">
          <h3>{connection.name}</h3>
          {isActive && <Badge tone="accent">Active</Badge>}
        </div>
        <span className="conn-status small secondary">
          <StatusDot tone={status.tone} />
          {status.label}
        </span>
      </div>
      <div className="conn-meta">
        <span className="mono secondary">{connection.accountIdShort}</span>
        {connection.jurisdiction !== "default" && <Badge>{JURISDICTION[connection.jurisdiction]}</Badge>}
        <SecretNote storage={connection.secretStorage} />
        {connection.lastTestedAt && <span className="small muted">Last tested {formatAgo(connection.lastTestedAt)}</span>}
      </div>
      <div className="conn-actions">
        <Button size="sm" icon={<Plug size={14} />} loading={testing} onClick={() => void test()}>
          {testing ? "Testing..." : "Test"}
        </Button>
        <Button
          size="sm"
          icon={<Pencil size={14} />}
          onClick={() => openConnect({ mode: "edit", id: connection.id, replaceSecret: false, showHelp: false })}
        >
          Edit
        </Button>
        <Button
          size="sm"
          icon={<KeyRound size={14} />}
          onClick={() => openConnect({ mode: "edit", id: connection.id, replaceSecret: true, showHelp: false })}
        >
          Replace secret
        </Button>
        <span className="spacer" />
        <Button size="sm" variant="ghost" icon={<Trash2 size={14} />} onClick={() => setRemoveOpen(true)}>
          Remove
        </Button>
      </div>
      <BucketUrls connection={connection} extra={buckets.rows.map((b) => b.name)} />
      <ConfirmDialog
        open={removeOpen}
        onOpenChange={setRemoveOpen}
        title={`Remove ${connection.name}?`}
        body={<p>Its keys will be deleted from your system keychain. Your files in R2 are not affected.</p>}
        confirmLabel="Remove"
        danger
        onConfirm={remove}
      />
    </article>
  );
}

export function ConnectionsSection() {
  const connections = useConnections();
  const active = useActiveConnection();
  const openConnect = useUiStore((s) => s.openConnect);

  return (
    <section className="settings-section" aria-labelledby="settings-connections">
      <div className="settings-section-head">
        <h2 id="settings-connections" className="settings-heading">
          Connections
        </h2>
        <Button variant="primary" size="sm" icon={<Plus size={14} />} onClick={() => openConnect({ mode: "new", showHelp: false })}>
          Add connection
        </Button>
      </div>
      {connections.isPending && (
        <div className="stack">
          <Skeleton height={96} />
          <Skeleton height={96} />
        </div>
      )}
      {connections.isError && (
        <EmptyState
          compact
          icon={<Plug size={32} />}
          title="Could not load connections"
          body={errorMessage(connections.error)}
          actions={<Button onClick={() => void connections.refetch()}>Try again</Button>}
        />
      )}
      {connections.data && connections.data.length === 0 && (
        <EmptyState compact icon={<Plug size={32} />} title="No connections" body="Add an R2 account to get started." />
      )}
      {connections.data && connections.data.length > 0 && (
        <div className="stack">
          {connections.data.map((c) => (
            <ConnectionCard key={c.id} connection={c} isActive={c.id === active?.id} />
          ))}
        </div>
      )}
    </section>
  );
}
