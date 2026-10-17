import { describe, expect, it } from "vitest";
import { INITIAL_TRANSFER_STATE, orderedJobs, reduceTransfers } from "./transferStore";
import { makeJob } from "./testJobs";

const a = makeJob({ id: "a", name: "a.txt" });
const b = makeJob({ id: "b", name: "b.txt" });
const c = makeJob({ id: "c", name: "c.txt" });

describe("reduceTransfers", () => {
  it("starts empty and unseen", () => {
    expect(INITIAL_TRANSFER_STATE.order).toEqual([]);
    expect(INITIAL_TRANSFER_STATE.seen).toBe(false);
  });

  it("snapshot replaces all jobs and marks seen", () => {
    const s1 = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "added", jobs: [a] });
    const s2 = reduceTransfers(s1, { type: "snapshot", jobs: [c, b] });
    expect(s2.order).toEqual(["c", "b"]);
    expect(s2.jobs.a).toBeUndefined();
    expect(s2.seen).toBe(true);
  });

  it("empty snapshot does not mark seen", () => {
    const s = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "snapshot", jobs: [] });
    expect(s.seen).toBe(false);
  });

  it("added appends in insertion order and updates existing ids in place", () => {
    let s = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "added", jobs: [a, b] });
    s = reduceTransfers(s, { type: "added", jobs: [c, { ...a, state: "running" }] });
    expect(s.order).toEqual(["a", "b", "c"]);
    expect(s.jobs.a?.state).toBe("running");
    expect(orderedJobs(s).map((j) => j.id)).toEqual(["a", "b", "c"]);
  });

  it("updated replaces a job or appends a new one", () => {
    let s = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "added", jobs: [a, b] });
    s = reduceTransfers(s, { type: "updated", job: { ...a, state: "completed" } });
    expect(s.jobs.a?.state).toBe("completed");
    expect(s.order).toEqual(["a", "b"]);
    s = reduceTransfers(s, { type: "updated", job: c });
    expect(s.order).toEqual(["a", "b", "c"]);
  });

  it("progress updates byte counts and ignores unknown ids", () => {
    const s1 = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "added", jobs: [a] });
    const s2 = reduceTransfers(s1, {
      type: "progress",
      updates: [
        { id: "a", transferredBytes: 500, speedBps: 100, etaSecs: 5, partsDone: 2 },
        { id: "zzz", transferredBytes: 1, speedBps: null, etaSecs: null, partsDone: null },
      ],
    });
    expect(s2.jobs.a).toMatchObject({ transferredBytes: 500, speedBps: 100, etaSecs: 5, partsDone: 2 });
    expect(s2.jobs.zzz).toBeUndefined();
    expect(s2.order).toEqual(["a"]);
    const s3 = reduceTransfers(s2, {
      type: "progress",
      updates: [{ id: "nope", transferredBytes: 1, speedBps: null, etaSecs: null, partsDone: null }],
    });
    expect(s3).toBe(s2);
  });

  it("removed drops jobs and keeps the order of the rest", () => {
    let s = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "added", jobs: [a, b, c] });
    s = reduceTransfers(s, { type: "removed", ids: ["b"] });
    expect(s.order).toEqual(["a", "c"]);
    expect(s.jobs.b).toBeUndefined();
    expect(s.seen).toBe(true);
  });

  it("authFailed records the connection with a counter", () => {
    const s1 = reduceTransfers(INITIAL_TRANSFER_STATE, { type: "authFailed", connectionId: "conn-1" });
    expect(s1.authFailed).toEqual({ connectionId: "conn-1", count: 1 });
    const s2 = reduceTransfers(s1, { type: "authFailed", connectionId: "conn-2" });
    expect(s2.authFailed).toEqual({ connectionId: "conn-2", count: 2 });
  });
});
