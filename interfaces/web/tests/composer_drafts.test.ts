import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";

const KEY = "timem-web-composer-drafts-v1";

function setup(values: Record<string, string> = {}, blocked = false) {
  const data = new Map(Object.entries(values));
  vi.stubGlobal("window", {
    localStorage: {
      getItem: (key: string) => {
        if (blocked) throw Error("blocked");
        return data.get(key) ?? null;
      },
      setItem: (key: string, value: string) => {
        if (blocked) throw Error("blocked");
        data.set(key, value);
      },
    },
  });
  return data;
}

async function load() {
  return import("../src/composer_drafts");
}

afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
});

describe("composer draft persistence", () => {
  it("round-trips drafts keyed by session id", async () => {
    const data = setup();
    const mod = await load();
    mod.persistComposerDraftTexts(
      { "session-1": "长草稿 A", "session-2": "draft B" },
      null,
    );
    expect(JSON.parse(data.get(KEY)!)).toEqual({
      "session-1": { text: "长草稿 A", savedAtMs: expect.any(Number) },
      "session-2": { text: "draft B", savedAtMs: expect.any(Number) },
    });
    const mod2 = await load();
    expect(mod2.readComposerDraftTexts()).toEqual({
      "session-1": "长草稿 A",
      "session-2": "draft B",
    });
  });

  it("drops an entry whose session is known live (draft submitted or cleared)", async () => {
    setup();
    const mod = await load();
    mod.persistComposerDraftTexts(
      { "session-1": "kept", "session-2": "gone" },
      null,
    );
    mod.persistComposerDraftTexts({ "session-1": "kept" }, new Set(["session-2"]));
    expect(mod.readComposerDraftTexts()).toEqual({ "session-1": "kept" });
  });

  it("freezes unknown-session entries while sessions restore progressively", async () => {
    setup();
    const mod = await load();
    mod.persistComposerDraftTexts({ "old-session": "restoring draft" }, null);
    // Sessions not yet observed (restoring in background) keep their draft.
    mod.persistComposerDraftTexts({}, null);
    expect(mod.readComposerDraftTexts()).toEqual({
      "old-session": "restoring draft",
    });
    // Once the session was seen live and disappeared, the draft is dropped.
    mod.persistComposerDraftTexts({}, null, new Set(["old-session"]));
    expect(mod.readComposerDraftTexts()).toEqual({});
  });

  it("bounds the store: most recently saved sessions first under hard caps", async () => {
    const now = Date.now();
    const seeded: Record<string, { text: string; savedAtMs: number }> = {};
    for (let i = 0; i < 40; i += 1) {
      seeded[`session-${i}`] = { text: `text ${i}`, savedAtMs: now - (40 - i) };
    }
    const data = setup({ [KEY]: JSON.stringify(seeded) });
    const mod = await load();
    // Re-persist without touching texts: bounding must keep the newest 32.
    mod.persistComposerDraftTexts({}, null);
    const stored = JSON.parse(data.get(KEY)!) as Record<string, { text: string }>;
    expect(Object.keys(stored)).toHaveLength(32);
    expect(stored["session-39"]!.text).toBe("text 39");
    expect(stored["session-8"]!.text).toBe("text 8");
    expect(stored["session-7"]).toBeUndefined();
  });

  it("ages frozen entries out after the TTL on the next persist", async () => {
    const stale = Date.now() - 8 * 24 * 60 * 60 * 1000;
    const data = setup({
      [KEY]: JSON.stringify({
        "old-session": { text: "stale draft", savedAtMs: stale },
      }),
    });
    const mod = await load();
    // Reads stay literal; expiry is enforced by the write path.
    expect(mod.readComposerDraftTexts()).toEqual({
      "old-session": "stale draft",
    });
    mod.persistComposerDraftTexts({}, null);
    expect(mod.readComposerDraftTexts()).toEqual({});
    expect(data.size).toBe(1);
  });

  it("ignores corrupted, non-object, or invalid persisted data", async () => {
    setup({ [KEY]: "not json {" });
    const mod = await load();
    expect(mod.readComposerDraftTexts()).toEqual({});
    const data = setup({
      [KEY]: JSON.stringify({
        ok: { text: "fine", savedAtMs: 1 },
        empty: { text: "", savedAtMs: 1 },
        wrongType: { text: 7, savedAtMs: 1 },
        noTime: { text: "fine too" },
        array: ["x"],
      }),
    });
    const mod2 = await load();
    expect(mod2.readComposerDraftTexts()).toEqual({ ok: "fine" });
    expect(data.size).toBe(1);
  });

  it("fails soft when storage is unavailable", async () => {
    setup({}, true);
    const mod = await load();
    expect(() =>
      mod.persistComposerDraftTexts({ "session-1": "private mode" }, null),
    ).not.toThrow();
    expect(mod.readComposerDraftTexts()).toEqual({});
  });

  it("keeps a smaller previous copy when a new draft exceeds the per-entry cap", async () => {
    setup();
    const mod = await load();
    mod.persistComposerDraftTexts({ "session-1": "small" }, null);
    mod.persistComposerDraftTexts({ "session-1": "x".repeat(100_001) }, null);
    expect(mod.readComposerDraftTexts()).toEqual({ "session-1": "small" });
  });
});

describe("composer draft wiring", () => {
  const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

  it("restores drafts at mount and persists them debounced with a teardown flush", () => {
    expect(source).toContain("useState<\n    Record<string, string>\n  >(() => readComposerDraftTexts())");
    expect(source.match(/persistComposerDraftTexts/g)?.length).toBeGreaterThanOrEqual(3);
    expect(source).toContain('window.addEventListener("pagehide", flush)');
    expect(source).toContain('document.addEventListener("visibilitychange", onVisibility)');
    expect(source).toContain('document.visibilityState === "hidden"');
  });

  it("protects restored drafts from progressive session restore pruning", () => {
    expect(source).toContain("restoredDraftSessionIdsRef");
    expect(source).toContain("everLiveSessionIdsRef");
    expect(source).toMatch(
      /pruneSessionDrafts\(\s*current,\s*sessionIds,\s*restoredDraftSessionIdsRef\.current,\s*everLiveSessionIdsRef\.current,/,
    );
  });

  it("wires the fullscreen composer editor for write-through and explicit discard", () => {
    expect(source).toContain("onLiveCommit={commitComposerExpandedDraft}");
    expect(source).toContain("onDiscard={discardComposerExpandedDraft}");
    expect(source).toContain("composerExpandedBaseRef.current = draft");
    // The pending write-through timer must not be reset by newer keystrokes.
    expect(source).toContain("liveCommitTimerRef.current !== null) return;");
  });
});
