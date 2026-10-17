import { useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, CircleAlert, Clock } from "lucide-react";
import { useNavStore } from "../../app/navStore";
import { qk, useActiveConnection } from "../../app/queries";
import { Button } from "../../components/Button";
import { Badge, Skeleton, type BadgeTone } from "../../components/Controls";
import { EmptyState } from "../../components/Feedback";
import { Tooltip } from "../../components/Tooltip";
import { errorMessage, transferHistory } from "../../ipc/commands";
import type { HistoryEntry } from "../../ipc/bindings/HistoryEntry";
import type { JobState } from "../../ipc/bindings/JobState";
import { formatBytes, formatDate, formatIsoLocal } from "../../lib/format";
import { parentPrefix, visibleText } from "../../lib/keys";
import "./views.css";

const HISTORY_LIMIT = 200;

function stateBadge(state: JobState): { tone: BadgeTone; label: string } {
  switch (state) {
    case "completed":
      return { tone: "success", label: "Completed" };
    case "failed":
      return { tone: "danger", label: "Failed" };
    case "cancelled":
      return { tone: "neutral", label: "Cancelled" };
    case "skipped":
      return { tone: "neutral", label: "Skipped" };
    case "interrupted":
      return { tone: "warning", label: "Interrupted" };
    default:
      return { tone: "neutral", label: "In progress" };
  }
}

function RecentRow({ entry, canOpen }: { entry: HistoryEntry; canOpen: boolean }) {
  const openBucket = useNavStore((s) => s.openBucket);
  const badge = stateBadge(entry.state);
  const Icon = entry.direction === "upload" ? ArrowUp : ArrowDown;
  const content = (
    <>
      <Icon size={16} className="recent-dir" aria-label={entry.direction === "upload" ? "Upload" : "Download"} />
      <span className="recent-name">
        <span className="truncate">{visibleText(entry.name)}</span>
        <span className="mono muted truncate">
          {entry.bucket}/{visibleText(entry.key)}
        </span>
      </span>
      <span className="num secondary recent-size">{formatBytes(entry.bytes)}</span>
      <Tooltip content={formatIsoLocal(entry.finishedAt)}>
        <span className="num secondary recent-date">{formatDate(entry.finishedAt)}</span>
      </Tooltip>
      <Badge tone={badge.tone}>{badge.label}</Badge>
    </>
  );
  if (!canOpen) {
    return <li className="recent-row">{content}</li>;
  }
  return (
    <li>
      <button
        type="button"
        className="recent-row is-button"
        onClick={() => openBucket(entry.bucket, parentPrefix(entry.key))}
        title="Open containing folder"
      >
        {content}
      </button>
    </li>
  );
}

export function RecentView() {
  const active = useActiveConnection();
  const history = useQuery({ queryKey: qk.history, queryFn: () => transferHistory(HISTORY_LIMIT) });

  return (
    <div className="page">
      <div className="page-header">
        <h1 className="page-title">Recent</h1>
      </div>
      {history.isPending && (
        <div className="view-list" aria-busy="true">
          {Array.from({ length: 8 }, (_, i) => (
            <div key={i} className="recent-row">
              <Skeleton width="60%" />
            </div>
          ))}
        </div>
      )}
      {history.isError && (
        <EmptyState
          icon={<CircleAlert size={32} />}
          title="Could not load history"
          body={errorMessage(history.error)}
          actions={<Button onClick={() => void history.refetch()}>Try again</Button>}
        />
      )}
      {history.data && history.data.length === 0 && (
        <EmptyState icon={<Clock size={32} />} title="Nothing here yet" body="Files you upload or download appear here." />
      )}
      {history.data && history.data.length > 0 && (
        <ul className="view-list">
          {history.data.map((entry) => (
            <RecentRow key={entry.id} entry={entry} canOpen={!!active && entry.connectionId === active.id} />
          ))}
        </ul>
      )}
    </div>
  );
}
