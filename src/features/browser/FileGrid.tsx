import { useVirtualizer } from "@tanstack/react-virtual";
import { useEffect, useId, useRef, useState } from "react";
import { FileIcon } from "../../components/FileIcon";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import { PREVIEW_IMAGE, THUMBNAIL_LIMIT } from "../../lib/fileTypes";
import { extension } from "../../lib/keys";
import { formatBytes, formatPercent } from "../../lib/format";
import { useMediaUrl } from "../preview/useMediaUrl";
import { moreText, type ListProps } from "./FileTable";
import { NameText, UnusualBadge } from "./NameCell";
import { rowDisplayName, rowName, type Row } from "./rows";
import "./browser.css";

/** Matches --sp-2, the gap between tiles in browser.css. */
const GAP = 8;

function Thumbnail({ connectionId, bucket, entry, size }: { connectionId: string; bucket: string; entry: ObjectEntry; size: number }) {
  const media = useMediaUrl(connectionId, bucket, entry.key);
  const [broken, setBroken] = useState(false);
  if (!media.url || broken) return <FileIcon name={entry.name} size={size} />;
  return <img className="tile-thumb" src={media.url} alt="" draggable={false} loading="lazy" onError={() => setBroken(true)} />;
}

export interface FileGridProps extends ListProps {
  tileSize: number;
  connectionId: string;
  bucket: string;
  showThumbnails: boolean;
  /** Reports the number of columns, for arrow-key navigation. */
  onColumns: (columns: number) => void;
}

export function FileGrid({
  rows,
  selected,
  focusId,
  highlight,
  hasMore,
  loadingMore,
  onLoadMore,
  onRowClick,
  onRowDoubleClick,
  onKeyDown,
  onContextMenu,
  label,
  tileSize,
  connectionId,
  bucket,
  showThumbnails,
  onColumns,
}: FileGridProps) {
  const listId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const [width, setWidth] = useState(800);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const update = () => setWidth(el.clientWidth || 800);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const columns = Math.max(1, Math.floor((width - 24 + GAP) / (tileSize + GAP)));
  useEffect(() => onColumns(columns), [columns, onColumns]);

  const rowCount = Math.ceil(rows.length / columns) + (hasMore ? 1 : 0);
  const rowHeight = tileSize + GAP;
  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 3,
    initialRect: { width: 800, height: 600 },
  });

  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, virtualizer]);

  const items = virtualizer.getVirtualItems();
  const lastLine = items.length > 0 ? (items[items.length - 1]?.index ?? 0) : 0;
  const tileLines = Math.ceil(rows.length / columns);
  useEffect(() => {
    if (hasMore && !loadingMore && lastLine >= tileLines - 1) onLoadMore();
  }, [hasMore, loadingMore, lastLine, tileLines, onLoadMore]);

  const focusIndex = focusId === null ? -1 : rows.findIndex((r) => r.id === focusId);
  useEffect(() => {
    if (focusIndex >= 0) virtualizer.scrollToIndex(Math.floor(focusIndex / columns), { align: "auto" });
  }, [focusIndex, columns, virtualizer]);

  const tileId = (i: number) => `${listId}-tile-${i}`;

  const renderTile = (row: Row, index: number) => {
    if (row.kind === "ghost") {
      return (
        <div key={row.id} role="gridcell" className="tile is-ghost" style={{ width: tileSize, height: tileSize }}>
          <FileIcon name={row.name} size={tileSize > 120 ? 40 : 32} />
          <span className="tile-name">{row.name}</span>
          <span className="tile-meta num">{row.done ? "Uploaded" : `Uploading ${formatPercent(row.transferred, row.total)}`}</span>
        </div>
      );
    }
    const isSelected = selected.has(row.id);
    const name = rowName(row);
    const display = rowDisplayName(row);
    const thumb =
      showThumbnails &&
      row.kind === "file" &&
      PREVIEW_IMAGE.has(extension(row.entry.name)) &&
      row.entry.size < THUMBNAIL_LIMIT;
    return (
      <div
        key={row.id}
        id={tileId(index)}
        role="gridcell"
        aria-selected={isSelected}
        data-row-index={index}
        className={["tile", isSelected ? "is-selected" : "", index === focusIndex ? "is-focused" : ""].filter(Boolean).join(" ")}
        style={{ width: tileSize, height: tileSize }}
        onClick={(e) => onRowClick(row, e)}
        onDoubleClick={() => onRowDoubleClick(row)}
      >
        <span className="tile-visual">
          {thumb && row.kind === "file" ? (
            <Thumbnail connectionId={connectionId} bucket={bucket} entry={row.entry} size={tileSize > 120 ? 40 : 32} />
          ) : (
            <FileIcon name={name} folder={row.kind === "folder"} size={tileSize > 120 ? 40 : 32} />
          )}
        </span>
        <span className="tile-name">
          <NameText text={display} highlight={highlight} />
          {(row.kind === "file" ? row.entry.unusual : row.unusual) && <UnusualBadge />}
        </span>
        <span className="tile-meta num">{row.kind === "folder" ? "Folder" : formatBytes(row.entry.size)}</span>
      </div>
    );
  };

  return (
    <div className="file-grid">
      <div
        ref={scrollRef}
        className="file-scroll grid-scroll"
        role="grid"
        aria-label={label}
        aria-rowcount={hasMore ? -1 : tileLines}
        aria-multiselectable="true"
        aria-activedescendant={focusIndex >= 0 ? tileId(focusIndex) : undefined}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onContextMenu={(e) => {
          const el = (e.target as HTMLElement).closest<HTMLElement>("[data-row-index]");
          const idx = el ? Number(el.dataset.rowIndex) : -1;
          const row = idx >= 0 ? rows[idx] : undefined;
          onContextMenu(row && row.kind !== "ghost" ? row : null);
        }}
      >
        <div className="grid-rows" style={{ height: virtualizer.getTotalSize() }}>
          {items.map((item) => {
            const start = item.index * columns;
            if (start >= rows.length) {
              return (
                <div key="more" role="row" className="grid-line grid-more" style={{ transform: `translateY(${item.start}px)`, height: rowHeight }}>
                  <div role="gridcell" className="more-cell" aria-live="polite">
                    {moreText(rows.length)}
                  </div>
                </div>
              );
            }
            const slice = rows.slice(start, start + columns);
            return (
              <div
                key={item.key}
                role="row"
                aria-rowindex={item.index + 1}
                className="grid-line"
                style={{ transform: `translateY(${item.start}px)`, height: rowHeight }}
              >
                {slice.map((row, i) => renderTile(row, start + i))}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
