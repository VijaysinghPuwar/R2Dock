import { TriangleAlert } from "lucide-react";
import { FileIcon } from "../../components/FileIcon";
import { Tooltip } from "../../components/Tooltip";
import { splitMatch, parentPrefix, visibleText } from "../../lib/keys";

const TAIL = 12;

/** Name text with the end kept visible (middle truncation), or a highlighted match. */
export function NameText({ text, highlight }: { text: string; highlight?: string }) {
  const parts = highlight ? splitMatch(text, highlight.trim()) : null;
  if (parts) {
    return (
      <span className="name-text truncate" title={text}>
        {parts[0]}
        <mark className="hl">{parts[1]}</mark>
        {parts[2]}
      </span>
    );
  }
  if (text.length <= TAIL + 4) {
    return (
      <span className="name-text truncate" title={text}>
        {text}
      </span>
    );
  }
  return (
    <span className="name-text name-split" title={text}>
      <span className="name-head">{text.slice(0, -TAIL)}</span>
      <span className="name-tail">{text.slice(-TAIL)}</span>
    </span>
  );
}

export function UnusualBadge() {
  return (
    <Tooltip content="Unusual key name. It may contain hidden characters, empty segments, or extra slashes.">
      <span className="unusual-badge" tabIndex={-1} role="img" aria-label="Unusual key name">
        <TriangleAlert size={12} aria-hidden="true" />
      </span>
    </Tooltip>
  );
}

export interface NameCellProps {
  name: string;
  displayName: string;
  folder: boolean;
  unusual: boolean;
  highlight?: string;
  /** Full key, shown as a muted path for whole-bucket search results. */
  pathKey?: string;
}

export function NameCell({ name, displayName, folder, unusual, highlight, pathKey }: NameCellProps) {
  const path = pathKey ? parentPrefix(pathKey) : "";
  return (
    <>
      <FileIcon name={name} folder={folder} />
      <NameText text={displayName} highlight={highlight} />
      {unusual && <UnusualBadge />}
      {path && <span className="name-path mono truncate">{visibleText(path)}</span>}
    </>
  );
}
