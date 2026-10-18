import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ConnectionSummary } from "../ipc/bindings/ConnectionSummary";
import { makeConnection } from "../test/connections";
import { testQueryClient } from "../test/utils";
import { useBrowserStore } from "../features/browser/browserStore";
import { usePreviewStore } from "../features/preview/previewStore";
import { useToastStore } from "../components/toastStore";
import { useNavStore } from "./navStore";
import { qk } from "./queries";

let saved: ConnectionSummary[] = [];
const connectionSetActive = vi.fn((id: string) => {
  saved = saved.map((c) => ({ ...c, active: c.id === id }));
  return Promise.resolve();
});

vi.mock("../ipc/commands", async (orig) => ({
  ...(await orig<typeof import("../ipc/commands")>()),
  connectionList: () => Promise.resolve(saved),
  connectionSetActive: (id: string) => connectionSetActive(id),
}));

const { showInBucket } = await import("./showInBucket");

const A = makeConnection({ id: "conn-a", name: "Studio" });
const B = makeConnection({ id: "conn-b", name: "Archive", active: true });

function setup(connections: ConnectionSummary[] = [A, B]) {
  saved = connections;
  const client = testQueryClient();
  client.setQueryData(qk.connections, connections);
  useNavStore.getState().reset("buckets");
  useNavStore.getState().openBucket("media-library", "b-only/");
  useBrowserStore.getState().setSelected(["b-only/secret.txt"], "b-only/secret.txt");
  usePreviewStore.getState().close();
  useToastStore.getState().clear();
  return client;
}

const target = { connectionId: "conn-a", bucket: "media-library", prefix: "covers/" };

describe("showInBucket", () => {
  beforeEach(() => connectionSetActive.mockClear());

  it("switches to the transfer's account before opening a bucket with the same name", async () => {
    const client = setup();
    await showInBucket(client, target);
    expect(connectionSetActive).toHaveBeenCalledWith("conn-a");
    const conns = client.getQueryData<ConnectionSummary[]>(qk.connections);
    expect(conns?.find((c) => c.active)?.id).toBe("conn-a");
    expect(useNavStore.getState().loc).toEqual({ view: "browser", bucket: "media-library", prefix: "covers/" });
    expect(useBrowserStore.getState().selected.size).toBe(0);
    // Back must not lead into the other account's folder.
    expect(useNavStore.getState().back.some((l) => l.prefix === "b-only/")).toBe(false);
  });

  it("navigates without switching when the account is already active", async () => {
    const client = setup([{ ...A, active: true }, { ...B, active: false }]);
    await showInBucket(client, target);
    expect(connectionSetActive).not.toHaveBeenCalled();
    expect(useNavStore.getState().loc).toEqual({ view: "browser", bucket: "media-library", prefix: "covers/" });
  });

  it("explains when the account was removed and stays put", async () => {
    const client = setup([B]);
    await showInBucket(client, target);
    expect(connectionSetActive).not.toHaveBeenCalled();
    expect(useNavStore.getState().loc.prefix).toBe("b-only/");
    const t = useToastStore.getState().toasts.at(-1);
    expect(t?.tone).toBe("danger");
    expect(t?.message).toMatch(/connection .* was removed/i);
  });

  it("reports a failed switch and does not navigate", async () => {
    const client = setup();
    connectionSetActive.mockImplementationOnce(() => Promise.reject({ code: "credentialMissing", message: "The secret is missing." }));
    await showInBucket(client, target);
    expect(useNavStore.getState().loc.prefix).toBe("b-only/");
    const t = useToastStore.getState().toasts.at(-1);
    expect(t?.tone).toBe("danger");
    expect(t?.message).toContain("Studio");
  });

  it("only the newest request navigates when two overlap", async () => {
    const client = setup();
    let release: () => void = () => undefined;
    connectionSetActive.mockImplementationOnce((id: string) => {
      saved = saved.map((c) => ({ ...c, active: c.id === id }));
      return new Promise<void>((resolve) => {
        release = resolve;
      });
    });
    const first = showInBucket(client, target);
    await vi.waitFor(() => expect(connectionSetActive).toHaveBeenCalledTimes(1));
    // The second request opens another folder in the same account.
    const second = showInBucket(client, { connectionId: "conn-a", bucket: "media-library", prefix: "later/" });
    release();
    await Promise.all([first, second]);
    expect(useNavStore.getState().loc.prefix).toBe("later/");
  });

  it("keeps the latest folder's account active when an older switch finishes late", async () => {
    const client = setup();
    let release: () => void = () => undefined;
    connectionSetActive.mockImplementationOnce((id: string) => new Promise<void>((resolve) => {
      release = () => {
        saved = saved.map((c) => ({ ...c, active: c.id === id }));
        resolve();
      };
    }));
    const first = showInBucket(client, target);
    await vi.waitFor(() => expect(connectionSetActive).toHaveBeenCalledTimes(1));
    const second = showInBucket(client, { connectionId: "conn-b", bucket: "media-library", prefix: "latest-b/" });
    release();
    await Promise.all([first, second]);

    expect(useNavStore.getState().loc.prefix).toBe("latest-b/");
    expect(saved.find((c) => c.active)?.id).toBe("conn-b");
    expect(client.getQueryData<ConnectionSummary[]>(qk.connections)?.find((c) => c.active)?.id).toBe("conn-b");
    expect(connectionSetActive.mock.calls.map(([id]) => id)).toEqual(["conn-a", "conn-b"]);
  });
});
