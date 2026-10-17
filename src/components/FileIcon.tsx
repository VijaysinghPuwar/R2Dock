import { File, FileArchive, FileAudio, FileCode, FileImage, FileText, FileVideo, Folder } from "lucide-react";
import { fileKind, type FileKind } from "../lib/fileTypes";

const ICONS: Record<FileKind, typeof File> = {
  folder: Folder,
  image: FileImage,
  audio: FileAudio,
  video: FileVideo,
  document: FileText,
  archive: FileArchive,
  code: FileCode,
  unknown: File,
};

export function FileIcon({ name, folder, size = 16 }: { name: string; folder?: boolean; size?: number }) {
  const kind: FileKind = folder ? "folder" : fileKind(name);
  const Icon = ICONS[kind];
  return <Icon size={size} className={`file-icon tint-${kind}`} aria-hidden="true" strokeWidth={size > 24 ? 1.25 : 1.75} />;
}
