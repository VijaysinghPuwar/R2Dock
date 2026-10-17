import { activityNote } from "../ipc/commands";

/** Shorten a message and drop anything that looks like a URL query. */
export function cleanMessage(value: unknown): string {
  const text = value instanceof Error ? `${value.name}: ${value.message}` : String(value);
  return text.replace(/\?[^\s]*/g, "?…").slice(0, 400);
}

/** Send uncaught interface errors to the activity log. Call once at startup. */
export function installProblemReporter(target: Window = window): void {
  target.addEventListener("error", (event) => {
    void activityNote("ui_error", cleanMessage(event.error ?? event.message));
  });
  target.addEventListener("unhandledrejection", (event) => {
    void activityNote("ui_unhandled_rejection", cleanMessage(event.reason));
  });
}
