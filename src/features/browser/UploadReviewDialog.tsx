import { ChevronRight, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { useNavStore } from "../../app/navStore";
import { Button } from "../../components/Button";
import { Switch } from "../../components/Controls";
import { Dialog } from "../../components/Dialog";
import { InlineResult } from "../../components/Feedback";
import type { CompressionEstimate } from "../../ipc/bindings/CompressionEstimate";
import type { ConflictPolicy } from "../../ipc/bindings/ConflictPolicy";
import type { UploadPlan } from "../../ipc/bindings/UploadPlan";
import { formatBytes, plural } from "../../lib/format";
import { splitPrefix } from "../../lib/keys";
import { startPlan, useUploadStore, type UploadTarget } from "./upload";
import "./browser.css";

const POLICIES: Array<{ value: ConflictPolicy; label: string }> = [
  { value: "ask", label: "Ask for each" },
  { value: "replace", label: "Replace existing" },
  { value: "skip", label: "Skip existing" },
  { value: "keepBoth", label: 'Keep both (adds " (1)" before the extension)' },
];

export interface UploadReviewProps {
  plan: UploadPlan;
  target: UploadTarget;
  onCancel: () => void;
  onStart: (policy: ConflictPolicy, compress: boolean) => void | Promise<void>;
}

function CompressionBox({
  estimate,
  compress,
  onChange,
  onOpenSettings,
}: {
  estimate: CompressionEstimate;
  compress: boolean;
  onChange: (v: boolean) => void;
  onOpenSettings: () => void;
}) {
  const [showKeys, setShowKeys] = useState(false);
  if (estimate.files === 0) {
    return (
      <p className="secondary review-compression">
        {estimate.unchanged === 1
          ? "The audio file here is already small and uploads as it is."
          : "The audio files here are already small and upload as they are."}
      </p>
    );
  }
  const audioFiles = estimate.files - estimate.videoFiles;
  const what = [
    audioFiles > 0 ? plural(audioFiles, "audio file") : null,
    estimate.videoFiles > 0 ? plural(estimate.videoFiles, "video") : null,
  ]
    .filter(Boolean)
    .join(" and ");
  return (
    <div className="review-compression">
      <Switch
        label={estimate.videoFiles > 0 ? "Compress audio and video before upload" : "Compress audio before upload"}
        description={`${what}. Estimated ${formatBytes(estimate.bytesBefore)} to ${formatBytes(estimate.bytesAfter)}.${
          estimate.unchanged > 0 ? ` ${plural(estimate.unchanged, "other audio file")} already small, uploaded as is.` : ""
        }`}
        checked={compress}
        onChange={onChange}
      />
      {compress && !estimate.toolsReady && (
        <InlineResult tone="warning">
          Compression tools are not installed, so files upload unchanged.{" "}
          <button type="button" className="link-button" onClick={onOpenSettings}>
            Open Compression settings
          </button>
        </InlineResult>
      )}
      {compress && estimate.keyChangeCount > 0 && (
        <>
          <button type="button" className="disclosure" aria-expanded={showKeys} onClick={() => setShowKeys((v) => !v)}>
            <ChevronRight size={14} aria-hidden="true" />
            {estimate.keyChangeCount === 1 ? "1 file gets a new extension" : `${estimate.keyChangeCount.toLocaleString("en-US")} files get a new extension`}
          </button>
          {showKeys && (
            <ul className="skipped-list">
              {estimate.keyChanges.map((k) => (
                <li key={k.from}>
                  <span className="mono">{k.from}</span>
                  <span className="secondary"> becomes </span>
                  <span className="mono">{k.to}</span>
                </li>
              ))}
              {estimate.keyChangeCount > estimate.keyChanges.length && (
                <li className="secondary">and {(estimate.keyChangeCount - estimate.keyChanges.length).toLocaleString("en-US")} more</li>
              )}
            </ul>
          )}
        </>
      )}
      {compress && estimate.collisions.length > 0 && (
        <InlineResult tone="warning">
          {estimate.collisions.length === 1 ? "One name is" : `${estimate.collisions.length} names are`} used by more than one file after
          compression ({estimate.collisions.slice(0, 3).join(", ")}). The choice for existing files decides what happens.
        </InlineResult>
      )}
    </div>
  );
}

/** Upload review (Screen 4, step 2). */
export function UploadReview({ plan, target, onCancel, onStart }: UploadReviewProps) {
  const [policy, setPolicy] = useState<ConflictPolicy>(plan.defaultPolicy);
  const [showSkipped, setShowSkipped] = useState(false);
  const [busy, setBusy] = useState(false);
  const [compress, setCompress] = useState(true);
  const openSettings = useNavStore((s) => s.openSettings);
  useEffect(() => setPolicy(plan.defaultPolicy), [plan.planId, plan.defaultPolicy]);

  const items = plan.files + plan.folders;
  const destination = [target.bucket, ...splitPrefix(target.prefix).map((c) => c.label)].join(" / ");
  const summary = [
    plan.folders > 0 ? plural(plan.folders, "folder") : null,
    plural(plan.files, "file"),
    formatBytes(plan.bytes),
  ]
    .filter(Boolean)
    .join(", ");

  const start = async () => {
    setBusy(true);
    try {
      await onStart(policy, compress);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      title={`Upload ${plural(items, "item")}`}
      width="review"
      footer={
        <>
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button variant="primary" onClick={() => void start()} loading={busy} disabled={plan.files === 0}>
            Upload
          </Button>
        </>
      }
    >
      <p className="review-dest">
        <span className="secondary">To: </span>
        <span className="mono">{destination}</span>
      </p>
      <p className="num">{summary}</p>

      {plan.compression && (
        <CompressionBox
          estimate={plan.compression}
          compress={compress}
          onChange={setCompress}
          onOpenSettings={() => {
            onCancel();
            openSettings("compression");
          }}
        />
      )}

      {plan.tooMany && (
        <InlineResult tone="warning">
          This is a very large upload. It can take a long time and use many R2 operations.
        </InlineResult>
      )}

      {plan.conflicts > 0 && (
        <fieldset className="radio-list review-conflicts">
          <legend className="review-legend">
            {plan.conflicts === 1 ? "1 file already exists in this folder." : `${plan.conflicts.toLocaleString("en-US")} files already exist in this folder.`}
          </legend>
          {POLICIES.map((p) => (
            <label key={p.value} className="radio-option">
              <input
                type="radio"
                name="conflict-policy"
                value={p.value}
                checked={policy === p.value}
                onChange={() => setPolicy(p.value)}
              />
              {p.label}
            </label>
          ))}
        </fieldset>
      )}

      {plan.skipped.length > 0 && (
        <div className="review-skipped">
          <p className="review-skipped-title">
            <TriangleAlert size={14} aria-hidden="true" />
            {plan.skipped.length === 1 ? "1 item will be skipped." : `${plan.skipped.length.toLocaleString("en-US")} items will be skipped.`}
          </p>
          <button type="button" className="disclosure" aria-expanded={showSkipped} onClick={() => setShowSkipped((v) => !v)}>
            <ChevronRight size={14} aria-hidden="true" />
            {showSkipped ? "Hide list" : "Show list"}
          </button>
          {showSkipped && (
            <ul className="skipped-list">
              {plan.skipped.map((s, i) => (
                <li key={`${s.path}-${i}`}>
                  <span className="mono">{s.path}</span>
                  <span className="secondary"> {s.reason}</span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </Dialog>
  );
}

/** Global host for the review dialog. */
export function UploadReviewDialog() {
  const review = useUploadStore((s) => s.review);
  const setReview = useUploadStore((s) => s.setReview);
  if (!review) return null;
  return (
    <UploadReview
      key={review.plan.planId}
      plan={review.plan}
      target={review.target}
      onCancel={() => setReview(null)}
      onStart={async (policy, compress) => {
        const ok = await startPlan(review.plan, review.target, policy, compress);
        if (ok) setReview(null);
      }}
    />
  );
}
