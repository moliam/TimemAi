import { describe, expect, it } from "vitest";
import { formatTokens } from "../src/token_format";

describe("token count formatting", () => {
  it("preserves empty and sub-thousand values", () => {
    expect(formatTokens(undefined)).toBeUndefined();
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(999)).toBe("999");
  });

  it("keeps the existing K formatting below one million", () => {
    expect(formatTokens(1_000)).toBe("1.0K");
    expect(formatTokens(9_999)).toBe("10.0K");
    expect(formatTokens(10_000)).toBe("10K");
    expect(formatTokens(999_999)).toBe("1000K");
  });

  it("uses one decimal place in M from 1000K onward", () => {
    expect(formatTokens(1_000_000)).toBe("1.0M");
    expect(formatTokens(1_200_000)).toBe("1.2M");
    expect(formatTokens(12_345_678)).toBe("12.3M");
  });
});
