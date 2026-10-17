import { errorMessage, transferControl, transferControlMany } from "../../ipc/commands";
import type { JobAction } from "../../ipc/bindings/JobAction";
import { toast } from "../../components/toastStore";

/** Send a control action for one job; failures become a toast. */
export function control(id: string, action: JobAction): void {
  transferControl(id, action).catch((e: unknown) => toast.danger(errorMessage(e)));
}

/** Send a control action for several jobs; failures become a toast. */
export function controlMany(ids: string[], action: JobAction): void {
  if (ids.length === 0) return;
  transferControlMany(ids, action).catch((e: unknown) => toast.danger(errorMessage(e)));
}

/** Run a command and toast any error. */
export function run(task: () => Promise<unknown>): void {
  task().catch((e: unknown) => toast.danger(errorMessage(e)));
}
