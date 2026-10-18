import type { ConnectionSummary } from "../ipc/bindings/ConnectionSummary";

/** Build a connection summary for tests. Fictional data only. */
export function makeConnection(overrides: Partial<ConnectionSummary> = {}): ConnectionSummary {
  return {
    id: "conn-1",
    name: "Studio",
    accountId: "0000000000000000000000000000abcd",
    accountIdShort: "0000...abcd",
    accessKeyHint: "...wxyz",
    jurisdiction: "default",
    secretStorage: "keychain",
    status: "online",
    bucketScoped: false,
    manualBuckets: [],
    favoriteBuckets: [],
    publicBaseUrls: {},
    hasApiToken: false,
    lastTestedAt: null,
    active: false,
    ...overrides,
  };
}
