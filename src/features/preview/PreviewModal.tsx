import * as RadixDialog from "@radix-ui/react-dialog";
import { ChevronLeft, ChevronRight, Download, PanelRight, X } from "lucide-react";
import { useCallback, type KeyboardEvent } from "react";
import { IconButton } from "../../components/Button";
import { CopyButton } from "../../components/Feedback";
import { useUiStore } from "../../app/uiStore";
import { previewKind } from "../../lib/fileTypes";
import { DOT, formatBytes } from "../../lib/format";
import { visibleText } from "../../lib/keys";
import { useBrowserStore } from "../browser/browserStore";
import { startDownload } from "../transfers/download";
import { AudioBody, ImageBody, NoPreviewBody, TextBody, VideoBody, type BodyProps } from "./PreviewBodies";
import { usePreviewStore } from "./previewStore";
import "./preview.css";

function Body(props: BodyProps) {
  switch (previewKind(props.entry.name)) {
    case "image":
      return <ImageBody {...props} />;
    case "text":
      return <TextBody {...props} />;
    case "audio":
      return <AudioBody {...props} />;
    case "video":
      return <VideoBody {...props} />;
    case "none":
      return <NoPreviewBody {...props} />;
  }
}

/** Large preview of one file, with previous and next through the folder. */
export function PreviewModal() {
  const { open, connectionId, bucket, items, index, setIndex, close } = usePreviewStore();
  const entry = items[index];

  const prev = useCallback(() => {
    if (index > 0) setIndex(index - 1);
  }, [index, setIndex]);
  const next = useCallback(() => {
    if (index < items.length - 1) setIndex(index + 1);
  }, [index, items.length, setIndex]);

  if (!open || !entry) return null;

  const download = () => void startDownload(connectionId, bucket, [entry.key], []);
  const openDetails = () => {
    useBrowserStore.getState().setSelected([entry.key], entry.key);
    useUiStore.getState().setDetailsOpen(true);
    close();
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement;
    if (target.tagName === "INPUT") return;
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      prev();
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      next();
    }
  };

  return (
    <RadixDialog.Root open={open} onOpenChange={(o) => (!o ? close() : undefined)}>
      <RadixDialog.Portal>
        <RadixDialog.Overlay className="dialog-overlay" />
        <RadixDialog.Content
          className="preview-modal"
          onKeyDown={onKeyDown}
          tabIndex={-1}
          onOpenAutoFocus={(e) => {
            // Focus the dialog itself so a header button tooltip does not pop up.
            e.preventDefault();
            (e.currentTarget as HTMLElement | null)?.focus();
          }}
        >
          <header className="preview-header">
            <div className="preview-heading">
              <RadixDialog.Title className="preview-title truncate">{entry.displayName}</RadixDialog.Title>
              <RadixDialog.Description className="preview-meta small secondary num">
                {formatBytes(entry.size)}
                {items.length > 1 ? ` ${DOT} ${index + 1} of ${items.length}` : ""}
              </RadixDialog.Description>
            </div>
            <div className="preview-actions">
              <IconButton label="Download" icon={<Download size={16} />} onClick={download} />
              <CopyButton text={entry.key} label="Copy key" size="md" />
              <IconButton label="Open details" icon={<PanelRight size={16} />} onClick={openDetails} />
              <RadixDialog.Close asChild>
                <IconButton label="Close" icon={<X size={16} />} shortcut="Esc" />
              </RadixDialog.Close>
            </div>
          </header>
          <div className="preview-stage">
            <IconButton
              className="preview-nav preview-nav-prev"
              label="Previous file"
              icon={<ChevronLeft size={20} />}
              onClick={prev}
              disabled={index <= 0}
              tooltipSide="right"
            />
            <div className="preview-body" key={entry.key} aria-label={`Preview of ${visibleText(entry.name)}`}>
              <Body connectionId={connectionId} bucket={bucket} entry={entry} onDownload={download} />
            </div>
            <IconButton
              className="preview-nav preview-nav-next"
              label="Next file"
              icon={<ChevronRight size={20} />}
              onClick={next}
              disabled={index >= items.length - 1}
              tooltipSide="left"
            />
          </div>
        </RadixDialog.Content>
      </RadixDialog.Portal>
    </RadixDialog.Root>
  );
}
