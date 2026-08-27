import { describe, expect, it } from "vitest";
import { WireEvent } from "../src/protocol";
import { enablesSemanticDelivery, isDirectWireEvent, shouldReduceTopLevelWireEvent } from "../src/wire_delivery";

const authoritative = { type: "session_renamed", session_id: "session-a", display_name: "A2" } as const;

describe("production wire delivery contract", () => {
  it("keeps legacy raw authoritative events compatible before semantic delivery is advertised", () => {
    const hello = { type: "hello", snapshot: {} } as WireEvent;
    expect(enablesSemanticDelivery(hello)).toBe(false);
    expect(shouldReduceTopLevelWireEvent(authoritative, false)).toBe(true);
  });

  it("drops a raw authoritative duplicate on a cursor-capable Host", () => {
    const hello = { type: "hello", snapshot: {}, event_cursor: 0 } as WireEvent;
    expect(enablesSemanticDelivery(hello)).toBe(true);
    expect(shouldReduceTopLevelWireEvent(authoritative, true)).toBe(false);
    expect(shouldReduceTopLevelWireEvent({ type: "semantic_event", event_seq: 1, event: authoritative }, true)).toBe(true);
  });

  it("reduces an authoritative mutation exactly once when raw and semantic copies are both received", () => {
    const events: WireEvent[] = [
      { type: "hello", snapshot: {} as never, event_cursor: 0 },
      authoritative,
      { type: "semantic_event", event_seq: 1, event: authoritative },
      authoritative,
    ];
    let semanticDelivery = false;
    let mutationReductions = 0;
    for (const event of events) {
      if (enablesSemanticDelivery(event)) semanticDelivery = true;
      if (!shouldReduceTopLevelWireEvent(event, semanticDelivery)) continue;
      const reduced = event.type === "semantic_event" ? event.event : event;
      if (reduced.type === "session_renamed") mutationReductions += 1;
    }
    expect(mutationReductions).toBe(1);
  });

  it("also enables the duplicate gate when a semantic envelope is observed first", () => {
    const envelope = { type: "semantic_event", event_seq: 1, event: authoritative } as WireEvent;
    expect(enablesSemanticDelivery(envelope)).toBe(true);
    expect(shouldReduceTopLevelWireEvent(authoritative, enablesSemanticDelivery(envelope))).toBe(false);
  });

  it.each([
    { type: "command_ack", command_id: "cmd-a", status: "committed" },
    { type: "host_error", message: "query_failed" },
    { type: "runtime_notice", session_id: "session-a", level: "warning", title: "Runtime warning", message: "persist_failed" },
    { type: "history_page", session_id: "session-a", records: [], has_more: false },
    { type: "chat_search_result", query: "release", hits: [] },
    { type: "favorites_list", favorites: [], capacity: { used_bytes: 0, limit_bytes: 268435456, used_percent: 0 } },
    { type: "favorite_created", favorite: {} as never, capacity: { used_bytes: 1, limit_bytes: 268435456, used_percent: 1 }, nearing_limit: false },
    { type: "favorite_capacity_reached", capacity: { used_bytes: 268435456, limit_bytes: 268435456, used_percent: 100 } },
    { type: "favorite_capacity_updated", capacity: { used_bytes: 0, limit_bytes: null, used_percent: null } },
    { type: "favorite_deleted", favorite_id: "favorite-a" },
    { type: "session_api_key_revealed", session_id: "session-a", api_key: "secret" },
    { type: "mcp_server_secrets_revealed", server_id: "mcp-a", values: {} },
    { type: "model_endpoint_secret_revealed", endpoint_id: "endpoint-a", api_key: "secret" },
    { type: "tool_repo_search_result", session_id: "session-a", query: "q", tools: [] },
    { type: "tool_repo_detail", session_id: "session-a", detail: {} },
  ] as WireEvent[])("continues reducing direct $type responses in semantic mode", (event) => {
    expect(isDirectWireEvent(event)).toBe(true);
    expect(shouldReduceTopLevelWireEvent(event, true)).toBe(true);
  });

  it.each([
    "session_created",
    "session_renamed",
    "session_deleted",
    "session_runtime_updated",
    "session_runtime_config_updated",
    "core_topic",
    "worker_activity",
    "turn_finished",
    "turn_started",
    "turn_updated",
    "host_config_updated",
    "file_uploaded",
    "attachment_removed",
    "tool_repo_updated",
    "mcp_updated",
  ] as WireEvent["type"][])("requires semantic envelopes for authoritative %s events", (type) => {
    const event = { type } as WireEvent;
    expect(isDirectWireEvent(event)).toBe(false);
    expect(shouldReduceTopLevelWireEvent(event, true)).toBe(false);
  });
});
