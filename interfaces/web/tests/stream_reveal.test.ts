import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  splitMarkdownBlocks,
  streamRevealDelta,
  StreamText,
} from "../src/stream_reveal";

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
    expect(streamRevealDelta(0, 20, 100)).toBe(2.4);
    expect(streamRevealDelta(0, 20, 16)).toBe(0.96);
    expect(streamRevealDelta(0, 5000, 1000)).toBe(9.6);
    expect(streamRevealDelta(0, 5000, 8)).toBe(1.92);
    expect(streamRevealDelta(0, 5, 1)).toBe(0.06);
  });

  it("catches up faster as lag grows while each frame stays bounded", () => {
    const slow = streamRevealDelta(0, 90, 100);
    const fast = streamRevealDelta(0, 900, 100);
    expect(fast).toBeGreaterThan(slow);
    expect(streamRevealDelta(0, 5, 100)).toBeLessThanOrEqual(160);
  });
});

describe("stream reveal integration", () => {
  it("preserves fractional progress across high-refresh frames", () => {
    expect(streamRevealDelta(0, 90, 8) * 2).toBe(streamRevealDelta(0, 90, 16));
    expect(streamRevealDelta(0, 1, 40)).toBe(1);
  });
  it("keeps reading handoff mounted and defaults stream details closed", () => {
    expect(mainSource).toContain("streamUiMode || turn.sub_answers.length");
    expect(mainSource).toContain("useState(() => !streamUiMode && isWorking)");
    expect(mainSource).not.toContain("stream-reading-hold");
    expect(mainSource).not.toContain("stream-thought-card");
    expect(mainSource).toContain('className="stream-working-trailer"');
    expect(mainSource).toContain('const previewText = intermediate ? ""');
    expect(mainSource).toContain("activity.detail && <MarkdownContent text={activity.detail}");
  });

  it("collapses completed and interrupted work independently of stream mode", () => {
    expect(mainSource).toContain(
      'if (finalArrived || (wasWorking && turn.state !== "working"))\n      setShowWorkStream(false);',
    );
    expect(mainSource).not.toContain("hasVisibleProcess && !onlyFreeTalk");
  });

  it("counts only framed lifecycle-coalesced tools while work is collapsed", () => {
    expect(mainSource).toContain("count + run.summary.activities.length");
    expect(mainSource).toContain("!workStreamVisible && !isToolGenTurn && framedToolCount > 0");
    expect(mainSource).toContain('framedToolCount === 1 ? "tool" : "tools"');
  });

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
    expect(mainSource.match(/<StreamText /g)?.length).toBe(5);
    expect(mainSource).toContain("liveAnswer.provisional ? <StreamText text={liveAnswer.answer} /> : <MarkdownContent text={liveAnswer.answer} />");
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

describe("splitMarkdownBlocks incremental stability", () => {
  it("never rewrites finished blocks while text grows", () => {
    const bases = [
      "para one\n\npara two",
      "intro\n\n```rust\nfn a() {}",
      "# Title\n\nbody text",
      "list start\n- a\n- b",
      "math\n\n$$\nE=mc^2",
    ];
    const extensions = ["", " more", "\n\nnext para", "\n```\n\nafter", "\n$$\n\nafter math"];
    for (const base of bases) {
      const baseBlocks = splitMarkdownBlocks(base);
      for (const ext of extensions) {
        const grown = splitMarkdownBlocks(base + ext);
        // The last base block is still open and may grow in place; every
        // earlier block must stay byte-identical (never rewritten).
        baseBlocks.slice(0, -1).forEach((block, index) => {
          expect(grown[index]).toBe(block);
        });
        // For closed-text bases, a new-paragraph extension must keep the
        // previous split as a strict prefix. Bases ending inside an open
        // fence/math block correctly keep growing the open tail instead.
        const openTail = base.includes("```") || base.includes("$$");
        if (ext === "" || (ext.startsWith("\n\n") && !openTail)) {
          expect(grown.slice(0, baseBlocks.length)).toEqual(baseBlocks);
        }
      }
    }
  });

  it("splits completed paragraphs and keeps open fences as one tail block", () => {
    expect(splitMarkdownBlocks("a\n\nb")).toEqual(["a", "b"]);
    expect(splitMarkdownBlocks("```js\n1\n```\n\ntail")).toEqual([
      "```js\n1\n```",
      "tail",
    ]);
    const open = splitMarkdownBlocks("text\n\n```js\nlet x = 1;\n");
    expect(open).toEqual(["text", "```js\nlet x = 1;\n"]);
  });

  it("renders frozen blocks through memoized markdown so old blocks stay stable", () => {
    const html = renderToStaticMarkup(
      createElement(StreamText, { text: "one\n\ntwo **bold**" }),
    );
    expect(html).toContain("stream-markdown");
    expect(html.match(/class="markdown-body"/g)?.length).toBe(2);
  });
});

 it("uses the trailer instead of a typing caret and archives only after working ends", () => {
   const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
   expect(css).not.toContain("stream-caret-pulse");
   expect(mainSource).toContain('<StreamProcess closing={turn.state !== "working"}');
   expect(mainSource).toContain('className="stream-working-dot"');
 });

 it("bounds repeated stream rendering and interaction listeners", () => {
   expect(mainSource).toContain("const StreamToolRow = memo(");
   expect(mainSource).toContain("const runningStreamTools = useMemo(");
   expect(mainSource.match(/document.addEventListener\("selectionchange"/g)?.length).toBe(1);
   expect(mainSource).toContain("if (followBottom) frame = requestAnimationFrame(follow)");
   expect(mainSource).toContain("if (mergeReady || completed.length < 2) return");
 });

it("animates growing merged counts without remounting the toggle", () => {
  expect(mainSource).toContain("merged && completed.length > previousMergedCount.current");
  expect(mainSource).toContain('<span key={countRevision}');
  expect(mainSource).toContain("toolResultCountsLabel(succeededCount, failedCount)");
  const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
  expect(css).toContain("@keyframes stream-tool-count-increment");
  expect(css).toContain(".stream-tool-count.incremented { animation: none; }");
});
