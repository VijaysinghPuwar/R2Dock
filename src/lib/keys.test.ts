import { baseName, extension, folderLabel, parentPrefix, splitMatch, splitPrefix, visibleText } from "./keys";
import { fileKind, humanType, previewKind } from "./fileTypes";

const RLO = String.fromCharCode(0x202e);
const ZWSP = String.fromCharCode(0x200b);
const NUL = String.fromCharCode(0);

describe("visibleText", () => {
  it("shows bidi overrides, zero-width and control characters as placeholders", () => {
    const out = visibleText(`invoice${RLO}fdp.exe`);
    expect(out).not.toContain(RLO);
    expect(out).toContain("U+202E");
    expect(visibleText(`a${ZWSP}b`)).toContain("U+200B");
    expect(visibleText(`a${NUL}b`)).toContain("U+0000");
  });

  it("keeps normal text unchanged", () => {
    expect(visibleText("cover.webp")).toBe("cover.webp");
    expect(visibleText("Kapitel 1 ä")).toBe("Kapitel 1 ä");
  });
});

describe("prefix helpers", () => {
  it.each([
    ["a/b/c.txt", "a/b/"],
    ["a/b/", "a/"],
    ["a/", ""],
    ["file.txt", ""],
    ["a//b/", "a//"],
  ])("parentPrefix(%s) = %s", (input, expected) => {
    expect(parentPrefix(input)).toBe(expected);
  });

  it("gets base names", () => {
    expect(baseName("a/b/c.txt")).toBe("c.txt");
    expect(baseName("a/b/")).toBe("b");
  });

  it("splits prefixes into crumbs and marks empty segments", () => {
    const crumbs = splitPrefix("a//b/");
    expect(crumbs.map((c) => c.prefix)).toEqual(["a/", "a//", "a//b/"]);
    expect(crumbs[1]?.empty).toBe(true);
    expect(crumbs[1]?.label).toBe("(empty)");
    expect(splitPrefix("")).toEqual([]);
  });

  it("labels folders", () => {
    expect(folderLabel("media", "")).toBe("media");
    expect(folderLabel("media", "covers/2026/")).toBe("2026");
  });

  it("finds matches case-insensitively", () => {
    expect(splitMatch("Dragon Ball.mp3", "ball")).toEqual(["Dragon ", "Ball", ".mp3"]);
    expect(splitMatch("cover.webp", "zzz")).toBeNull();
  });
});

describe("file types", () => {
  it("detects kinds, previews and labels", () => {
    expect(extension("archive.tar.gz")).toBe("gz");
    expect(extension(".env")).toBe("");
    expect(fileKind("cover.webp")).toBe("image");
    expect(fileKind("intro.mp3")).toBe("audio");
    expect(previewKind("notes.md")).toBe("text");
    expect(previewKind("movie.mp4")).toBe("video");
    expect(previewKind("setup.exe")).toBe("none");
    expect(humanType("cover.webp")).toBe("WebP image");
    expect(humanType("intro.mp3")).toBe("MP3 audio");
    expect(humanType("noext")).toBe("Unknown");
  });
});
