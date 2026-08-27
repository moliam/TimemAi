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
