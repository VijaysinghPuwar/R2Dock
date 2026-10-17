import { useVirtualizer } from "@tanstack/react-virtual";
import { useRef } from "react";
import type { ListItem } from "./groups";
import { GroupRow, TransferRow } from "./TransferRow";

export interface TransferListProps {
  items: ListItem[];
  compact?: boolean;
  label: string;
  onToggleGroup: (id: string) => void;
  id?: string;
  labelledBy?: string;
}

/** Virtualized list of transfer rows and group rows. */
export function TransferList({ items, compact = false, label, onToggleGroup, id, labelledBy }: TransferListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const rowH = compact ? 52 : 56;
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowH,
    overscan: 8,
    initialRect: { width: 800, height: 600 },
    getItemKey: (i) => {
      const it = items[i];
      if (!it) return i;
      return it.type === "group" ? `g:${it.group.id}` : `j:${it.job.id}`;
    },
  });

  return (
    <div
      ref={scrollRef}
      className="xfer-scroll"
      id={id}
      role="tabpanel"
      aria-labelledby={labelledBy}
      tabIndex={0}
    >
      <div role="list" aria-label={label} style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((v) => {
          const item = items[v.index];
          if (!item) return null;
          return (
            <div
              key={v.key}
              data-index={v.index}
              ref={virtualizer.measureElement}
              className="xfer-vrow"
              style={{ transform: `translateY(${v.start}px)` }}
            >
              {item.type === "group" ? (
                <GroupRow
                  group={item.group}
                  jobs={item.jobs}
                  all={item.all}
                  expanded={item.expanded}
                  onToggle={() => onToggleGroup(item.group.id)}
                />
              ) : (
                <TransferRow job={item.job} compact={compact} child={item.child} />
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
