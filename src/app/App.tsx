import { QueryClientProvider, useQueryClient } from "@tanstack/react-query";
import { PlugZap } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { TooltipProvider } from "../components/Tooltip";
import { Toaster } from "../components/Toaster";
import { Button } from "../components/Button";
import { EmptyState } from "../components/Feedback";
import { errorMessage } from "../ipc/commands";
import { startTransferSubscription } from "../features/transfers/subscription";
import { useAppearance } from "./appearance";
import { useNavStore } from "./navStore";
import { createQueryClient, useConnections, usePreferences } from "./queries";
import { Shell } from "./Shell";
import "./app.css";

function Bootstrap() {
  const client = useQueryClient();
  const prefs = usePreferences();
  const connections = useConnections();
  const routed = useRef(false);
  useAppearance(prefs);

  useEffect(() => {
    startTransferSubscription(client);
  }, [client]);

  // Pick the first screen once connections are known.
  useEffect(() => {
    if (routed.current || !connections.data) return;
    routed.current = true;
    useNavStore.getState().reset(connections.data.length === 0 ? "welcome" : "buckets");
  }, [connections.data]);

  // Return to Welcome when the last connection is removed.
  useEffect(() => {
    if (!routed.current || !connections.data) return;
    const { loc, reset } = useNavStore.getState();
    if (connections.data.length === 0 && loc.view !== "welcome" && loc.view !== "settings") reset("welcome");
    if (connections.data.length > 0 && loc.view === "welcome") reset("buckets");
  }, [connections.data]);

  if (connections.isError) {
    return (
      <div className="boot-state">
        <EmptyState
          icon={<PlugZap size={32} />}
          title="Could not load connections"
          body={errorMessage(connections.error)}
          actions={<Button onClick={() => void connections.refetch()}>Try again</Button>}
        />
      </div>
    );
  }

  if (!connections.data) return <div className="boot-state" aria-busy="true" />;
  return <Shell />;
}

export function App() {
  const [client] = useState(createQueryClient);
  return (
    <QueryClientProvider client={client}>
      <TooltipProvider>
        <Bootstrap />
        <Toaster />
      </TooltipProvider>
    </QueryClientProvider>
  );
}
