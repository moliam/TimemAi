import { describe, expect, it } from "vitest";
import { classifyEventSequence, eventCursorStorageKey, loadEventCursor, resolveHelloEventCursor, saveEventCursor } from "../src/event_cursor";

describe("semantic event cursor", () => {
  it("accepts only the next sequence and detects duplicates and gaps", () => {
    expect(classifyEventSequence(7, 7)).toBe("duplicate");
    expect(classifyEventSequence(7, 6)).toBe("duplicate");
    expect(classifyEventSequence(7, 8)).toBe("next");
    expect(classifyEventSequence(7, 9)).toBe("gap");
  });

  it("persists a cursor independently for each mem scope", () => {
    const values = new Map<string, string>();
    const storage = {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
    };
    expect(saveEventCursor(storage, "host\u0000/mem/a", 12)).toBe(true);
    expect(loadEventCursor(storage, "host\u0000/mem/a")).toBe(12);
    expect(loadEventCursor(storage, "host\u0000/mem/b")).toBe(0);
    expect(eventCursorStorageKey("host\u0000/mem/a")).not.toBe(eventCursorStorageKey("host\u0000/mem/b"));
  });

  it("does not advance storage for invalid cursors", () => {
    const storage = { setItem: () => { throw new Error("must not write"); } };
    expect(saveEventCursor(storage, "scope", -1)).toBe(false);
    expect(saveEventCursor(storage, "scope", 1.5)).toBe(false);
  });

  it("reconnects after a mem switch when that mem has an earlier durable cursor", () => {
    expect(resolveHelloEventCursor("scope-a", "scope-b", 5, 10)).toEqual({ cursor: 5, reconnectForReplay: true });
    expect(resolveHelloEventCursor("scope-a", "scope-b", 0, 10)).toEqual({ cursor: 10, reconnectForReplay: false });
    expect(resolveHelloEventCursor("scope-b", "scope-b", 5, 10)).toEqual({ cursor: 5, reconnectForReplay: false });
  });

  it("treats a server journal reset as a new snapshot baseline", () => {
    expect(resolveHelloEventCursor("scope-a", "scope-b", 12, 3)).toEqual({ cursor: 3, reconnectForReplay: false });
  });

  it("uses the snapshot when the saved cursor is older than the retained journal floor", () => {
    expect(resolveHelloEventCursor("scope-a", "scope-b", 4, 20, 10)).toEqual({ cursor: 20, reconnectForReplay: false });
    expect(resolveHelloEventCursor("scope-a", "scope-b", 10, 20, 10)).toEqual({ cursor: 10, reconnectForReplay: true });
  });

  it("does not skip a gap after duplicates and replay", () => {
    let cursor = 4;
    for (const incoming of [4, 5, 5, 6]) {
      const state = classifyEventSequence(cursor, incoming);
      if (state === "next") cursor = incoming;
    }
    expect(cursor).toBe(6);
    expect(classifyEventSequence(cursor, 8)).toBe("gap");
    expect(cursor).toBe(6);
    expect(classifyEventSequence(cursor, 7)).toBe("next");
  });

  it("keeps two browser tabs on independent reducer cursors", () => {
    const tabAValues = new Map<string, string>();
    const tabBValues = new Map<string, string>();
    const tabA = { getItem: (key: string) => tabAValues.get(key) ?? null, setItem: (key: string, value: string) => { tabAValues.set(key, value); } };
    const tabB = { getItem: (key: string) => tabBValues.get(key) ?? null, setItem: (key: string, value: string) => { tabBValues.set(key, value); } };
    saveEventCursor(tabA, "scope", 9);
    saveEventCursor(tabB, "scope", 4);
    expect(loadEventCursor(tabA, "scope")).toBe(9);
    expect(loadEventCursor(tabB, "scope")).toBe(4);
    expect(classifyEventSequence(loadEventCursor(tabB, "scope"), 5)).toBe("next");
  });
});
