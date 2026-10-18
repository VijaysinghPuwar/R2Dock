import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import type { ListPage } from "../../ipc/bindings/ListPage";
import { qk } from "../../app/queries";
import { testQueryClient } from "../../test/utils";
import { makeJob } from "../transfers/testJobs";
import { INITIAL_TRANSFER_STATE, useTransferStore } from "../transfers/transferStore";
import { useExplorerData } from "./useExplorerData";

const listed: string[] = [];
vi.mock("../../ipc/commands", () => ({
  objectList: vi.fn(
    async (): Promise<ListPage> => ({
      prefix: "covers/",
      folders: [],
      objects: listed.map((key) => ({
        key,
        name: key.split("/").pop() ?? key,
        displayName: key.split("/").pop() ?? key,
        size: 10,
        lastModified: "2026-10-17T10:00:00Z",
        etag: null,
        storageClass: "STANDARD",
        unusual: false,
      })),
      nextToken: null,
    }),
  ),
  objectSearch: vi.fn(),
}));

function setJobs(...jobs: ReturnType<typeof makeJob>[]) {
  useTransferStore.setState({
    ...INITIAL_TRANSFER_STATE,
    jobs: Object.fromEntries(jobs.map((j) => [j.id, j])),
    order: jobs.map((j) => j.id),
  });
}

describe("useExplorerData ghost rows", () => {
  beforeEach(() => {
    listed.length = 0;
    useTransferStore.setState(INITIAL_TRANSFER_STATE);
  });

  it("keeps a finished upload visible until the listing refresh shows it", async () => {
    const client = testQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const { result } = renderHook(() => useExplorerData("conn-1", "media", "covers/"), { wrapper });
    await waitFor(() => expect(result.current.listing.isSuccess).toBe(true));

    act(() => setJobs(makeJob({ state: "running", transferredBytes: 500_000 })));
    expect(result.current.allRows.map((r) => r.kind)).toEqual(["ghost"]);

    // Completed after the listing loaded: still shown, as uploaded.
    const finishedAt = new Date(Date.now() + 1000).toISOString();
    act(() => setJobs(makeJob({ state: "completed", transferredBytes: 1_000_000, finishedAt })));
    expect(result.current.allRows).toMatchObject([{ kind: "ghost", done: true }]);

    // The refresh lists the object: one real row, no ghost.
    listed.push("covers/cover.webp");
    await act(() => client.invalidateQueries({ queryKey: qk.objects("conn-1", "media", "covers/") }));
    await waitFor(() => expect(result.current.allRows.map((r) => r.kind)).toEqual(["file"]));
  });

  it("does not show uploads that finished before the listing loaded", async () => {
    setJobs(makeJob({ state: "completed", finishedAt: "2026-01-01T00:00:00Z" }), makeJob({ id: "f", state: "failed" }));
    const client = testQueryClient();
    const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    const { result } = renderHook(() => useExplorerData("conn-1", "media", "covers/"), { wrapper });
    await waitFor(() => expect(result.current.listing.isSuccess).toBe(true));
    expect(result.current.allRows).toEqual([]);
  });
});
