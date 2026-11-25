import { describe, expect, it } from "vitest";
import { resolvePreviewDelta } from "../src/view_model";

type Preview = NonNullable<Parameters<typeof resolvePreviewDelta>[0]>;

function frame(overrides: Record<string, unknown> = {}): Preview {
  return {
    attempt: 1,
    revision: 1,
    response: { attempt: 1, revision: 1, text: "", status: "streaming" },
    ...overrides,
  } as Preview;
}

describe("resolvePreviewDelta", () => {
  it("treats a frame that carries text as the new base", () => {
    const incoming = frame({
      response: { attempt: 1, revision: 1, text: "完整文本", status: "streaming" },
    });
    expect(resolvePreviewDelta(incoming, undefined)).toBe(incoming);
  });

  it("appends a delta that continues the local copy", () => {
    const previous = frame({
      response: { attempt: 1, revision: 1, text: "你好", status: "streaming" },
    });
    const incoming = frame({
      revision: 2,
      response: {
        attempt: 1,
        revision: 2,
        status: "streaming",
        text_delta: "，世界🙂",
        text_base_len: 2,
      },
    });
    const resolved = resolvePreviewDelta(incoming, previous);
    expect(resolved?.response?.text).toBe("你好，世界🙂");
    expect(resolved?.response?.text_delta).toBe("，世界🙂");
  });

  it("applies an empty delta without touching the local copy", () => {
    const previous = frame({
      response: { attempt: 1, revision: 1, text: "稳定文本", status: "streaming" },
    });
    const incoming = frame({
      revision: 2,
      response: {
        attempt: 1,
        revision: 2,
        status: "streaming",
        text_delta: "",
        text_base_len: 4,
      },
    });
    expect(resolvePreviewDelta(incoming, previous)?.response?.text).toBe("稳定文本");
  });

  it("drops a delta whose base does not match the local copy", () => {
    const previous = frame({
      response: { attempt: 1, revision: 1, text: "短", status: "streaming" },
    });
    const incoming = frame({
      revision: 5,
      response: {
        attempt: 1,
        revision: 5,
        status: "streaming",
        text_delta: "丢失的片段",
        text_base_len: 9,
      },
    });
    expect(resolvePreviewDelta(incoming, previous)).toBeNull();
  });

  it("drops a delta from another attempt so the new attempt starts clean", () => {
    const previous = frame({
      response: { attempt: 1, revision: 9, text: "旧尝试", status: "intermediate" },
    });
    const incoming = frame({
      attempt: 2,
      revision: 10,
      response: {
        attempt: 2,
        revision: 1,
        status: "streaming",
        text_delta: "新",
        text_base_len: 0,
      },
    });
    expect(resolvePreviewDelta(incoming, previous)).toBeNull();
  });

  it("drops a frame with neither text nor delta", () => {
    const incoming = frame({
      revision: 3,
      response: { attempt: 1, revision: 3, status: "streaming" },
    });
    expect(resolvePreviewDelta(incoming, undefined)).toBeNull();
  });
});
