import { useQueryClient } from "@tanstack/react-query";
import { useEffect } from "react";
import type { ConnectionSummary } from "../ipc/bindings/ConnectionSummary";
import { hasMod, isEditableTarget } from "../lib/platform";
import { copyKeys, deleteRows, renameRow, type ExplorerContext } from "../features/browser/actions";
import { selectedRows, useBrowserStore } from "../features/browser/browserStore";
import { pickAndUpload } from "../features/browser/upload";
import { usePreviewStore } from "../features/preview/previewStore";
import { useNavStore } from "./navStore";
import { qk, usePlatform } from "./queries";
import { refreshCurrentView } from "./refresh";
import { useUiStore } from "./uiStore";

function anyDialogOpen(): boolean {
  return document.querySelector('[role="dialog"], [role="alertdialog"], [role="menu"]') !== null;
}

/** App-wide keyboard shortcuts (DESIGN 7.3). */
export function useGlobalShortcuts() {
  const client = useQueryClient();
  const platform = usePlatform();

  useEffect(() => {
    const explorerContext = (): ExplorerContext | null => {
      const { loc } = useNavStore.getState();
      if (loc.view !== "browser" || !loc.bucket) return null;
      const connections = client.getQueryData<ConnectionSummary[]>(qk.connections) ?? [];
      const connection = connections.find((c) => c.active) ?? connections[0];
      if (!connection) return null;
      return { connection, bucket: loc.bucket, prefix: loc.prefix };
    };

    const onKeyDown = (e: KeyboardEvent) => {
      const mod = hasMod(e, platform);
      const key = e.key;
      const lower = key.toLowerCase();
      const editable = isEditableTarget(e.target);
      const nav = useNavStore.getState();
      const ui = useUiStore.getState();

      // Refresh never reloads the webview, with or without Shift (a hard
      // reload would wipe the app's state).
      if (key === "F5" || (mod && lower === "r")) {
        e.preventDefault();
        refreshCurrentView(client);
        return;
      }

      // Moving to another page or folder under an open dialog would leave the
      // dialog acting on a place no longer shown.
      const dialogOpen = anyDialogOpen();

      if (mod && key === "," && !dialogOpen) {
        e.preventDefault();
        nav.openSettings();
        return;
      }

      if (mod && lower === "j" && !e.shiftKey) {
        e.preventDefault();
        ui.setDockExpanded(!ui.dockExpanded);
        return;
      }

      if (usePreviewStore.getState().open) return;

      if (!dialogOpen && ((mod && key === "[") || (e.altKey && key === "ArrowLeft" && !editable))) {
        e.preventDefault();
        nav.goBack();
        return;
      }
      if (!dialogOpen && ((mod && key === "]") || (e.altKey && key === "ArrowRight" && !editable))) {
        e.preventDefault();
        nav.goForward();
        return;
      }

      if (key === "?" && !editable && !mod && !dialogOpen) {
        e.preventDefault();
        ui.setShortcutsOpen(true);
        return;
      }

      const ctx = explorerContext();
      if (!ctx || dialogOpen) return;
      const browser = useBrowserStore.getState();
      const selection = selectedRows(browser);

      if (mod && lower === "u") {
        e.preventDefault();
        void pickAndUpload(e.shiftKey ? "folder" : "files", {
          connectionId: ctx.connection.id,
          connectionName: ctx.connection.name,
          bucket: ctx.bucket,
          prefix: ctx.prefix,
        });
        return;
      }
      if (mod && e.shiftKey && lower === "n") {
        e.preventDefault();
        ui.setNewFolderOpen(true);
        return;
      }
      if (mod && lower === "f") {
        e.preventDefault();
        ui.focusSearch();
        return;
      }
      if (mod && (key === "1" || key === "2")) {
        e.preventDefault();
        ui.setViewMode(key === "1" ? "list" : "grid");
        return;
      }
      if (mod && lower === "i") {
        e.preventDefault();
        const current = ui.detailsOpen ?? !ui.narrow;
        ui.setDetailsOpen(!current);
        return;
      }
      if (mod && e.shiftKey && lower === "c") {
        if (selection.length === 0) return;
        e.preventDefault();
        copyKeys(selection);
        return;
      }

      if (editable) return;

      if (mod && lower === "a") {
        e.preventDefault();
        browser.setSelected(
          browser.visibleRows.map((r) => r.id),
          browser.visibleRows[0]?.id ?? null,
        );
        return;
      }
      if (key === "Delete" || (mod && key === "Backspace")) {
        if (selection.length === 0) return;
        e.preventDefault();
        deleteRows(ctx, selection);
        return;
      }
      // F2 renames, as in File Explorer.
      const only = selection.length === 1 ? selection[0] : undefined;
      if (key === "F2" && only) {
        e.preventDefault();
        renameRow(ctx, only);
        return;
      }
      if ((key === "Backspace" && !mod) || (e.altKey && key === "ArrowUp")) {
        e.preventDefault();
        nav.goUp();
        return;
      }
      if (key === "Escape") {
        // Escape that closed a dialog or menu should not also clear the selection.
        if (e.target instanceof Element && e.target.closest('[role="dialog"], [role="menu"], [role="alertdialog"]')) return;
        if (browser.selected.size > 0) {
          e.preventDefault();
          browser.clearSelection();
        } else if (browser.search) {
          browser.setSearch("");
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [client, platform]);
}
