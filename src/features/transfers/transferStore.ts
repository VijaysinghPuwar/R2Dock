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

/** Pure reducer for transfer events. */
export function reduceTransfers(state: TransferState, event: TransferEvent): TransferState {
  return reduceTransferBatch(state, [event]);
}

/**
 * Apply several events with at most one copy of the job map and order. A big
 * queue sends thousands of events, and copying every job for each one froze
 * the window, so the subscription applies them in batches.
 */
export function reduceTransferBatch(state: TransferState, events: TransferEvent[]): TransferState {
  let { jobs, order, authFailed } = state;
  let ownJobs = false;
  let ownOrder = false;
  let changed = false;
  const writableJobs = () => {
    if (!ownJobs) {
      jobs = { ...jobs };
      ownJobs = true;
    }
    return jobs;
  };
  const writableOrder = () => {
    if (!ownOrder) {
      order = [...order];
      ownOrder = true;
    }
    return order;
  };
  const put = (list: JobSnapshot[]) => {
    for (const job of list) {
      if (!(job.id in jobs)) writableOrder().push(job.id);
      writableJobs()[job.id] = job;
    }
  };

  for (const event of events) {
    switch (event.type) {
      case "snapshot":
        jobs = {};
        order = [];
        ownJobs = true;
        ownOrder = true;
        put(event.jobs);
        changed = true;
        break;
      case "added":
        put(event.jobs);
        changed = true;
        break;
      case "updated":
        put([event.job]);
        changed = true;
        break;
      case "progress":
        for (const u of event.updates) {
          const job = jobs[u.id];
          if (!job) continue;
          changed = true;
          writableJobs()[u.id] = {
            ...job,
            transferredBytes: u.transferredBytes,
            speedBps: u.speedBps,
            etaSecs: u.etaSecs,
            partsDone: u.partsDone,
          };
        }
        break;
      case "removed": {
        const drop = new Set(event.ids);
        const writable = writableJobs();
        for (const id of event.ids) delete writable[id];
        order = order.filter((id) => !drop.has(id));
        ownOrder = true;
        changed = true;
        break;
      }
      case "authFailed":
        authFailed = { connectionId: event.connectionId, count: (authFailed?.count ?? 0) + 1 };
        changed = true;
        break;
    }
  }
  if (!changed) return state;
  return { ...state, jobs, order, authFailed, seen: state.seen || order.length > 0 };
}

interface TransferStore extends TransferState {
  apply: (event: TransferEvent) => void;
  applyMany: (events: TransferEvent[]) => void;
  reset: () => void;
}

export const useTransferStore = create<TransferStore>((set) => ({
  ...INITIAL_TRANSFER_STATE,
  apply: (event) => set((s) => reduceTransfers(s, event)),
  applyMany: (events) => set((s) => reduceTransferBatch(s, events)),
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
