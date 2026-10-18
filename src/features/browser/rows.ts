import type { FolderEntry } from "../../ipc/bindings/FolderEntry";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import type { ListPage } from "../../ipc/bindings/ListPage";
import { fileKind, humanType } from "../../lib/fileTypes";

export interface FolderRow {
  kind: "folder";
  /** The prefix, used as the selection id. */
  id: string;
  prefix: string;
  name: string;
  displayName: string;
  unusual: boolean;
}

export interface FileRow {
  kind: "file";
  /** The key, used as the selection id. */
  id: string;
  entry: ObjectEntry;
}

/** An upload in progress into the current folder that is not listed yet. */
export interface GhostRow {
  kind: "ghost";
  id: string;
  key: string;
  name: string;
  transferred: number;
  total: number;
  /** Finished, waiting for the folder listing to refresh. */
  done: boolean;
}

export type Row = FolderRow | FileRow | GhostRow;
export type SelectableRow = FolderRow | FileRow;

export type SortKey = "name" | "size" | "modified" | "type";
export interface SortState {
  key: SortKey;
  dir: "asc" | "desc";
  foldersFirst: boolean;
}

export const DEFAULT_SORT: SortState = { key: "name", dir: "asc", foldersFirst: true };

export function folderRow(f: FolderEntry): FolderRow {
  return { kind: "folder", id: f.prefix, prefix: f.prefix, name: f.name, displayName: f.displayName, unusual: f.unusual };
}

export function fileRow(o: ObjectEntry): FileRow {
  return { kind: "file", id: o.key, entry: o };
}

/** Flatten listing pages into rows, dropping duplicates and the folder's own marker object. */
export function rowsFromPages(pages: ListPage[], prefix: string): SelectableRow[] {
  const seen = new Set<string>();
  const rows: SelectableRow[] = [];
  for (const page of pages) {
    for (const f of page.folders) {
      if (seen.has(f.prefix)) continue;
      seen.add(f.prefix);
      rows.push(folderRow(f));
    }
    for (const o of page.objects) {
      if (o.key === prefix || seen.has(o.key)) continue;
      seen.add(o.key);
      rows.push(fileRow(o));
    }
  }
  return rows;
}

export function rowName(row: Row): string {
  if (row.kind === "file") return row.entry.name;
  return row.name;
}

export function rowDisplayName(row: Row): string {
  if (row.kind === "file") return row.entry.displayName;
  if (row.kind === "folder") return row.displayName;
  return row.name;
}

const collator = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

export function sortRows(rows: SelectableRow[], sort: SortState): SelectableRow[] {
  const sign = sort.dir === "asc" ? 1 : -1;
  const byName = (a: SelectableRow, b: SelectableRow) => collator.compare(rowName(a), rowName(b));
  const cmp = (a: SelectableRow, b: SelectableRow): number => {
    if (sort.foldersFirst && a.kind !== b.kind) return a.kind === "folder" ? -1 : 1;
    let r = 0;
    switch (sort.key) {
      case "name":
        r = byName(a, b);
        break;
      case "size": {
        const sa = a.kind === "file" ? a.entry.size : -1;
        const sb = b.kind === "file" ? b.entry.size : -1;
        r = sa - sb;
        break;
      }
      case "modified": {
        const ta = a.kind === "file" && a.entry.lastModified ? Date.parse(a.entry.lastModified) : 0;
        const tb = b.kind === "file" && b.entry.lastModified ? Date.parse(b.entry.lastModified) : 0;
        r = ta - tb;
        break;
      }
      case "type": {
        const ka = a.kind === "folder" ? "" : humanType(a.entry.name);
        const kb = b.kind === "folder" ? "" : humanType(b.entry.name);
        r = collator.compare(ka, kb);
        break;
      }
    }
    if (r === 0) r = byName(a, b);
    return r * sign;
  };
  return [...rows].sort(cmp);
}

export function filterRows(rows: SelectableRow[], query: string): SelectableRow[] {
  const q = query.trim().toLowerCase();
  if (!q) return rows;
  return rows.filter((r) => rowDisplayName(r).toLowerCase().includes(q));
}

export function rowKind(row: Row) {
  return row.kind === "folder" ? "folder" : fileKind(rowName(row));
}

/** Files and folder prefixes for commands that take keys and prefixes. */
export function splitSelection(rows: SelectableRow[]): { keys: string[]; prefixes: string[] } {
  const keys: string[] = [];
  const prefixes: string[] = [];
  for (const r of rows) {
    if (r.kind === "file") keys.push(r.entry.key);
    else prefixes.push(r.prefix);
  }
  return { keys, prefixes };
}
