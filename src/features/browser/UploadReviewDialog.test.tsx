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
  compression: null,
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
    await waitFor(() => expect(commands.uploadStart).toHaveBeenCalledWith("plan-1", "keepBoth", true));
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
    await waitFor(() => expect(commands.uploadStart).toHaveBeenCalledWith("plan-1", "ask", true));
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

  it("shows the compression estimate and passes the switch", async () => {
    const user = userEvent.setup();
    commands.uploadStart.mockResolvedValue(3);
    renderWithProviders(<UploadReviewDialog />);
    open({
      ...plan,
      compression: {
        files: 195,
        bytesBefore: 2_400_000_000,
        bytesAfter: 1_200_000_000,
        unchanged: 2,
        videoFiles: 0,
        keyChanges: [{ from: "raw/Take 1.wav", to: "raw/Take 1.mp3" }],
        keyChangeCount: 1,
        collisions: [],
        toolsReady: true,
      },
    });
    const sw = await screen.findByRole("switch", { name: /Compress audio before upload/ });
    expect(screen.getByText(/195 audio files\. Estimated 2\.4 GB to 1\.2 GB\./)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "1 file gets a new extension" }));
    expect(screen.getByText("raw/Take 1.mp3")).toBeInTheDocument();
    await user.click(sw);
    await user.click(screen.getByRole("button", { name: "Upload" }));
    await waitFor(() => expect(commands.uploadStart).toHaveBeenCalledWith("plan-1", "skip", false));
  });

  it("warns when the compression tools are missing", async () => {
    renderWithProviders(<UploadReviewDialog />);
    open({
      ...plan,
      compression: {
        files: 1,
        bytesBefore: 10,
        bytesAfter: 5,
        unchanged: 0,
        videoFiles: 0,
        keyChanges: [],
        keyChangeCount: 0,
        collisions: ["a.mp3"],
        toolsReady: false,
      },
    });
    expect(await screen.findByText(/Compression tools are not installed/)).toBeInTheDocument();
    expect(screen.getByText(/used by more than one file/)).toBeInTheDocument();
  });
});
