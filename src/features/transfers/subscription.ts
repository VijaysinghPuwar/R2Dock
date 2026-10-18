import type { QueryClient } from "@tanstack/react-query";
import { errorMessage, transferControlMany, transferSubscribe } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { Direction } from "../../ipc/bindings/Direction";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { TransferEvent } from "../../ipc/bindings/TransferEvent";
import { invalidateObjectChanges, type ObjectChange } from "../../app/objectChanges";
import { qk } from "../../app/queries";
import { showInBucket } from "../../app/showInBucket";
import { useUiStore } from "../../app/uiStore";
import { toast } from "../../components/toastStore";
import { folderLabel, parentPrefix } from "../../lib/keys";
import { plural } from "../../lib/format";
import { isMoving, isTerminal } from "./stats";
import { useTransferStore } from "./transferStore";

let started = false;

/** Common parent prefix of several keys. */
function commonPrefix(keys: string[]): string {
  if (keys.length === 0) return "";
  let prefix = parentPrefix(keys[0] ?? "");
  for (const k of keys) {
    while (prefix && !k.startsWith(prefix)) prefix = parentPrefix(prefix);
  }
  return prefix;
}

/** How long transfer events are collected before they are applied together. */
const FLUSH_MS = 50;

/** Tracks jobs started since the queue was last idle, to toast once per batch. */
const batch = new Map<string, Direction>();
/** Ids in `batch` that are still working, so the idle check is O(1) per event. */
const moving = new Set<string>();

/** How long completions are collected before cached views are refreshed. */
const REFRESH_MS = 800;
/** Uploaded objects waiting for their views to refresh. */
let changedObjects: ObjectChange[] = [];
/** Whether a finished transfer is waiting to show up in history. */
let historyChanged = false;
let refreshTimer: number | null = null;

/** Refresh affected views once a burst of completions has settled. */
function scheduleRefresh(client: QueryClient) {
  if (refreshTimer !== null) window.clearTimeout(refreshTimer);
  refreshTimer = window.setTimeout(() => {
    refreshTimer = null;
    const changes = changedObjects;
    const history = historyChanged;
    changedObjects = [];
    historyChanged = false;
    invalidateObjectChanges(client, changes);
    if (history) void client.invalidateQueries({ queryKey: qk.history });
  }, REFRESH_MS);
}

function summarize(client: QueryClient, direction: Direction, jobs: JobSnapshot[]) {
  const completed = jobs.filter((j) => j.state === "completed");
  const failed = jobs.filter((j) => j.state === "failed");
  const skipped = jobs.filter((j) => j.state === "skipped").length;
  if (completed.length === 0 && failed.length === 0) return;
  const verb = direction === "upload" ? "Uploaded" : "Downloaded";
  if (failed.length > 0) {
    const total = completed.length + failed.length;
    toast.danger(`${verb} ${completed.length} of ${plural(total, "file")}. ${failed.length} failed.`, {
      label: "Retry failed",
      run: () => {
        transferControlMany(
          failed.map((j) => j.id),
          { action: "retry" },
        ).catch((e: unknown) => toast.danger(errorMessage(e)));
      },
    });
    return;
  }
  const skippedText = skipped > 0 ? ` ${skipped} skipped.` : "";
  if (direction === "upload") {
    const first = completed[0];
    const sameBucket = first && completed.every((j) => j.bucket === first.bucket && j.connectionId === first.connectionId);
    if (first && sameBucket) {
      const prefix = commonPrefix(completed.map((j) => j.key));
      toast.success(`${verb} ${plural(completed.length, "file")} to ${folderLabel(first.bucket, prefix)}.${skippedText}`, {
        label: "Show",
        run: () => void showInBucket(client, { connectionId: first.connectionId, bucket: first.bucket, prefix }),
      });
      return;
    }
  }
  toast.success(`${verb} ${plural(completed.length, "file")}.${skippedText}`);
}

function afterEvent(client: QueryClient, before: Record<string, JobSnapshot>, event: TransferEvent) {
  const state = useTransferStore.getState();
  const touched: JobSnapshot[] =
    event.type === "added" || event.type === "snapshot" ? event.jobs : event.type === "updated" ? [event.job] : [];

  // A snapshot replaces every job, so forget ids it no longer lists.
  if (event.type === "snapshot") moving.clear();
  for (const job of touched) {
    const prev = before[job.id];
    if (isMoving(job.state)) {
      if (!batch.has(job.id)) batch.set(job.id, job.direction);
      moving.add(job.id);
    } else {
      moving.delete(job.id);
    }
    if (isTerminal(job.state) && !(prev && isTerminal(prev.state))) {
      // Every finished transfer is written to history.
      historyChanged = true;
      if (job.direction === "upload" && job.state === "completed") {
        changedObjects.push({ connectionId: job.connectionId, bucket: job.bucket, key: job.key });
      }
      scheduleRefresh(client);
    }
  }

  if (event.type === "authFailed") {
    void client.invalidateQueries({ queryKey: qk.connections });
    const conns = client.getQueryData<ConnectionSummary[]>(qk.connections);
    const name = conns?.find((c) => c.id === event.connectionId)?.name ?? "this connection";
    toast.danger(`The keys for ${name} no longer work. Transfers for it are paused.`, {
      label: "Replace secret",
      run: () => useUiStore.getState().openConnect({ mode: "edit", id: event.connectionId, replaceSecret: true, showHelp: false }),
    });
  }

  if (event.type === "removed") for (const id of event.ids) moving.delete(id);
  if (batch.size === 0 || moving.size > 0) return; // still working
  const batchJobs: JobSnapshot[] = [];
  for (const id of batch.keys()) {
    const job = state.jobs[id];
    if (job && isMoving(job.state)) {
      moving.add(job.id);
      return;
    }
    if (job) batchJobs.push(job);
  }
  batch.clear();
  for (const direction of ["upload", "download"] as const) {
    const jobs = batchJobs.filter((j) => j.direction === direction && isTerminal(j.state));
    if (jobs.length > 0) summarize(client, direction, jobs);
  }
}

/** Subscribe to transfer events once for the app's lifetime. */
export function startTransferSubscription(client: QueryClient): void {
  if (started) return;
  started = true;
  // Events arrive in bursts (thousands for a big queue). Apply each burst as
  // one store update so the window stays responsive.
  let pending: TransferEvent[] = [];
  const flush = () => {
    const events = pending;
    pending = [];
    const before = useTransferStore.getState().jobs;
    useTransferStore.getState().applyMany(events);
    for (const event of events) afterEvent(client, before, event);
  };
  transferSubscribe((event) => {
    if (pending.length === 0) setTimeout(flush, FLUSH_MS);
    pending.push(event);
  }).catch((e: unknown) => {
    started = false;
    toast.danger(`Transfer updates are not available. ${errorMessage(e)}`);
  });
}
