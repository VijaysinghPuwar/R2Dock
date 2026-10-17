import { Button } from "../../components/Button";
import { Dialog } from "../../components/Dialog";
import type { KeyFailure } from "../../ipc/bindings/KeyFailure";
import { visibleText } from "../../lib/keys";
import "./manage.css";

export interface OpResult {
  title: string;
  summary: string;
  failures: KeyFailure[];
  copiedNotDeleted?: KeyFailure[];
}

function FailureList({ items }: { items: KeyFailure[] }) {
  return (
    <ul className="failure-list">
      {items.map((f, i) => (
        <li key={`${f.key}-${i}`}>
          <span className="mono">{visibleText(f.key)}</span>
          <span className="secondary">{f.message}</span>
        </li>
      ))}
    </ul>
  );
}

/** Final summary for bulk delete and move, listing anything that failed. */
export function OpResultDialog({ result, onClose, onRetry }: { result: OpResult; onClose: () => void; onRetry?: () => void }) {
  const leftovers = result.copiedNotDeleted ?? [];
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      title={result.title}
      width="review"
      footer={
        <>
          {onRetry && result.failures.length > 0 && (
            <Button variant="secondary" onClick={onRetry}>
              Retry failed
            </Button>
          )}
          <Button variant="primary" onClick={onClose}>
            Done
          </Button>
        </>
      }
    >
      <p>{result.summary}</p>
      {result.failures.length > 0 && (
        <section className="result-section">
          <h3 className="result-heading">Failed ({result.failures.length.toLocaleString("en-US")})</h3>
          <FailureList items={result.failures} />
        </section>
      )}
      {leftovers.length > 0 && (
        <section className="result-section">
          <h3 className="result-heading">Copied but not deleted ({leftovers.length.toLocaleString("en-US")})</h3>
          <p className="small secondary">These objects now exist in both places. Delete the originals when you are ready.</p>
          <FailureList items={leftovers} />
        </section>
      )}
    </Dialog>
  );
}
