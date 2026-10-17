import { useQuery } from "@tanstack/react-query";
import { CircleAlert, Download, FileQuestion, FileVideo, ImageOff } from "lucide-react";
import { useState, type WheelEvent } from "react";
import { Button } from "../../components/Button";
import { EmptyState } from "../../components/Feedback";
import { FileIcon } from "../../components/FileIcon";
import { Spinner } from "../../components/Spinner";
import { errorMessage, previewText } from "../../ipc/commands";
import type { ObjectEntry } from "../../ipc/bindings/ObjectEntry";
import { qk } from "../../app/queries";
import { IMAGE_PREVIEW_LIMIT } from "../../lib/fileTypes";
import { formatBytes } from "../../lib/format";
import { AudioPlayer } from "./AudioPlayer";
import { useMediaUrl } from "./useMediaUrl";
import "./preview.css";

export interface BodyProps {
  connectionId: string;
  bucket: string;
  entry: ObjectEntry;
  onDownload: () => void;
}

function Loading() {
  return (
    <div className="preview-center">
      <Spinner size={20} label="Loading preview" />
    </div>
  );
}

function ErrorBody({ error }: { error: unknown }) {
  return <EmptyState icon={<CircleAlert size={32} />} title="Could not load the preview" body={errorMessage(error)} />;
}

export function MediaImage({
  connectionId,
  bucket,
  entry,
  className,
  zoom = 1,
}: {
  connectionId: string;
  bucket: string;
  entry: ObjectEntry;
  className?: string;
  zoom?: number;
}) {
  const media = useMediaUrl(connectionId, bucket, entry.key);
  const [broken, setBroken] = useState(false);
  if (media.isError) return <ErrorBody error={media.error} />;
  if (!media.url) return <Loading />;
  if (broken) {
    return <EmptyState icon={<ImageOff size={32} />} title="Could not show this image" compact />;
  }
  return (
    <img
      className={className}
      src={media.url}
      alt={entry.displayName}
      draggable={false}
      onError={() => setBroken(true)}
      style={zoom !== 1 ? { transform: `scale(${zoom})` } : undefined}
    />
  );
}

export function ImageBody({ connectionId, bucket, entry }: BodyProps) {
  const [allowed, setAllowed] = useState(entry.size <= IMAGE_PREVIEW_LIMIT);
  const [zoom, setZoom] = useState(1);
  if (!allowed) {
    return (
      <EmptyState
        icon={<FileIcon name={entry.name} size={32} />}
        title={`Large image (${formatBytes(entry.size)}). Preview anyway?`}
        actions={
          <Button variant="primary" onClick={() => setAllowed(true)}>
            Preview anyway
          </Button>
        }
      />
    );
  }
  const onWheel = (e: WheelEvent<HTMLDivElement>) => {
    if (!(e.metaKey || e.ctrlKey)) return;
    e.preventDefault();
    setZoom((z) => Math.min(8, Math.max(0.25, z * (e.deltaY < 0 ? 1.1 : 1 / 1.1))));
  };
  return (
    <div className="preview-image-box" onWheel={onWheel} onDoubleClick={() => setZoom(1)}>
      <MediaImage connectionId={connectionId} bucket={bucket} entry={entry} className="preview-image" zoom={zoom} />
    </div>
  );
}

export function TextBody({ connectionId, bucket, entry }: BodyProps) {
  const query = useQuery({
    queryKey: qk.text(connectionId, bucket, entry.key),
    queryFn: () => previewText(connectionId, bucket, entry.key),
    staleTime: 60_000,
  });
  if (query.isError) return <ErrorBody error={query.error} />;
  if (!query.data) return <Loading />;
  if (query.data.binary) {
    return <EmptyState icon={<FileQuestion size={32} />} title="This file does not look like text" />;
  }
  return (
    <div className="preview-text-wrap">
      {query.data.truncated && <p className="preview-note small secondary">Showing first 1 MB</p>}
      <pre className="preview-text mono selectable">{query.data.text}</pre>
    </div>
  );
}

export function AudioBody({ connectionId, bucket, entry }: BodyProps) {
  const media = useMediaUrl(connectionId, bucket, entry.key);
  if (media.isError) return <ErrorBody error={media.error} />;
  if (!media.url) return <Loading />;
  return (
    <div className="preview-center preview-audio">
      <FileIcon name={entry.name} size={64} />
      <AudioPlayer src={media.url} title={entry.displayName} />
    </div>
  );
}

export function VideoBody({ onDownload }: BodyProps) {
  return (
    <EmptyState
      icon={<FileVideo size={32} />}
      title="Preview not available yet"
      actions={
        <Button icon={<Download size={16} />} onClick={onDownload}>
          Download
        </Button>
      }
    />
  );
}

export function NoPreviewBody({ entry, onDownload }: BodyProps) {
  return (
    <EmptyState
      icon={<FileIcon name={entry.name} size={64} />}
      title="No preview for this file type"
      actions={
        <Button variant="primary" icon={<Download size={16} />} onClick={onDownload}>
          Download
        </Button>
      }
    />
  );
}
