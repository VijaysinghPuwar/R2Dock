import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, CircleAlert, Pause, Play, RotateCw, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { transferShow } from "../../ipc/commands";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { GroupInfo } from "../../ipc/bindings/GroupInfo";
import { Button, IconButton } from "../../components/Button";
import { OpenActivityLogButton } from "../../components/Feedback";
import { ProgressBar, type ProgressTone } from "../../components/ProgressBar";
import { useNavStore } from "../../app/navStore";
import {
  formatBytes,
  formatDuration,
  formatEta,
  formatEtaWords,
  formatPercent,
  formatSpeed,
  joinDot,
} from "../../lib/format";
import { parentPrefix, visibleText } from "../../lib/keys";
import { control, controlMany, run } from "./actions";
import { compressionLine } from "./compressionText";
import { groupText } from "./groups";
import { isTerminal } from "./stats";
import "./transfers.css";

const PAUSABLE_STATES = new Set(["queued", "compressing", "preparing", "running", "retrying"]);

function toneFor(job: JobSnapshot): ProgressTone {
  switch (job.state) {
    case "paused":
    case "interrupted":
    case "needsDecision":
      return "warning";
    case "failed":
      return "danger";
    case "completed":
      return "success";
    default:
      return "accent";
  }
}

function durationSecs(job: JobSnapshot): number | null {
  if (!job.finishedAt) return null;
  const start = Date.parse(job.createdAt);
  const end = Date.parse(job.finishedAt);
  if (Number.isNaN(start) || Number.isNaN(end) || end < start) return null;
  return (end - start) / 1000;
}

function destination(job: JobSnapshot): string {
  if (job.direction === "upload") {
    const prefix = parentPrefix(job.key);
    return `Upload to ${visibleText(job.bucket + (prefix ? `/${prefix.slice(0, -1)}` : ""))}`;
  }
  return `Download to ${job.localDisplay}`;
}

/** Status line text and its screen-reader version. */
function statusParts(job: JobSnapshot): { text: string; aria: string } {
  const parts: Array<string | null> = [];
  const ariaParts: Array<string | null> = [];
  const push = (t: string | null, a?: string) => {
    parts.push(t);
    ariaParts.push(a ?? t);
  };
  switch (job.state) {
    case "queued":
      push("Queued");
      break;
    case "compressing": {
      const fraction = job.compression?.kind === "pending" ? job.compression.fraction : null;
      push(fraction === null ? "Compressing" : `Compressing ${Math.floor(fraction * 100)}%`);
      break;
    }
    case "preparing":
      push("Preparing...");
      break;
    case "verifying":
      push("Verifying...");
      break;
    case "paused":
      push("Paused");
      break;
    case "interrupted":
      push("Interrupted");
      break;
    case "needsDecision":
      push("Waiting for a decision: a file with this name already exists.");
      break;
    case "retrying":
      push(`Retrying (${job.attempt} of ${job.maxAttempts})`);
      break;
    case "cancelled":
      push("Cancelled");
      break;
    case "skipped":
      push("Skipped");
      break;
    case "failed":
      push(job.error?.message ?? "Failed");
      break;
    case "completed": {
      const secs = durationSecs(job);
      push(job.direction === "upload" ? "Uploaded" : "Downloaded");
      push(formatBytes(job.totalBytes));
      push(compressionLine(job.compression));
      if (secs !== null) {
        push(formatDuration(secs));
        if (secs > 0) push(`${formatSpeed(job.totalBytes / secs)} average`);
      }
      break;
    }
    case "running":
      break;
  }
  if (job.state === "running") {
    if (job.speedBps !== null) push(formatSpeed(job.speedBps));
    if (job.etaSecs !== null) push(formatEta(job.etaSecs), formatEtaWords(job.etaSecs));
  }
  if (job.partsTotal !== null && !isTerminal(job.state)) {
    push(`Part ${Math.min((job.partsDone ?? 0) + 1, job.partsTotal)} of ${job.partsTotal}`);
  }
  if (job.state === "running" && parts.length === 0) push("Starting...");
  return { text: joinDot(parts), aria: joinDot(ariaParts) };
}

export function JobActions({ job, compact = false }: { job: JobSnapshot; compact?: boolean }) {
  const size = "sm" as const;
  const buttons: ReactNode[] = [];
  if (job.state === "needsDecision") {
    buttons.push(
      <Button key="replace" size={size} variant="secondary" onClick={() => control(job.id, { action: "resolve", policy: "replace" })}>
        Replace
      </Button>,
      <Button key="skip" size={size} variant="secondary" onClick={() => control(job.id, { action: "resolve", policy: "skip" })}>
        Skip
      </Button>,
      <Button key="both" size={size} variant="secondary" onClick={() => control(job.id, { action: "resolve", policy: "keepBoth" })}>
        Keep both
      </Button>,
    );
  }
  if (job.pausable && PAUSABLE_STATES.has(job.state)) {
    buttons.push(<IconButton key="pause" size={size} label="Pause" icon={<Pause size={14} />} onClick={() => control(job.id, { action: "pause" })} />);
  }
  if (job.state === "paused" || job.state === "interrupted") {
    buttons.push(<IconButton key="resume" size={size} label="Resume" icon={<Play size={14} />} onClick={() => control(job.id, { action: "resume" })} />);
  }
  if (job.state === "failed") {
    buttons.push(
      compact ? (
        <IconButton key="retry" size={size} label="Retry" icon={<RotateCw size={14} />} onClick={() => control(job.id, { action: "retry" })} />
      ) : (
        <Button key="retry" size={size} variant="secondary" icon={<RotateCw size={14} />} onClick={() => control(job.id, { action: "retry" })}>
          Retry
        </Button>
      ),
    );
  }
  if (job.state === "completed" && job.direction === "download") {
    buttons.push(
      <Button key="show" size={size} variant="ghost" onClick={() => run(() => transferShow(job.id))}>
        Show in folder
      </Button>,
    );
  }
  if (job.state === "completed" && job.direction === "upload" && !compact) {
    buttons.push(
      <Button key="bucket" size={size} variant="ghost" onClick={() => useNavStore.getState().openBucket(job.bucket, parentPrefix(job.key))}>
        Show in bucket
      </Button>,
    );
  }
  if (!isTerminal(job.state)) {
    buttons.push(<IconButton key="cancel" size={size} label="Cancel" icon={<X size={14} />} onClick={() => control(job.id, { action: "cancel" })} />);
  } else {
    buttons.push(<IconButton key="dismiss" size={size} label="Dismiss" icon={<X size={14} />} onClick={() => control(job.id, { action: "dismiss" })} />);
  }
  return <div className="xfer-actions">{buttons}</div>;
}

export interface TransferRowProps {
  job: JobSnapshot;
  compact?: boolean;
  child?: boolean;
}

export function TransferRow({ job, compact = false, child = false }: TransferRowProps) {
  const [showDetails, setShowDetails] = useState(false);
  const status = statusParts(job);
  const pct = formatPercent(job.transferredBytes, job.totalBytes);
  const indeterminate = job.state === "preparing" || job.state === "verifying";
  const Icon = job.direction === "upload" ? ArrowUp : ArrowDown;
  const sizeText =
    job.state === "completed" ? formatBytes(job.totalBytes) : `${formatBytes(job.transferredBytes)} of ${formatBytes(job.totalBytes)}`;

  return (
    <div
      className={["xfer-row", compact ? "is-compact" : "", child ? "is-child" : "", `state-${job.state}`].filter(Boolean).join(" ")}
      role="listitem"
      aria-label={`${job.direction === "upload" ? "Upload" : "Download"} ${job.name}`}
    >
      <span className={`xfer-dir dir-${job.direction}`} aria-hidden="true">
        {job.state === "failed" ? <CircleAlert size={16} /> : <Icon size={16} />}
      </span>
      <div className="xfer-main">
        <div className="xfer-line1">
          <span className="xfer-name truncate" title={job.name}>
            {job.name}
          </span>
          <span className="xfer-size small secondary num">{sizeText}</span>
        </div>
        <div className="xfer-line2">
          <span className="xfer-dest small muted truncate">{destination(job)}</span>
          <ProgressBar
            className="xfer-progress"
            value={job.state === "completed" ? 1 : job.transferredBytes}
            max={job.state === "completed" ? 1 : job.totalBytes}
            label={indeterminate ? status.text || "Working" : pct}
            tone={toneFor(job)}
            indeterminate={indeterminate}
          />
          <span className="xfer-pct small secondary num">{job.state === "completed" ? "100%" : pct}</span>
        </div>
        <div className="xfer-line3">
          <span className={`xfer-status small truncate ${job.state === "failed" ? "is-failed" : "secondary"}`}>
            {status.text === status.aria ? (
              status.text
            ) : (
              <>
                <span aria-hidden="true">{status.text}</span>
                <span className="visually-hidden">{status.aria}</span>
              </>
            )}
          </span>
          {job.state === "failed" && job.error && !compact && (
            <button type="button" className="link-button small" aria-expanded={showDetails} onClick={() => setShowDetails((v) => !v)}>
              Details
            </button>
          )}
          <JobActions job={job} compact={compact} />
        </div>
        {showDetails && job.error && (
          <>
            <pre className="xfer-detail mono selectable">
              {joinDot([job.error.code, job.error.detail])}
            </pre>
            <OpenActivityLogButton />
          </>
        )}
      </div>
    </div>
  );
}

export function GroupRow({
  group,
  jobs,
  all,
  expanded,
  onToggle,
}: {
  group: GroupInfo;
  /** Children in the current tab. */
  jobs: JobSnapshot[];
  /** All children of the group. */
  all: JobSnapshot[];
  expanded: boolean;
  onToggle: () => void;
}) {
  const done = all.reduce((n, j) => n + (j.state === "completed" ? j.totalBytes : j.transferredBytes), 0);
  const total = all.reduce((n, j) => n + j.totalBytes, 0);
  const pct = formatPercent(done, total);
  const failed = all.filter((j) => j.state === "failed");
  const pausable = jobs.filter((j) => j.pausable && PAUSABLE_STATES.has(j.state));
  const resumable = jobs.filter((j) => j.state === "paused" || j.state === "interrupted");
  const cancellable = all.filter((j) => !isTerminal(j.state));
  const Icon = (all[0]?.direction ?? "upload") === "upload" ? ArrowUp : ArrowDown;
  const tone: ProgressTone = failed.length > 0 ? "danger" : cancellable.length === 0 ? "success" : "accent";

  return (
    <div className="xfer-row is-group" role="listitem">
      <button
        type="button"
        className="xfer-chevron"
        aria-expanded={expanded}
        aria-label={expanded ? `Collapse ${group.label}` : `Expand ${group.label}`}
        onClick={onToggle}
      >
        {expanded ? <ChevronDown size={16} /> : <ChevronRight size={16} />}
      </button>
      <span className="xfer-dir" aria-hidden="true">
        <Icon size={16} />
      </span>
      <div className="xfer-main">
        <div className="xfer-line1">
          <span className="xfer-name truncate">{groupText(group, all)}</span>
          <span className="xfer-size small secondary num">{`${formatBytes(done)} of ${formatBytes(total)}`}</span>
        </div>
        <div className="xfer-line2">
          <span className="xfer-dest small muted truncate">{failed.length > 0 ? `${failed.length} failed` : ""}</span>
          <ProgressBar className="xfer-progress" value={done} max={total} label={pct} tone={tone} />
          <span className="xfer-pct small secondary num">{pct}</span>
        </div>
        <div className="xfer-line3">
          <span className="xfer-status small secondary truncate" />
          <div className="xfer-actions">
            {pausable.length > 0 && (
              <IconButton size="sm" label="Pause folder" icon={<Pause size={14} />} onClick={() => controlMany(pausable.map((j) => j.id), { action: "pause" })} />
            )}
            {resumable.length > 0 && (
              <IconButton size="sm" label="Resume folder" icon={<Play size={14} />} onClick={() => controlMany(resumable.map((j) => j.id), { action: "resume" })} />
            )}
            {failed.length > 0 && (
              <Button size="sm" variant="secondary" icon={<RotateCw size={14} />} onClick={() => controlMany(failed.map((j) => j.id), { action: "retry" })}>
                Retry failed
              </Button>
            )}
            {cancellable.length > 0 && (
              <IconButton size="sm" label="Cancel folder" icon={<X size={14} />} onClick={() => controlMany(cancellable.map((j) => j.id), { action: "cancel" })} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
