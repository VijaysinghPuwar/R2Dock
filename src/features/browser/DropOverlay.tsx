import { CloudUpload, Database } from "lucide-react";
import { folderLabel } from "../../lib/keys";
import { useNavStore } from "../../app/navStore";
import { useDragStore } from "./useDragDrop";
import "./browser.css";

/** Overlay shown over the content area while OS files are dragged over the window. */
export function DropOverlay() {
  const active = useDragStore((s) => s.active);
  const loc = useNavStore((s) => s.loc);
  if (!active) return null;
  const canDrop = loc.view === "browser" && !!loc.bucket;
  return (
    <div className="drop-overlay" aria-live="polite">
      {canDrop && loc.bucket ? (
        <div className="drop-zone">
          <CloudUpload size={40} aria-hidden="true" />
          <p className="drop-title">
            Drop to upload to <strong>{folderLabel(loc.bucket, loc.prefix)}</strong>
          </p>
          <p className="secondary">Files and folders. Folder structure is kept.</p>
        </div>
      ) : (
        <div className="drop-zone is-blocked">
          <Database size={40} aria-hidden="true" />
          <p className="drop-title">Open a bucket to upload files</p>
          <p className="secondary">Choose a bucket or folder first, then drop your files.</p>
        </div>
      )}
    </div>
  );
}
