import { create } from "zustand";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import type { TransferEvent } from "../../ipc/bindings/TransferEvent";

export interface TransferState {
  /** Jobs keyed by id. */
  jobs: Record<string, JobSnapshot>;
  /** Job ids in insertion order. */
  order: string[];
  /** True once any job has been seen this session. */
  seen: boolean;
  /** Last connection whose credentials failed, with a counter so repeats are noticed. */
  authFailed: { connectionId: string; count: number } | null;
}

export const INITIAL_TRANSFER_STATE: TransferState = { jobs: {}, order: [], seen: false, authFailed: null };

function upsert(state: TransferState, list: JobSnapshot[], replace: boolean): TransferState {
  const jobs: Record<string, JobSnapshot> = replace ? {} : { ...state.jobs };
  const order = replace ? [] : [...state.order];
  for (const job of list) {
    if (!(job.id in jobs)) order.push(job.id);
    jobs[job.id] = job;
  }
  return { ...state, jobs, order, seen: state.seen || order.length > 0 };
}

/** Pure reducer for transfer events. */
export function reduceTransfers(state: TransferState, event: TransferEvent): TransferState {
  switch (event.type) {
    case "snapshot":
      return upsert(state, event.jobs, true);
    case "added":
      return upsert(state, event.jobs, false);
    case "updated":
      return upsert(state, [event.job], false);
    case "progress": {
      let changed = false;
      const jobs = { ...state.jobs };
      for (const u of event.updates) {
        const job = jobs[u.id];
        if (!job) continue;
        changed = true;
        jobs[u.id] = {
          ...job,
          transferredBytes: u.transferredBytes,
          speedBps: u.speedBps,
          etaSecs: u.etaSecs,
          partsDone: u.partsDone,
        };
      }
      return changed ? { ...state, jobs } : state;
    }
    case "removed": {
      const drop = new Set(event.ids);
      const jobs = { ...state.jobs };
      for (const id of event.ids) delete jobs[id];
      return { ...state, jobs, order: state.order.filter((id) => !drop.has(id)) };
    }
    case "authFailed":
      return {
        ...state,
        authFailed: { connectionId: event.connectionId, count: (state.authFailed?.count ?? 0) + 1 },
      };
  }
}

interface TransferStore extends TransferState {
  apply: (event: TransferEvent) => void;
  reset: () => void;
}

export const useTransferStore = create<TransferStore>((set) => ({
  ...INITIAL_TRANSFER_STATE,
  apply: (event) => set((s) => reduceTransfers(s, event)),
  reset: () => set(INITIAL_TRANSFER_STATE),
}));

/** Jobs in insertion order. */
export function orderedJobs(state: Pick<TransferState, "jobs" | "order">): JobSnapshot[] {
  const out: JobSnapshot[] = [];
  for (const id of state.order) {
    const job = state.jobs[id];
    if (job) out.push(job);
  }
  return out;
}
