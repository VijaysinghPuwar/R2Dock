// Cancel and pause requests waiting for the transfer engine to answer.
//
// Stopping can take a moment (an encode is stopped, parts in flight finish
// for up to a few seconds, a multipart upload is aborted), so a row shows
// "Cancelling..." or "Pausing..." from the click until the job's new state
// arrives, and its conflicting buttons are disabled meanwhile.

import { create } from "zustand";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { JobState } from "../../ipc/bindings/JobState";
import { isMoving } from "./stats";

export type PendingStop = "cancel" | "pause";

interface PendingState {
  pending: Record<string, PendingStop>;
  mark: (ids: string[], stop: PendingStop) => void;
  clear: (ids: Iterable<string>) => void;
}

export const usePendingStore = create<PendingState>((set) => ({
  pending: {},
  mark: (ids, stop) =>
    set((s) => {
      const pending = { ...s.pending };
      // A cancel is never downgraded to a pause.
      for (const id of ids) if (pending[id] !== "cancel") pending[id] = stop;
      return { pending };
    }),
  clear: (ids) =>
    set((s) => {
      let pending: Record<string, PendingStop> | null = null;
      for (const id of ids) {
        if (!(id in (pending ?? s.pending))) continue;
        pending ??= { ...s.pending };
        delete pending[id];
      }
      return pending ? { pending } : s;
    }),
}));

/** Whether a job in this state is still working, so a stop request is still on its way. */
export function awaitsStop(state: JobState): boolean {
  return isMoving(state);
}

/** Forget requests that the jobs' new states answer. */
export function settlePending(jobs: JobSnapshot[]): void {
  const { pending, clear } = usePendingStore.getState();
  const done: string[] = [];
  for (const job of jobs) {
    if (job.id in pending && !awaitsStop(job.state)) done.push(job.id);
  }
  if (done.length > 0) clear(done);
}

/** Jobs that "Pause all" pauses, as the engine decides it. */
export function pausableNow(job: JobSnapshot): boolean {
  if (job.state === "compressing") return true;
  return job.pausable && (job.state === "queued" || job.state === "preparing" || job.state === "running" || job.state === "retrying" || job.state === "verifying");
}
