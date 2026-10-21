import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Selection } from "../../ipc/bindings/Selection";
import type { UploadPlan } from "../../ipc/bindings/UploadPlan";
import { useToastStore } from "../../components/toastStore";

const prepare = vi.fn();

vi.mock("../../ipc/commands", async (orig) => ({
  ...(await orig<typeof import("../../ipc/commands")>()),
  uploadPrepare: (id: string) => prepare(id),
}));

const { uploadSelection, useUploadStore } = await import("./upload");

function plan(planId: string): UploadPlan {
  return {
    planId,
    files: 2,
    folders: 0,
    bytes: 10,
    conflicts: 1,
    skipped: 0,
    hasFolders: false,
    needsReview: true,
    tooMany: false,
    defaultPolicy: "ask",
    compression: null,
  } as UploadPlan;
}

const target = { connectionId: "c", connectionName: "Studio", bucket: "media", prefix: "" };
const selection = (id: string) => ({ id }) as unknown as Selection;

describe("uploadSelection", () => {
  beforeEach(() => {
    prepare.mockReset();
    useUploadStore.setState({ review: null, preparing: false });
    useToastStore.getState().clear();
  });

  it("keeps an open review when another drop needs one", async () => {
    prepare.mockImplementation((id: string) => Promise.resolve(plan(id)));
    await uploadSelection(selection("first"), target);
    await uploadSelection(selection("second"), target);
    expect(useUploadStore.getState().review?.plan.planId).toBe("first");
    expect(useToastStore.getState().toasts.map((t) => t.message)).toContain(
      "Finish the open upload review first, then add more files.",
    );
  });

  it("stays busy until the last of several prepares ends", async () => {
    let endFirst: (p: UploadPlan) => void = () => {};
    let endSecond: (p: UploadPlan) => void = () => {};
    prepare
      .mockImplementationOnce(() => new Promise((r) => (endFirst = r)))
      .mockImplementationOnce(() => new Promise((r) => (endSecond = r)));
    const a = uploadSelection(selection("first"), target);
    const b = uploadSelection(selection("second"), target);
    endFirst(plan("first"));
    await a;
    expect(useUploadStore.getState().preparing).toBe(true);
    endSecond(plan("second"));
    await b;
    expect(useUploadStore.getState().preparing).toBe(false);
  });
});
