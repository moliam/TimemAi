import { afterEach, describe, expect, it, vi } from "vitest";
import { copyTextToClipboard } from "../src/clipboard_copy";

const originalDescriptors = new Map<PropertyKey, PropertyDescriptor | undefined>();
for (const key of ["navigator", "document", "window"] as const) {
  originalDescriptors.set(key, Object.getOwnPropertyDescriptor(globalThis, key));
}

function setGlobal(key: "navigator" | "document" | "window", value: unknown) {
  Object.defineProperty(globalThis, key, { configurable: true, writable: true, value });
}

afterEach(() => {
  vi.restoreAllMocks();
  for (const [key, descriptor] of originalDescriptors) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else Reflect.deleteProperty(globalThis, key);
  }
});

function fallbackEnvironment(execResult = true) {
  const textarea = {
    value: "",
    style: {} as Record<string, string>,
    setAttribute: vi.fn(),
    focus: vi.fn(),
    select: vi.fn(),
  };
  const appendChild = vi.fn();
  const removeChild = vi.fn();
  const execCommand = vi.fn(() => execResult);
  const removeAllRanges = vi.fn();
  setGlobal("document", {
    createElement: vi.fn(() => textarea),
    body: { appendChild, removeChild },
    execCommand,
  });
  setGlobal("window", { getSelection: vi.fn(() => ({ removeAllRanges })) });
  return { textarea, appendChild, removeChild, execCommand, removeAllRanges };
}

describe("copyTextToClipboard", () => {
  it("uses the asynchronous Clipboard API when available", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    setGlobal("navigator", { clipboard: { writeText } });
    const fallback = fallbackEnvironment();

    await copyTextToClipboard("exact text");

    expect(writeText).toHaveBeenCalledOnce();
    expect(writeText).toHaveBeenCalledWith("exact text");
    expect(fallback.appendChild).not.toHaveBeenCalled();
  });

  it("falls back to a hidden readonly textarea and cleans it up", async () => {
    setGlobal("navigator", { clipboard: { writeText: vi.fn().mockRejectedValue(new Error("denied")) } });
    const fallback = fallbackEnvironment();

    await copyTextToClipboard("fallback text");

    expect(fallback.textarea.value).toBe("fallback text");
    expect(fallback.textarea.setAttribute).toHaveBeenCalledWith("readonly", "true");
    expect(fallback.textarea.style).toMatchObject({ position: "fixed", left: "-9999px", top: "0" });
    expect(fallback.appendChild).toHaveBeenCalledWith(fallback.textarea);
    expect(fallback.textarea.focus).toHaveBeenCalledOnce();
    expect(fallback.textarea.select).toHaveBeenCalledOnce();
    expect(fallback.execCommand).toHaveBeenCalledWith("copy");
    expect(fallback.removeChild).toHaveBeenCalledWith(fallback.textarea);
    expect(fallback.removeAllRanges).toHaveBeenCalledOnce();
  });

  it("rejects failed fallback copies but still cleans up", async () => {
    setGlobal("navigator", {});
    const fallback = fallbackEnvironment(false);

    await expect(copyTextToClipboard("cannot copy")).rejects.toThrow("execCommand copy failed");
    expect(fallback.removeChild).toHaveBeenCalledWith(fallback.textarea);
    expect(fallback.removeAllRanges).toHaveBeenCalledOnce();
  });
});
