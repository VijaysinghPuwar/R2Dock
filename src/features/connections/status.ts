import type { ConnectionStatus } from "../../ipc/bindings/ConnectionStatus";

export function statusTone(status: ConnectionStatus): "success" | "warning" | "muted" {
  if (status === "online") return "success";
  if (status === "needsAttention") return "warning";
  return "muted";
}

export function statusLabel(status: ConnectionStatus): string {
  switch (status) {
    case "online":
      return "Online";
    case "offline":
      return "Offline";
    case "needsAttention":
      return "Needs attention";
    case "unknown":
      return "Not checked yet";
  }
}
