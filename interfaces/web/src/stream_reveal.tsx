import { memo, useEffect, useMemo, useRef, useState } from "react";
import { MarkdownContent } from "./markdown_render";

// Gradual reveal for streamed preview text. Delivered chunks are replayed at a
// frame-independent cadence that adapts to backlog, so bursty SSE pushes and
// protocol retries never make provisional text jump. Only monotonically
// growing prefixes animate; retraction, retry resets, restored snapshots and
// reduced-motion users always see the exact delivered text immediately.
export function streamRevealDelta(
  visibleLength: number,
  targetLength: number,
  dtMs: number,
): number {
  if (!Number.isFinite(dtMs) || dtMs <= 0) return 0;
  if (targetLength <= visibleLength) return 0;
  const lag = targetLength - visibleLength;
  const charsPerSecond = Math.min(240, 60 + Math.max(0, lag - 120) * 0.12);
  return Math.min(lag, (charsPerSecond * Math.min(dtMs, 40)) / 1000);
}

const prefersReducedMotion = () =>
  typeof window !== "undefined" &&
  !!window.matchMedia &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

const FENCE_OPEN = /^ {0,3}(```|~~~)/;
const FENCE_CLOSE = /^ {0,3}(```|~~~)[ \t]*$/;
const MATH_FENCE = /^ {0,3}\$\$[ \t]*$/;
const ATX_HEADING = /^ {0,3}#{1,6}(?:\s|$)/;
const HTML_OPEN = /^ {0,3}<([a-zA-Z][\w-]*)/;
const HTML_CLOSE = /^ {0,3}<\/([a-zA-Z][\w-]*)>/;
const SELF_CLOSING = /\/>[ \t]*$/;
const VOID_TAGS = new Set([
  "br", "hr", "img", "input", "meta", "link", "source", "area", "col", "embed", "track", "wbr",
]);

/**
 * Splits markdown source into top-level blocks. Splitting a prefix of the
 * text must produce a prefix of the split of the longer text: once a line is
 * emitted inside a finished block, appending more text can never rewrite that
 * block. Standalone ATX headings and blank lines close the current block;
 * fenced code, $$ math blocks and balanced raw HTML lines are kept atomic so
 * inner blank lines never split them.
 */
export function splitMarkdownBlocks(text: string): string[] {
  if (!text) return [];
  const lines = text.split("\n");
  const blocks: string[] = [];
  let current: string[] = [];
  let fence: string | null = null;
  let mathFence = false;
  let htmlDepth = 0;
  const flush = () => {
    if (current.length > 0) {
      blocks.push(current.join("\n"));
      current = [];
    }
  };
  for (const line of lines) {
    if (fence) {
      current.push(line);
      const close = line.match(FENCE_CLOSE);
      if (close && close[1].trim() === fence) fence = null;
      continue;
    }
    if (mathFence) {
      current.push(line);
      if (MATH_FENCE.test(line)) mathFence = false;
      continue;
    }
    const fenceOpen = line.match(FENCE_OPEN);
    if (fenceOpen) {
      if (current.length > 0) {
        blocks.push(current.join("\n"));
        current = [];
      }
      current.push(line);
      fence = fenceOpen[1].trim();
      continue;
    }
    if (MATH_FENCE.test(line)) {
      flush();
      current.push(line);
      mathFence = true;
      continue;
    }
    if (htmlDepth > 0) {
      current.push(line);
      for (const open of line.matchAll(HTML_OPEN)) {
        const tag = open[1].toLowerCase();
        if (SELF_CLOSING.test(line) || VOID_TAGS.has(tag)) continue;
        htmlDepth += 1;
      }
      const closes = [...line.matchAll(HTML_CLOSE)];
      for (const close of closes) {
        if (htmlDepth > 0) htmlDepth -= 1;
      }
      if (htmlDepth === 0 && (closes.length > 0 || line.trim() === "")) flush();
      continue;
    }
    if (line.trim() === "") {
      flush();
      continue;
    }
    if (ATX_HEADING.test(line) && current.length === 0) {
      blocks.push(line);
      continue;
    }
    const htmlOpen = line.match(HTML_OPEN);
    if (htmlOpen) {
      flush();
      current.push(line);
      const tag = htmlOpen[1].toLowerCase();
      const closes = line.match(HTML_CLOSE);
      if (!SELF_CLOSING.test(line) && !VOID_TAGS.has(tag) && !closes) htmlDepth = 1;
      else flush();
      continue;
    }
    current.push(line);
  }
  flush();
  return blocks;
}

/** Frozen blocks never re-parse or replay animations once delivered. */
const FrozenBlock = memo(function FrozenBlock({ text }: { text: string }) {
  return <MarkdownContent text={text} />;
});

export const StreamText = memo(function StreamText({ text }: { text: string }) {
  const [visible, setVisible] = useState(text);
  const visibleRef = useRef(text);
  const targetRef = useRef(text);
  const creditRef = useRef(0);
  const frameRef = useRef<number | null>(null);
  const lastTsRef = useRef<number | null>(null);

  useEffect(() => {
    targetRef.current = text;
    if (!text.startsWith(visibleRef.current) || prefersReducedMotion()) {
      visibleRef.current = text;
      creditRef.current = 0;
      setVisible(text);
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
      return;
    }
    if (visibleRef.current === text || frameRef.current !== null) return;
    lastTsRef.current = null;
    const step = (ts: number) => {
      const target = targetRef.current;
      const last = lastTsRef.current ?? ts;
      lastTsRef.current = ts;
      creditRef.current += streamRevealDelta(visibleRef.current.length, target.length, ts - last);
      const count = Math.floor(creditRef.current);
      if (count > 0) {
        let end = Math.min(target.length, visibleRef.current.length + count);
        // Never paint half of a UTF-16 surrogate pair.
        if (end < target.length && /[\\uD800-\\uDBFF]/.test(target[end - 1])) end += 1;
        creditRef.current = Math.max(0, creditRef.current - (end - visibleRef.current.length));
        visibleRef.current = target.slice(0, end);
        setVisible(visibleRef.current);
      }
      if (visibleRef.current === target) {
        frameRef.current = null;
        creditRef.current = 0;
        return;
      }
      frameRef.current = requestAnimationFrame(step);
    };
    frameRef.current = requestAnimationFrame(step);
  }, [text]);

  useEffect(() => () => {
    if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
  }, []);

  const blocks = useMemo(() => splitMarkdownBlocks(visible), [visible]);
  return (
    <div className="stream-markdown">
      {blocks.map((block, index) => (
        <FrozenBlock key={`block-${index}`} text={block} />
      ))}
    </div>
  );
});
