import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { useThrottledValue } from "../src/use_throttled_value";

const source = readFileSync(new URL("../src/use_throttled_value.ts", import.meta.url), "utf8");
const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("useThrottledValue", () => {
  it("is exported as a usable hook module", () => {
    expect(typeof useThrottledValue).toBe("function");
  });

  it("refreshes on an interval and always clears it on teardown", () => {
    expect(source).toContain("window.setInterval");
    expect(source).toContain("window.clearInterval(timer)");
    expect(source).toContain("}, [intervalMs]);");
  });

  it("short-circuits stable values through Object.is", () => {
    expect(source).toContain(
      "Object.is(current, latestRef.current) ? current : latestRef.current",
    );
  });

  it("is the single definition and serves every streaming render path", () => {
    expect(main).toContain('import { useThrottledValue } from "./use_throttled_value";');
    expect(main).not.toContain("function useThrottledValue");
    expect(main.split("useThrottledValue(").length - 1).toBe(3);
    expect(main).toContain("useThrottledValue(text, STREAM_MARKDOWN_INTERVAL_MS)");
    expect(main).toContain("useThrottledValue(turn.final_answer || previewText, 200)");
  });
});
