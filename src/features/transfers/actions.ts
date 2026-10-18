import { errorMessage, transferControl, transferControlMany, transferPauseAll } from "../../ipc/commands";
import type { JobAction } from "../../ipc/bindings/JobAction";
import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";
import { toast } from "../../components/toastStore";
import { pausableNow, usePendingStore, type PendingStop } from "./pendingStore";

function stopOf(action: JobAction): PendingStop | null {
  if (action.action === "cancel") return "cancel";
  if (action.action === "pause") return "pause";
  return null;
}

/** Send a control action for one job; failures become a toast. */
export function control(id: string, action: JobAction): void {
  controlMany([id], action);
}

/**
 * Send a control action for several jobs; failures become a toast. Cancel
 * and pause show at once on the rows ("Cancelling...", "Pausing...") until
 * the engine sends the jobs' new states.
 */
export function controlMany(ids: string[], action: JobAction): void {
  if (ids.length === 0) return;
  const stop = stopOf(action);
  const pending = usePendingStore.getState();
  if (stop) pending.mark(ids, stop);
  else pending.clear(ids);
  const sent = ids.length === 1 && ids[0] !== undefined ? transferControl(ids[0], action) : transferControlMany(ids, action);
  sent.catch((e: unknown) => {
    if (stop) usePendingStore.getState().clear(ids);
    toast.danger(errorMessage(e));
  });
}

/** Pause every transfer that supports pausing. */
export function pauseAll(jobs: JobSnapshot[]): void {
  const ids = jobs.filter(pausableNow).map((j) => j.id);
  usePendingStore.getState().mark(ids, "pause");
  transferPauseAll().catch((e: unknown) => {
    usePendingStore.getState().clear(ids);
    toast.danger(errorMessage(e));
  });
}

/** Run a command and toast any error. */
export function run(task: () => Promise<unknown>): void {
  task().catch((e: unknown) => toast.danger(errorMessage(e)));
}
