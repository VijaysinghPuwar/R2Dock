import { listen } from "@tauri-apps/api/event";
import { useEffect } from "react";
import { create } from "zustand";
import { DRAG_EVENT } from "../../ipc/commands";
import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { DragEvent } from "../../ipc/bindings/DragEvent";
import { useQueryClient } from "@tanstack/react-query";
import { qk } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import { toast } from "../../components/toastStore";
import { uploadSelection } from "./upload";

interface DragState {
  active: boolean;
  count: number;
  set: (active: boolean, count?: number) => void;
}

/** Whether OS files are being dragged over the window. */
export const useDragStore = create<DragState>((set) => ({
  active: false,
  count: 0,
  set: (active, count = 0) => set({ active, count }),
}));

/** Listen for OS drag and drop events from Rust and upload drops into the open folder. */
export function useDragDrop() {
  const client = useQueryClient();
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    listen<DragEvent>(DRAG_EVENT, (event) => {
      const payload = event.payload;
      const drag = useDragStore.getState();
      switch (payload.type) {
        case "enter":
          drag.set(true, payload.count);
          break;
        case "over":
          if (!drag.active) drag.set(true, drag.count);
          break;
        case "leave":
          drag.set(false);
          break;
        case "drop": {
          drag.set(false);
          const { loc } = useNavStore.getState();
          const connections = client.getQueryData<ConnectionSummary[]>(qk.connections) ?? [];
          const connection = connections.find((c) => c.active) ?? connections[0];
          if (loc.view !== "browser" || !loc.bucket || !connection) {
            toast.neutral("Open a bucket first, then drop files to upload them.");
            return;
          }
          void uploadSelection(payload.selection, {
            connectionId: connection.id,
            connectionName: connection.name,
            bucket: loc.bucket,
            prefix: loc.prefix,
          });
          break;
        }
      }
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(() => {
        // Not running inside the desktop shell (for example in tests).
      });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [client]);
}
