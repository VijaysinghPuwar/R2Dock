import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { Dialog } from "../../components/Dialog";
import { toast } from "../../components/toastStore";
import { connectionSetActive } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import { useNavStore } from "../../app/navStore";
import { qk, useAppInfoValue, useConnections } from "../../app/queries";
import { useUiStore } from "../../app/uiStore";
import { ConnectForm } from "./ConnectForm";
import { HelpPanel } from "./HelpPanel";
import "./connections.css";

const WIDE = 1100;

function useWide(): boolean {
  const [wide, setWide] = useState(() => window.innerWidth >= WIDE);
  useEffect(() => {
    const onResize = () => setWide(window.innerWidth >= WIDE);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return wide;
}

export function ConnectDialog() {
  const state = useUiStore((s) => s.connectDialog);
  const close = useUiStore((s) => s.closeConnect);
  const info = useAppInfoValue();
  const { data: connections } = useConnections();
  const client = useQueryClient();
  const wide = useWide();
  const [helpOpen, setHelpOpen] = useState(false);

  useEffect(() => {
    setHelpOpen(state?.showHelp ?? false);
  }, [state]);

  if (!state) return null;
  const existing = state.mode === "edit" ? (connections?.find((c) => c.id === state.id) ?? null) : null;
  if (state.mode === "edit" && !existing) return null;

  const onSaved = async (summary: ConnectionSummary) => {
    const isNew = state.mode === "new";
    if (isNew) {
      try {
        await connectionSetActive(summary.id);
      } catch {
        // The connection is saved; switching can be done from the sidebar.
      }
    }
    await client.invalidateQueries({ queryKey: qk.connections });
    void client.invalidateQueries({ queryKey: qk.buckets(summary.id) });
    if (!isNew) void client.invalidateQueries({ queryKey: qk.objectsAll(summary.id) });
    close();
    if (isNew) {
      useNavStore.getState().reset("buckets");
      toast.success(`Connected to ${summary.name}.`);
    } else {
      toast.success(`Saved ${summary.name}.`);
    }
  };

  const showHelpColumn = wide;
  const title = state.mode === "new" ? "Connect R2 account" : `Edit ${existing?.name ?? "connection"}`;

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
      title={title}
      width={showHelpColumn ? "wide" : "form"}
    >
      <div className={showHelpColumn ? "connect-layout is-two-column" : "connect-layout"}>
        <div className="connect-main">
          {!showHelpColumn && helpOpen && <HelpPanel />}
          <ConnectForm
            key={state.mode === "edit" ? `${state.id}-${String(state.replaceSecret)}` : "new"}
            existing={existing}
            replaceSecret={state.mode === "edit" ? state.replaceSecret : false}
            credentialStoreAvailable={info.credentialStoreAvailable}
            onCancel={close}
            onSaved={onSaved}
            onShowHelp={showHelpColumn ? undefined : () => setHelpOpen((v) => !v)}
          />
        </div>
        {showHelpColumn && (
          <aside className="connect-help">
            <HelpPanel />
          </aside>
        )}
      </div>
    </Dialog>
  );
}
