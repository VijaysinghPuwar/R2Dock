import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { JobState } from "../../ipc/bindings/JobState";
import { formatEta, formatPercent, formatSpeed, joinDot, plural } from "../../lib/format";

export type TransferTab = "active" | "queued" | "completed" | "failed";

const TAB_OF: Record<JobState, TransferTab> = {
  preparing: "active",
  running: "active",
  retrying: "active",
  verifying: "active",
  paused: "active",
  needsDecision: "active",
  interrupted: "active",
  queued: "queued",
  completed: "completed",
  skipped: "completed",
  cancelled: "completed",
  failed: "failed",
};

export function tabOf(state: JobState): TransferTab {
  return TAB_OF[state];
}

/** States where work is happening or about to happen without user input. */
export function isMoving(state: JobState): boolean {
  return state === "queued" || state === "preparing" || state === "running" || state === "retrying" || state === "verifying";
}

export function isTerminal(state: JobState): boolean {
  return state === "completed" || state === "failed" || state === "cancelled" || state === "skipped";
}

export interface TabCounts {
  active: number;
  queued: number;
  completed: number;
  failed: number;
}

export function countTabs(jobs: JobSnapshot[]): TabCounts {
  const counts: TabCounts = { active: 0, queued: 0, completed: 0, failed: 0 };
  for (const j of jobs) counts[tabOf(j.state)] += 1;
  return counts;
}

export interface Aggregate {
  /** Collapsed dock text. */
  text: string;
  /** Text for screen readers. */
  ariaText: string;
  done: number;
  total: number;
  failed: number;
  moving: number;
  allDone: boolean;
}

/** Aggregate status for the collapsed dock bar. */
export function aggregate(jobs: JobSnapshot[]): Aggregate {
  const moving = jobs.filter((j) => isMoving(j.state));
  const failed = jobs.filter((j) => j.state === "failed").length;
  const completed = jobs.filter((j) => j.state === "completed").length;
  const paused = jobs.filter((j) => j.state === "paused" || j.state === "interrupted").length;
  const deciding = jobs.filter((j) => j.state === "needsDecision").length;

  if (moving.length > 0) {
    const uploads = moving.filter((j) => j.direction === "upload").length;
    const downloads = moving.length - uploads;
    const verb = downloads === 0 ? "Uploading" : uploads === 0 ? "Downloading" : "Transferring";
    let done = 0;
    let total = 0;
    let speed = 0;
    let speedKnown = false;
    let eta: number | null = null;
    for (const j of moving) {
      done += j.transferredBytes;
      total += j.totalBytes;
      if (j.speedBps !== null) {
        speed += j.speedBps;
        speedKnown = true;
      }
      if (j.etaSecs !== null) eta = Math.max(eta ?? 0, j.etaSecs);
    }
    const pct = formatPercent(done, total);
    const parts = [
      `${verb} ${plural(moving.length, "file")}`,
      pct,
      speedKnown && speed > 0 ? formatSpeed(speed) : null,
      eta !== null ? formatEta(eta) : null,
      failed > 0 ? `${failed} failed` : null,
    ];
    return {
      text: joinDot(parts),
      ariaText: joinDot(parts),
      done,
      total,
      failed,
      moving: moving.length,
      allDone: false,
    };
  }

  const waiting = paused + deciding;
  if (failed > 0 || waiting > 0) {
    const parts = [
      failed > 0 ? `${failed} failed` : null,
      deciding > 0 ? `${deciding} need a decision` : null,
      paused > 0 ? `${paused} paused` : null,
      completed > 0 ? `${completed} completed` : null,
    ];
    return { text: joinDot(parts), ariaText: joinDot(parts), done: 0, total: 0, failed, moving: 0, allDone: false };
  }

  return {
    text: "All transfers complete",
    ariaText: "All transfers complete",
    done: 1,
    total: 1,
    failed: 0,
    moving: 0,
    allDone: true,
  };
}
