import type { QueryClient } from "@tanstack/react-query";
import { errorMessage, transferControlMany, transferSubscribe } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { Direction } from "../../ipc/bindings/Direction";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { TransferEvent } from "../../ipc/bindings/TransferEvent";
import { qk } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
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

/** Tracks jobs started since the queue was last idle, to toast once per batch. */
const batch = new Map<string, Direction>();
const pendingInvalidations = new Map<string, number>();

function scheduleListingRefresh(client: QueryClient, connectionId: string, bucket: string) {
  const id = `${connectionId}\u0000${bucket}`;
  const existing = pendingInvalidations.get(id);
  if (existing !== undefined) window.clearTimeout(existing);
  const t = window.setTimeout(() => {
    pendingInvalidations.delete(id);
    void client.invalidateQueries({ queryKey: qk.objectsBucket(connectionId, bucket) });
  }, 800);
  pendingInvalidations.set(id, t);
}

function summarize(direction: Direction, jobs: JobSnapshot[]) {
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
        run: () => useNavStore.getState().openBucket(first.bucket, prefix),
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

  for (const job of touched) {
    const prev = before[job.id];
    if (isMoving(job.state) && !batch.has(job.id)) batch.set(job.id, job.direction);
    if (job.direction === "upload" && job.state === "completed" && prev?.state !== "completed") {
      scheduleListingRefresh(client, job.connectionId, job.bucket);
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

  if (batch.size === 0) return;
  const batchJobs: JobSnapshot[] = [];
  for (const id of batch.keys()) {
    const job = state.jobs[id];
    if (job && isMoving(job.state)) return; // still working
    if (job) batchJobs.push(job);
  }
  batch.clear();
  for (const direction of ["upload", "download"] as const) {
    const jobs = batchJobs.filter((j) => j.direction === direction && isTerminal(j.state));
    if (jobs.length > 0) summarize(direction, jobs);
  }
}

/** Subscribe to transfer events once for the app's lifetime. */
export function startTransferSubscription(client: QueryClient): void {
  if (started) return;
  started = true;
  transferSubscribe((event) => {
    const before = useTransferStore.getState().jobs;
    useTransferStore.getState().apply(event);
    afterEvent(client, before, event);
  }).catch((e: unknown) => {
    started = false;
    toast.danger(`Transfer updates are not available. ${errorMessage(e)}`);
  });
}
