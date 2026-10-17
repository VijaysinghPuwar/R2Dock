import { act, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { UploadPlan } from "../../ipc/bindings/UploadPlan";
import { renderWithProviders } from "../../test/utils";
import { useUploadStore } from "./upload";
import { UploadReviewDialog } from "./UploadReviewDialog";

const commands = vi.hoisted(() => ({ uploadStart: vi.fn() }));

vi.mock("../../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/commands")>();
  return { ...actual, uploadStart: commands.uploadStart };
});

const plan: UploadPlan = {
  planId: "plan-1",
  files: 212,
  folders: 2,
  bytes: 3_400_000_000,
  conflicts: 4,
  skipped: [{ path: "~/Desktop/link", reason: "Shortcut (symbolic link)" }],
  hasFolders: true,
  needsReview: true,
  tooMany: false,
  defaultPolicy: "skip",
};

function open(p: UploadPlan = plan) {
  act(() => {
    useUploadStore.getState().setReview({
      plan: p,
      target: { connectionId: "c1", connectionName: "Personal R2", bucket: "media-library", prefix: "Audiobook G/" },
      selection: { id: "sel-1", count: 214, names: ["a"] },
    });
  });
}

beforeEach(() => {
  commands.uploadStart.mockReset();
  useUploadStore.getState().setReview(null);
});

describe("UploadReviewDialog", () => {
  it("preselects the plan's default policy", () => {
    renderWithProviders(<UploadReviewDialog />);
    open();
    expect(screen.getByRole("heading", { name: "Upload 214 items" })).toBeInTheDocument();
    expect(screen.getByText("2 folders, 212 files, 3.4 GB")).toBeInTheDocument();
    expect(screen.getByText("4 files already exist in this folder.")).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Skip existing" })).toBeChecked();
    expect(screen.getByRole("radio", { name: "Ask for each" })).not.toBeChecked();
  });

  it("passes the chosen policy to uploadStart", async () => {
    const user = userEvent.setup();
    commands.uploadStart.mockResolvedValue(212);
    renderWithProviders(<UploadReviewDialog />);
    open();
    await user.click(screen.getByRole("radio", { name: /Keep both/ }));
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await waitFor(() => expect(commands.uploadStart).toHaveBeenCalledWith("plan-1", "keepBoth"));
    await waitFor(() => expect(useUploadStore.getState().review).toBeNull());
  });

  it("uses the default policy when nothing is changed and shows skipped items on request", async () => {
    const user = userEvent.setup();
    commands.uploadStart.mockResolvedValue(212);
    renderWithProviders(<UploadReviewDialog />);
    open({ ...plan, defaultPolicy: "ask" });
    expect(screen.getByRole("radio", { name: "Ask for each" })).toBeChecked();
    await user.click(screen.getByRole("button", { name: "Show list" }));
    expect(screen.getByText("~/Desktop/link")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await waitFor(() => expect(commands.uploadStart).toHaveBeenCalledWith("plan-1", "ask"));
  });

  it("keeps the dialog open when starting fails", async () => {
    const user = userEvent.setup();
    commands.uploadStart.mockRejectedValue({ code: "network", message: "Could not connect.", detail: null, retryable: true });
    renderWithProviders(<UploadReviewDialog />);
    open();
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await waitFor(() => expect(commands.uploadStart).toHaveBeenCalled());
    expect(useUploadStore.getState().review).not.toBeNull();
  });
});
