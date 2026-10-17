import { screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders } from "../../test/utils";
import { makeJob } from "./testJobs";

const transferControl = vi.fn((_id: string, _action: unknown) => Promise.resolve());
const transferShow = vi.fn((_id: string) => Promise.resolve());
const openActivityFolder = vi.fn(() => Promise.resolve());

vi.mock("../../ipc/commands", async (orig) => ({
  ...(await orig<typeof import("../../ipc/commands")>()),
  transferControl: (id: string, action: unknown) => transferControl(id, action),
  transferControlMany: vi.fn(() => Promise.resolve()),
  transferShow: (id: string) => transferShow(id),
  openActivityFolder: () => openActivityFolder(),
}));

const { TransferRow } = await import("./TransferRow");

describe("TransferRow", () => {
  beforeEach(() => {
    transferControl.mockClear();
    transferShow.mockClear();
  });

  it("shows Pause only when the job is pausable", () => {
    const { unmount } = renderWithProviders(<TransferRow job={makeJob({ state: "running", pausable: false })} />);
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeInTheDocument();
    unmount();
    renderWithProviders(<TransferRow job={makeJob({ state: "running", pausable: true })} />);
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
  });

  it("offers Resume for paused and interrupted jobs", async () => {
    const { unmount } = renderWithProviders(<TransferRow job={makeJob({ id: "p", state: "paused", pausable: true })} />);
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Resume" }));
    expect(transferControl).toHaveBeenCalledWith("p", { action: "resume" });
    unmount();
    renderWithProviders(<TransferRow job={makeJob({ state: "interrupted" })} />);
    expect(screen.getByRole("button", { name: "Resume" })).toBeInTheDocument();
  });

  it("offers Retry for failed jobs and shows the error", async () => {
    renderWithProviders(
      <TransferRow
        job={makeJob({
          id: "f",
          state: "failed",
          error: { code: "network", message: "Connection lost after 3 attempts.", detail: "req-1", retryable: true },
        })}
      />,
    );
    expect(screen.getByText("Connection lost after 3 attempts.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(transferControl).toHaveBeenCalledWith("f", { action: "retry" });
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull();
  });

  it("links failed job details to the activity log", async () => {
    renderWithProviders(
      <TransferRow
        job={makeJob({
          state: "failed",
          error: { code: "accessDenied", message: "This key cannot do that.", detail: "request id r1", retryable: false },
        })}
      />,
    );
    expect(screen.queryByRole("button", { name: "Open activity log" })).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Details" }));
    expect(screen.getByText(/request id r1/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Open activity log" }));
    expect(openActivityFolder).toHaveBeenCalledTimes(1);
  });

  it("shows compression progress and the saving", () => {
    const { unmount } = renderWithProviders(
      <TransferRow job={makeJob({ state: "compressing", pausable: true, compression: { kind: "pending", fraction: 0.42 } })} />,
    );
    expect(screen.getByText(/Compressing 42%/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Pause" })).toBeInTheDocument();
    unmount();
    renderWithProviders(
      <TransferRow
        job={makeJob({
          state: "completed",
          totalBytes: 330_000_000,
          compression: { kind: "saved", originalBytes: 660_000_000, bytes: 330_000_000, preset: "mp3-32k" },
          finishedAt: "2026-10-17T10:03:00Z",
        })}
      />,
    );
    expect(screen.getByText(/Saved 330 MB \(50%\)/)).toBeInTheDocument();
  });

  it("resolves conflicts with the chosen policy", async () => {
    renderWithProviders(<TransferRow job={makeJob({ id: "d", state: "needsDecision" })} />);
    await userEvent.click(screen.getByRole("button", { name: "Replace" }));
    await userEvent.click(screen.getByRole("button", { name: "Skip" }));
    await userEvent.click(screen.getByRole("button", { name: "Keep both" }));
    expect(transferControl.mock.calls).toEqual([
      ["d", { action: "resolve", policy: "replace" }],
      ["d", { action: "resolve", policy: "skip" }],
      ["d", { action: "resolve", policy: "keepBoth" }],
    ]);
  });

  it("shows Show in folder for completed downloads only", async () => {
    const { unmount } = renderWithProviders(
      <TransferRow job={makeJob({ id: "dl", direction: "download", state: "completed", finishedAt: "2026-10-17T10:00:10Z" })} />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Show in folder" }));
    expect(transferShow).toHaveBeenCalledWith("dl");
    unmount();
    renderWithProviders(<TransferRow job={makeJob({ state: "completed" })} />);
    expect(screen.queryByRole("button", { name: "Show in folder" })).toBeNull();
  });

  it("shows retry attempts, speed and eta only when known", () => {
    const { unmount } = renderWithProviders(<TransferRow job={makeJob({ state: "retrying", attempt: 2, maxAttempts: 3 })} />);
    expect(screen.getByText("Retrying (2 of 3)")).toBeInTheDocument();
    unmount();
    renderWithProviders(<TransferRow job={makeJob({ state: "running", speedBps: null, etaSecs: null })} />);
    expect(screen.queryByText(/\/s/)).toBeNull();
    expect(screen.queryByText(/left/)).toBeNull();
  });
});
