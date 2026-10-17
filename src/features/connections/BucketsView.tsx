import { useQueryClient } from "@tanstack/react-query";
import { Database, Plus, RotateCw, Star } from "lucide-react";
import { useState, type FormEvent, type KeyboardEvent } from "react";
import { Button, IconButton } from "../../components/Button";
import { Badge, Skeleton } from "../../components/Controls";
import { EmptyState } from "../../components/Feedback";
import { TextField } from "../../components/Fields";
import { Tooltip } from "../../components/Tooltip";
import { toast } from "../../components/toastStore";
import { connectionAddBucket, connectionToggleFavorite, errorMessage } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import { formatDate, formatIsoLocal } from "../../lib/format";
import { errorCode } from "../../lib/errors";
import { useNavStore } from "../../app/navStore";
import { qk, useActiveConnection, useBuckets, type BucketRow } from "../../app/queries";
import { ErrorView, useDelayed } from "./ErrorStates";
import "./connections.css";

const JURISDICTION_LABEL = { default: "", eu: "EU", fedramp: "FedRAMP" } as const;

function AddBucketForm({ connection, onDone }: { connection: ConnectionSummary; onDone: () => void }) {
  const client = useQueryClient();
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    const bucket = name.trim();
    if (!bucket) return;
    setBusy(true);
    try {
      const summary = await connectionAddBucket(connection.id, bucket);
      client.setQueryData<ConnectionSummary[]>(qk.connections, (old) => old?.map((c) => (c.id === summary.id ? summary : c)));
      void client.invalidateQueries({ queryKey: qk.connections });
      toast.success(`Added ${bucket}.`);
      setName("");
      onDone();
    } catch (err) {
      toast.danger(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };
  return (
    <form className="add-bucket-form" onSubmit={(e) => void submit(e)}>
      <TextField label="Bucket name" mono value={name} onChange={(e) => setName(e.target.value)} autoFocus disabled={busy} />
      <Button type="submit" variant="primary" loading={busy} disabled={!name.trim()}>
        Add
      </Button>
    </form>
  );
}

function BucketTable({ connection, rows }: { connection: ConnectionSummary; rows: BucketRow[] }) {
  const openBucket = useNavStore((s) => s.openBucket);
  const client = useQueryClient();
  const toggleFavorite = async (bucket: string) => {
    try {
      const summary = await connectionToggleFavorite(connection.id, bucket);
      client.setQueryData<ConnectionSummary[]>(qk.connections, (old) => old?.map((c) => (c.id === summary.id ? summary : c)));
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };
  const onKeyDown = (e: KeyboardEvent<HTMLTableRowElement>, bucket: string) => {
    if (e.key === "Enter") {
      e.preventDefault();
      openBucket(bucket);
    }
  };
  return (
    <table className="table buckets-table">
      <thead>
        <tr>
          <th className="col-star">
            <span className="visually-hidden">Favorite</span>
          </th>
          <th>Name</th>
          <th className="col-created">Created</th>
        </tr>
      </thead>
      <tbody>
        {rows.map((b) => (
          <tr
            key={b.name}
            className="bucket-row"
            tabIndex={0}
            onDoubleClick={() => openBucket(b.name)}
            onKeyDown={(e) => onKeyDown(e, b.name)}
            aria-label={b.name}
          >
            <td className="col-star">
              <IconButton
                size="sm"
                className={b.favorite ? "star-btn is-on" : "star-btn"}
                label={b.favorite ? "Remove from favorites" : "Add to favorites"}
                icon={<Star size={14} fill={b.favorite ? "currentColor" : "none"} />}
                onClick={() => void toggleFavorite(b.name)}
                aria-pressed={b.favorite}
              />
            </td>
            <td>
              <button type="button" className="bucket-name-cell link-like" onClick={() => openBucket(b.name)}>
                <Database size={16} aria-hidden="true" />
                <span className="truncate">{b.name}</span>
                {b.manual && <Badge>Added manually</Badge>}
              </button>
            </td>
            <td className="col-created num">
              {b.created ? (
                <Tooltip content={formatIsoLocal(b.created)}>
                  <span tabIndex={-1}>{formatDate(b.created)}</span>
                </Tooltip>
              ) : (
                "--"
              )}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

function SkeletonTable() {
  return (
    <table className="table buckets-table" aria-hidden="true">
      <tbody>
        {Array.from({ length: 6 }, (_, i) => (
          <tr key={i}>
            <td className="col-star" />
            <td>
              <Skeleton width={`${40 + ((i * 17) % 40)}%`} />
            </td>
            <td className="col-created">
              <Skeleton width={90} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

export function BucketsView() {
  const connection = useActiveConnection();
  const buckets = useBuckets(connection);
  const [adding, setAdding] = useState(false);
  const showSkeleton = useDelayed(buckets.isLoading);
  if (!connection) return null;

  const deniedList = buckets.isError && errorCode(buckets.error) === "accessDenied";
  const jurisdiction = JURISDICTION_LABEL[connection.jurisdiction];
  const scoped = connection.bucketScoped || deniedList;

  let body;
  if (buckets.isLoading) {
    body = showSkeleton ? <SkeletonTable /> : null;
  } else if (buckets.isError && !(deniedList && buckets.usable)) {
    body = deniedList ? (
      <EmptyState
        icon={<Database size={32} />}
        title="This key cannot list buckets"
        body="If your token is limited to specific buckets, add a bucket by name."
        actions={<AddBucketForm connection={connection} onDone={() => setAdding(false)} />}
      />
    ) : (
      <ErrorView error={buckets.error} connection={connection} onRetry={() => void buckets.refetch()} />
    );
  } else if (buckets.rows.length === 0) {
    body = (
      <EmptyState
        icon={<Database size={32} />}
        title="No buckets yet"
        body="Create a bucket in the Cloudflare dashboard, or add one by name if your key is limited to certain buckets."
        actions={<Button icon={<Plus size={16} />} onClick={() => setAdding(true)}>Add bucket...</Button>}
      />
    );
  } else {
    body = <BucketTable connection={connection} rows={buckets.rows} />;
  }

  return (
    <div className="page">
      <div className="page-header">
        <h1 className="page-title">Buckets</h1>
        <span className="page-subtitle">
          {connection.name}
          {jurisdiction && ` (${jurisdiction})`}
        </span>
        <span className="spacer" />
        <IconButton label="Refresh" icon={<RotateCw size={16} />} onClick={() => void buckets.refetch()} />
      </div>
      {body}
      {scoped && buckets.rows.length > 0 && (
        <p className="scoped-note small">
          This key can only see buckets you added manually.
          <button type="button" className="link-button" onClick={() => setAdding(true)}>
            Add bucket...
          </button>
        </p>
      )}
      {!scoped && buckets.rows.length > 0 && (
        <p className="scoped-note small">
          <button type="button" className="link-button" onClick={() => setAdding(true)}>
            Add bucket...
          </button>
        </p>
      )}
      {adding && <AddBucketForm connection={connection} onDone={() => setAdding(false)} />}
    </div>
  );
}
