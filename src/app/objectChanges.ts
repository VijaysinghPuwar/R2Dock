import type { Query, QueryClient } from "@tanstack/react-query";
import { parentPrefix } from "../lib/keys";
import { qk } from "./queries";

/** An object that was written in a bucket. */
export interface ObjectChange {
  connectionId: string;
  bucket: string;
  key: string;
}

interface BucketChanges {
  connectionId: string;
  bucket: string;
  keys: Set<string>;
  /** Every folder that contains a changed key, including the bucket root (""). */
  prefixes: Set<string>;
}

/** Query kinds keyed by a single object: [kind, connectionId, bucket, key]. */
const OBJECT_KINDS = new Set(["head", "text", "media"]);

function group(changes: Iterable<ObjectChange>): Map<string, BucketChanges> {
  const groups = new Map<string, BucketChanges>();
  for (const c of changes) {
    const id = `${c.connectionId}\u0000${c.bucket}`;
    let g = groups.get(id);
    if (!g) {
      g = { connectionId: c.connectionId, bucket: c.bucket, keys: new Set(), prefixes: new Set() };
      groups.set(id, g);
    }
    g.keys.add(c.key);
    let p = parentPrefix(c.key);
    while (!g.prefixes.has(p)) {
      g.prefixes.add(p);
      if (p === "") break;
      p = parentPrefix(p);
    }
  }
  return groups;
}

function inBucket(query: Query, g: BucketChanges): boolean {
  const k = query.queryKey;
  return k[1] === g.connectionId && k[2] === g.bucket;
}

/**
 * Refresh every cached view of the changed objects: folder listings, bucket
 * search, details, text and media previews. Calculated folder sizes that
 * include a changed key are cleared, since recalculating costs R2 operations.
 * Only mounted queries refetch; the rest are marked stale. A few calls per
 * bucket, however many keys changed.
 */
export function invalidateObjectChanges(client: QueryClient, changes: Iterable<ObjectChange>): void {
  for (const g of group(changes).values()) {
    void client.invalidateQueries({ queryKey: qk.objectsBucket(g.connectionId, g.bucket) });
    void client.invalidateQueries({ queryKey: ["search", g.connectionId, g.bucket] });
    void client.invalidateQueries({
      predicate: (q) => OBJECT_KINDS.has(String(q.queryKey[0])) && inBucket(q, g) && g.keys.has(String(q.queryKey[3])),
    });
    void client.resetQueries({
      predicate: (q) => q.queryKey[0] === "prefixSize" && inBucket(q, g) && g.prefixes.has(String(q.queryKey[3])),
    });
  }
}
