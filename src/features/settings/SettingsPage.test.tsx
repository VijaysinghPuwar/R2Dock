import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DEFAULT_PREFERENCES, qk } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { Preferences } from "../../ipc/bindings/Preferences";
import type { SettingsView } from "../../ipc/bindings/SettingsView";
import { renderWithProviders, testQueryClient } from "../../test/utils";
import { SettingsPage } from "./SettingsPage";

const settingsSet = vi.fn((prefs: Preferences) => Promise.resolve<SettingsView>({ ...baseView(), preferences: prefs }));
const connectionRemove = vi.fn(() => Promise.resolve());

vi.mock("../../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../ipc/commands")>();
  return {
    ...actual,
    settingsSet: (prefs: Preferences) => settingsSet(prefs),
    connectionRemove: () => connectionRemove(),
    connectionList: () => Promise.resolve([connection()]),
    bucketList: () => Promise.resolve([]),
    settingsGet: () => Promise.resolve(baseView()),
  };
});

function baseView(): SettingsView {
  return {
    preferences: { ...DEFAULT_PREFERENCES },
    downloadDir: "~/Downloads",
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

function setup() {
  const client = testQueryClient();
  client.setQueryData(qk.settings, baseView());
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
