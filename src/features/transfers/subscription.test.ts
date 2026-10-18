import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryObserver, type QueryClient } from "@tanstack/react-query";
import type { TransferEvent } from "../../ipc/bindings/TransferEvent";
import { makeConnection } from "../../test/connections";
import { testQueryClient } from "../../test/utils";
import { makeJob } from "./testJobs";

let deliver: ((event: TransferEvent) => void) | null = null;
const connectionSetActive = vi.fn((_id: string) => Promise.resolve());

vi.mock("../../ipc/commands", async (orig) => ({
  ...(await orig<typeof import("../../ipc/commands")>()),
  transferSubscribe: (cb: (event: TransferEvent) => void) => {
    deliver = cb;
    return Promise.resolve();
  },
  transferControlMany: vi.fn(() => Promise.resolve()),
  connectionSetActive: (id: string) => connectionSetActive(id),
}));

function conn(id: string, active: boolean) {
  return makeConnection({ id, name: id === "conn-a" ? "Studio" : "Archive", active });
}

/** Fresh module state for every test: the subscription starts once per app. */
async function setup() {
  vi.resetModules();
  deliver = null;
  const sub = await import("./subscription");
  const { qk } = await import("../../app/queries");
  const { useToastStore } = await import("../../components/toastStore");
  const { useNavStore } = await import("../../app/navStore");
  const client = testQueryClient();
  sub.startTransferSubscription(client);
  await Promise.resolve();
  return { client, qk, useToastStore, useNavStore };
}

function send(event: TransferEvent) {
  if (!deliver) throw new Error("not subscribed");
  deliver(event);
  vi.advanceTimersByTime(60);
}

function invalidated(client: QueryClient, key: readonly unknown[]): boolean {
  return client.getQueryCache().find({ queryKey: key, exact: true })?.state.isInvalidated ?? false;
}

function seed(client: QueryClient, key: readonly unknown[]) {
  client.setQueryData(key, { seeded: true });
}

describe("transfer subscription", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    connectionSetActive.mockClear();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("refreshes every view of an uploaded object once the burst settles", async () => {
    const { client, qk } = await setup();
    const key = "covers/2026/cover.webp";
    const keys = {
      listing: qk.objects("conn-a", "media-library", "covers/2026/"),
      search: qk.search("conn-a", "media-library", "cover"),
      head: qk.head("conn-a", "media-library", key),
      text: qk.text("conn-a", "media-library", key),
      media: qk.media("conn-a", "media-library", key),
      history: qk.history,
      otherHead: qk.head("conn-a", "media-library", "covers/other.webp"),
      otherAccount: qk.head("conn-b", "media-library", key),
      otherAccountSearch: qk.search("conn-b", "media-library", "cover"),
    };
    for (const k of Object.values(keys)) seed(client, k);
    const sizeKeys = {
      parent: qk.prefixSize("conn-a", "media-library", "covers/2026/"),
      grand: qk.prefixSize("conn-a", "media-library", "covers/"),
      root: qk.prefixSize("conn-a", "media-library", ""),
      sibling: qk.prefixSize("conn-a", "media-library", "audio/"),
    };
    for (const k of Object.values(sizeKeys)) seed(client, k);

    const job = makeJob({ id: "u1", connectionId: "conn-a", bucket: "media-library", key, state: "running" });
    send({ type: "added", jobs: [job] });
    send({ type: "updated", job: { ...job, state: "completed" } });

    // Coalesced: nothing happens until the burst is quiet.
    expect(invalidated(client, keys.listing)).toBe(false);
    vi.advanceTimersByTime(1000);

    expect(invalidated(client, keys.listing)).toBe(true);
    expect(invalidated(client, keys.search)).toBe(true);
    expect(invalidated(client, keys.head)).toBe(true);
    expect(invalidated(client, keys.text)).toBe(true);
    expect(invalidated(client, keys.media)).toBe(true);
    expect(invalidated(client, keys.history)).toBe(true);
    expect(invalidated(client, keys.otherHead)).toBe(false);
    expect(invalidated(client, keys.otherAccount)).toBe(false);
    expect(invalidated(client, keys.otherAccountSearch)).toBe(false);

    // A calculated size of a folder that now holds more is dropped, not shown stale.
    expect(client.getQueryData(sizeKeys.parent)).toBeUndefined();
    expect(client.getQueryData(sizeKeys.grand)).toBeUndefined();
    expect(client.getQueryData(sizeKeys.root)).toBeUndefined();
    expect(client.getQueryData(sizeKeys.sibling)).toEqual({ seeded: true });
  });

  it("refetches a mounted text preview of an overwritten object", async () => {
    const { client, qk } = await setup();
    const key = "notes/readme.txt";
    let fetches = 0;
    const observer = new QueryObserver(client, {
      queryKey: qk.text("conn-a", "media-library", key),
      queryFn: () => Promise.resolve(`version ${++fetches}`),
    });
    const stop = observer.subscribe(() => undefined);
    await vi.advanceTimersByTimeAsync(0);
    expect(fetches).toBe(1);
    const job = makeJob({ id: "u1", connectionId: "conn-a", bucket: "media-library", key, state: "running" });
    send({ type: "added", jobs: [job] });
    send({ type: "updated", job: { ...job, state: "completed" } });
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetches).toBe(2);
    expect(client.getQueryData(qk.text("conn-a", "media-library", key))).toBe("version 2");
    stop();
  });

  it("refreshes history when a download completes", async () => {
    const { client, qk } = await setup();
    seed(client, qk.history);
    seed(client, qk.objectsBucket("conn-a", "media-library"));
    const job = makeJob({ id: "d1", direction: "download", connectionId: "conn-a", bucket: "media-library", state: "running" });
    send({ type: "added", jobs: [job] });
    send({ type: "updated", job: { ...job, state: "completed" } });
    vi.advanceTimersByTime(1000);
    expect(invalidated(client, qk.history)).toBe(true);
    // A download does not change the bucket.
    expect(invalidated(client, qk.objectsBucket("conn-a", "media-library"))).toBe(false);
  });

  it("coalesces a burst of completions into one refresh per bucket", async () => {
    const { client, qk } = await setup();
    const spy = vi.spyOn(client, "invalidateQueries");
    const jobs = Array.from({ length: 200 }, (_, i) =>
      makeJob({ id: `u${i}`, connectionId: "conn-a", bucket: "media-library", key: `batch/f${i}.txt`, state: "running" }),
    );
    send({ type: "added", jobs });
    for (const j of jobs) {
      deliver?.({ type: "updated", job: { ...j, state: "completed" } });
    }
    vi.advanceTimersByTime(60);
    vi.advanceTimersByTime(1000);
    const listingCalls = spy.mock.calls.filter(
      ([filters]) => JSON.stringify(filters?.queryKey) === JSON.stringify(qk.objectsBucket("conn-a", "media-library")),
    );
    expect(listingCalls).toHaveLength(1);
    // Listing, search, per-object views, and history: a small fixed number of calls.
    expect(spy.mock.calls.length).toBeLessThanOrEqual(5);
  });

  it("Show on the upload toast switches to the transfer's account", async () => {
    const { client, qk, useToastStore, useNavStore } = await setup();
    client.setQueryData(qk.connections, [conn("conn-a", false), conn("conn-b", true)]);
    const job = makeJob({ id: "u1", connectionId: "conn-a", bucket: "media-library", key: "covers/a.webp", state: "running" });
    send({ type: "added", jobs: [job] });
    send({ type: "updated", job: { ...job, state: "completed" } });
    const t = useToastStore.getState().toasts.find((x) => x.action?.label === "Show");
    expect(t).toBeDefined();
    vi.useRealTimers();
    t?.action?.run();
    await vi.waitFor(() => expect(useNavStore.getState().loc).toEqual({ view: "browser", bucket: "media-library", prefix: "covers/" }));
    expect(connectionSetActive).toHaveBeenCalledWith("conn-a");
  });
});
