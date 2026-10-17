import { ArrowDownUp, Pause, RotateCw, Trash2 } from "lucide-react";
import { useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { transferClearFinished, transferPauseAll } from "../../ipc/commands";
import { Button } from "../../components/Button";
import { SegmentedControl, Tabs } from "../../components/Controls";
import { EmptyState } from "../../components/Feedback";
import { joinDot } from "../../lib/format";
import { controlMany, run } from "./actions";
import { buildItems, filterDirection, type DirectionFilter } from "./groups";
import { InterruptedBanner } from "./InterruptedBanner";
import { countTabs, type TransferTab } from "./stats";
import { TransferList } from "./TransferList";
import { orderedJobs, useTransferStore } from "./transferStore";
import "./transfers.css";

const EMPTY_TEXT: Record<TransferTab, { title: string; body: string }> = {
  active: { title: "No active transfers", body: "Uploads and downloads in progress show here." },
  queued: { title: "Nothing queued", body: "Transfers wait here until a slot is free." },
  completed: { title: "No completed transfers", body: "Finished transfers from this session show here." },
  failed: { title: "No failed transfers", body: "Transfers that could not finish show here." },
};

export function useTransferGroups() {
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set());
  const toggle = (id: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  return { expanded, toggle };
}

export function TransfersPage() {
  const jobs = useTransferStore(useShallow((s) => orderedJobs(s)));
  const [tab, setTab] = useState<TransferTab>("active");
  const [filter, setFilter] = useState<DirectionFilter>("all");
  const { expanded, toggle } = useTransferGroups();

  const filtered = useMemo(() => filterDirection(jobs, filter), [jobs, filter]);
  const counts = useMemo(() => countTabs(filtered), [filtered]);
  const items = useMemo(() => buildItems(filtered, tab, expanded), [filtered, tab, expanded]);
  const failedIds = useMemo(() => filtered.filter((j) => j.state === "failed").map((j) => j.id), [filtered]);

  const summary = joinDot([
    `${counts.active.toLocaleString("en-US")} active`,
    `${counts.queued.toLocaleString("en-US")} queued`,
    counts.failed > 0 ? `${counts.failed.toLocaleString("en-US")} failed` : null,
  ]);

  return (
    <div className="xfer-page">
      <InterruptedBanner onReview={() => setTab("active")} />
      <div className="xfer-page-head">
        <div className="page-header">
          <div className="xfer-title-block">
            <h1 className="page-title">Transfers</h1>
            <p className="page-subtitle num">{summary}</p>
          </div>
          <div className="spacer" />
          <SegmentedControl<DirectionFilter>
            label="Show"
            value={filter}
            onChange={setFilter}
            options={[
              { value: "all", label: "All" },
              { value: "upload", label: "Uploads" },
              { value: "download", label: "Downloads" },
            ]}
          />
          <Button icon={<Pause size={16} />} onClick={() => run(transferPauseAll)} title="Only affects transfers that support pausing">
            Pause all
          </Button>
          <Button icon={<Trash2 size={16} />} onClick={() => run(transferClearFinished)} disabled={counts.completed === 0}>
            Clear completed
          </Button>
        </div>
        <div className="xfer-tabs-row">
          <Tabs<TransferTab>
            label="Transfer status"
            idPrefix="xfer-page"
            value={tab}
            onChange={setTab}
            items={[
              { value: "active", label: "Active", count: counts.active },
              { value: "queued", label: "Queued", count: counts.queued },
              { value: "completed", label: "Completed", count: counts.completed },
              { value: "failed", label: "Failed", count: counts.failed },
            ]}
          />
          {tab === "failed" && failedIds.length > 0 && (
            <Button size="sm" variant="primary" icon={<RotateCw size={14} />} onClick={() => controlMany(failedIds, { action: "retry" })}>
              Retry all
            </Button>
          )}
        </div>
      </div>
      {items.length === 0 ? (
        <div className="xfer-empty" role="tabpanel" id={`xfer-page-panel-${tab}`} aria-labelledby={`xfer-page-tab-${tab}`}>
          <EmptyState icon={<ArrowDownUp size={32} />} title={EMPTY_TEXT[tab].title} body={EMPTY_TEXT[tab].body} />
        </div>
      ) : (
        <TransferList
          items={items}
          label={`${tab} transfers`}
          onToggleGroup={toggle}
          id={`xfer-page-panel-${tab}`}
          labelledBy={`xfer-page-tab-${tab}`}
        />
      )}
    </div>
  );
}
