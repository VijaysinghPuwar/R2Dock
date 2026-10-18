import { describe, expect, it } from "vitest";
import { makeConnection } from "../test/connections";
import { testQueryClient } from "../test/utils";
import { useNavStore } from "./navStore";
import { qk } from "./queries";
import { refreshCurrentView } from "./refresh";

describe("refreshCurrentView", () => {
  it("reloads details and text previews of the open bucket", () => {
    const client = testQueryClient();
    client.setQueryData(qk.connections, [makeConnection({ id: "conn-a", active: true })]);
    const text = qk.text("conn-a", "media-library", "notes/readme.txt");
    const head = qk.head("conn-a", "media-library", "notes/readme.txt");
    const other = qk.text("conn-a", "other-bucket", "notes/readme.txt");
    for (const k of [text, head, other]) client.setQueryData(k, "cached");
    useNavStore.getState().reset("buckets");
    useNavStore.getState().openBucket("media-library", "notes/");
    refreshCurrentView(client);
    const stale = (k: readonly unknown[]) => client.getQueryCache().find({ queryKey: k, exact: true })?.state.isInvalidated;
    expect(stale(text)).toBe(true);
    expect(stale(head)).toBe(true);
    expect(stale(other)).toBe(false);
  });
});
