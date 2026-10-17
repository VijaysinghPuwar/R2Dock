import type { Platform } from "../ipc/bindings/Platform";

/** Best guess before appInfo() answers. */
export function guessPlatform(): Platform {
  const ua = typeof navigator === "undefined" ? "" : navigator.userAgent;
  if (/Mac/i.test(ua)) return "macos";
  if (/Win/i.test(ua)) return "windows";
  return "linux";
}

export function modKeyLabel(platform: Platform): string {
  return platform === "macos" ? "Cmd" : "Ctrl";
}

/** True when the platform modifier (Cmd on macOS, Ctrl elsewhere) is held. */
export function hasMod(e: { metaKey: boolean; ctrlKey: boolean }, platform: Platform): boolean {
  return platform === "macos" ? e.metaKey : e.ctrlKey;
}

/** True when keyboard input belongs to a text field. */
export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === "TEXTAREA" || tag === "SELECT") return true;
  if (tag === "INPUT") {
    const type = (target as HTMLInputElement).type;
    return !["checkbox", "radio", "button", "submit", "range"].includes(type);
  }
  return false;
}
