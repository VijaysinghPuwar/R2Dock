import type { QueryClient } from "@tanstack/react-query";
import { connectionList, connectionSetActive, errorMessage } from "../ipc/commands";
import type { ConnectionSummary } from "../ipc/bindings/ConnectionSummary";
import { toast } from "../components/toastStore";
import { useBrowserStore } from "../features/browser/browserStore";
import { usePreviewStore } from "../features/preview/previewStore";
import { useNavStore } from "./navStore";
import { qk } from "./queries";

export interface BucketTarget {
  connectionId: string;
  bucket: string;
  prefix: string;
}

/** Numbers each request, so a slow account switch cannot navigate after a newer request. */
let latest = 0;

async function currentConnections(client: QueryClient): Promise<ConnectionSummary[]> {
  try {
    // Fresh from the core: the cached list may still show a removed account.
    return await client.fetchQuery({ queryKey: qk.connections, queryFn: connectionList, staleTime: 0 });
  } catch {
    return client.getQueryData<ConnectionSummary[]>(qk.connections) ?? [];
  }
}

/**
 * Open a folder of a bucket in the account it belongs to. Switches the active
 * account first when needed, because two accounts can have buckets with the
 * same name. Never shows another account's folder.
 */
export async function showInBucket(client: QueryClient, target: BucketTarget): Promise<void> {
  const request = ++latest;
  const connections = await currentConnections(client);
  if (request !== latest) return;
  const owner = connections.find((c) => c.id === target.connectionId);
  if (!owner) {
    toast.danger(`The connection for ${target.bucket} was removed, so this folder cannot be shown.`);
    return;
  }
  const active = connections.find((c) => c.active) ?? connections[0];
  if (active?.id === owner.id) {
    useNavStore.getState().openBucket(target.bucket, target.prefix);
    return;
  }

  try {
    await connectionSetActive(owner.id);
  } catch (e) {
    if (request === latest) toast.danger(`Could not switch to ${owner.name}. ${errorMessage(e)}`);
    return;
  }
  // Refresh the list before navigating, so the folder opens with the new account.
  await currentConnections(client);
  if (request !== latest) return;
  // Nothing selected or previewed in the previous account carries over.
  useBrowserStore.getState().resetForLocation();
  usePreviewStore.getState().close();
  useNavStore.getState().reset("buckets");
  useNavStore.getState().openBucket(target.bucket, target.prefix);
}
