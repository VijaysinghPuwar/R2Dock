import { describe, expect, it } from "vitest";
import { aggregate, countTabs, tabOf } from "./stats";
import { makeJob } from "./testJobs";
import { buildItems, groupText } from "./groups";

describe("aggregate", () => {
  it("joins parts with a middle dot and includes speed and eta when known", () => {
    const jobs = [
      makeJob({ id: "1", state: "running", totalBytes: 1000, transferredBytes: 420, speedBps: 6_100_000, etaSecs: 120 }),
      makeJob({ id: "2", state: "queued", totalBytes: 0 }),
      makeJob({ id: "3", state: "queued", totalBytes: 0 }),
    ];
    const agg = aggregate(jobs);
    expect(agg.text).toBe("Uploading 3 files · 42% · 6.1 MB/s · about 2 min left");
  });

  it("omits speed and eta when null", () => {
    const agg = aggregate([makeJob({ state: "running", totalBytes: 100, transferredBytes: 50 })]);
    expect(agg.text).toBe("Uploading 1 file · 50%");
  });

  it("says Downloading for downloads and reports failures", () => {
    const agg = aggregate([
      makeJob({ id: "1", direction: "download", state: "running", totalBytes: 10, transferredBytes: 1 }),
      makeJob({ id: "2", state: "failed" }),
    ]);
    expect(agg.text.startsWith("Downloading 1 file")).toBe(true);
    expect(agg.text).toContain("1 failed");
  });

  it("summarizes failures when nothing is moving", () => {
    const agg = aggregate([makeJob({ id: "1", state: "failed" }), makeJob({ id: "2", state: "completed" })]);
    expect(agg.text).toBe("1 failed · 1 completed");
    expect(agg.allDone).toBe(false);
  });

  it("reports all complete", () => {
    const agg = aggregate([makeJob({ state: "completed" })]);
    expect(agg.text).toBe("All transfers complete");
    expect(agg.allDone).toBe(true);
  });
});

describe("tabs and groups", () => {
  it("maps states to tabs", () => {
    expect(tabOf("interrupted")).toBe("active");
    expect(tabOf("skipped")).toBe("completed");
    expect(countTabs([makeJob({ state: "failed" }), makeJob({ state: "queued" })])).toEqual({
      active: 0,
      queued: 1,
      completed: 0,
      failed: 1,
    });
  });

  it("groups jobs that share a group and counts finished children", () => {
    const group = { id: "g", label: "ch 1-195" };
    const jobs = [
      makeJob({ id: "1", group, state: "completed" }),
      makeJob({ id: "2", group, state: "running" }),
      makeJob({ id: "3", state: "running" }),
    ];
    const items = buildItems(jobs, "active", new Set());
    expect(items.map((i) => i.type)).toEqual(["group", "job"]);
    expect(groupText(group, jobs.slice(0, 2))).toBe("Uploading folder ch 1-195 · 1 of 2 files");
    const open = buildItems(jobs, "active", new Set(["g"]));
    expect(open.map((i) => i.type)).toEqual(["group", "job", "job"]);
  });
});
