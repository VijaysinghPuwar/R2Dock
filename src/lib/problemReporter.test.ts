import { cleanMessage } from "./problemReporter";

describe("cleanMessage", () => {
  it("keeps the error name and message", () => {
    expect(cleanMessage(new TypeError("x is undefined"))).toBe("TypeError: x is undefined");
  });

  it("drops URL queries that could carry tokens", () => {
    expect(cleanMessage("failed https://a/b?X-Amz-Signature=abc&x=1 now")).toBe("failed https://a/b?… now");
  });

  it("caps long messages", () => {
    expect(cleanMessage("a".repeat(1000))).toHaveLength(400);
  });
});
