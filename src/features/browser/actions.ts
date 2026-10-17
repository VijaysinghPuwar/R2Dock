import { clipboardWrite, errorMessage } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import { toast } from "../../components/toastStore";
import { useNavStore } from "../../app/navStore";
import { useUiStore } from "../../app/uiStore";
import { previewKind } from "../../lib/fileTypes";
import { parentPrefix } from "../../lib/keys";
import { plural } from "../../lib/format";
import { useOpsStore } from "../manage/opsStore";
import { usePreviewStore } from "../preview/previewStore";
import { startDownload } from "../transfers/download";
import { useBrowserStore } from "./browserStore";
import { rowDisplayName, splitSelection, type FileRow, type SelectableRow } from "./rows";

export interface ExplorerContext {
  connection: ConnectionSummary;
  bucket: string;
  prefix: string;
}

async function copy(text: string, done: string) {
  try {
    await clipboardWrite(text);
    toast.success(done);
  } catch (e) {
    toast.danger(errorMessage(e));
  }
}

/** Files of the current view, for preview navigation. */
function fileEntries(): ObjectEntry[] {
  return useBrowserStore
    .getState()
    .visibleRows.filter((r): r is FileRow => r.kind === "file")
    .map((r) => r.entry);
}

export function previewFile(ctx: ExplorerContext, entry: ObjectEntry) {
  const items = fileEntries();
  let index = items.findIndex((e) => e.key === entry.key);
  if (index < 0) {
    items.unshift(entry);
    index = 0;
  }
  usePreviewStore.getState().show(ctx.connection.id, ctx.bucket, items, index);
}

/** Open a folder, or preview a file. */
export function openRow(ctx: ExplorerContext, row: SelectableRow) {
  if (row.kind === "folder") {
    useNavStore.getState().openBucket(ctx.bucket, row.prefix);
    return;
  }
  previewFile(ctx, row.entry);
}

/** Space: quick preview of the focused file. */
export function quickPreview(ctx: ExplorerContext, row: SelectableRow | undefined) {
  if (!row || row.kind !== "file") return;
  previewFile(ctx, row.entry);
}

export function downloadRows(ctx: ExplorerContext, rows: SelectableRow[]) {
  const { keys, prefixes } = splitSelection(rows);
  void startDownload(ctx.connection.id, ctx.bucket, keys, prefixes);
}

export function deleteRows(ctx: ExplorerContext, rows: SelectableRow[]) {
  if (rows.length === 0) return;
  const { keys, prefixes } = splitSelection(rows);
  const first = rows[0];
  useOpsStore.getState().requestDelete({
    connectionId: ctx.connection.id,
    bucket: ctx.bucket,
    keys,
    prefixes,
    label: rows.length === 1 && first ? rowDisplayName(first) : plural(rows.length, "item"),
  });
}

export function renameRow(ctx: ExplorerContext, row: SelectableRow) {
  useOpsStore.getState().requestRename({
    connectionId: ctx.connection.id,
    bucket: ctx.bucket,
    target:
      row.kind === "file"
        ? { kind: "file", key: row.entry.key, name: row.entry.name, displayName: row.entry.displayName }
        : { kind: "folder", prefix: row.prefix, name: row.name, displayName: row.displayName },
  });
}

export function moveRows(ctx: ExplorerContext, rows: SelectableRow[]) {
  if (rows.length === 0) return;
  const { keys, prefixes } = splitSelection(rows);
  useOpsStore.getState().requestMoveTo({ connectionId: ctx.connection.id, bucket: ctx.bucket, keys, prefixes, fromPrefix: ctx.prefix });
}

export function copyKeys(rows: SelectableRow[]) {
  if (rows.length === 0) return;
  const text = rows.map((r) => (r.kind === "file" ? r.entry.key : r.prefix)).join("\n");
  void copy(text, rows.length === 1 ? "Key copied." : `${rows.length} keys copied.`);
}

/** Public URL built from the configured base URL. Display helper only. */
export function publicUrlFor(ctx: ExplorerContext, key: string): string | null {
  const base = ctx.connection.publicBaseUrls[ctx.bucket];
  if (!base) return null;
  const encoded = key
    .split("/")
    .map((part) => encodeURIComponent(part))
    .join("/");
  return `${base.replace(/\/+$/, "")}/${encoded}`;
}

export function copyPublicUrl(ctx: ExplorerContext, row: SelectableRow) {
  if (row.kind !== "file") return;
  const url = publicUrlFor(ctx, row.entry.key);
  if (url) void copy(url, "Public URL copied.");
}

export function copyCurrentPath(ctx: ExplorerContext) {
  void copy(`${ctx.bucket}/${ctx.prefix}`, "Path copied.");
}

export function showDetails(row: SelectableRow) {
  useBrowserStore.getState().setSelected([row.id], row.id);
  useUiStore.getState().setDetailsOpen(true);
}

export function showInFolder(ctx: ExplorerContext, key: string) {
  useNavStore.getState().openBucket(ctx.bucket, parentPrefix(key));
  window.setTimeout(() => useBrowserStore.getState().setSelected([key], key), 0);
}

export function canPreview(row: SelectableRow): boolean {
  return row.kind === "file" && previewKind(row.entry.name) !== "none";
}
