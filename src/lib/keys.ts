// Key and prefix helpers. Display only: Rust validates every key that matters.

// Matching control characters is the point here.
// eslint-disable-next-line no-control-regex
const HIDDEN_CHARS = /[\u0000-\u001f\u007f-\u009f\u061c\u200b-\u200f\u2028-\u202e\u2060-\u2069\ufeff]/g;

/** Show control, bidi, and zero-width characters as visible placeholders like \u27e8U+202E\u27e9. */
export function visibleText(text: string): string {
  return text.replace(HIDDEN_CHARS, (ch) => {
    const code = ch.codePointAt(0) ?? 0;
    return `\u27e8U+${code.toString(16).toUpperCase().padStart(4, "0")}\u27e9`;
  });
}

/** Label for an empty key segment, as in "a//b". */
export const EMPTY_SEGMENT = "(empty)";

/** Parent prefix of a key or prefix: "a/b/c.txt" -> "a/b/", "a/b/" -> "a/", "a/" -> "". */
export function parentPrefix(keyOrPrefix: string): string {
  const trimmed = keyOrPrefix.endsWith("/") ? keyOrPrefix.slice(0, -1) : keyOrPrefix;
  const idx = trimmed.lastIndexOf("/");
  return idx < 0 ? "" : trimmed.slice(0, idx + 1);
}

/** Last name segment of a key or prefix, without the trailing slash. */
export function baseName(keyOrPrefix: string): string {
  const trimmed = keyOrPrefix.endsWith("/") ? keyOrPrefix.slice(0, -1) : keyOrPrefix;
  const idx = trimmed.lastIndexOf("/");
  return idx < 0 ? trimmed : trimmed.slice(idx + 1);
}

export interface Crumb {
  /** Visible label (hidden characters shown, empty segments marked). */
  label: string;
  /** Prefix to navigate to, with trailing slash. */
  prefix: string;
  empty: boolean;
}

/** Split a prefix into breadcrumb segments: "a//b/" -> a, (empty), b. */
export function splitPrefix(prefix: string): Crumb[] {
  if (!prefix) return [];
  const body = prefix.endsWith("/") ? prefix.slice(0, -1) : prefix;
  const parts = body.split("/");
  const crumbs: Crumb[] = [];
  let acc = "";
  for (const part of parts) {
    acc += `${part}/`;
    crumbs.push({ label: part === "" ? EMPTY_SEGMENT : visibleText(part), prefix: acc, empty: part === "" });
  }
  return crumbs;
}

/** Name of the folder a prefix points to, or the bucket for the root. */
export function folderLabel(bucket: string, prefix: string): string {
  if (!prefix) return bucket;
  const name = baseName(prefix);
  return name === "" ? EMPTY_SEGMENT : visibleText(name);
}

/** Lowercase extension without the dot, or "". */
export function extension(name: string): string {
  const idx = name.lastIndexOf(".");
  if (idx <= 0 || idx === name.length - 1) return "";
  return name.slice(idx + 1).toLowerCase();
}

/** Split text around the first case-insensitive match, for highlighting. */
export function splitMatch(text: string, query: string): [string, string, string] | null {
  if (!query) return null;
  const idx = text.toLowerCase().indexOf(query.toLowerCase());
  if (idx < 0) return null;
  return [text.slice(0, idx), text.slice(idx, idx + query.length), text.slice(idx + query.length)];
}
