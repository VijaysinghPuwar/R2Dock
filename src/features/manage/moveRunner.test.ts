import { describe, expect, it } from "vitest";
import { moveProblems } from "./moveRunner";

describe("moveProblems", () => {
  it("returns null when everything moved", () => {
    expect(moveProblems({ moved: 3, failures: [], copiedNotDeleted: [] })).toBeNull();
  });

  it("says kept originals exist in both places", () => {
    const res = moveProblems({
      moved: 1,
      failures: [{ key: "a.txt", message: "Something with that name already exists at the destination." }],
      copiedNotDeleted: [{ key: "b.txt", message: "Copied, but the original changed during the move, so it was kept." }],
    });
    expect(res?.summary).toContain("1 were not moved and are still in the original place.");
    expect(res?.summary).toContain("the originals were kept, so they now exist in both places.");
    expect(res?.copiedNotDeleted).toHaveLength(1);
  });
});
