import { useMemo } from "react";
import { useShallow } from "zustand/react/shallow";
import { Button } from "../../components/Button";
import { Banner } from "../../components/Feedback";
import { useNavStore } from "../../app/navStore";
import { controlMany } from "./actions";
import { useTransferStore } from "./transferStore";

/** Warning shown when jobs were interrupted by the app closing. */
export function InterruptedBanner({ onReview }: { onReview?: () => void }) {
  const ids = useTransferStore(
    useShallow((s) => s.order.filter((id) => s.jobs[id]?.state === "interrupted")),
  );
  const text = useMemo(
    () =>
      ids.length === 1
        ? "1 transfer was interrupted when R2Dock closed. Large uploads can continue where they stopped."
        : `${ids.length.toLocaleString("en-US")} transfers were interrupted when R2Dock closed. Large uploads can continue where they stopped.`,
    [ids.length],
  );
  if (ids.length === 0) return null;
  return (
    <Banner
      tone="warning"
      actions={
        <>
          <Button size="sm" variant="primary" onClick={() => controlMany(ids, { action: "resume" })}>
            Resume all
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={() => {
              if (onReview) onReview();
              else useNavStore.getState().navigate({ view: "transfers" });
            }}
          >
            Review
          </Button>
          <Button size="sm" variant="ghost" onClick={() => controlMany(ids, { action: "cancel" })}>
            Discard
          </Button>
        </>
      }
    >
      {text}
    </Banner>
  );
}
