import {
  Copy,
  Download,
  Eye,
  FolderInput,
  FolderOpen,
  FolderPlus,
  Info,
  Link,
  Pencil,
  RotateCw,
  TextCursorInput,
  Trash2,
  Upload,
} from "lucide-react";
import { ContextMenuItem, ContextMenuSeparator } from "../../components/Menu";
import { modKeyLabel } from "../../lib/platform";
import type { Platform } from "../../ipc/bindings/Platform";
import {
  copyKeys,
  copyPublicUrl,
  deleteRows,
  downloadRows,
  moveRows,
  openRow,
  renameRow,
  showDetails,
  showInFolder,
  type ExplorerContext,
} from "./actions";
import type { SelectableRow } from "./rows";
import { mayHavePublicLink } from "../preview/publicLink";

export interface MenuTarget {
  /** Rows the menu acts on (empty for the empty-area menu). */
  rows: SelectableRow[];
}

export function ExplorerContextMenu({
  ctx,
  target,
  platform,
  inSearch,
  onUpload,
  onNewFolder,
  onRefresh,
  onGoToPath,
}: {
  ctx: ExplorerContext;
  target: MenuTarget;
  platform: Platform;
  inSearch: boolean;
  onUpload: (kind: "files" | "folder") => void;
  onNewFolder: () => void;
  onRefresh: () => void;
  onGoToPath: () => void;
}) {
  const mod = modKeyLabel(platform);
  const rows = target.rows;
  const first = rows[0];

  if (!first) {
    return (
      <>
        <ContextMenuItem icon={<Upload size={16} />} shortcut={`${mod}+U`} onSelect={() => onUpload("files")}>
          Upload files...
        </ContextMenuItem>
        <ContextMenuItem icon={<FolderPlus size={16} />} shortcut={`${mod}+Shift+U`} onSelect={() => onUpload("folder")}>
          Upload folder...
        </ContextMenuItem>
        <ContextMenuItem icon={<FolderPlus size={16} />} shortcut={`${mod}+Shift+N`} onSelect={onNewFolder}>
          New folder
        </ContextMenuItem>
        <ContextMenuItem icon={<RotateCw size={16} />} shortcut={`${mod}+R`} onSelect={onRefresh}>
          Refresh
        </ContextMenuItem>
        <ContextMenuItem icon={<TextCursorInput size={16} />} onSelect={onGoToPath}>
          Go to path...
        </ContextMenuItem>
      </>
    );
  }

  if (rows.length > 1) {
    return (
      <>
        <ContextMenuItem icon={<Download size={16} />} onSelect={() => downloadRows(ctx, rows)}>
          Download {rows.length} items
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<Copy size={16} />} shortcut={`${mod}+Shift+C`} onSelect={() => copyKeys(rows)}>
          Copy keys
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<FolderInput size={16} />} onSelect={() => moveRows(ctx, rows)}>
          Move to...
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<Trash2 size={16} />} danger shortcut="Delete" onSelect={() => deleteRows(ctx, rows)}>
          Delete {rows.length} items
        </ContextMenuItem>
      </>
    );
  }

  if (first.kind === "folder") {
    return (
      <>
        <ContextMenuItem icon={<FolderOpen size={16} />} onSelect={() => openRow(ctx, first)}>
          Open
        </ContextMenuItem>
        <ContextMenuItem icon={<Download size={16} />} onSelect={() => downloadRows(ctx, rows)}>
          Download folder
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<Copy size={16} />} onSelect={() => copyKeys(rows)}>
          Copy key prefix
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<Pencil size={16} />} onSelect={() => renameRow(ctx, first)}>
          Rename...
        </ContextMenuItem>
        <ContextMenuItem icon={<FolderInput size={16} />} onSelect={() => moveRows(ctx, rows)}>
          Move to...
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<Info size={16} />} onSelect={() => showDetails(first)}>
          Details
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem icon={<Trash2 size={16} />} danger shortcut="Delete" onSelect={() => deleteRows(ctx, rows)}>
          Delete
        </ContextMenuItem>
      </>
    );
  }

  const hasPublic = mayHavePublicLink(ctx.connection, ctx.bucket);
  return (
    <>
      <ContextMenuItem icon={<Eye size={16} />} shortcut="Space" onSelect={() => openRow(ctx, first)}>
        Open / Preview
      </ContextMenuItem>
      {inSearch && (
        <ContextMenuItem icon={<FolderOpen size={16} />} onSelect={() => showInFolder(ctx, first.entry.key)}>
          Show in folder
        </ContextMenuItem>
      )}
      <ContextMenuItem icon={<Download size={16} />} onSelect={() => downloadRows(ctx, rows)}>
        Download
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem icon={<Copy size={16} />} shortcut={`${mod}+Shift+C`} onSelect={() => copyKeys(rows)}>
        Copy key
      </ContextMenuItem>
      {hasPublic && (
        <ContextMenuItem icon={<Link size={16} />} onSelect={() => void copyPublicUrl(ctx, first)}>
          Copy public URL
        </ContextMenuItem>
      )}
      <ContextMenuSeparator />
      <ContextMenuItem icon={<Pencil size={16} />} onSelect={() => renameRow(ctx, first)}>
        Rename...
      </ContextMenuItem>
      <ContextMenuItem icon={<FolderInput size={16} />} onSelect={() => moveRows(ctx, rows)}>
        Move to...
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem icon={<Info size={16} />} onSelect={() => showDetails(first)}>
        Details
      </ContextMenuItem>
      <ContextMenuSeparator />
      <ContextMenuItem icon={<Trash2 size={16} />} danger shortcut="Delete" onSelect={() => deleteRows(ctx, rows)}>
        Delete
      </ContextMenuItem>
    </>
  );
}
