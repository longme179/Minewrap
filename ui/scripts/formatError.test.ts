import { describe, it, expect } from "vitest";
import { formatError, plainErrorForTest } from "../src/lib/formatError.js";

describe("formatError", () => {
  it("unwraps the structured {code, message} envelope", () => {
    expect(formatError({ code: "invalid", message: "bad config" })).toBe("invalid: bad config");
    expect(formatError({ code: "unauthorised", message: "bad token" })).toBe("unauthorised: bad token");
    expect(formatError(new Error("boom"))).toBe("boom");
    expect(formatError("plain")).toBe("plain");
  });

  it("never renders an object as [object Object]", () => {
    expect(plainErrorForTest({ code: "invalid", message: "x" })).not.toBe("[object Object]");
    expect(() => plainErrorForTest({ code: "invalid", message: "x" })).not.toThrow("[object Object]");
  });
});
