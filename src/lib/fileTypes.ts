import { extension } from "./keys";

export type FileKind = "folder" | "image" | "audio" | "video" | "document" | "archive" | "code" | "unknown";
export type PreviewKind = "image" | "text" | "audio" | "video" | "none";

const IMAGE = new Set(["png", "jpg", "jpeg", "webp", "gif", "avif", "svg", "bmp", "ico", "tif", "tiff", "heic"]);
const AUDIO = new Set(["mp3", "m4a", "aac", "ogg", "oga", "wav", "flac", "opus", "aiff", "wma"]);
const VIDEO = new Set(["mp4", "m4v", "mov", "webm", "mkv", "avi", "wmv"]);
const ARCHIVE = new Set(["zip", "gz", "tgz", "tar", "7z", "rar", "bz2", "xz", "zst", "dmg", "iso"]);
const CODE = new Set([
  "js", "mjs", "cjs", "ts", "tsx", "jsx", "json", "css", "scss", "html", "htm", "xml", "yaml", "yml", "toml",
  "rs", "py", "go", "java", "rb", "php", "sh", "sql", "c", "h", "cpp", "swift", "kt", "ini", "conf", "env",
]);
const DOCUMENT = new Set(["txt", "md", "markdown", "csv", "tsv", "log", "pdf", "doc", "docx", "rtf", "odt", "xls", "xlsx", "ppt", "pptx", "srt", "vtt"]);

/** Images previewed through the media scheme. */
export const PREVIEW_IMAGE = new Set(["png", "jpg", "jpeg", "webp", "gif", "avif", "svg"]);
/** Audio previewed with the compact player. */
export const PREVIEW_AUDIO = new Set(["mp3", "m4a", "aac", "ogg", "oga", "wav", "flac", "opus"]);
/** Text and code shown as plain text. */
export const PREVIEW_TEXT = new Set([
  "txt", "md", "markdown", "json", "csv", "tsv", "log", "xml", "yaml", "yml", "toml", "ini", "conf",
  "js", "mjs", "cjs", "ts", "tsx", "jsx", "css", "scss", "html", "htm", "rs", "py", "go", "java", "rb",
  "php", "sh", "sql", "c", "h", "cpp", "swift", "kt", "srt", "vtt", "env",
]);

/** Programs that can run when opened. */
export const EXECUTABLE = new Set(["exe", "msi", "app", "sh", "bat", "cmd", "ps1", "command", "scr", "com", "jar", "vbs"]);

export const IMAGE_PREVIEW_LIMIT = 25 * 1024 * 1024;
export const THUMBNAIL_LIMIT = 10 * 1024 * 1024;

export function fileKind(name: string): FileKind {
  const ext = extension(name);
  if (IMAGE.has(ext)) return "image";
  if (AUDIO.has(ext)) return "audio";
  if (VIDEO.has(ext)) return "video";
  if (ARCHIVE.has(ext)) return "archive";
  if (CODE.has(ext)) return "code";
  if (DOCUMENT.has(ext)) return "document";
  return "unknown";
}

export function previewKind(name: string): PreviewKind {
  const ext = extension(name);
  if (PREVIEW_IMAGE.has(ext)) return "image";
  if (PREVIEW_AUDIO.has(ext)) return "audio";
  if (PREVIEW_TEXT.has(ext)) return "text";
  if (VIDEO.has(ext)) return "video";
  return "none";
}

const LABELS: Record<string, string> = {
  png: "PNG image", jpg: "JPEG image", jpeg: "JPEG image", webp: "WebP image", gif: "GIF image",
  avif: "AVIF image", svg: "SVG image", bmp: "BMP image", ico: "Icon", tif: "TIFF image", tiff: "TIFF image",
  heic: "HEIC image", mp3: "MP3 audio", m4a: "M4A audio", aac: "AAC audio", ogg: "Ogg audio", oga: "Ogg audio",
  wav: "WAV audio", flac: "FLAC audio", opus: "Opus audio", aiff: "AIFF audio", wma: "WMA audio",
  mp4: "MP4 video", m4v: "M4V video", mov: "QuickTime video", webm: "WebM video", mkv: "MKV video",
  avi: "AVI video", wmv: "WMV video", zip: "ZIP archive", gz: "Gzip archive", tgz: "Gzip archive",
  tar: "TAR archive", "7z": "7z archive", rar: "RAR archive", bz2: "Bzip2 archive", xz: "XZ archive",
  zst: "Zstd archive", dmg: "Disk image", iso: "Disk image", txt: "Text", md: "Markdown", markdown: "Markdown",
  csv: "CSV", tsv: "TSV", log: "Log", pdf: "PDF", doc: "Word document", docx: "Word document", rtf: "Rich text",
  odt: "Document", xls: "Spreadsheet", xlsx: "Spreadsheet", ppt: "Presentation", pptx: "Presentation",
  srt: "Subtitles", vtt: "Subtitles", js: "JavaScript", mjs: "JavaScript", cjs: "JavaScript", ts: "TypeScript",
  tsx: "TypeScript", jsx: "JavaScript", json: "JSON", css: "CSS", scss: "SCSS", html: "HTML", htm: "HTML",
  xml: "XML", yaml: "YAML", yml: "YAML", toml: "TOML", rs: "Rust", py: "Python", go: "Go", java: "Java",
  rb: "Ruby", php: "PHP", sh: "Shell script", sql: "SQL", c: "C", h: "C header", cpp: "C++", swift: "Swift",
  kt: "Kotlin", ini: "Config", conf: "Config", env: "Config", exe: "Program", msi: "Installer", app: "Application",
  bat: "Batch file", ps1: "PowerShell", wasm: "WebAssembly",
};

/** Human type: "WebP image", "MP3 audio", "Unknown". */
export function humanType(name: string): string {
  const ext = extension(name);
  if (!ext) return "Unknown";
  return LABELS[ext] ?? `${ext.toUpperCase()} file`;
}
