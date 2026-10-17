import { useQueryClient } from "@tanstack/react-query";
import { connectionSetActive, errorMessage } from "../../ipc/commands";
import { qk } from "../../app/queries";
import { useNavStore } from "../../app/navStore";
import { toast } from "../../components/toastStore";
import { useBrowserStore } from "../browser/browserStore";

/** Make a connection active and return to its bucket list. */
export function useSwitchConnection() {
  const client = useQueryClient();
  return async (id: string) => {
    try {
      await connectionSetActive(id);
      await client.invalidateQueries({ queryKey: qk.connections });
      useBrowserStore.getState().resetForLocation();
      useNavStore.getState().reset("buckets");
    } catch (e) {
      toast.danger(errorMessage(e));
    }
  };
}
