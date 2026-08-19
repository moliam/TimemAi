import { describe, expect, it } from "vitest";
import { applyQueuedMessagesAck, claimQueuedMessage, clearSessionQueuedMessages, COLLAPSED_QUEUE_LIMIT, loadQueuedMessages, QueuedMessage, queuedMessageKey, queuedMessagesStorageKey, releaseQueuedMessageClaim, releaseSessionQueuedMessageClaims, removeQueuedMessage, reorderQueuedMessages, reservedQueuedAttachmentIds, saveQueuedMessages, selectQueuedDispatches, clearQueuedMessagesPause, loadQueuedMessagesPause, queuedMessagesPauseStorageKey, saveQueuedMessagesPause, shouldDirectManualMessage, shouldPauseQueuedMessages, stopQueuedAutoSend, unclaimedQueuedMessages } from "../src/queued_messages";

const messages: QueuedMessage[] = ["a", "b", "c", "d", "e"].map((id, index) => ({
  id,
  text: `message ${id}`,
  createdAtMs: index,
attachmentIds: index === 0 ? ["upload-a"] : [], }));

describe("queued messages", () => {
  it("direct-sends only an idle session without backlog or a queue pause", () => {
    expect(shouldDirectManualMessage("ready", 0, false)).toBe(true);
    expect(shouldDirectManualMessage("working", 0, false)).toBe(false);
    expect(shouldDirectManualMessage("error", 0, false)).toBe(false);
    expect(shouldDirectManualMessage("stopped", 0, false)).toBe(false);
    expect(shouldDirectManualMessage("ready", 1, false)).toBe(false);
    expect(shouldDirectManualMessage("ready", 0, true)).toBe(false);
  });

  it("requires a new ready-state decision after a direct submission is occupied", () => {
    expect(shouldDirectManualMessage("ready", 0, false)).toBe(true);
    expect(shouldDirectManualMessage("working", 0, false)).toBe(false);
  });

  it("pauses automatic queue dispatch after every abnormal turn stop", () => {
    expect(shouldPauseQueuedMessages("CancelledByUser")).toBe(true);
    expect(shouldPauseQueuedMessages("ModelError")).toBe(true);
    expect(shouldPauseQueuedMessages("OutputLimitStoppedByUser")).toBe(true);
    expect(shouldPauseQueuedMessages("RoundLimitReached")).toBe(true);
    expect(shouldPauseQueuedMessages("ProtocolRepairFailed")).toBe(true);
    expect(shouldPauseQueuedMessages("")).toBe(false);
    expect(shouldPauseQueuedMessages(undefined)).toBe(false);
  });

  it("only degrades automatic sending from enabled to disabled", () => {
    const stoppedByError = stopQueuedAutoSend(null, "model unavailable", "error", 100);
    expect(stoppedByError).toEqual({
      paused: true,
      source: "error",
      reason: "model unavailable",
      stoppedAtMs: 100,
    });

    const stoppedByUser = stopQueuedAutoSend(null, "user disabled automatic sending", "user", 200);
    expect(stopQueuedAutoSend(stoppedByUser, "runtime disconnected", "error", 300)).toBe(stoppedByUser);
    expect(stoppedByUser).toEqual({
      paused: true,
      source: "user",
      reason: "user disabled automatic sending",
      stoppedAtMs: 200,
    });
  });

  it("never turns automatic sending back on while handling a system event", () => {
    const stopped = stopQueuedAutoSend(null, "network failure", "error", 1);
    expect(stopQueuedAutoSend(stopped, "model failure", "error", 2)).toBe(stopped);
    expect(stopQueuedAutoSend(stopped, "later successful lifecycle event", "error", 3)).toBe(stopped);
  });

  it("hides claimed messages without removing them from the durable queue", () => {
    const claims = new Set([queuedMessageKey("session_a", "a")]);
    expect(unclaimedQueuedMessages(messages.slice(0, 3), claims, "session_a").map(({ id }) => id)).toEqual(["b", "c"]);
    expect(messages.slice(0, 3).map(({ id }) => id)).toEqual(["a", "b", "c"]);
    releaseQueuedMessageClaim(claims, "session_a", "a");
    expect(unclaimedQueuedMessages(messages.slice(0, 3), claims, "session_a").map(({ id }) => id)).toEqual(["a", "b", "c"]);
  });

 it("keeps multiple per-message worker roles bound through durable queue storage", () => {
 const values = new Map<string, string>();
 const storage = {
 get length() { return values.size; },
 key: (index: number) => Array.from(values.keys())[index] ?? null,
 getItem: (key: string) => values.get(key) ?? null,
 setItem: (key: string, value: string) => { values.set(key, value); },
 removeItem: (key: string) => { values.delete(key); },
 };
 const message: QueuedMessage = { id: "role-message", text: "review this", createdAtMs: 1, attachmentIds: [], roleIds: ["role-reviewer", "role-tester"] };
 expect(saveQueuedMessages(storage, "scope", { session_a: [message] })).toBe(true);
 expect(loadQueuedMessages(storage, "scope")).toEqual({ session_a: [message] });
 });
 it("upgrades a legacy single worker role while loading durable queue storage", () => {
 const values = new Map<string, string>();
 const scope = "legacy-role";
 const key = queuedMessagesStorageKey(scope, "legacy-message");
 values.set(key, JSON.stringify({ sessionId: "session_a", position: 0, message: { id: "legacy-message", text: "review", createdAtMs: 1, attachmentIds: [], roleId: "role-reviewer" } }));
 const storage = { get length() { return values.size; }, key: (index: number) => Array.from(values.keys())[index] ?? null, getItem: (storageKey: string) => values.get(storageKey) ?? null };
 expect(loadQueuedMessages(storage, scope).session_a[0].roleIds).toEqual(["role-reviewer"]);
 });
 it("keeps attachment ids bound to their queued message", () => {
 const ids = reservedQueuedAttachmentIds([
 { id: "one", text: "first", createdAtMs: 1, attachmentIds: ["upload-a", "upload-b"] },
 { id: "two", text: "second", createdAtMs: 2, attachmentIds: ["upload-b", "upload-c"] },
 ]);
 expect(ids).toEqual(new Set(["upload-a", "upload-b", "upload-c"]));
 });
  it("limits the collapsed queue to four rows", () => {
    expect(COLLAPSED_QUEUE_LIMIT).toBe(4);
    expect(messages.slice(0, COLLAPSED_QUEUE_LIMIT).map(({ id }) => id)).toEqual(["a", "b", "c", "d"]);
  });

  it("moves a dragged message to the sortable target position without mutation", () => {
    expect(reorderQueuedMessages(messages, "d", "b").map(({ id }) => id)).toEqual(["a", "d", "b", "c", "e"]);
    expect(reorderQueuedMessages(messages, "b", "d").map(({ id }) => id)).toEqual(["a", "c", "d", "b", "e"]);
    expect(messages.map(({ id }) => id)).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("keeps the queue unchanged for stale drag identifiers", () => {
    expect(reorderQueuedMessages(messages, "missing", "b")).toEqual(messages);
  });

  it("lets only one competing immediate or automatic dispatch claim a message", () => {
    const claims = new Set<string>();
    expect(claimQueuedMessage(claims, "session_a", messages, "a")).toBe(true);
    expect(claimQueuedMessage(claims, "session_a", messages, "a")).toBe(false);
    expect(claimQueuedMessage(claims, "session_b", messages, "a")).toBe(true);
    expect(claimQueuedMessage(claims, "session_a", messages, "missing")).toBe(false);
    expect(releaseQueuedMessageClaim(claims, "session_a", "a")).toBe(true);
    expect(claimQueuedMessage(claims, "session_a", messages, "a")).toBe(true);
  });

  it("clears only the stopped session queue and releases its claims", () => {
    const queues = {
      session_a: messages.slice(0, 2),
      session_b: messages.slice(2, 4),
    };
    const claims = new Set([
      queuedMessageKey("session_a", "a"),
      queuedMessageKey("session_a", "b"),
      queuedMessageKey("session_b", "c"),
    ]);

    expect(clearSessionQueuedMessages(queues, "session_a")).toEqual({
      session_b: messages.slice(2, 4),
    });
    expect(releaseSessionQueuedMessageClaims(claims, "session_a")).toBe(2);
    expect(claims).toEqual(new Set([queuedMessageKey("session_b", "c")]));
    expect(queues.session_a.map(({ id }) => id)).toEqual(["a", "b"]);
  });

  it("blocks delete and reorder while a stable message id is claimed", () => {
    const claims = new Set([queuedMessageKey("session_a", "b")]);
    expect(removeQueuedMessage(messages, "b", claims, "session_a")).toEqual(messages);
    expect(reorderQueuedMessages(messages, "b", "d", claims, "session_a")).toEqual(messages);
    expect(reorderQueuedMessages(messages, "d", "b", claims, "session_a")).toEqual(messages);
    expect(removeQueuedMessage(messages, "c", claims, "session_a").map(({ id }) => id)).toEqual(["a", "b", "d", "e"]);
  });

  it("deletes and reorders strictly by id after neighboring rows move", () => {
    const reordered = reorderQueuedMessages(messages, "e", "b");
    expect(removeQueuedMessage(reordered, "c").map(({ id }) => id)).toEqual(["a", "e", "b", "d"]);
  });

  it("persists pending and rejected messages within a mem-scoped queue", () => {
    const values = new Map<string, string>();
    const storage = {
      get length() { return values.size; },
      key: (index: number) => Array.from(values.keys())[index] ?? null,
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
      removeItem: (key: string) => { values.delete(key); },
    };
    const rejected = { ...messages[0], deliveryError: "rejected" };
    expect(saveQueuedMessages(storage, "host\u0000/mem/a", { session_a: [rejected] })).toBe(true);
    expect(loadQueuedMessages(storage, "host\u0000/mem/a")).toEqual({ session_a: [rejected] });
    expect(loadQueuedMessages(storage, "host\u0000/mem/b")).toEqual({});
    expect(queuedMessagesStorageKey("host\u0000/mem/a")).not.toBe(queuedMessagesStorageKey("host\u0000/mem/b"));
  });

  it("stores concurrent tab messages as independent records without last-writer loss", () => {
    const values = new Map<string, string>();
    const storage = {
      get length() { return values.size; },
      key: (index: number) => Array.from(values.keys())[index] ?? null,
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => { values.set(key, value); },
      removeItem: (key: string) => { values.delete(key); },
    };
    const scope = "host\u0000/mem/a";
    expect(saveQueuedMessages(storage, scope, { session_a: [{ id: "tab-a", text: "A", createdAtMs: 1 }] })).toBe(true);
    expect(saveQueuedMessages(storage, scope, { session_b: [{ id: "tab-b", text: "B", createdAtMs: 2 }] })).toBe(true);
    expect(loadQueuedMessages(storage, scope)).toEqual({
      session_a: [{ id: "tab-a", text: "A", createdAtMs: 1, attachmentIds: [] }],
      session_b: [{ id: "tab-b", text: "B", createdAtMs: 2, attachmentIds: [] }],
    });
  });

  it("does not expose a queue mutation when storage fails", () => {
    const storage = {
      setItem: () => { throw new Error("quota"); },
      removeItem: () => undefined,
    };
    expect(saveQueuedMessages(storage, "scope", { session_a: [{ id: "a", text: "A", createdAtMs: 1 }] })).toBe(false);
  });

  it("writes the next queue before removing old records so quota failure cannot erase the durable message", () => {
    const operations: string[] = [];
    const storage = {
      setItem: () => { operations.push("set"); throw new Error("quota"); },
      removeItem: () => { operations.push("remove"); },
    };
    const previous = { session_a: [{ id: "old", text: "Old", createdAtMs: 1 }] };
    const next = { session_a: [{ id: "new", text: "New", createdAtMs: 2 }] };
    expect(saveQueuedMessages(storage, "scope", next, previous)).toBe(false);
    expect(operations).toEqual(["set"]);
  });

  it("applies out-of-order acknowledgements only to their owning session", () => {
    let queues = {
      session_a: [{ id: "a1", text: "A1", createdAtMs: 1 }, { id: "a2", text: "A2", createdAtMs: 2 }],
      session_b: [{ id: "b1", text: "B1", createdAtMs: 3 }],
      session_c: [{ id: "c1", text: "C1", createdAtMs: 4 }],
    };
    queues = applyQueuedMessagesAck(queues, "b1", "committed", undefined, "unused").queues;
    expect(queues.session_a.map(({ id }) => id)).toEqual(["a1", "a2"]);
    expect(queues.session_b).toEqual([]);
    expect(queues.session_c.map(({ id }) => id)).toEqual(["c1"]);
    queues = applyQueuedMessagesAck(queues, "a1", "accepted", undefined, "unused").queues;
    expect(queues.session_a.map(({ id }) => id)).toEqual(["a1", "a2"]);
    queues = applyQueuedMessagesAck(queues, "c1", "rejected", "busy", "c-retry").queues;
    expect(queues.session_c).toEqual([{ id: "c-retry", text: "C1", createdAtMs: 4, deliveryError: "busy" }]);
    expect(queues.session_a.map(({ id }) => id)).toEqual(["a1", "a2"]);
  });

  it("makes duplicate and stale terminal acknowledgements harmless", () => {
    const queues = { session_a: [{ id: "a1", text: "A1", createdAtMs: 1 }] };
    const committed = applyQueuedMessagesAck(queues, "a1", "committed", undefined, "unused");
    expect(committed.matchedSessionId).toBe("session_a");
    const duplicate = applyQueuedMessagesAck(committed.queues, "a1", "committed", undefined, "unused");
    expect(duplicate.matchedSessionId).toBeUndefined();
    expect(duplicate.queues).toEqual({ session_a: [] });
  });

  it("keeps claims session-scoped across active-session switches", () => {
    const claims = new Set<string>();
    expect(claimQueuedMessage(claims, "session_a", [{ id: "same", text: "A", createdAtMs: 1 }], "same")).toBe(true);
    expect(claimQueuedMessage(claims, "session_b", [{ id: "same", text: "B", createdAtMs: 2 }], "same")).toBe(true);
    expect(removeQueuedMessage([{ id: "same", text: "B", createdAtMs: 2 }], "same", claims, "session_b")).toHaveLength(1);
    expect(releaseQueuedMessageClaim(claims, "session_a", "same")).toBe(true);
    expect(claims.has(queuedMessageKey("session_b", "same"))).toBe(true);
  });

  it("does not let delete, edit, or reorder win against an in-flight claim", () => {
    const claims = new Set([queuedMessageKey("session_a", "a")]);
    const original = messages.slice(0, 3);
    expect(removeQueuedMessage(original, "a", claims, "session_a")).toEqual(original);
    expect(reorderQueuedMessages(original, "a", "c", claims, "session_a")).toEqual(original);
    expect(reorderQueuedMessages(original, "c", "a", claims, "session_a")).toEqual(original);
  });

  it("routes background auto-dispatch by owning session rather than active UI session", () => {
    const sessions = [
      { session_id: "session_a", state: "ready" },
      { session_id: "session_b", state: "working" },
      { session_id: "session_c", state: "ready" },
    ];
    const queues = {
      session_a: [{ id: "a1", text: "A next", createdAtMs: 1 }],
      session_b: [{ id: "b1", text: "B next", createdAtMs: 2 }],
      session_c: [{ id: "c1", text: "C retry", createdAtMs: 3, deliveryError: "rejected" }],
    };
    expect(selectQueuedDispatches(sessions, queues, new Set(), "session_b")).toEqual([
      { sessionId: "session_a", message: queues.session_a[0] },
    ]);
  });

  it("holds the second message across committed ack until authoritative lifecycle unlocks its session", () => {
    const queues = { session_a: [{ id: "a2", text: "second", createdAtMs: 2 }] };
    const dispatching = new Set(["session_a"]);
    expect(selectQueuedDispatches([{ session_id: "session_a", state: "ready" }], queues, dispatching)).toEqual([]);
    expect(selectQueuedDispatches([{ session_id: "session_a", state: "working" }], queues, new Set())).toEqual([]);
    expect(selectQueuedDispatches([{ session_id: "session_a", state: "ready" }], queues, new Set())).toEqual([
      { sessionId: "session_a", message: queues.session_a[0] },
    ]);
  });

  it("keeps simultaneous session dispatch locks independent under interleaved lifecycle and rejection", () => {
    const queues = {
      session_a: [{ id: "a1", text: "A next", createdAtMs: 1 }],
      session_b: [{ id: "b1", text: "B next", createdAtMs: 2 }],
      session_c: [{ id: "c-retry", text: "C retry", createdAtMs: 3, deliveryError: "busy" }],
    };
    const dispatching = new Set<string>();
    let sessions = [
      { session_id: "session_a", state: "working" },
      { session_id: "session_b", state: "working" },
      { session_id: "session_c", state: "ready" },
    ];
    expect(selectQueuedDispatches(sessions, queues, dispatching)).toEqual([]);

    // A finishes while B is still working. Neither B nor C's rejected item can
    // prevent A from becoming independently dispatchable.
    sessions = sessions.map((session) => session.session_id === "session_a" ? { ...session, state: "ready" } : session);
    expect(selectQueuedDispatches(sessions, queues, dispatching)).toEqual([
      { sessionId: "session_a", message: queues.session_a[0] },
    ]);
    dispatching.add("session_a");

    // B finishing remains independently dispatchable while A awaits its
    // authoritative working transition/terminal lifecycle event.
    sessions = sessions.map((session) => session.session_id === "session_b" ? { ...session, state: "ready" } : session);
    expect(selectQueuedDispatches(sessions, queues, dispatching)).toEqual([
      { sessionId: "session_b", message: queues.session_b[0] },
    ]);
    dispatching.add("session_b");

    // An authoritative working transition releases only A's transient lock.
    sessions = sessions.map((session) => session.session_id === "session_a" ? { ...session, state: "working" } : session);
    dispatching.delete("session_a");
    expect(dispatching).toEqual(new Set(["session_b"]));
    expect(selectQueuedDispatches(sessions, queues, dispatching)).toEqual([]);
  });

 it("persists and clears a mem-scoped global pause without changing queued messages", () => {
 const values = new Map<string, string>();
 const storage = {
 get length() { return values.size; },
 key: (index: number) => Array.from(values.keys())[index] ?? null,
 getItem: (key: string) => values.get(key) ?? null,
 setItem: (key: string, value: string) => { values.set(key, value); },
 removeItem: (key: string) => { values.delete(key); },
 };
 expect(saveQueuedMessages(storage, "scope-a", { session_a: messages.slice(0, 2) })).toBe(true);
 expect(saveQueuedMessagesPause(storage, "scope-a", {
 paused: true,
 source: "user",
 reason: "用户关闭了自动发送",
 stoppedAtMs: 123,
 })).toBe(true);
 expect(loadQueuedMessagesPause(storage, "scope-a")).toEqual({
 paused: true,
 source: "user",
 reason: "用户关闭了自动发送",
 stoppedAtMs: 123,
 });
 expect(loadQueuedMessagesPause(storage, "scope-b")).toBeNull();
 expect(loadQueuedMessages(storage, "scope-a").session_a.map(({ id }) => id)).toEqual(["a", "b"]);
 expect(clearQueuedMessagesPause(storage, "scope-a")).toBe(true);
 expect(loadQueuedMessagesPause(storage, "scope-a")).toBeNull();
 expect(loadQueuedMessages(storage, "scope-a").session_a.map(({ id }) => id)).toEqual(["a", "b"]);
 });

 it("ignores malformed queue pause records", () => {
 const values = new Map<string, string>();
 const storage = { getItem: (key: string) => values.get(key) ?? null };
 const key = queuedMessagesPauseStorageKey("scope-a");
 for (const malformed of [
 "not-json",
 "{}",
 JSON.stringify({ paused: false, stoppedAtMs: 1 }),
 JSON.stringify({ paused: true, stoppedAtMs: "now" }),
 JSON.stringify({ paused: true, stoppedAtMs: -1 }),
 JSON.stringify({ paused: true, stoppedAtMs: 1, reason: 42 }),
 JSON.stringify({ paused: true, stoppedAtMs: 1, source: "system" }),
 ]) {
 values.set(key, malformed);
 expect(loadQueuedMessagesPause(storage, "scope-a")).toBeNull();
 }
 });
});
