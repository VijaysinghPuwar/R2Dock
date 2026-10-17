import {
  DOT,
  formatAgo,
  formatBytes,
  formatBytesLong,
  formatDate,
  formatDateTime,
  formatDuration,
  formatEta,
  formatEtaWords,
  formatExactBytes,
  formatIsoLocal,
  formatPercent,
  formatSpeed,
  joinDot,
  middleTruncate,
  plural,
  shortId,
} from "./format";

describe("formatBytes", () => {
  it.each([
    [0, "0 bytes"],
    [1, "1 byte"],
    [999, "999 bytes"],
    [1000, "1.0 KB"],
    [3100, "3.1 KB"],
    [412_000, "412 KB"],
    [18_400_000, "18.4 MB"],
    [999_999, "1.0 MB"],
    [2_400_000_000, "2.4 GB"],
    [5_000_000_000_000, "5.0 TB"],
  ])("%d -> %s", (input, expected) => {
    expect(formatBytes(input)).toBe(expected);
  });

  it("handles invalid input", () => {
    expect(formatBytes(-1)).toBe("--");
    expect(formatBytes(Number.NaN)).toBe("--");
  });

  it("formats exact and long forms", () => {
    expect(formatExactBytes(19_293_184)).toBe("19,293,184 bytes");
    expect(formatBytesLong(19_293_184)).toBe("19.3 MB (19,293,184 bytes)");
    expect(formatBytesLong(12)).toBe("12 bytes");
  });
});

describe("dates", () => {
  const now = new Date(2026, 9, 17, 15, 0, 0);

  it("shows today with a time", () => {
    const iso = new Date(2026, 9, 17, 14, 32).toISOString();
    expect(formatDate(iso, now)).toBe("Today, 14:32");
  });

  it("shows other days as a date", () => {
    const iso = new Date(2026, 9, 2, 9, 0).toISOString();
    expect(formatDate(iso, now)).toBe("Oct 2, 2026");
    expect(formatDateTime(new Date(2026, 8, 28, 9, 14).toISOString())).toBe("Sep 28, 2026, 09:14");
  });

  it("handles missing and invalid dates", () => {
    expect(formatDate(null, now)).toBe("--");
    expect(formatDate("not a date", now)).toBe("--");
    expect(formatIsoLocal(null)).toBe("");
  });

  it("formats a local ISO timestamp with offset", () => {
    expect(formatIsoLocal("2026-10-02T12:00:00Z")).toMatch(/^2026-10-0[23]T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
  });

  it("formats relative time", () => {
    expect(formatAgo(new Date(now.getTime() - 20_000).toISOString(), now)).toBe("just now");
    expect(formatAgo(new Date(now.getTime() - 5 * 60_000).toISOString(), now)).toBe("5 min ago");
    expect(formatAgo(new Date(now.getTime() - 3 * 3600_000).toISOString(), now)).toBe("3 h ago");
    expect(formatAgo(new Date(now.getTime() - 2 * 86400_000).toISOString(), now)).toBe("2 days ago");
  });
});

describe("speeds and durations", () => {
  it("formats speed", () => {
    expect(formatSpeed(6_100_000)).toBe("6.1 MB/s");
  });

  it("formats time left", () => {
    expect(formatEta(1)).toBe("1 s left");
    expect(formatEta(130)).toBe("about 2 min left");
    expect(formatEta(4800)).toBe("about 1 h 20 min left");
    expect(formatEta(3600)).toBe("about 1 h left");
  });

  it("gives time left in words for screen readers", () => {
    expect(formatEtaWords(130)).toBe("about 2 minutes remaining");
    expect(formatEtaWords(1)).toBe("about 1 second remaining");
    expect(formatEtaWords(7200)).toBe("about 2 hours remaining");
  });

  it("formats durations", () => {
    expect(formatDuration(45)).toBe("45 s");
    expect(formatDuration(72)).toBe("1 min 12 s");
    expect(formatDuration(7380)).toBe("2 h 3 min");
  });

  it("formats percent", () => {
    expect(formatPercent(42, 100)).toBe("42%");
    expect(formatPercent(1, 0)).toBe("0%");
    expect(formatPercent(150, 100)).toBe("100%");
  });
});

describe("text helpers", () => {
  it("joins with a middle dot and skips empty parts", () => {
    expect(joinDot(["Uploading 3 files", null, "42%", "", false])).toBe(`Uploading 3 files ${DOT} 42%`);
  });

  it("pluralizes", () => {
    expect(plural(1, "file")).toBe("1 file");
    expect(plural(1200, "file")).toBe("1,200 files");
  });

  it("shortens ids and truncates in the middle", () => {
    expect(shortId("a1b2c3d4e5f6a7b8c9d0e1f2a3b49f0e")).toBe("a1b2...9f0e");
    expect(shortId("short")).toBe("short");
    const t = middleTruncate("The Strongest Legend of Dragon Ball.mp3", 24);
    expect(t.length).toBeLessThanOrEqual(24);
    expect(t.endsWith(".mp3")).toBe(true);
    expect(t).toContain("...");
  });
});
