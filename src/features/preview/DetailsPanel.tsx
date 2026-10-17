import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, CircleAlert, Download, Ellipsis, FolderInput, FolderOpen, Info, Link, Pencil, Trash2, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { Button, IconButton } from "../../components/Button";
import { Skeleton } from "../../components/Controls";
import { CopyButton, EmptyState } from "../../components/Feedback";
import { FileIcon } from "../../components/FileIcon";
import { Menu, MenuItem, MenuSeparator } from "../../components/Menu";
import { Tooltip } from "../../components/Tooltip";
import { toast } from "../../components/toastStore";
import { clipboardWrite, errorMessage, objectHead, prefixSize, previewText } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import type { PrefixSize } from "../../ipc/bindings/PrefixSize";
import { qk, usePreferences } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import { useUiStore } from "../../app/uiStore";
import { THUMBNAIL_LIMIT, previewKind } from "../../lib/fileTypes";
import { formatAgo, formatBytes, formatBytesLong, formatCount, formatDateTime, formatIsoLocal, joinDot, plural } from "../../lib/format";
import { visibleText } from "../../lib/keys";
import type { FileRow, FolderRow, SelectableRow } from "../browser/rows";
import { splitSelection } from "../browser/rows";
import { useOpsStore } from "../manage/opsStore";
import { startDownload } from "../transfers/download";
import { AudioPlayer } from "./AudioPlayer";
import { MediaImage } from "./PreviewBodies";
import { useMediaUrl } from "./useMediaUrl";
import "./preview.css";

export interface DetailsPanelProps {
  connection: ConnectionSummary;
  bucket: string;
  /** Current folder prefix. */
  prefix: string;
  /** Selected rows in display order (may be empty). */
  rows: SelectableRow[];
  /** Render as an overlay sheet (narrow windows) instead of a docked column. */
  overlay: boolean;
  onClose: () => void;
}

const ETAG_INFO =
  "Identifies this version of the object. It is not always an MD5 hash of the file. Files uploaded in parts have a different format.";
const SIZE_INFO = "Calculating lists every object in this folder and uses R2 operations that may be billed.";
const TEXT_SNIPPET_LIMIT = 256 * 1000;

interface Ctx {
  connectionId: string;
  bucket: string;
  prefix: string;
}

/* Preview area */

function TextSnippet({ ctx, entry }: { ctx: Ctx; entry: ObjectEntry }) {
  const query = useQuery({
    queryKey: qk.text(ctx.connectionId, ctx.bucket, entry.key),
    queryFn: () => previewText(ctx.connectionId, ctx.bucket, entry.key),
    staleTime: 60_000,
  });
  if (!query.data) return <FileIcon name={entry.name} size={48} />;
  if (query.data.binary) return <FileIcon name={entry.name} size={48} />;
  const snippet = query.data.text.split("\n").slice(0, 14).join("\n");
  return <pre className="details-snippet mono">{snippet}</pre>;
}

function CompactAudio({ ctx, entry }: { ctx: Ctx; entry: ObjectEntry }) {
  const media = useMediaUrl(ctx.connectionId, ctx.bucket, entry.key);
  return (
    <div className="details-audio">
      <FileIcon name={entry.name} size={40} />
      {media.url ? <AudioPlayer src={media.url} title={entry.displayName} compact /> : <Skeleton height={24} />}
    </div>
  );
}

function PreviewArea({ ctx, entry }: { ctx: Ctx; entry: ObjectEntry }) {
  const prefs = usePreferences();
  const kind = previewKind(entry.name);
  let content = <FileIcon name={entry.name} size={48} />;
  if (kind === "image" && prefs.showThumbnails && entry.size < THUMBNAIL_LIMIT) {
    content = <MediaImage connectionId={ctx.connectionId} bucket={ctx.bucket} entry={entry} className="details-thumb" />;
  } else if (kind === "audio") {
    content = <CompactAudio ctx={ctx} entry={entry} />;
  } else if (kind === "text" && entry.size < TEXT_SNIPPET_LIMIT) {
    content = <TextSnippet ctx={ctx} entry={entry} />;
  }
  return (
    <div className="details-preview" key={entry.key}>
      {content}
    </div>
  );
}

/* Prefix size */

function PrefixSizeRow({ ctx, prefix }: { ctx: Ctx; prefix: string }) {
  const client = useQueryClient();
  const key = qk.prefixSize(ctx.connectionId, ctx.bucket, prefix);
  const { data } = useQuery<PrefixSize>({ queryKey: key, queryFn: () => prefixSize(ctx.connectionId, ctx.bucket, prefix), enabled: false, staleTime: Infinity });
  const [busy, setBusy] = useState(false);
  const calculate = async () => {
    setBusy(true);
    try {
      const result = await prefixSize(ctx.connectionId, ctx.bucket, prefix);
      client.setQueryData(key, result);
    } catch (e) {
      toast.danger(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="details-size-row">
      <span className="secondary small">
        {data
          ? joinDot([plural(data.objects, "object"), formatBytes(data.bytes), `calculated ${formatAgo(data.calculatedAt)}`])
          : "Size not calculated"}
      </span>
      <Tooltip content={SIZE_INFO}>
        <Button size="sm" variant="ghost" onClick={() => void calculate()} loading={busy}>
          {data ? "Recalculate" : "Calculate"}
        </Button>
      </Tooltip>
    </div>
  );
}

/* File */

function MetaRow({ label, children, info }: { label: string; children: ReactNode; info?: string }) {
  return (
    <>
      <dt>
        {label}
        {info && (
          <Tooltip content={info} side="top">
            <button type="button" className="info-btn details-info" aria-label={info}>
              <Info size={12} aria-hidden="true" />
            </button>
          </Tooltip>
        )}
      </dt>
      <dd>{children}</dd>
    </>
  );
}

function FileDetails({ ctx, row }: { ctx: Ctx; row: FileRow }) {
  const entry = row.entry;
  const ops = useOpsStore();
  const setBucketSettings = useUiStore((s) => s.setBucketSettings);
  const [metaOpen, setMetaOpen] = useState(false);
  const head = useQuery({
    queryKey: qk.head(ctx.connectionId, ctx.bucket, entry.key),
    queryFn: () => objectHead(ctx.connectionId, ctx.bucket, entry.key),
  });
  const info = head.data;
  const details = info?.details;
  const customMeta = details ? Object.entries(details.metadata) : [];

  const copyPublicUrl = async () => {
    if (!info?.publicUrl) return;
    try {
      await clipboardWrite(info.publicUrl);
      toast.success("Public URL copied.");
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };

  return (
    <div className="details-content">
      <PreviewArea ctx={ctx} entry={entry} />
      <h3 className="details-name">{entry.displayName}</h3>
      <div className="details-actions">
        <Button variant="primary" size="sm" icon={<Download size={14} />} onClick={() => void startDownload(ctx.connectionId, ctx.bucket, [entry.key], [])}>
          Download
        </Button>
        <CopyButton text={entry.key} label="Copy key" size="md" />
        <Menu trigger={<IconButton label="More actions" icon={<Ellipsis size={16} />} />} align="end">
          {info?.publicUrl && (
            <MenuItem icon={<Link size={16} />} onSelect={() => void copyPublicUrl()}>
              Copy public URL
            </MenuItem>
          )}
          <MenuItem
            icon={<Pencil size={16} />}
            onSelect={() =>
              ops.requestRename({
                connectionId: ctx.connectionId,
                bucket: ctx.bucket,
                target: { kind: "file", key: entry.key, name: entry.name, displayName: entry.displayName },
              })
            }
          >
            Rename
          </MenuItem>
          <MenuItem
            icon={<FolderInput size={16} />}
            onSelect={() =>
              ops.requestMoveTo({ connectionId: ctx.connectionId, bucket: ctx.bucket, keys: [entry.key], prefixes: [], fromPrefix: ctx.prefix })
            }
          >
            Move to...
          </MenuItem>
          <MenuSeparator />
          <MenuItem
            icon={<Trash2 size={16} />}
            danger
            onSelect={() =>
              ops.requestDelete({ connectionId: ctx.connectionId, bucket: ctx.bucket, keys: [entry.key], prefixes: [], label: entry.displayName })
            }
          >
            Delete
          </MenuItem>
        </Menu>
      </div>

      {head.isError ? (
        <EmptyState compact icon={<CircleAlert size={24} />} title="Could not load details" body={errorMessage(head.error)} actions={<Button size="sm" onClick={() => void head.refetch()}>Try again</Button>} />
      ) : !details ? (
        <div className="details-skeleton" aria-busy="true" aria-label="Loading details">
          {[70, 50, 80, 40, 90, 60].map((w, i) => (
            <Skeleton key={i} width={`${w}%`} />
          ))}
        </div>
      ) : (
        <>
          <dl className="meta-list">
            <MetaRow label="Size">
              <span className="meta-value num">{formatBytesLong(details.size)}</span>
            </MetaRow>
            <MetaRow label="Type">
              <span className="meta-value">{details.contentType ?? "Unknown"}</span>
            </MetaRow>
            <MetaRow label="Modified">
              <span className="meta-value num" title={formatIsoLocal(details.lastModified)}>
                {formatDateTime(details.lastModified)}
              </span>
            </MetaRow>
            <MetaRow label="Storage class">
              <span className="meta-value">{details.storageClass}</span>
            </MetaRow>
            <MetaRow label="Key">
              <span className="meta-value mono selectable">{visibleText(details.key)}</span>
              <CopyButton text={details.key} label="Copy key" />
            </MetaRow>
            {details.etag && (
              <MetaRow label="ETag" info={ETAG_INFO}>
                <span className="meta-value mono truncate" title={details.etag}>
                  {details.etag}
                </span>
                <CopyButton text={details.etag} label="Copy ETag" />
              </MetaRow>
            )}
            {details.cacheControl && (
              <MetaRow label="Cache control">
                <span className="meta-value mono">{details.cacheControl}</span>
              </MetaRow>
            )}
            {details.contentEncoding && (
              <MetaRow label="Encoding">
                <span className="meta-value mono">{details.contentEncoding}</span>
              </MetaRow>
            )}
          </dl>
          {customMeta.length > 0 && (
            <div className="details-section">
              <button type="button" className="disclosure" aria-expanded={metaOpen} onClick={() => setMetaOpen((v) => !v)}>
                <ChevronRight size={14} aria-hidden="true" />
                Custom metadata ({customMeta.length})
              </button>
              {metaOpen && (
                <dl className="meta-list">
                  {customMeta.map(([k, v]) => (
                    <MetaRow key={k} label={visibleText(k)}>
                      <span className="meta-value mono selectable">{visibleText(v)}</span>
                    </MetaRow>
                  ))}
                </dl>
              )}
            </div>
          )}
          <div className="details-section">
            {info.publicUrl ? (
              <dl className="meta-list">
                <MetaRow label="Public URL">
                  <span className="meta-value mono selectable">{info.publicUrl}</span>
                  <CopyButton text={info.publicUrl} label="Copy public URL" />
                </MetaRow>
              </dl>
            ) : (
              <p className="small muted">
                No public URL configured. Set one in{" "}
                <button type="button" className="link-button" onClick={() => setBucketSettings({ connectionId: ctx.connectionId, bucket: ctx.bucket })}>
                  Bucket settings
                </button>
                .
              </p>
            )}
          </div>
        </>
      )}
    </div>
  );
}

/* Folder */

function FolderDetails({ ctx, row }: { ctx: Ctx; row: FolderRow }) {
  const ops = useOpsStore();
  return (
    <div className="details-content">
      <div className="details-preview">
        <FileIcon name={row.name} folder size={48} />
      </div>
      <h3 className="details-name">{row.displayName}</h3>
      <p className="small secondary">Folder (key prefix)</p>
      <div className="details-actions">
        <Button variant="primary" size="sm" icon={<FolderOpen size={14} />} onClick={() => useNavStore.getState().openPrefix(row.prefix)}>
          Open
        </Button>
        <Button size="sm" icon={<Download size={14} />} onClick={() => void startDownload(ctx.connectionId, ctx.bucket, [], [row.prefix])}>
          Download folder
        </Button>
        <IconButton
          label="Delete"
          icon={<Trash2 size={16} />}
          onClick={() => ops.requestDelete({ connectionId: ctx.connectionId, bucket: ctx.bucket, keys: [], prefixes: [row.prefix], label: row.displayName })}
        />
      </div>
      <dl className="meta-list">
        <MetaRow label="Prefix">
          <span className="meta-value mono selectable">{visibleText(row.prefix)}</span>
          <CopyButton text={row.prefix} label="Copy prefix" />
        </MetaRow>
      </dl>
      <PrefixSizeRow ctx={ctx} prefix={row.prefix} />
    </div>
  );
}

/* Multiple */

function MultiDetails({ ctx, rows }: { ctx: Ctx; rows: SelectableRow[] }) {
  const ops = useOpsStore();
  const { keys, prefixes } = splitSelection(rows);
  const total = rows.reduce((sum, r) => sum + (r.kind === "file" ? r.entry.size : 0), 0);
  return (
    <div className="details-content">
      <h3 className="details-name">{formatCount(rows.length)} items selected</h3>
      <p className="small secondary num">
        {keys.length > 0 ? `Total of selected files: ${formatBytes(total)}` : "No files selected, only folders"}
      </p>
      {prefixes.length > 0 && keys.length > 0 && <p className="small muted">Folder sizes are not included.</p>}
      <div className="details-actions">
        <Button variant="primary" size="sm" icon={<Download size={14} />} onClick={() => void startDownload(ctx.connectionId, ctx.bucket, keys, prefixes)}>
          Download
        </Button>
        <Button
          size="sm"
          icon={<FolderInput size={14} />}
          onClick={() => ops.requestMoveTo({ connectionId: ctx.connectionId, bucket: ctx.bucket, keys, prefixes, fromPrefix: ctx.prefix })}
        >
          Move to...
        </Button>
        <Button
          size="sm"
          variant="ghost"
          icon={<Trash2 size={14} />}
          onClick={() => ops.requestDelete({ connectionId: ctx.connectionId, bucket: ctx.bucket, keys, prefixes, label: `${rows.length} items` })}
        >
          Delete
        </Button>
      </div>
    </div>
  );
}

export function DetailsPanel({ connection, bucket, prefix, rows, overlay, onClose }: DetailsPanelProps) {
  const ctx: Ctx = { connectionId: connection.id, bucket, prefix };
  const first = rows[0];
  let content;
  if (!first) {
    content = <p className="details-empty small muted">Select a file to see its details.</p>;
  } else if (rows.length > 1) {
    content = <MultiDetails ctx={ctx} rows={rows} />;
  } else if (first.kind === "file") {
    content = <FileDetails key={first.id} ctx={ctx} row={first} />;
  } else {
    content = <FolderDetails key={first.id} ctx={ctx} row={first} />;
  }
  return (
    <aside className={overlay ? "details-panel is-overlay" : "details-panel"} aria-label="Details">
      <div className="details-header">
        <h2 className="details-title">Details</h2>
        <IconButton label="Close details" icon={<X size={16} />} size="sm" onClick={onClose} />
      </div>
      <div className="details-scroll">{content}</div>
    </aside>
  );
}
