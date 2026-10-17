import { ArrowDownUp, ChevronDown, ChevronUp } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { IconButton } from "../../components/Button";
import { StatusDot, Tabs } from "../../components/Controls";
import { EmptyState } from "../../components/Feedback";
import { ProgressBar } from "../../components/ProgressBar";
import { useNavStore } from "../../app/navStore";
import { useUiStore } from "../../app/uiStore";
import { formatPercent } from "../../lib/format";
import { buildItems } from "./groups";
import { aggregate, countTabs, type TransferTab } from "./stats";
import { TransferList } from "./TransferList";
import { useTransferGroups } from "./TransfersPage";
import { orderedJobs, useTransferStore } from "./transferStore";
import "./transfers.css";

const FADE_AFTER_MS = 8000;
const FADE_DURATION_MS = 240;

function setToastOffset(value: string) {
  document.documentElement.style.setProperty("--toast-offset", value);
}

export function TransferDock() {
  const seen = useTransferStore((s) => s.seen);
  const jobs = useTransferStore(useShallow((s) => orderedJobs(s)));
  const expanded = useUiStore((s) => s.dockExpanded);
  const setExpanded = useUiStore((s) => s.setDockExpanded);
  const view = useNavStore((s) => s.loc.view);
  const [tab, setTab] = useState<TransferTab>("active");
  const { expanded: openGroups, toggle } = useTransferGroups();

  const agg = useMemo(() => aggregate(jobs), [jobs]);
  const counts = useMemo(() => countTabs(jobs), [jobs]);
  const items = useMemo(() => (expanded ? buildItems(jobs, tab, openGroups) : []), [expanded, jobs, tab, openGroups]);

  // Hide "All transfers complete" after 8 s, until new activity arrives.
  const activityKey = `${jobs.length}:${jobs[jobs.length - 1]?.id ?? ""}:${agg.allDone ? "done" : "busy"}`;
  const [fadedKey, setFadedKey] = useState<string | null>(null);
  const [fadingKey, setFadingKey] = useState<string | null>(null);
  const canFade = agg.allDone && agg.failed === 0 && !expanded;

  useEffect(() => {
    if (!canFade) return;
    const t1 = window.setTimeout(() => setFadingKey(activityKey), FADE_AFTER_MS);
    const t2 = window.setTimeout(() => setFadedKey(activityKey), FADE_AFTER_MS + FADE_DURATION_MS);
    return () => {
      window.clearTimeout(t1);
      window.clearTimeout(t2);
    };
  }, [canFade, activityKey]);

  const hidden = !seen || (canFade && fadedKey === activityKey);
  const fading = canFade && fadingKey === activityKey && !hidden;

  useEffect(() => {
    setToastOffset(hidden ? "0px" : expanded ? "var(--dock-h-expanded)" : "var(--dock-h)");
  }, [hidden, expanded]);
  useEffect(() => () => setToastOffset("0px"), []);

  if (hidden) return null;

  const pct = agg.total > 0 ? formatPercent(agg.done, agg.total) : "0%";
  const openPage = () => {
    setExpanded(false);
    useNavStore.getState().navigate({ view: "transfers" });
  };

  return (
    <section
      className={["dock", expanded ? "is-expanded" : "", fading ? "is-fading" : ""].filter(Boolean).join(" ")}
      aria-label="Transfers"
    >
      {!agg.allDone && agg.moving > 0 && (
        <ProgressBar className="dock-line" thin value={agg.done} max={agg.total} label={pct} />
      )}
      <div className="dock-bar">
        <ArrowDownUp size={16} className="dock-icon" aria-hidden="true" />
        {agg.failed > 0 && <StatusDot tone="danger" label="Some transfers failed" />}
        <span className="dock-text truncate num">
          {agg.text}
        </span>
        <div className="spacer" />
        {view !== "transfers" && (
          <button type="button" className="link-button small" onClick={openPage}>
            Open Transfers
          </button>
        )}
        <IconButton
          label={expanded ? "Collapse transfers" : "Expand transfers"}
          icon={expanded ? <ChevronDown size={16} /> : <ChevronUp size={16} />}
          shortcut={undefined}
          tooltipSide="top"
          aria-expanded={expanded}
          onClick={() => setExpanded(!expanded)}
        />
      </div>
      {expanded && (
        <div className="dock-body">
          <Tabs<TransferTab>
            className="dock-tabs"
            label="Transfer status"
            idPrefix="xfer-dock"
            value={tab}
            onChange={setTab}
            items={[
              { value: "active", label: "Active", count: counts.active },
              { value: "queued", label: "Queued", count: counts.queued },
              { value: "completed", label: "Completed", count: counts.completed },
              { value: "failed", label: "Failed", count: counts.failed },
            ]}
          />
          {items.length === 0 ? (
            <div className="dock-empty" role="tabpanel" id={`xfer-dock-panel-${tab}`} aria-labelledby={`xfer-dock-tab-${tab}`}>
              <EmptyState compact icon={<ArrowDownUp size={24} />} title="Nothing here" />
            </div>
          ) : (
            <TransferList
              items={items}
              compact
              label={`${tab} transfers`}
              onToggleGroup={toggle}
              id={`xfer-dock-panel-${tab}`}
              labelledBy={`xfer-dock-tab-${tab}`}
            />
          )}
        </div>
      )}
    </section>
  );
}
