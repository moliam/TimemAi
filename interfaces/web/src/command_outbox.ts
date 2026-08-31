import { ClientCommand } from "./protocol";

const UNCORRELATED_REQUESTS = new Set<ClientCommand["type"]>([
  "session_api_key_reveal",
  "history_page",
  "tool_repo_search",
  "tool_repo_detail",
  "tool_repo_open_terminal",
  "mcp_server_secrets_reveal",
  "model_endpoint_secret_reveal",
  "mem_temporary_items_list",
]);

/**
 * Returns whether a live, one-shot WebSocket command needs a correlation id.
 * This does not imply persistence, retry, replay, or browser ownership.
 */
export function commandNeedsReliableDelivery(command: ClientCommand) {
  return !UNCORRELATED_REQUESTS.has(command.type);
}
