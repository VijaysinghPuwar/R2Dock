// Tells the user when an upload with compression on goes up as the original.

import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { Unchanged } from "../../ipc/bindings/Unchanged";
import { unchangedText } from "./compressionText";

/** How long notices are collected, so a batch of files shows one toast. */
export const NOTICE_BATCH_MS = 1000;

interface Notice {
  id: string;
  name: string;
  /** Why it was not compressed, or null when compression failed. */
  reason: string | null;
  /** Compression failed and the "Skip the file" setting stopped the upload. */
  stopped: boolean;
}

/** Results where compression did not apply at all, so there is nothing to tell. */
function notApplicable(reason: Unchanged): boolean {
  if (reason.kind === "notAudio") return true;
  if (reason.kind !== "plan") return false;
  // Video compression is off by the user's choice, or video goes through its own path.
  return reason.reason.reason === "videoOff" || reason.reason.reason === "video";
}

function noticeFor(job: JobSnapshot): Notice | null {
  if (job.direction !== "upload") return null;
  const view = job.compression;
  if (!view) return null;
  if (view.kind === "failed") {
    return { id: job.id, name: job.name, reason: null, stopped: job.state === "failed" };
  }
  if (view.kind === "unchanged" && !notApplicable(view.reason)) {
    return { id: job.id, name: job.name, reason: unchangedText(view.reason), stopped: false };
  }
  return null;
}

function message(notices: Notice[]): string {
  const [only] = notices;
  if (notices.length === 1 && only) {
    if (only.stopped) return `Couldn't compress ${only.name}, so it was skipped as set in Settings > Compression.`;
    return only.reason === null
      ? `Couldn't compress ${only.name}. Uploading the original.`
      : `${only.name} can't be compressed: ${only.reason}. Uploading the original.`;
  }
  const stopped = notices.filter((n) => n.stopped).length;
  const uploading = notices.length - stopped;
  if (stopped === 0) return `${notices.length} files can't be compressed. Uploading the originals.`;
  if (uploading === 0) return `${stopped} files couldn't be compressed, so they were skipped as set in Settings > Compression.`;
  return `${notices.length} files can't be compressed. Uploading ${uploading} originals; ${stopped} skipped as set in Settings > Compression.`;
}

export interface CompressionNotices {
  /** Look at a new or changed job. Each job is reported at most once. */
  note: (job: JobSnapshot) => void;
  /** Record a job restored from an earlier session without telling again. */
  remember: (job: JobSnapshot) => void;
  /** Forget jobs that left the queue. */
  forget: (ids: Iterable<string>) => void;
}

export function createCompressionNotices(show: (message: string) => void): CompressionNotices {
  const reported = new Set<string>();
  let pending: Notice[] = [];
  let timer: number | null = null;

  const flush = () => {
    timer = null;
    const notices = pending;
    pending = [];
    if (notices.length > 0) show(message(notices));
  };

  return {
    note: (job) => {
      if (reported.has(job.id)) {
        // The job may stop (Skip the file) after compression failed but before
        // the notice is shown; keep the waiting notice accurate.
        const waiting = pending.find((n) => n.id === job.id);
        if (waiting && waiting.reason === null) waiting.stopped = job.state === "failed";
        return;
      }
      const notice = noticeFor(job);
      if (!notice) return;
      reported.add(job.id);
      pending.push(notice);
      timer ??= window.setTimeout(flush, NOTICE_BATCH_MS);
    },
    remember: (job) => {
      if (noticeFor(job)) reported.add(job.id);
    },
    forget: (ids) => {
      for (const id of ids) reported.delete(id);
    },
  };
}
