import type { QueryClient } from "@tanstack/react-query";
import type { ConnectionSummary } from "../ipc/bindings/ConnectionSummary";
import { qk } from "./queries";
import { useNavStore } from "./navStore";

/** Reload what the current view shows. Never reloads the webview. */
export function refreshCurrentView(client: QueryClient): void {
  const { loc } = useNavStore.getState();
  const connections = client.getQueryData<ConnectionSummary[]>(qk.connections) ?? [];
  const connection = connections.find((c) => c.active) ?? connections[0];
  switch (loc.view) {
    case "browser":
      if (connection && loc.bucket) {
        void client.resetQueries({ queryKey: qk.objects(connection.id, loc.bucket, loc.prefix) });
        void client.resetQueries({ queryKey: ["search", connection.id, loc.bucket] });
        void client.invalidateQueries({ queryKey: ["head", connection.id, loc.bucket] });
        void client.invalidateQueries({ queryKey: ["text", connection.id, loc.bucket] });
      }
      break;
    case "buckets":
    case "favorites":
      void client.invalidateQueries({ queryKey: qk.connections });
      if (connection) void client.invalidateQueries({ queryKey: qk.buckets(connection.id) });
      break;
    case "recent":
      void client.invalidateQueries({ queryKey: qk.history });
      break;
    case "settings":
      void client.invalidateQueries({ queryKey: qk.connections });
      void client.invalidateQueries({ queryKey: qk.settings });
      break;
    default:
      break;
  }
}
