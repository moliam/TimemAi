import { memo, useEffect, useRef, useState } from "react";
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
  const charsPerSecond = Math.min(6000, 300 + Math.max(0, lag - 100));
  return Math.max(1, Math.min(160, Math.round((charsPerSecond * dtMs) / 1000)));
}

const prefersReducedMotion = () =>
  typeof window !== "undefined" &&
  !!window.matchMedia &&
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export const StreamText = memo(function StreamText({ text }: { text: string }) {
  const [visible, setVisible] = useState(text);
  const visibleRef = useRef(text);
  const frameRef = useRef<number | null>(null);
  const lastTsRef = useRef<number | null>(null);

  useEffect(() => {
    if (visibleRef.current === text) return;
    if (!text.startsWith(visibleRef.current) || prefersReducedMotion()) {
      visibleRef.current = text;
      setVisible(text);
      return;
    }
    if (frameRef.current !== null) return;
    lastTsRef.current = null;
    const step = (ts: number) => {
      const last = lastTsRef.current ?? ts;
      lastTsRef.current = ts;
      const delta = streamRevealDelta(visibleRef.current.length, text.length, ts - last);
      const next = text.slice(0, visibleRef.current.length + delta);
      visibleRef.current = next;
      setVisible(next);
      if (next === text) {
        frameRef.current = null;
        return;
      }
      frameRef.current = requestAnimationFrame(step);
    };
    frameRef.current = requestAnimationFrame(step);
    return () => {
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [text]);

  useEffect(
    () => () => {
      if (frameRef.current !== null) cancelAnimationFrame(frameRef.current);
    },
    [],
  );

  return <MarkdownContent text={visible} />;
});
