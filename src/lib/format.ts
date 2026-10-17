// Display formatting for sizes, dates, speeds, and durations.

const UNITS = ["KB", "MB", "GB", "TB", "PB", "EB"] as const;
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"] as const;

/** Decimal units: "412 KB", "18.4 MB", "3.1 KB", "512 bytes". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "--";
  if (bytes < 1000) return bytes === 1 ? "1 byte" : `${Math.round(bytes)} bytes`;
  let value = bytes / 1000;
  let unit = 0;
  for (;;) {
    const rounded = value < 100 ? Math.round(value * 10) / 10 : Math.round(value);
    if (rounded >= 1000 && unit < UNITS.length - 1) {
      value /= 1000;
      unit += 1;
      continue;
    }
    const text = value < 100 ? rounded.toFixed(1) : String(rounded);
    return `${text} ${UNITS[unit]}`;
  }
}

/** "19,293,184 bytes". */
export function formatExactBytes(bytes: number): string {
  return `${formatCount(bytes)} ${bytes === 1 ? "byte" : "bytes"}`;
}

/** "18.4 MB (19,293,184 bytes)". */
export function formatBytesLong(bytes: number): string {
  if (bytes < 1000) return formatExactBytes(bytes);
  return `${formatBytes(bytes)} (${formatExactBytes(bytes)})`;
}

export function formatCount(n: number): string {
  return Math.round(n).toLocaleString("en-US");
}

/** "1 file", "3 files". */
export function plural(n: number, one: string, many = `${one}s`): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`;
}

export function formatSpeed(bytesPerSecond: number): string {
  return `${formatBytes(bytesPerSecond)}/s`;
}

export function formatPercent(done: number, total: number): string {
  if (total <= 0) return "0%";
  const pct = Math.floor((done / total) * 100);
  return `${Math.min(100, Math.max(0, pct))}%`;
}

function parse(iso: string | null | undefined): Date | null {
  if (!iso) return null;
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? null : d;
}

function pad(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

function sameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

/** "Oct 2, 2026", or "Today, 14:32" for today. */
export function formatDate(iso: string | null | undefined, now: Date = new Date()): string {
  const d = parse(iso);
  if (!d) return "--";
  if (sameDay(d, now)) return `Today, ${pad(d.getHours())}:${pad(d.getMinutes())}`;
  return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}`;
}

/** "Sep 28, 2026, 09:14". */
export function formatDateTime(iso: string | null | undefined): string {
  const d = parse(iso);
  if (!d) return "--";
  return `${MONTHS[d.getMonth()]} ${d.getDate()}, ${d.getFullYear()}, ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** Local ISO 8601 timestamp with offset, for tooltips: "2026-10-02T14:32:05+02:00". */
export function formatIsoLocal(iso: string | null | undefined): string {
  const d = parse(iso);
  if (!d) return "";
  const offset = -d.getTimezoneOffset();
  const sign = offset >= 0 ? "+" : "-";
  const abs = Math.abs(offset);
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}` +
    `T${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}` +
    `${sign}${pad(Math.floor(abs / 60))}:${pad(abs % 60)}`
  );
}

/** "just now", "5 min ago", "3 h ago", "2 days ago". */
export function formatAgo(iso: string | null | undefined, now: Date = new Date()): string {
  const d = parse(iso);
  if (!d) return "";
  const secs = Math.max(0, Math.round((now.getTime() - d.getTime()) / 1000));
  if (secs < 60) return "just now";
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  const days = Math.floor(hours / 24);
  return days === 1 ? "1 day ago" : `${days} days ago`;
}

/** Short time left: "8 s left", "about 2 min left", "about 1 h 20 min left". */
export function formatEta(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} s left`;
  const mins = Math.round(s / 60);
  if (mins < 60) return `about ${mins} min left`;
  const hours = Math.floor(mins / 60);
  const rest = mins % 60;
  return rest === 0 ? `about ${hours} h left` : `about ${hours} h ${rest} min left`;
}

/** Time left in words for screen readers: "about 2 minutes remaining". */
export function formatEtaWords(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return s === 1 ? "about 1 second remaining" : `about ${s} seconds remaining`;
  const mins = Math.round(s / 60);
  if (mins < 60) return mins === 1 ? "about 1 minute remaining" : `about ${mins} minutes remaining`;
  const hours = Math.round(mins / 60);
  return hours === 1 ? "about 1 hour remaining" : `about ${hours} hours remaining`;
}

/** "1 min 12 s", "45 s", "2 h 3 min". */
export function formatDuration(secs: number): string {
  const s = Math.max(0, Math.round(secs));
  if (s < 60) return `${s} s`;
  const mins = Math.floor(s / 60);
  if (mins < 60) return s % 60 === 0 ? `${mins} min` : `${mins} min ${s % 60} s`;
  const hours = Math.floor(mins / 60);
  return mins % 60 === 0 ? `${hours} h` : `${hours} h ${mins % 60} min`;
}

/** Middle-dot joined summary, skipping empty parts. */
export function joinDot(parts: Array<string | null | undefined | false>): string {
  return parts.filter((p): p is string => typeof p === "string" && p.length > 0).join(` ${DOT} `);
}

/** "a1b2...9f0e" for long identifiers. */
export function shortId(id: string): string {
  if (id.length <= 12) return id;
  return `${id.slice(0, 4)}...${id.slice(-4)}`;
}

/** Truncate in the middle, keeping the extension visible. */
export function middleTruncate(text: string, max: number): string {
  if (text.length <= max) return text;
  const keepEnd = Math.min(Math.ceil(max * 0.4), 16);
  const keepStart = Math.max(1, max - keepEnd - 3);
  return `${text.slice(0, keepStart)}...${text.slice(-keepEnd)}`;
}

/** Middle dot used between summary parts. */
export const DOT = "·";
/** Ellipsis character for loading labels. */
export const ELLIPSIS = "...";
