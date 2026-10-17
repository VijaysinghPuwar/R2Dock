import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_PREFERENCES, qk } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { Preferences } from "../../ipc/bindings/Preferences";
import type { SettingsView } from "../../ipc/bindings/SettingsView";
import { useToastStore } from "../../components/toastStore";
import { renderWithProviders, testQueryClient } from "../../test/utils";
import { SettingsPage } from "./SettingsPage";

const settingsSet = vi.fn((prefs: Preferences) => Promise.resolve<SettingsView>({ ...baseView(), preferences: prefs }));
const connectionRemove = vi.fn(() => Promise.resolve());
const logCopyPath = vi.fn((_kind: string) => Promise.resolve());
const openActivityFolder = vi.fn(() => Promise.resolve());
const openLogFolder = vi.fn(() => Promise.resolve());
const activityExportCsv = vi.fn((_range: string) => Promise.resolve<number | null>(12));
const activityResetDir = vi.fn(() => Promise.resolve<SettingsView>(baseView()));

vi.mock("../../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/commands")>();
  return {
    ...actual,
    settingsSet: (prefs: Preferences) => settingsSet(prefs),
    connectionRemove: () => connectionRemove(),
    connectionList: () => Promise.resolve([connection()]),
    bucketList: () => Promise.resolve([]),
    settingsGet: () => Promise.resolve(baseView()),
    logCopyPath: (kind: string) => logCopyPath(kind),
    openActivityFolder: () => openActivityFolder(),
    openLogFolder: () => openLogFolder(),
    activityExportCsv: (range: string) => activityExportCsv(range),
    activityResetDir: () => activityResetDir(),
  };
});

function baseView(): SettingsView {
  return {
    preferences: { ...DEFAULT_PREFERENCES },
    downloadDir: "~/Downloads",
    activityDir: "~/Library/Logs/app.r2dock.desktop/activity",
    activityDirIsDefault: true,
    logDir: "~/Library/Logs/app.r2dock.desktop",
    credentialStore: "macOS Keychain",
    credentialStoreAvailable: true,
  };
}

function connection(): ConnectionSummary {
  return {
    id: "c1",
    name: "Personal R2",
    accountId: "a1b2c3d4e5f6a1b2c3d4e5f6a1b29f0e",
    accountIdShort: "a1b2...9f0e",
    accessKeyHint: "AKIA...1234",
    jurisdiction: "default",
    secretStorage: "keychain",
    status: "online",
    bucketScoped: false,
    manualBuckets: [],
    favoriteBuckets: [],
    publicBaseUrls: {},
    lastTestedAt: null,
    active: true,
  };
}

function setup(view: SettingsView = baseView()) {
  const client = testQueryClient();
  client.setQueryData(qk.settings, view);
  client.setQueryData(qk.connections, [connection()]);
  client.setQueryData(qk.appInfo, {
    name: "R2Dock",
    version: "0.1.0",
    platform: "macos",
    credentialStore: "macOS Keychain",
    credentialStoreAvailable: true,
  });
  return renderWithProviders(<SettingsPage />, { client });
}

describe("SettingsPage", () => {
  beforeEach(() => {
    settingsSet.mockClear();
    connectionRemove.mockClear();
    document.documentElement.dataset.theme = "dark";
    useNavStore.getState().setSettingsSection("connections");
  });

  it("reaches Appearance through the sub-navigation", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Appearance" }));
    expect(screen.getByRole("button", { name: "Appearance" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("radiogroup", { name: "Theme" })).toBeInTheDocument();
  });

  it("applies the Paper theme immediately and saves it", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Appearance" }));
    await user.click(screen.getByRole("radio", { name: "Paper" }));
    expect(document.documentElement.dataset.theme).toBe("paper");
    await waitFor(() => expect(settingsSet).toHaveBeenCalled());
    expect(settingsSet.mock.calls[0]?.[0].theme).toBe("paper");
  });

  it("asks before removing a connection with the exact copy", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Remove" }));
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByRole("heading", { name: "Remove Personal R2?" })).toBeInTheDocument();
    expect(
      within(dialog).getByText("Its keys will be deleted from your system keychain. Your files in R2 are not affected."),
    ).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(connectionRemove).toHaveBeenCalledTimes(1));
  });
});

describe("Activity log settings", () => {
  it("shows the activity log folder and keeps names off by default", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Advanced" }));
    expect(await screen.findByText("~/Library/Logs/app.r2dock.desktop/activity")).toBeInTheDocument();
    const names = screen.getByRole("switch", { name: /Include file and folder names/ });
    expect(names).toHaveAttribute("aria-checked", "false");
    expect(screen.getByRole("switch", { name: /^Activity log/ })).toHaveAttribute("aria-checked", "true");
  });

  it("shows both log locations and calls the right commands", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Advanced" }));
    expect(await screen.findByText("~/Library/Logs/app.r2dock.desktop")).toBeInTheDocument();
    const opens = screen.getAllByRole("button", { name: "Open folder" });
    const copies = screen.getAllByRole("button", { name: "Copy path" });
    expect(opens).toHaveLength(2);
    expect(copies).toHaveLength(2);
    await user.click(opens[0]!);
    expect(openActivityFolder).toHaveBeenCalledTimes(1);
    await user.click(opens[1]!);
    expect(openLogFolder).toHaveBeenCalledTimes(1);
    await user.click(copies[0]!);
    expect(logCopyPath).toHaveBeenLastCalledWith("activity");
    await user.click(copies[1]!);
    expect(logCopyPath).toHaveBeenLastCalledWith("technical");
    expect(screen.queryByRole("button", { name: "Reset to default" })).not.toBeInTheDocument();
  });

  it("exports the chosen range as CSV", async () => {
    const user = userEvent.setup();
    setup();
    await user.click(screen.getByRole("button", { name: "Advanced" }));
    await user.selectOptions(await screen.findByLabelText("Export activity"), "last30Days");
    await user.click(screen.getByRole("button", { name: "Export as CSV..." }));
    await waitFor(() => expect(activityExportCsv).toHaveBeenCalledWith("last30Days"));
    await waitFor(() =>
      expect(useToastStore.getState().toasts.map((t) => t.message)).toContain("Exported 12 rows."),
    );
  });

  it("offers Reset to default for a custom activity folder", async () => {
    const user = userEvent.setup();
    setup({ ...baseView(), activityDir: "~/Documents/r2dock-logs", activityDirIsDefault: false });
    await user.click(screen.getByRole("button", { name: "Advanced" }));
    await user.click(await screen.findByRole("button", { name: "Reset to default" }));
    expect(activityResetDir).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("~/Library/Logs/app.r2dock.desktop/activity")).toBeInTheDocument();
  });
});
