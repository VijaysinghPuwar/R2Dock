import type { QueryClient } from "@tanstack/react-query";
import { create } from "zustand";
import { qk } from "../../app/queries";
import { toast } from "../../components/toastStore";
import { compressionInstall, compressionPreview, errorMessage, isAppError } from "../../ipc/commands";
import type { CompressionStatus } from "../../ipc/bindings/CompressionStatus";
import type { PreviewSample } from "../../ipc/bindings/PreviewSample";

export interface InstallProgressState {
  done: number;
  total: number;
}

interface CompressionTasks {
  /** The running tools install. */
  progress: InstallProgressState | null;
  /** Remove tools / Use FFmpeg from my system / Use downloaded tools. */
  working: boolean;
  /** "Compare quality" encoding, and its last result. */
  previewBusy: boolean;
  samples: PreviewSample[] | null;
}

/**
 * Work started in Settings > Compression. It lives outside the page, so
 * leaving the page (or switching sections) keeps it, and coming back shows
 * the same download or encode instead of a fresh button that would start
 * it a second time.
 */
export const useToolsInstallStore = create<CompressionTasks>(() => ({
  progress: null,
  working: false,
  previewBusy: false,
  samples: null,
}));

/** Start the install unless one is already running from this window. */
export async function startToolsInstall(client: QueryClient, total: number) {
  if (useToolsInstallStore.getState().progress) return;
  useToolsInstallStore.setState({ progress: { done: 0, total } });
  // Cached status must say installing too, so a page opened later polls it.
  client.setQueryData<CompressionStatus>(qk.compression, (s) => (s ? { ...s, state: "installing" } : s));
  try {
    const status = await compressionInstall((p) => useToolsInstallStore.setState({ progress: { done: p.done, total: p.total } }));
    client.setQueryData(qk.compression, status);
    toast.success("Compression tools installed.");
  } catch (e) {
    if (!(isAppError(e) && e.code === "cancelled")) toast.danger(errorMessage(e));
    void client.invalidateQueries({ queryKey: qk.compression });
  } finally {
    useToolsInstallStore.setState({ progress: null });
  }
}

/** Run one tools action at a time and store the status it returns. */
export async function runToolsAction(client: QueryClient, call: () => Promise<CompressionStatus | null>) {
  if (useToolsInstallStore.getState().working) return;
  useToolsInstallStore.setState({ working: true });
  try {
    const s = await call();
    if (s) client.setQueryData(qk.compression, s);
  } catch (e) {
    toast.danger(errorMessage(e));
  } finally {
    useToolsInstallStore.setState({ working: false });
  }
}

/** "Compare quality": pick a file and encode the samples. */
export async function startPreview() {
  const run = ++previewRun;
  useToolsInstallStore.setState({ previewBusy: true });
  try {
    const result = await compressionPreview();
    if (result && run === previewRun) useToolsInstallStore.setState({ samples: result });
  } catch (e) {
    // A newer comparison stopped this one.
    if (!(isAppError(e) && e.code === "cancelled")) toast.danger(errorMessage(e));
  } finally {
    if (run === previewRun) useToolsInstallStore.setState({ previewBusy: false });
  }
}

let previewRun = 0;
