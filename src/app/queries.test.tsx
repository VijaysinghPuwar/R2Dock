import { QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it, vi } from "vitest";
import { DEFAULT_PREFERENCES, qk, useUpdatePreferences } from "./queries";
import type { Preferences } from "../ipc/bindings/Preferences";
import type { SettingsView } from "../ipc/bindings/SettingsView";
import { testQueryClient } from "../test/utils";

const pending: Array<{ prefs: Preferences; resolve: (v: SettingsView) => void; reject: (e: unknown) => void }> = [];

vi.mock("../ipc/commands", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../ipc/commands")>();
  return {
    ...actual,
    settingsSet: (prefs: Preferences) =>
      new Promise<SettingsView>((resolve, reject) => pending.push({ prefs, resolve, reject })),
  };
});

function view(preferences: Preferences): SettingsView {
  return {
    preferences,
    downloadDir: "~/Downloads",
    activityDir: "~/activity",
    activityDirIsDefault: true,
    logDir: "~/logs",
    credentialStore: "system keychain",
    credentialStoreAvailable: true,
  };
}

function setup() {
  pending.length = 0;
  const client = testQueryClient();
  client.setQueryData(qk.settings, view({ ...DEFAULT_PREFERENCES }));
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  const { result } = renderHook(() => useUpdatePreferences(), { wrapper });
  const prefs = () => client.getQueryData<SettingsView>(qk.settings)?.preferences;
  return { client, update: result, prefs };
}

describe("useUpdatePreferences", () => {
  it("ignores a late reply to an older change", async () => {
    const { update, prefs } = setup();
    act(() => update.current({ concurrency: 4 }));
    await waitFor(() => expect(pending).toHaveLength(1));
    act(() => update.current({ concurrency: 6 }));
    await waitFor(() => expect(pending).toHaveLength(2));
    expect(prefs()?.concurrency).toBe(6);

    // The older save answers last-but-one; the newer value must stay.
    await act(async () => pending[0]?.resolve(view(pending[0].prefs)));
    expect(prefs()?.concurrency).toBe(6);
    await act(async () => pending[1]?.resolve(view(pending[1].prefs)));
    expect(prefs()?.concurrency).toBe(6);
  });

  it("a failed older change does not roll back a newer one", async () => {
    const { update, prefs } = setup();
    act(() => update.current({ concurrency: 4 }));
    await waitFor(() => expect(pending).toHaveLength(1));
    act(() => update.current({ historyDays: 7 }));
    await waitFor(() => expect(pending).toHaveLength(2));

    await act(async () => pending[0]?.reject(new Error("disk full")));
    expect(prefs()?.historyDays).toBe(7);
    await act(async () => pending[1]?.resolve(view(pending[1].prefs)));
    expect(prefs()).toMatchObject({ concurrency: 4, historyDays: 7 });
  });
});
