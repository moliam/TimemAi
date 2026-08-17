import { describe, expect, it } from "vitest";
import { acceptOutboxCommand, addCommandToOutbox, commandMayPersist, commandNeedsReliableDelivery, commandOutboxStorageKey, finishOutboxCommand, loadCommandOutbox, reliableStorageScope, removeCommandOutboxItem, saveCommandOutboxItem } from "../src/command_outbox";

function memoryStorage() {
  const values = new Map<string, string>();
  return {
    get length() { return values.size; },
    key: (index: number) => Array.from(values.keys())[index] ?? null,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); },
    removeItem: (key: string) => { values.delete(key); },
  };
}

describe("reliable command outbox", () => {
  it("keeps one-shot mutations reliable while allowing read-only requests to be best effort", () => {
    expect(commandNeedsReliableDelivery({ type: "turn_submit", session_id: "s", text: "work" })).toBe(true);
    expect(commandNeedsReliableDelivery({ type: "session_delete", session_id: "s" })).toBe(true);
    expect(commandNeedsReliableDelivery({ type: "chat_message_delete", session_id: "s", turn_id: "t", role: "user", role_index: 0 })).toBe(true);
    expect(commandNeedsReliableDelivery({ type: "history_page", session_id: "s" })).toBe(false);
  });

  it("deduplicates retries by command id and removes only a terminal command", () => {
    const command = { type: "turn_supplement" as const, session_id: "s", text: "more" };
    const pending = addCommandToOutbox([], command, "command-1", 10);
    expect(addCommandToOutbox(pending, command, "command-1", 20)).toEqual(pending);
    expect(acceptOutboxCommand(pending, "command-1")[0].status).toBe("accepted");
    expect(finishOutboxCommand(pending, "unrelated")).toEqual(pending);
    expect(finishOutboxCommand(pending, "command-1")).toEqual([]);
  });

  it("persists the same command id and isolates outboxes by origin and mem", () => {
    const storage = memoryStorage();
    const scopeA = reliableStorageScope("https://host:7", "/mem/a");
    const scopeB = reliableStorageScope("https://host:7", "/mem/b");
    const items = addCommandToOutbox([], { type: "turn_submit", session_id: "s", text: "work" }, "command-1", 10);
    expect(commandOutboxStorageKey(scopeA)).not.toBe(commandOutboxStorageKey(scopeB));
    expect(saveCommandOutboxItem(storage, scopeA, items[0])).toBe(true);
    expect(loadCommandOutbox(storage, scopeA)).toEqual(items);
    expect(loadCommandOutbox(storage, scopeB)).toEqual([]);
  });

  it("never persists commands that contain credentials", () => {
    expect(commandMayPersist({ type: "session_api_key_update", session_id: "s", api_key: "secret" })).toBe(false);
    expect(commandMayPersist({ type: "mcp_server_upsert", session_id: "s", config: { id: "m", name: "M", transport: { type: "stdio", command: "x", args: [], env: {} }, enabled: true } })).toBe(false);
    expect(commandMayPersist({ type: "session_create", env: { ACCESS_TOKEN: "secret" } })).toBe(false);
  });

  it("ignores corrupt or unknown commands in browser storage", () => {
    const storage = memoryStorage();
    storage.setItem(commandOutboxStorageKey("scope", "bad"), JSON.stringify({ commandId: "bad", command: { type: "future_unknown", command_id: "bad" }, status: "pending", createdAtMs: 1 }));
    expect(loadCommandOutbox(storage, "scope")).toEqual([]);
  });

  it("never restores a sensitive command even when browser storage was manually seeded", () => {
    const storage = memoryStorage();
    storage.setItem(commandOutboxStorageKey("scope", "secret"), JSON.stringify({
      commandId: "secret",
      command: { type: "session_api_key_update", session_id: "s", api_key: "injected", command_id: "secret" },
      status: "pending",
      createdAtMs: 1,
    }));
    expect(loadCommandOutbox(storage, "scope")).toEqual([]);
  });

  it("uses independent records so interleaved tabs cannot overwrite each other's commands", () => {
    const storage = memoryStorage();
    const scope = reliableStorageScope("https://host:7", "/mem/a");
    const tabA = addCommandToOutbox([], { type: "turn_submit", session_id: "a", text: "A" }, "a-1", 1)[0];
    const tabB = addCommandToOutbox([], { type: "session_delete", session_id: "b" }, "b-1", 2)[0];
    expect(saveCommandOutboxItem(storage, scope, tabA)).toBe(true);
    expect(saveCommandOutboxItem(storage, scope, tabB)).toBe(true);
    expect(loadCommandOutbox(storage, scope).map(({ commandId }) => commandId)).toEqual(["a-1", "b-1"]);
    expect(removeCommandOutboxItem(storage, scope, "b-1")).toBe(true);
    expect(loadCommandOutbox(storage, scope).map(({ commandId }) => commandId)).toEqual(["a-1"]);
  });

  it("does not change memory when durable storage rejects a write", () => {
    const storage = { setItem: () => { throw new Error("quota"); } };
    const item = addCommandToOutbox([], { type: "turn_submit", session_id: "a", text: "A" }, "a-1", 1)[0];
    expect(saveCommandOutboxItem(storage, "scope", item)).toBe(false);
  });
});
