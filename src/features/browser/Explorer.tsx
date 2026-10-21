import { useQueryClient } from "@tanstack/react-query";
import { FolderOpen, FolderPlus, Inbox, SearchX, Upload } from "lucide-react";
import { useCallback, useEffect, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { Button } from "../../components/Button";
import { Banner, EmptyState } from "../../components/Feedback";
import { ContextMenu } from "../../components/Menu";
import { toast } from "../../components/toastStore";
import { errorMessage } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import { errorCode } from "../../lib/errors";
import { folderLabel } from "../../lib/keys";
import { formatBytes, formatCount, joinDot } from "../../lib/format";
import { hasMod } from "../../lib/platform";
import { useNavStore } from "../../app/navStore";
import { calculatePrefixSize, useActiveConnection, usePlatform, usePreferences, useUpdatePreferences } from "../../app/queries";
import { refreshCurrentView } from "../../app/refresh";
import { useUiStore } from "../../app/uiStore";
import { ErrorView, useDelayed } from "../connections/ErrorStates";
import { DetailsPanel } from "../preview/DetailsPanel";
import { copyCurrentPath, copyKeys, deleteRows, downloadRows, moveRows, openRow, quickPreview, type ExplorerContext } from "./actions";
import { selectedRows, useBrowserStore } from "./browserStore";
import { ExplorerContextMenu, type MenuTarget } from "./ExplorerMenus";
import { FileGrid } from "./FileGrid";
import { FileTable, SkeletonRows } from "./FileTable";
import { rowName, type SelectableRow, type SortKey } from "./rows";
import { Toolbar } from "./Toolbar";
import { pickAndUpload, useUploadStore } from "./upload";
import { useExplorerData } from "./useExplorerData";
import "./browser.css";

function useOnline(): boolean {
  const [online, setOnline] = useState(() => (typeof navigator === "undefined" ? true : navigator.onLine));
  useEffect(() => {
    const on = () => setOnline(true);
    const off = () => setOnline(false);
    window.addEventListener("online", on);
    window.addEventListener("offline", off);
    return () => {
      window.removeEventListener("online", on);
      window.removeEventListener("offline", off);
    };
  }, []);
  return online;
}

export function Explorer() {
  const connection = useActiveConnection();
  const bucket = useNavStore((s) => s.loc.bucket);
  const prefix = useNavStore((s) => s.loc.prefix);
  if (!connection || !bucket) return null;
  return <ExplorerView connection={connection} bucket={bucket} prefix={prefix} />;
}

/** The folder the explorer last showed, kept across remounts. */
let lastExplorerLoc: string | null = null;

function ExplorerView({ connection, bucket, prefix }: { connection: ConnectionSummary; bucket: string; prefix: string }) {
  const client = useQueryClient();
  const platform = usePlatform();
  const prefs = usePreferences();
  const updatePrefs = useUpdatePreferences();
  const narrow = useUiStore((s) => s.narrow);
  const viewModeState = useUiStore((s) => s.viewMode);
  const detailsState = useUiStore((s) => s.detailsOpen);
  const setViewMode = useUiStore((s) => s.setViewMode);
  const setDetailsOpen = useUiStore((s) => s.setDetailsOpen);
  const setNewFolderOpen = useUiStore((s) => s.setNewFolderOpen);
  const uploading = useUploadStore((s) => s.preparing);
  const online = useOnline();
  const [attentionDismissed, setAttentionDismissed] = useState(false);

  const selected = useBrowserStore((s) => s.selected);
  const anchor = useBrowserStore((s) => s.anchor);
  const focusId = useBrowserStore((s) => s.focusId);
  const search = useBrowserStore((s) => s.search);
  const scope = useBrowserStore((s) => s.scope);
  const sort = useBrowserStore((s) => s.sort);
  const store = useBrowserStore.getState;

  const data = useExplorerData(connection.id, bucket, prefix);
  const { rows, allRows, listing, active } = data;

  const ctx: ExplorerContext = { connection, bucket, prefix };
  const viewMode = viewModeState ?? prefs.defaultView;
  const detailsOpen = detailsState ?? (prefs.showDetailsPanel && !narrow);
  const rowHeight = prefs.density === "compact" ? 26 : 32;
  const tileSize = prefs.density === "compact" ? 104 : 132;
  const columnsRef = useRef(1);

  // New location: clear selection and search.
  const locKey = `${connection.id}\u0000${bucket}\u0000${prefix}`;
  const lastLoc = useRef(locKey);
  useEffect(() => {
    if (lastLoc.current !== locKey) {
      lastLoc.current = locKey;
      useBrowserStore.getState().resetForLocation();
    } else if (lastExplorerLoc !== null && lastExplorerLoc !== locKey) {
      // Opened again at another folder after another page: the search
      // typed in the old folder must not filter (or search) this one.
      useBrowserStore.getState().setSearch("");
      useBrowserStore.getState().setScope("folder");
    }
    lastExplorerLoc = locKey;
  }, [locKey]);

  useEffect(() => {
    useBrowserStore.getState().setVisibleRows(rows);
  }, [rows]);

  // Drop selected ids that are no longer shown.
  useEffect(() => {
    const { selected: sel } = store();
    if (sel.size === 0) return;
    const ids = new Set(rows.map((r) => r.id));
    const kept = [...sel].filter((id) => ids.has(id));
    if (kept.length !== sel.size) store().setSelected(kept);
  }, [rows, store]);

  const selection = selectedRows({ selected, visibleRows: rows });
  const showSkeleton = useDelayed(active.isLoading);

  const onLoadMore = useCallback(() => {
    if (active.hasNextPage && !active.isFetchingNextPage) void active.fetchNextPage();
  }, [active]);

  const upload = (kind: "files" | "folder") =>
    void pickAndUpload(kind, { connectionId: connection.id, connectionName: connection.name, bucket, prefix });

  const refresh = () => refreshCurrentView(client);

  const calculateSize = async () => {
    try {
      const size = await calculatePrefixSize(client, connection.id, bucket, prefix);
      toast.neutral(
        `${folderLabel(bucket, prefix)}: ${joinDot([`${formatCount(size.objects)} objects`, formatBytes(size.bytes)])}`,
      );
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };

  /* Selection */

  const onRowClick = (row: SelectableRow, e: MouseEvent) => {
    const s = store();
    if (e.shiftKey && anchor) {
      const a = rows.findIndex((r) => r.id === anchor);
      const b = rows.findIndex((r) => r.id === row.id);
      if (a >= 0 && b >= 0) {
        const [from, to] = a < b ? [a, b] : [b, a];
        const range = rows.slice(from, to + 1).map((r) => r.id);
        s.setSelected(hasMod(e, platform) ? [...s.selected, ...range] : range, anchor);
        s.setFocus(row.id);
        return;
      }
    }
    if (hasMod(e, platform)) s.toggle(row.id);
    else s.setSelected([row.id], row.id);
    s.setFocus(row.id);
  };

  const onToggle = (row: SelectableRow) => {
    store().toggle(row.id);
    store().setFocus(row.id);
  };

  /* Keyboard */

  const typeBuffer = useRef({ text: "", at: 0 });
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return;
    const s = store();
    const mod = hasMod(e, platform);
    const idx = Math.max(0, rows.findIndex((r) => r.id === s.focusId));
    const focused = rows.find((r) => r.id === s.focusId);
    const cols = viewMode === "grid" ? columnsRef.current : 1;
    const pageSize = Math.max(1, Math.floor(600 / rowHeight));
    let next: number;
    switch (e.key) {
      case "ArrowDown":
        next = s.focusId === null ? 0 : Math.min(rows.length - 1, idx + cols);
        break;
      case "ArrowUp":
        if (e.altKey) return; // Alt+Up is "parent folder", handled globally
        next = s.focusId === null ? 0 : Math.max(0, idx - cols);
        break;
      case "ArrowRight":
        if (viewMode !== "grid" || e.altKey) return;
        next = Math.min(rows.length - 1, idx + 1);
        break;
      case "ArrowLeft":
        if (viewMode !== "grid" || e.altKey) return;
        next = Math.max(0, idx - 1);
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = rows.length - 1;
        break;
      case "PageDown":
        next = Math.min(rows.length - 1, idx + pageSize * cols);
        break;
      case "PageUp":
        next = Math.max(0, idx - pageSize * cols);
        break;
      case "Enter":
        if (focused) {
          e.preventDefault();
          openRow(ctx, focused);
        }
        return;
      case " ":
        e.preventDefault();
        if (!focused) return;
        if (mod) s.toggle(focused.id);
        else quickPreview(ctx, focused);
        return;
      default: {
        if (e.key.length === 1 && !mod && !e.altKey && /\S/.test(e.key)) {
          const now = Date.now();
          const buf = now - typeBuffer.current.at < 700 ? typeBuffer.current.text + e.key.toLowerCase() : e.key.toLowerCase();
          typeBuffer.current = { text: buf, at: now };
          const start = buf.length === 1 ? idx + 1 : idx;
          for (let i = 0; i < rows.length; i += 1) {
            const r = rows[(start + i) % rows.length];
            if (r && rowName(r).toLowerCase().startsWith(buf)) {
              e.preventDefault();
              s.setSelected([r.id], r.id);
              s.setFocus(r.id);
              break;
            }
          }
        }
        return;
      }
    }
    if (next < 0) return;
    e.preventDefault();
    const target = rows[next];
    if (!target) return;
    if (e.shiftKey) {
      const anchorId = s.anchor ?? s.focusId ?? target.id;
      const a = rows.findIndex((r) => r.id === anchorId);
      const [from, to] = a < next ? [a, next] : [next, a];
      s.setSelected(
        rows.slice(Math.max(0, from), to + 1).map((r) => r.id),
        anchorId,
      );
    } else if (!mod) {
      s.setSelected([target.id], target.id);
    }
    s.setFocus(target.id);
  };

  /* Context menu */

  const [menuTarget, setMenuTarget] = useState<MenuTarget>({ rows: [] });
  const onContextMenu = (row: SelectableRow | null) => {
    const s = store();
    if (!row) {
      setMenuTarget({ rows: [] });
      return;
    }
    if (s.selected.has(row.id)) {
      setMenuTarget({ rows: selectedRows({ selected: s.selected, visibleRows: rows }) });
    } else {
      s.setSelected([row.id], row.id);
      s.setFocus(row.id);
      setMenuTarget({ rows: [row] });
    }
  };

  const onSort = (key: SortKey) => {
    useBrowserStore.getState().setSort(sort.key === key ? { ...sort, dir: sort.dir === "asc" ? "desc" : "asc" } : { ...sort, key, dir: "asc" });
  };

  /* Content */

  const listingError = listing.isError && !listing.data ? listing.error : null;
  const searchError = data.bucketSearch && data.searchQuery.isError ? data.searchQuery.error : null;
  const offline = !online || errorCode(listing.error) === "network";
  const where = folderLabel(bucket, prefix);

  let content;
  if (listingError && !data.bucketSearch) {
    content = <ErrorView error={listingError} connection={connection} bucket={bucket} onRetry={refresh} />;
  } else if (searchError) {
    content = <ErrorView error={searchError} connection={connection} bucket={bucket} onRetry={refresh} />;
  } else if (active.isLoading) {
    content = showSkeleton && viewMode === "list" ? <SkeletonRows rowHeight={rowHeight} /> : <div className="explorer-pending" aria-busy="true" />;
  } else if (allRows.length === 0) {
    if (data.term) {
      content = data.bucketSearch ? (
        <EmptyState
          icon={<SearchX size={32} />}
          title={`No keys start with "${data.term}"`}
          body="Whole bucket search matches the start of the full key, for example covers/dragon."
          actions={
            <>
              <Button onClick={() => useBrowserStore.getState().setScope("folder")}>Search this folder</Button>
              <Button variant="ghost" onClick={() => useBrowserStore.getState().setSearch("")}>
                Clear search
              </Button>
            </>
          }
        />
      ) : (
        <EmptyState
          icon={<SearchX size={32} />}
          title={`No matches for "${data.term}" in this folder`}
          body={
            data.hasMore
              ? "Search checks names already loaded in this folder. Try searching the whole bucket."
              : "Search checks names in the current folder. Try searching the whole bucket."
          }
          actions={
            <>
              <Button onClick={() => useBrowserStore.getState().setScope("bucket")}>Search whole bucket</Button>
              <Button variant="ghost" onClick={() => useBrowserStore.getState().setSearch("")}>
                Clear search
              </Button>
            </>
          }
        />
      );
    } else if (prefix === "") {
      content = (
        <EmptyState
          icon={<Inbox size={32} />}
          title="This bucket is empty"
          body="Drop files here or use Upload to add some."
          actions={
            <>
              <Button variant="primary" icon={<Upload size={16} />} onClick={() => upload("files")}>
                Upload files
              </Button>
              <Button icon={<FolderPlus size={16} />} onClick={() => setNewFolderOpen(true)}>
                New folder
              </Button>
            </>
          }
        />
      );
    } else {
      content = (
        <EmptyState
          icon={<FolderOpen size={32} />}
          title="This folder is empty"
          body="Drop files here to upload them to this folder."
          actions={
            <Button variant="primary" icon={<Upload size={16} />} onClick={() => upload("files")}>
              Upload files
            </Button>
          }
        />
      );
    }
  } else {
    const common = {
      rows: allRows,
      selected,
      focusId,
      rowHeight,
      highlight: data.term && !data.bucketSearch ? data.term : undefined,
      showPath: data.bucketSearch,
      hasMore: data.hasMore,
      loadingMore: data.loadingMore,
      onLoadMore,
      onRowClick,
      onRowDoubleClick: (row: SelectableRow) => openRow(ctx, row),
      onToggle,
      onKeyDown,
      onContextMenu,
      label: data.bucketSearch ? `Keys starting with ${data.term}` : `Contents of ${where}`,
    };
    content =
      viewMode === "grid" ? (
        <FileGrid
          {...common}
          tileSize={tileSize}
          connectionId={connection.id}
          bucket={bucket}
          showThumbnails={prefs.showThumbnails}
          onColumns={(c) => {
            columnsRef.current = c;
          }}
        />
      ) : (
        <FileTable {...common} sort={sort} onSort={onSort} compactColumns={narrow} />
      );
  }

  const searchNote =
    data.term && !data.bucketSearch && data.hasMore && allRows.length > 0
      ? "Only loaded items are searched. Scroll to load more, or search the whole bucket."
      : null;

  return (
    <div className="explorer">
      <Toolbar
        connectionName={connection.name}
        bucket={bucket}
        prefix={prefix}
        selectedCount={selection.length}
        onDownloadSelected={() => downloadRows(ctx, selection)}
        onMoveSelected={() => moveRows(ctx, selection)}
        onCopyKeys={() => copyKeys(selection)}
        onDeleteSelected={() => deleteRows(ctx, selection)}
        onClearSelection={() => useBrowserStore.getState().clearSelection()}
        search={search}
        onSearch={(v) => useBrowserStore.getState().setSearch(v)}
        scope={scope}
        onScope={(v) => useBrowserStore.getState().setScope(v)}
        onUpload={upload}
        onNewFolder={() => setNewFolderOpen(true)}
        onRefresh={refresh}
        sort={sort}
        onSort={(v) => useBrowserStore.getState().setSort(v)}
        viewMode={viewMode}
        onViewMode={setViewMode}
        density={prefs.density}
        onDensity={(density) => updatePrefs({ density })}
        detailsOpen={detailsOpen}
        onDetailsOpen={setDetailsOpen}
        onCopyPath={() => copyCurrentPath(ctx)}
        onBucketSettings={() => useUiStore.getState().setBucketSettings({ connectionId: connection.id, bucket })}
        onCalculateSize={() => void calculateSize()}
        uploading={uploading}
      />
      {offline && (
        <Banner tone="warning">You are offline. R2Dock will reconnect automatically.</Banner>
      )}
      {!offline && connection.status === "needsAttention" && !attentionDismissed && (
        <Banner
          tone="warning"
          onDismiss={() => setAttentionDismissed(true)}
          actions={
            <Button
              size="sm"
              onClick={() => useUiStore.getState().openConnect({ mode: "edit", id: connection.id, replaceSecret: true, showHelp: false })}
            >
              Replace secret
            </Button>
          }
        >
          {connection.name} needs attention. Its keys may have been changed or removed.
        </Banner>
      )}
      {searchNote && <p className="search-note small secondary">{searchNote}</p>}
      <div className="explorer-body">
        <ContextMenu
          content={
            <ExplorerContextMenu
              ctx={ctx}
              target={menuTarget}
              platform={platform}
              inSearch={data.bucketSearch}
              onUpload={upload}
              onNewFolder={() => setNewFolderOpen(true)}
              onRefresh={refresh}
              onGoToPath={() => useUiStore.getState().setGoToPathOpen(true)}
            />
          }
        >
          <div
            className="explorer-main"
            onContextMenu={(e) => {
              // Right-click outside any row: empty-area menu.
              if (!(e.target as HTMLElement).closest("[data-row-index]")) setMenuTarget({ rows: [] });
            }}
            onClick={(e) => {
              if (e.target === e.currentTarget) useBrowserStore.getState().clearSelection();
            }}
          >
            {content}
          </div>
        </ContextMenu>
        {detailsOpen && (
          <DetailsPanel
            connection={connection}
            bucket={bucket}
            prefix={prefix}
            rows={selection}
            overlay={narrow}
            onClose={() => setDetailsOpen(false)}
          />
        )}
      </div>
    </div>
  );
}
