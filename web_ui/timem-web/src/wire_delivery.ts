import { WireEvent } from "./protocol";

/**
 * Events which are intentionally connection-local and therefore never enter
 * the durable semantic journal. Everything else (apart from the transport
 * envelopes themselves) is authoritative state and must arrive in a
 * semantic_event envelope on a cursor-capable Host.
 */
const DIRECT_EVENT_TYPES: ReadonlySet<WireEvent["type"]> = new Set([
  "command_ack",
  "host_error",
  "runtime_notice",
  "history_page",
  "mcp_server_secrets_revealed",
  "session_api_key_revealed",
  "tool_repo_detail",
  "tool_repo_search_result",
]);

export function enablesSemanticDelivery(event: WireEvent): boolean {
  return event.type === "semantic_event"
    || (event.type === "hello" && event.event_cursor !== undefined);
}

export function isDirectWireEvent(event: WireEvent): boolean {
  return DIRECT_EVENT_TYPES.has(event.type);
}

/**
 * Legacy Hosts have no cursor and deliver authoritative events directly. Once
 * a cursor-capable Host is detected, accepting a raw authoritative event would
 * apply the same mutation twice when its journal envelope arrives as well.
 */
export function shouldReduceTopLevelWireEvent(event: WireEvent, semanticDelivery: boolean): boolean {
  if (!semanticDelivery) return true;
  return event.type === "hello"
    || event.type === "semantic_event"
    || isDirectWireEvent(event);
}
