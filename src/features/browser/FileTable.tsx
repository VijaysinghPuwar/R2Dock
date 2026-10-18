import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp } from "lucide-react";
import { useEffect, useId, useRef, type KeyboardEvent, type MouseEvent } from "react";
import { ProgressBar } from "../../components/ProgressBar";
import { Skeleton } from "../../components/Controls";
import { Tooltip } from "../../components/Tooltip";
import { formatBytes, formatCount, formatDate, formatIsoLocal, formatPercent } from "../../lib/format";
import { humanType } from "../../lib/fileTypes";
import { NameCell } from "./NameCell";
import { rowDisplayName, rowName, type Row, type SelectableRow, type SortKey, type SortState } from "./rows";
import "./browser.css";

export interface ListProps {
  rows: Row[];
  selected: Set<string>;
  focusId: string | null;
  rowHeight: number;
  /** Search text to highlight in names. */
  highlight?: string;
  /** Show the parent path of each key (whole-bucket search). */
  showPath?: boolean;
  /** More pages exist. */
  hasMore: boolean;
  loadingMore: boolean;
  onLoadMore: () => void;
  onRowClick: (row: SelectableRow, e: MouseEvent) => void;
  onRowDoubleClick: (row: SelectableRow) => void;
  onToggle: (row: SelectableRow) => void;
  onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => void;
  onContextMenu: (row: SelectableRow | null) => void;
  label: string;
  /** Hide the Type column (narrow windows). */
  compactColumns?: boolean;
}

export interface FileTableProps extends ListProps {
  sort: SortState;
  onSort: (key: SortKey) => void;
}

/** Footer text while more pages exist. Never claims a total. */
export function moreText(loaded: number): string {
  return `Showing ${formatCount(loaded)} of more objects. Loading more...`;
}

function SortHeader({ label, k, sort, onSort, className }: { label: string; k: SortKey; sort: SortState; onSort: (k: SortKey) => void; className: string }) {
  const active = sort.key === k;
  return (
    <div
      role="columnheader"
      className={className}
      aria-sort={active ? (sort.dir === "asc" ? "ascending" : "descending") : "none"}
    >
      <button type="button" className="col-sort" onClick={() => onSort(k)} tabIndex={-1}>
        {label}
        {active && (sort.dir === "asc" ? <ArrowUp size={12} aria-hidden="true" /> : <ArrowDown size={12} aria-hidden="true" />)}
      </button>
    </div>
  );
}

export function FileTable({
  rows,
  selected,
  focusId,
  rowHeight,
  highlight,
  showPath,
  hasMore,
  loadingMore,
  onLoadMore,
  onRowClick,
  onRowDoubleClick,
  onToggle,
  onKeyDown,
  onContextMenu,
  label,
  sort,
  onSort,
  compactColumns,
}: FileTableProps) {
  const listId = useId();
  const scrollRef = useRef<HTMLDivElement>(null);
  const count = rows.length + (hasMore ? 1 : 0);
  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
    initialRect: { width: 800, height: 600 },
  });

  useEffect(() => {
    virtualizer.measure();
  }, [rowHeight, virtualizer]);

  const items = virtualizer.getVirtualItems();
  const lastIndex = items.length > 0 ? (items[items.length - 1]?.index ?? 0) : 0;
  useEffect(() => {
    if (hasMore && !loadingMore && lastIndex >= rows.length - 1) onLoadMore();
  }, [hasMore, loadingMore, lastIndex, rows.length, onLoadMore]);

  const focusIndex = focusId === null ? -1 : rows.findIndex((r) => r.id === focusId);
  useEffect(() => {
    if (focusIndex >= 0) virtualizer.scrollToIndex(focusIndex, { align: "auto" });
  }, [focusIndex, virtualizer]);

  const anySelected = selected.size > 0;
  const rowId = (i: number) => `${listId}-row-${i}`;

  return (
    <div className={compactColumns ? "file-table is-compact-cols" : "file-table"}>
      <div
        ref={scrollRef}
        className="file-scroll"
        role="grid"
        aria-label={label}
        aria-rowcount={hasMore ? -1 : rows.length + 1}
        aria-multiselectable="true"
        aria-activedescendant={focusIndex >= 0 ? rowId(focusIndex) : undefined}
        tabIndex={0}
        onKeyDown={onKeyDown}
        onContextMenu={(e) => {
          const el = (e.target as HTMLElement).closest<HTMLElement>("[data-row-index]");
          const idx = el ? Number(el.dataset.rowIndex) : -1;
          const row = idx >= 0 ? rows[idx] : undefined;
          onContextMenu(row && row.kind !== "ghost" ? row : null);
        }}
      >
        <div role="row" aria-rowindex={1} className="file-header">
          <div role="columnheader" className="cell col-check">
            <span className="visually-hidden">Selected</span>
          </div>
          <SortHeader label="Name" k="name" sort={sort} onSort={onSort} className="cell col-name" />
          {!compactColumns && <SortHeader label="Type" k="type" sort={sort} onSort={onSort} className="cell col-type" />}
          <SortHeader label="Size" k="size" sort={sort} onSort={onSort} className="cell col-size" />
          <SortHeader label="Modified" k="modified" sort={sort} onSort={onSort} className="cell col-modified" />
        </div>
        <div className="file-rows" style={{ height: virtualizer.getTotalSize() }}>
          {items.map((item) => {
            const row = rows[item.index];
            const style = { transform: `translateY(${item.start}px)`, height: rowHeight };
            if (!row) {
              return (
                <div key="more" role="row" className="file-row file-more" style={style} aria-rowindex={item.index + 2}>
                  <div role="gridcell" className="cell more-cell" aria-live="polite">
                    {moreText(rows.length)}
                  </div>
                </div>
              );
            }
            if (row.kind === "ghost") {
              return (
                <div key={row.id} role="row" className="file-row is-ghost" style={style} aria-rowindex={item.index + 2} data-row-index={item.index}>
                  <div role="gridcell" className="cell col-check" />
                  <div role="gridcell" className="cell col-name">
                    <NameCell name={row.name} displayName={row.name} folder={false} unusual={false} />
                  </div>
                  {!compactColumns && (
                    <div role="gridcell" className="cell col-type secondary truncate">
                      {humanType(row.name)}
                    </div>
                  )}
                  <div role="gridcell" className="cell col-size ghost-progress">
                    <ProgressBar value={row.done ? row.total : row.transferred} max={row.total} label={row.done ? "Uploaded" : `Uploading ${formatPercent(row.transferred, row.total)}`} />
                    <span className="small secondary num">{row.done ? "Uploaded" : `Uploading ${formatPercent(row.transferred, row.total)}`}</span>
                  </div>
                  <div role="gridcell" className="cell col-modified" />
                </div>
              );
            }
            const isSelected = selected.has(row.id);
            const isFocused = item.index === focusIndex;
            const name = rowName(row);
            const display = rowDisplayName(row);
            return (
              <div
                key={row.id}
                id={rowId(item.index)}
                role="row"
                aria-rowindex={item.index + 2}
                aria-selected={isSelected}
                data-row-index={item.index}
                className={["file-row", isSelected ? "is-selected" : "", isFocused ? "is-focused" : "", anySelected ? "show-check" : ""]
                  .filter(Boolean)
                  .join(" ")}
                style={style}
                onClick={(e) => onRowClick(row, e)}
                onDoubleClick={() => onRowDoubleClick(row)}
              >
                <div role="gridcell" className="cell col-check">
                  <input
                    type="checkbox"
                    className="checkbox row-check"
                    checked={isSelected}
                    tabIndex={-1}
                    aria-label={`Select ${display}`}
                    onClick={(e) => e.stopPropagation()}
                    onChange={() => onToggle(row)}
                  />
                </div>
                <div role="gridcell" className="cell col-name">
                  <NameCell
                    name={name}
                    displayName={display}
                    folder={row.kind === "folder"}
                    unusual={row.kind === "file" ? row.entry.unusual : row.unusual}
                    highlight={highlight}
                    pathKey={showPath && row.kind === "file" ? row.entry.key : undefined}
                  />
                </div>
                {!compactColumns && (
                  <div role="gridcell" className="cell col-type secondary truncate">
                    {row.kind === "folder" ? "Folder" : humanType(name)}
                  </div>
                )}
                <div role="gridcell" className="cell col-size num secondary">
                  {row.kind === "folder" ? "--" : formatBytes(row.entry.size)}
                </div>
                <div role="gridcell" className="cell col-modified num secondary">
                  {row.kind === "file" && row.entry.lastModified ? (
                    <Tooltip content={formatIsoLocal(row.entry.lastModified)}>
                      <span>{formatDate(row.entry.lastModified)}</span>
                    </Tooltip>
                  ) : (
                    "--"
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

/** Placeholder rows while the first page loads. */
export function SkeletonRows({ rowHeight }: { rowHeight: number }) {
  return (
    <div className="file-table" aria-hidden="true">
      <div className="file-scroll">
        <div className="file-header">
          <div className="cell col-check" />
          <div className="cell col-name small secondary">Name</div>
          <div className="cell col-type small secondary">Type</div>
          <div className="cell col-size small secondary">Size</div>
          <div className="cell col-modified small secondary">Modified</div>
        </div>
        {Array.from({ length: 8 }, (_, i) => (
          <div key={i} className="file-row is-static" style={{ height: rowHeight }}>
            <div className="cell col-check" />
            <div className="cell col-name">
              <Skeleton width={`${35 + ((i * 23) % 45)}%`} />
            </div>
            <div className="cell col-type">
              <Skeleton width={70} />
            </div>
            <div className="cell col-size">
              <Skeleton width={48} />
            </div>
            <div className="cell col-modified">
              <Skeleton width={84} />
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
