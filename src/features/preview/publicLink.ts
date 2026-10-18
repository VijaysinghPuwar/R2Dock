// Public links are resolved in Rust (manual public URL, then custom domain,
// then r2.dev). This module only holds the labels and hint text.

import type { ConnectionSummary } from "../../ipc/bindings/ConnectionSummary";
import type { PublicLinkSource } from "../../ipc/bindings/PublicLinkSource";
import type { PublicLinkStatus } from "../../ipc/bindings/PublicLinkStatus";

export const SOURCE_LABEL: Record<PublicLinkSource, string> = {
  publicUrl: "Public URL",
  customDomain: "Custom domain",
  r2Dev: "r2.dev",
};

/** Whether a public link can exist without asking Rust: a manual URL or a saved API token. */
export function mayHavePublicLink(connection: ConnectionSummary, bucket: string): boolean {
  return Boolean(connection.publicBaseUrls[bucket]) || connection.hasApiToken;
}

/** A calm hint when no link is shown. It always ends with "in", followed by a Bucket settings link. */
export function publicLinkHint(status: PublicLinkStatus): string {
  switch (status) {
    case "noDomain":
      return "This bucket has no public custom domain or r2.dev access. You can set a public URL in";
    case "tokenDenied":
      return "The saved API token cannot read this bucket's domains (an Admin Read token can). You can also set a public URL in";
    case "tokenMissing":
      return "The saved API token was removed outside R2Dock. Edit the connection to add it again, or set a public URL in";
    case "unavailable":
      return "Could not check this bucket's custom domains right now. Try again later, or set a public URL in";
    case "notConfigured":
    case "found":
      return "No public URL configured. Add an API token to the connection to find custom domains, or set one in";
  }
}
