import type { QueryClient } from "@tanstack/react-query";
import { moveExecute } from "../../ipc/commands";
import type { MoveResult } from "../../ipc/bindings/MoveResult";
import type { OpProgress } from "../../ipc/bindings/OpProgress";
import { qk } from "../../app/queries";
import { formatCount, plural } from "../../lib/format";
import { useBrowserStore } from "../browser/browserStore";
import type { OpResult } from "./OpResultDialog";

/** Run a previewed move and refresh listings. Throws on command errors. */
export async function runMove(
  client: QueryClient,
  connectionId: string,
  bucket: string,
  token: string,
  onProgress: (p: OpProgress) => void,
): Promise<MoveResult> {
  try {
    return await moveExecute(token, onProgress);
  } finally {
    void client.invalidateQueries({ queryKey: qk.objectsBucket(connectionId, bucket) });
    void client.invalidateQueries({ queryKey: ["search", connectionId, bucket] });
    void client.invalidateQueries({ queryKey: ["head", connectionId, bucket] });
    void client.invalidateQueries({ queryKey: ["prefixSize", connectionId, bucket] });
    useBrowserStore.getState().clearSelection();
  }
}

/** Summary dialog content when anything went wrong, or null when all moved. */
export function moveProblems(res: MoveResult): OpResult | null {
  if (res.failures.length === 0 && res.copiedNotDeleted.length === 0) return null;
  const parts = [`Moved ${plural(res.moved, "object")}.`];
  if (res.failures.length > 0) parts.push(`${formatCount(res.failures.length)} were not moved and are still in the original place.`);
  if (res.copiedNotDeleted.length > 0) parts.push(`${formatCount(res.copiedNotDeleted.length)} were copied, but the originals were kept, so they now exist in both places.`);
  return {
    title: "The move did not fully complete",
    summary: parts.join(" "),
    failures: res.failures,
    copiedNotDeleted: res.copiedNotDeleted,
  };
}

export function moveProgressLabel(p: OpProgress | null): { done: number; total: number; label: string } | null {
  if (!p) return null;
  return { done: p.done, total: p.total, label: `Moved ${formatCount(p.done)} of ${formatCount(p.total)}` };
}

/** Text for the folder move confirmation. */
export function moveConfirmText(objects: number, destination: string): string {
  return `Move ${plural(objects, "object")} to ${destination}? Each object is copied and then the original is deleted. If something fails, some objects may exist in both places.`;
}
