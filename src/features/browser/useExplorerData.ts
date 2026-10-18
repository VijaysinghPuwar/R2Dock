import { useInfiniteQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useState } from "react";
import { objectList, objectSearch } from "../../ipc/commands";
import { qk } from "../../app/queries";
import { parentPrefix, baseName } from "../../lib/keys";
import { isTerminal } from "../transfers/stats";
import { useTransferStore } from "../transfers/transferStore";
import { useBrowserStore } from "./browserStore";
import { filterRows, rowsFromPages, sortRows, type GhostRow, type Row, type SelectableRow } from "./rows";

export function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const t = window.setTimeout(() => setDebounced(value), ms);
    return () => window.clearTimeout(t);
  }, [value, ms]);
  return debounced;
}

/** Listing, search, sorting, and ghost rows for the open folder. */
export function useExplorerData(connectionId: string, bucket: string, prefix: string) {
  const search = useBrowserStore((s) => s.search);
  const scope = useBrowserStore((s) => s.scope);
  const sort = useBrowserStore((s) => s.sort);
  const term = useDebounced(search, 150).trim();
  const bucketSearch = scope === "bucket" && term !== "";

  const listing = useInfiniteQuery({
    queryKey: qk.objects(connectionId, bucket, prefix),
    queryFn: ({ pageParam }) => objectList(connectionId, bucket, prefix, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextToken ?? undefined,
  });

  const searchQuery = useInfiniteQuery({
    queryKey: qk.search(connectionId, bucket, term),
    queryFn: ({ pageParam }) => objectSearch(connectionId, bucket, term, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.nextToken ?? undefined,
    enabled: bucketSearch,
  });

  const listed = useMemo(() => rowsFromPages(listing.data?.pages ?? [], prefix), [listing.data, prefix]);
  const searched = useMemo(() => rowsFromPages(searchQuery.data?.pages ?? [], ""), [searchQuery.data]);

  const rows: SelectableRow[] = useMemo(() => {
    if (bucketSearch) return sortRows(searched, sort);
    return sortRows(filterRows(listed, term), sort);
  }, [bucketSearch, searched, listed, term, sort]);

  // Uploads into this folder that are not listed yet: in progress, or
  // finished after the listing was loaded (until the refresh lands, so the
  // row does not vanish and come back).
  const jobs = useTransferStore((s) => s.jobs);
  const order = useTransferStore((s) => s.order);
  const listedAt = listing.dataUpdatedAt;
  const ghosts: GhostRow[] = useMemo(() => {
    if (bucketSearch || term) return [];
    const listedKeys = new Set(listed.map((r) => r.id));
    const out: GhostRow[] = [];
    for (const id of order) {
      const j = jobs[id];
      if (!j || j.direction !== "upload" || j.connectionId !== connectionId || j.bucket !== bucket) continue;
      if (parentPrefix(j.key) !== prefix || listedKeys.has(j.key)) continue;
      const done = j.state === "completed" && j.finishedAt !== null && Date.parse(j.finishedAt) >= listedAt;
      if (isTerminal(j.state) && !done) continue;
      out.push({ kind: "ghost", id: `ghost:${j.id}`, key: j.key, name: baseName(j.key), transferred: j.transferredBytes, total: j.totalBytes, done });
    }
    return out;
  }, [jobs, order, listed, listedAt, connectionId, bucket, prefix, bucketSearch, term]);

  const allRows: Row[] = useMemo(() => (ghosts.length ? [...rows, ...ghosts] : rows), [rows, ghosts]);
  const active = bucketSearch ? searchQuery : listing;

  return {
    listing,
    searchQuery,
    active,
    rows,
    allRows,
    term,
    bucketSearch,
    listedCount: listed.length,
    hasMore: !!active.hasNextPage,
    loadingMore: active.isFetchingNextPage,
  };
}
