import { describe, expect, it } from "vitest";
import { flippingTimeSegments } from "../src/flipping_time";
import { readFileSync } from "node:fs";

const source = readFileSync(new URL("../src/flipping_time.tsx", import.meta.url), "utf8");
const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("flipping live time", () => {
  it("keeps number groups stable by their following unit", () => {
    expect(flippingTimeSegments("3m3s")).toEqual([
      { kind: "digits", key: "digits:m:0", value: "3" },
      { kind: "text", key: "text:1:m", value: "m" },
      { kind: "digits", key: "digits:s:0", value: "3" },
      { kind: "text", key: "text:3:s", value: "s" },
    ]);
    expect(flippingTimeSegments("1:00:09").filter(segment => segment.kind === "digits").map(segment => segment.key))
      .toEqual(["digits:::1", "digits:::0", "digits:end:0"]);
  });

  it("right-aligns changing digit cells across width and unit boundaries", () => {
    const nine = flippingTimeSegments("9s").find(segment => segment.kind === "digits")!;
    const ten = flippingTimeSegments("10s").find(segment => segment.kind === "digits")!;
    const secondsBefore = flippingTimeSegments("59s").find(segment => segment.key === "digits:s:0")!;
    const secondsAfter = flippingTimeSegments("1m0s").find(segment => segment.key === "digits:s:0")!;
    const minutesBeforeHour = flippingTimeSegments("59:59").find(segment => segment.key === "digits:::0")!;
    const minutesAfterHour = flippingTimeSegments("1:00:00").find(segment => segment.key === "digits:::0")!;
    expect(ten.key).toBe(nine.key);
    expect(secondsAfter.key).toBe(secondsBefore.key);
    expect(minutesAfterHour.key).toBe(minutesBeforeHour.key);
    expect(flippingTimeSegments("1:00:00").some(segment => segment.key === "digits:::1")).toBe(true);
    expect(source).toContain("key={segment.value.length - index - 1}");
  });

  it("animates only live clocks and keeps settled durations static", () => {
    expect(main).toContain('<FlippingTime value={formattedElapsed} />');
    expect(main).toContain('running ? <FlippingTime value={elapsed} /> : elapsed');
    expect(main).toContain('<LocalizedFlippingTime message="tools.remaining" time={time} />');
    expect(main).toContain('<LocalizedFlippingTime message="tools.elapsed" time={time} />');
    expect(styles).toContain("width: 1ch;");
    expect(styles).toContain("animation: time-digit-roll-out .24s");
    expect(styles).toContain("animation: time-digit-roll-in .24s");
    expect(styles).toContain("contain: paint;");
    expect(styles).toContain("isolation: isolate;");
    expect(styles).not.toMatch(/time-digit-roll-(?:in|out)[^}]*opacity/);
    expect(styles).toMatch(/prefers-reduced-motion: reduce[\s\S]*\.time-flip-digit-out \{ display: none; animation: none;/);
  });
});
