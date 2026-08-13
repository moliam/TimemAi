import { describe, expect, it } from "vitest";
import { claimQueuedMessage, COLLAPSED_QUEUE_LIMIT, QueuedMessage, queuedMessageKey, releaseQueuedMessageClaim, removeQueuedMessage, reorderQueuedMessages } from "../src/queued_messages";

const messages: QueuedMessage[] = ["a", "b", "c", "d", "e"].map((id, index) => ({
  id,
  text: `message ${id}`,
  createdAtMs: index,
}));

describe("queued messages", () => {
  it("limits the collapsed queue to four rows", () => {
    expect(COLLAPSED_QUEUE_LIMIT).toBe(4);
    expect(messages.slice(0, COLLAPSED_QUEUE_LIMIT).map(({ id }) => id)).toEqual(["a", "b", "c", "d"]);
  });

  it("moves a dragged message to the target position without mutation", () => {
    expect(reorderQueuedMessages(messages, "d", "b").map(({ id }) => id)).toEqual(["a", "d", "b", "c", "e"]);
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
});
