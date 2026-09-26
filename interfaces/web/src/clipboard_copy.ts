import { useEffect, useRef, useState } from "react";

export function useTimedClipboardCopy(text: string, labels: { idle: string; copied: string; failed: string }) {
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const resetTimerRef = useRef<number | null>(null);
  useEffect(() => () => {
    if (resetTimerRef.current !== null) window.clearTimeout(resetTimerRef.current);
  }, []);
  useEffect(() => {
    if (resetTimerRef.current !== null) {
      window.clearTimeout(resetTimerRef.current);
      resetTimerRef.current = null;
    }
    setCopyState("idle");
  }, [text]);
  const copy = async () => {
    if (resetTimerRef.current !== null) window.clearTimeout(resetTimerRef.current);
    try {
      await copyTextToClipboard(text);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
    resetTimerRef.current = window.setTimeout(() => {
      setCopyState("idle");
      resetTimerRef.current = null;
    }, 1400);
  };
  const copyLabel = copyState === "copied" ? labels.copied : copyState === "failed" ? labels.failed : labels.idle;
  const copyClass = copyState === "copied" ? "copy-success" : copyState === "failed" ? "copy-failed" : "";
  return { copyState, copy, copyLabel, copyClass };
}

export async function copyTextToClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.left = "-9999px";
    textarea.style.top = "0";
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.select();
    try {
      if (!document.execCommand("copy")) throw new Error("execCommand copy failed");
    } finally {
      document.body.removeChild(textarea);
      window.getSelection()?.removeAllRanges();
    }
  }
}

// Handle selection endpoints in surrounding layout without rewriting mixed-message copies.
export function selectedUserMessageText(root: HTMLElement, selection: Selection | null): string | null {
  if (!selection || selection.isCollapsed || selection.rangeCount !== 1) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer)) return null;
  const entries = Array.from(root.querySelectorAll<HTMLElement>(".turn-user-entry"))
    .filter((entry) => range.intersectsNode(entry));
  if (entries.length !== 1) return null;
  const entry = entries[0];
  if (!entry.contains(range.startContainer)) {
    const before = range.cloneRange();
    before.setEndBefore(entry);
    if (before.toString().trim()) return null;
  }
  if (!entry.contains(range.endContainer)) {
    const after = range.cloneRange();
    after.setStartAfter(entry);
    if (after.toString().trim()) return null;
  }
  return selection.toString().replace(/(?:\r?\n)+$/, "");
}
