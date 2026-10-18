import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CompressionView } from "../../ipc/bindings/CompressionView";
import { createCompressionNotices, NOTICE_BATCH_MS } from "./compressionNotice";
import { unchangedText } from "./compressionText";
import { makeJob } from "./testJobs";

const alreadyCompressed: CompressionView = {
  kind: "unchanged",
  reason: { kind: "plan", reason: { reason: "alreadyCompressed", kbps: 128 } },
};

function setup() {
  const shown: string[] = [];
  const notices = createCompressionNotices((m) => shown.push(m));
  return { shown, notices };
}

describe("compression notices", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("tells why a single file goes up as the original", () => {
    const { shown, notices } = setup();
    notices.note(makeJob({ id: "a", name: "intro.wav", state: "running", compression: alreadyCompressed }));
    expect(shown).toEqual([]);
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown).toEqual(["intro.wav can't be compressed: Already compressed (128 kbps). Uploading the original."]);
  });

  it("reuses the reason text shown in the transfer row", () => {
    const { shown, notices } = setup();
    const view: CompressionView = { kind: "unchanged", reason: { kind: "toolsMissing" } };
    notices.note(makeJob({ id: "a", name: "intro.wav", compression: view }));
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown[0]).toContain(unchangedText({ kind: "toolsMissing" }));
  });

  it("tells when compression failed", () => {
    const { shown, notices } = setup();
    notices.note(makeJob({ id: "a", name: "intro.wav", compression: { kind: "failed", message: "encoder stopped" } }));
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown).toEqual(["Couldn't compress intro.wav. Uploading the original."]);
  });

  it("says the file was skipped when the Skip the file setting stopped it", () => {
    const { shown, notices } = setup();
    const job = makeJob({ id: "a", name: "intro.wav", state: "compressing", compression: { kind: "failed", message: "x" } });
    notices.note(job);
    notices.note({ ...job, state: "failed" });
    notices.note(makeJob({ id: "b", name: "b.wav", state: "failed", compression: { kind: "failed", message: "x" } }));
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown).toEqual(["2 files couldn't be compressed, so they were skipped as set in Settings > Compression."]);
    notices.note(makeJob({ id: "c", name: "c.wav", state: "failed", compression: { kind: "failed", message: "x" } }));
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown[1]).toBe("Couldn't compress c.wav, so it was skipped as set in Settings > Compression.");
  });

  it("shows one toast for a batch of files", () => {
    const { shown, notices } = setup();
    notices.note(makeJob({ id: "a", name: "a.wav", compression: alreadyCompressed }));
    vi.advanceTimersByTime(300);
    notices.note(makeJob({ id: "b", name: "b.wav", compression: { kind: "failed", message: "x" } }));
    notices.note(makeJob({ id: "c", name: "c.wav", compression: alreadyCompressed }));
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown).toEqual(["3 files can't be compressed. Uploading the originals."]);
  });

  it("says nothing when compression does not apply or is off", () => {
    const { shown, notices } = setup();
    notices.note(makeJob({ id: "a", name: "cover.webp", compression: { kind: "unchanged", reason: { kind: "notAudio" } } }));
    notices.note(makeJob({ id: "b", name: "clip.mp4", compression: { kind: "unchanged", reason: { kind: "plan", reason: { reason: "videoOff" } } } }));
    notices.note(makeJob({ id: "c", name: "intro.wav", compression: null }));
    notices.note(makeJob({ id: "d", name: "intro.wav", compression: { kind: "pending", fraction: 0.4 } }));
    notices.note(makeJob({ id: "e", name: "intro.wav", compression: { kind: "saved", originalBytes: 10, bytes: 5, preset: "speech" } }));
    notices.note(makeJob({ id: "f", direction: "download", compression: alreadyCompressed }));
    vi.advanceTimersByTime(NOTICE_BATCH_MS * 2);
    expect(shown).toEqual([]);
  });

  it("tells once per job across progress events and retries", () => {
    const { shown, notices } = setup();
    const job = makeJob({ id: "a", name: "intro.wav", state: "running", compression: alreadyCompressed });
    notices.note(job);
    notices.note({ ...job, transferredBytes: 500 });
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    notices.note({ ...job, state: "failed" });
    notices.note({ ...job, state: "running", attempt: 2 });
    notices.note({ ...job, state: "completed" });
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown).toHaveLength(1);
  });

  it("does not repeat jobs restored from an earlier session", () => {
    const { shown, notices } = setup();
    const job = makeJob({ id: "a", name: "intro.wav", compression: alreadyCompressed });
    notices.remember(job);
    notices.note({ ...job, state: "completed" });
    vi.advanceTimersByTime(NOTICE_BATCH_MS);
    expect(shown).toEqual([]);
  });
});
