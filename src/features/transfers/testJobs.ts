import type { JobSnapshot } from "../../ipc/bindings/JobSnapshot";

/** Build a job snapshot for tests. */
export function makeJob(overrides: Partial<JobSnapshot> = {}): JobSnapshot {
  return {
    id: "job-1",
    group: null,
    direction: "upload",
    connectionId: "conn-1",
    bucket: "media",
    key: "covers/cover.webp",
    name: "cover.webp",
    localDisplay: "~/Pictures/cover.webp",
    totalBytes: 1_000_000,
    transferredBytes: 0,
    state: "queued",
    speedBps: null,
    etaSecs: null,
    attempt: 1,
    maxAttempts: 3,
    partsDone: null,
    partsTotal: null,
    pausable: false,
    error: null,
    createdAt: "2026-10-17T10:00:00Z",
    finishedAt: null,
    ...overrides,
  };
}
