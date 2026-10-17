import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_PREFERENCES, qk } from "../../app/queries";
import type { CompressionStatus } from "../../ipc/bindings/CompressionStatus";
import type { InstallProgress } from "../../ipc/bindings/InstallProgress";
import type { Preferences } from "../../ipc/bindings/Preferences";
import type { SettingsView } from "../../ipc/bindings/SettingsView";
import { renderWithProviders, testQueryClient } from "../../test/utils";
import { CompressionSection } from "./CompressionSection";

const notInstalled: CompressionStatus = {
  state: "notInstalled",
  source: null,
  version: null,
  license: null,
  downloadBytes: 17_547_000,
  systemPath: null,
  problem: null,
};
const ready: CompressionStatus = { ...notInstalled, state: "ready", source: "bundled", version: "9.0.2", license: "lgplV3" };

const m = vi.hoisted(() => ({
  status: vi.fn(),
  install: vi.fn(),
  useDownloaded: vi.fn(),
  preview: vi.fn(),
  settingsSet: vi.fn(),
}));

vi.mock("../../lib/media", () => ({ mediaUrl: (token: string) => `r2dock-media://localhost/${token}` }));

vi.mock("../../ipc/commands", async (orig) => ({
  ...(await orig<typeof import("../../ipc/commands")>()),
  compressionStatus: () => m.status(),
  compressionInstall: (cb: (p: InstallProgress) => void) => m.install(cb),
  compressionUseDownloaded: () => m.useDownloaded(),
  compressionPreview: () => m.preview(),
  settingsSet: (p: Preferences) => m.settingsSet(p),
}));

function view(prefs: Preferences = DEFAULT_PREFERENCES): SettingsView {
  return {
    preferences: prefs,
    downloadDir: "~/Downloads",
    activityDir: "~/Library/Logs/app.r2dock.desktop/activity",
    activityDirIsDefault: true,
    logDir: "~/Library/Logs/app.r2dock.desktop",
    credentialStore: "macOS Keychain",
    credentialStoreAvailable: true,
  };
}

function setup() {
  const client = testQueryClient();
  client.setQueryData(qk.settings, view());
  return renderWithProviders(<CompressionSection />, { client });
}

describe("CompressionSection", () => {
  beforeEach(() => {
    m.status.mockReset();
    m.install.mockReset();
    m.useDownloaded.mockReset();
    m.settingsSet.mockReset();
    m.settingsSet.mockImplementation((p: Preferences) => Promise.resolve(view(p)));
  });

  it("is off by default and saves the switch", async () => {
    m.status.mockResolvedValue(notInstalled);
    const user = userEvent.setup();
    setup();
    const sw = screen.getByRole("switch", { name: /Compress audio and video before upload/ });
    expect(sw).toHaveAttribute("aria-checked", "false");
    await user.click(sw);
    await waitFor(() => expect(m.settingsSet).toHaveBeenCalled());
    const saved = m.settingsSet.mock.calls[0]![0] as Preferences;
    expect(saved.compression.enabled).toBe(true);
    expect(saved.compression.audio.speech).toBe("kbps32");
  });

  it("installs the tools with progress", async () => {
    m.status.mockResolvedValue(notInstalled);
    let report: ((p: InstallProgress) => void) | undefined;
    let finish: ((s: CompressionStatus) => void) | undefined;
    m.install.mockImplementation((cb: (p: InstallProgress) => void) => {
      report = cb;
      return new Promise<CompressionStatus>((resolve) => {
        finish = resolve;
      });
    });
    const user = userEvent.setup();
    setup();
    expect(await screen.findByText(/Compression tools are not installed \(17\.5 MB download\)/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Install compression tools" }));
    await waitFor(() => expect(report).toBeDefined());
    report!({ done: 8_000_000, total: 17_547_000 });
    expect(await screen.findByText(/Downloading compression tools: 8\.0 MB of 17\.5 MB/)).toBeInTheDocument();
    expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", "45");
    finish!(ready);
    expect(await screen.findByText(/FFmpeg 9\.0\.2 \(LGPL 3\), downloaded and checked by R2Dock/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Remove tools" })).toBeInTheDocument();
  });

  it("can switch from a system FFmpeg back to the download", async () => {
    m.status.mockResolvedValue({ ...ready, source: "system", systemPath: "/opt/homebrew/bin/ffmpeg" });
    m.useDownloaded.mockResolvedValue(notInstalled);
    const user = userEvent.setup();
    setup();
    expect(await screen.findByText(/from your system/)).toBeInTheDocument();
    expect(screen.getByText("/opt/homebrew/bin/ffmpeg")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Use downloaded tools" }));
    expect(await screen.findByRole("button", { name: "Install compression tools" })).toBeInTheDocument();
  });

  it("explains an unusable FFmpeg", async () => {
    m.status.mockResolvedValue({ ...notInstalled, state: "unusable", problem: "it has no MP3 encoder (libmp3lame)" });
    setup();
    expect(await screen.findByText(/cannot be used: it has no MP3 encoder/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Use FFmpeg from my system..." })).toBeInTheDocument();
  });

  it("plays quality samples", async () => {
    m.status.mockResolvedValue(ready);
    m.preview.mockResolvedValue([
      { label: "Original", bytesPerHour: 160_000_000, token: "a".repeat(32) },
      { label: "Speech 32 kbps (MP3)", bytesPerHour: 14_400_000, token: "b".repeat(32) },
    ]);
    const user = userEvent.setup();
    setup();
    const button = await screen.findByRole("button", { name: "Choose a file..." });
    await waitFor(() => expect(button).toBeEnabled());
    await user.click(button);
    expect(await screen.findByText(/^Speech 32 kbps \(MP3\)/)).toBeInTheDocument();
    expect(screen.getByText(/about 14\.4 MB per hour/)).toBeInTheDocument();
    expect(screen.getByLabelText("Speech 32 kbps (MP3) sample")).toHaveAttribute("src", expect.stringContaining("b".repeat(32)));
  });

  it("saves the video choices", async () => {
    m.status.mockResolvedValue(ready);
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("radio", { name: "Most compatible (H.264)" }));
    await waitFor(() => expect(m.settingsSet).toHaveBeenCalled());
    expect((m.settingsSet.mock.calls.at(-1)![0] as Preferences).compression.video.codec).toBe("h264");
    await user.selectOptions(screen.getByLabelText("Video size"), "off");
    await waitFor(() =>
      expect((m.settingsSet.mock.calls.at(-1)![0] as Preferences).compression.video.size).toBe("off"),
    );
  });
});
