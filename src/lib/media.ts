import { convertFileSrc } from "@tauri-apps/api/core";

/** URL for an image or audio preview served by the Rust media scheme. */
export function mediaUrl(token: string): string {
  return convertFileSrc(token, "r2dock-media");
}
