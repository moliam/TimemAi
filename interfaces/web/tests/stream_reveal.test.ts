import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { streamRevealDelta, StreamText } from "../src/stream_reveal";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const revealSource = readFileSync(new URL("../src/stream_reveal.tsx", import.meta.url), "utf8");

describe("stream reveal pacing", () => {
  it("emits nothing without elapsed time or backlog", () => {
    expect(streamRevealDelta(10, 10, 100)).toBe(0);
    expect(streamRevealDelta(15, 10, 100)).toBe(0);
    expect(streamRevealDelta(0, 40, 0)).toBe(0);
    expect(streamRevealDelta(0, 40, -5)).toBe(0);
    expect(streamRevealDelta(0, 40, Number.NaN)).toBe(0);
  });

  it("paces a small backlog slowly and bounds each frame", () => {
    expect(streamRevealDelta(0, 20, 100)).toBe(30);
    expect(streamRevealDelta(0, 20, 16)).toBe(5);
    expect(streamRevealDelta(0, 5000, 1000)).toBe(160);
    expect(streamRevealDelta(0, 5000, 8)).toBe(42);
    expect(streamRevealDelta(0, 5, 1)).toBe(1);
  });

  it("catches up faster as lag grows while each frame stays bounded", () => {
    const slow = streamRevealDelta(0, 90, 100);
    const fast = streamRevealDelta(0, 900, 100);
    expect(fast).toBeGreaterThan(slow);
    expect(streamRevealDelta(0, 5, 100)).toBeLessThanOrEqual(160);
  });
});

describe("stream reveal integration", () => {
  it("renders delivered markdown immediately without a DOM test double", () => {
    const html = renderToStaticMarkup(
      createElement(StreamText, { text: "Hello **stream** world" }),
    );
    expect(html).toContain("markdown-body");
    expect(html).toContain("<strong>stream</strong>");
  });

  it("keeps provisional DOM classes and routes only provisional text through StreamText", () => {
    expect(mainSource).toContain(
      'turn-interim-item${item.provisional ? " provisional-chat" : ""}',
    );
    expect(mainSource).toContain(
      'provisional ? `response-preview${streaming ? " streaming" : ""}` : "turn-final-delivery"',
    );
    expect(mainSource.match(/<StreamText /g)?.length).toBe(2);
    expect(mainSource).toContain("{item.provisional ? <StreamText text={item.answer} />");
  });

  it("marks actively streaming containers and drops the marker on interruption", () => {
    expect(mainSource).toContain(
      '${item.provisional && !preview?.interruption ? " streaming" : ""}',
    );
    expect(mainSource).toContain(
      'streaming={!hasFinal && preview?.response?.status === "streaming"}',
    );
  });

  it("snaps non-monotonic and reduced-motion updates immediately", () => {
    expect(revealSource).toContain("startsWith(visibleRef.current)");
    expect(revealSource).toContain("prefers-reduced-motion: reduce");
    expect(revealSource).toContain("cancelAnimationFrame");
  });
});
