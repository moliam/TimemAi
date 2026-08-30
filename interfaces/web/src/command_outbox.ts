import { ClientCommand, CommandWithId } from "./protocol";

export type CommandDeliveryStatus = "pending" | "accepted";

export type CommandOutboxItem = {
  commandId: string;
  command: CommandWithId;
  status: CommandDeliveryStatus;
  createdAtMs: number;
};

const STORAGE_PREFIX = "timem-web-command-outbox:v2";
const MAX_STORED_OUTBOX_ITEMS = 4_096;
const MAX_STORED_COMMAND_BYTES = 1024 * 1024;

const BEST_EFFORT_COMMANDS = new Set<ClientCommand["type"]>([
  "session_api_key_reveal",
  "history_page",
  "tool_repo_search",
  "tool_repo_detail",
  "tool_repo_open_terminal",
  "mcp_server_secrets_reveal",
  "model_endpoint_secret_reveal",
  "mem_temporary_items_list",
]);

const CLIENT_COMMAND_TYPES = new Set<ClientCommand["type"]>([
  "session_create",
  "session_rename",
  "session_group_create",
  "session_group_update",
  "session_group_delete",
  "session_groups_reorder",
  "session_group_move",
  "session_api_key_update",
  "session_api_key_reveal",
  "session_stop",
  "session_delete",
  "chat_message_delete",
  "worker_role_create",
  "worker_role_update",
  "worker_role_delete",
  "turn_submit",
  "turn_supplement",
  "turn_cancel",
  "attachment_remove",
  "history_page",
  "tool_repo_search",
  "tool_repo_detail",
  "tool_repo_rename",
  "tool_repo_open_terminal",
  "runtime_update",
  "session_runtime_update",
  "mcp_server_upsert",
  "mcp_server_delete",
  "mcp_session_toggle",
  "mcp_server_reconnect",
  "mcp_server_secrets_reveal",
  "model_endpoint_upsert",
  "model_endpoint_delete",
  "model_endpoint_apply",
  "model_endpoint_secret_reveal",
  "mem_switch",
  "mem_temporary_retention_update",
  "mem_conversation_capacity_update",
  "mem_temporary_items_list",
  "mem_temporary_items_delete",
  "topic_reply",
]);

export function reliableStorageScope(origin: string, memSpaceDir: string) {
  return `${origin}\u0000${memSpaceDir}`;
}

export function commandOutboxStorageKey(scope: string, commandId?: string) {
  const base = `${STORAGE_PREFIX}:${encodeURIComponent(scope)}`;
  return commandId === undefined
    ? base
    : `${base}:${encodeURIComponent(commandId)}`;
}

export function commandNeedsReliableDelivery(command: ClientCommand) {
  return !BEST_EFFORT_COMMANDS.has(command.type);
}

export function commandMayPersist(command: ClientCommand) {
  // API keys and MCP transport headers/env may contain credentials. They stay in
  // the in-memory outbox across reconnects, but must never be written to browser storage.
  return (
    command.type !== "session_api_key_update" &&
    command.type !== "mcp_server_upsert" &&
    command.type !== "model_endpoint_upsert" &&
    !(
      command.type === "session_create" &&
      Object.keys(command.env ?? {}).length > 0
    )
  );
}

export function addCommandToOutbox(
  items: readonly CommandOutboxItem[],
  command: ClientCommand,
  commandId: string,
  createdAtMs = Date.now(),
) {
  if (items.some((item) => item.commandId === commandId)) return [...items];
  return orderCommandOutbox([
    ...items,
    {
      commandId,
      command: { ...command, command_id: commandId },
      status: "pending" as const,
      createdAtMs,
    },
  ]);
}

export function acceptOutboxCommand(
  items: readonly CommandOutboxItem[],
  commandId: string,
) {
  return items.map((item) =>
    item.commandId === commandId
      ? { ...item, status: "accepted" as const }
      : item,
  );
}

export function finishOutboxCommand(
  items: readonly CommandOutboxItem[],
  commandId: string,
) {
  return items.filter((item) => item.commandId !== commandId);
}

function parseCommandOutboxItem(raw: string | null): CommandOutboxItem | null {
  try {
    if (!raw || raw.length > MAX_STORED_COMMAND_BYTES) return null;
    const value = JSON.parse(raw) as Partial<CommandOutboxItem>;
    if (!value || typeof value !== "object") return null;
    return typeof value.commandId === "string" &&
      typeof value.createdAtMs === "number" &&
      (value.status === "pending" || value.status === "accepted") &&
      !!value.command &&
      typeof value.command === "object" &&
      value.command.command_id === value.commandId &&
      typeof value.command.type === "string" &&
      CLIENT_COMMAND_TYPES.has(value.command.type as ClientCommand["type"]) &&
      commandMayPersist(value.command as ClientCommand)
      ? (value as CommandOutboxItem)
      : null;
  } catch {
    return null;
  }
}

export function orderCommandOutbox(items: readonly CommandOutboxItem[]) {
  const ordered = [...items].sort(
    (left, right) =>
      left.createdAtMs - right.createdAtMs ||
      left.commandId.localeCompare(right.commandId),
  );
  const byId = new Map(ordered.map((item) => [item.commandId, item]));
  const emitted = new Set<string>();
  const result: CommandOutboxItem[] = [];
  const emit = (item: CommandOutboxItem, visiting: Set<string>) => {
    if (emitted.has(item.commandId)) return;
    if (visiting.has(item.commandId)) return;
    visiting.add(item.commandId);
    if (item.command.type === "turn_cancel") {
      const target = item.command.target_command_id;
      if (target) {
        const dependency = byId.get(target);
        if (dependency) emit(dependency, visiting);
      }
    }
    visiting.delete(item.commandId);
    emitted.add(item.commandId);
    result.push(item);
  };
  for (const item of ordered) emit(item, new Set());
  return result;
}

export function pendingTurnCancelTargetCommandIds(
  items: readonly CommandOutboxItem[],
) {
  const result: Record<string, string> = {};
  for (const item of orderCommandOutbox(items)) {
    if (item.command.type !== "turn_cancel") continue;
    if (!item.command.target_command_id) continue;
    result[item.command.session_id] = item.command.target_command_id;
  }
  return result;
}

export function pendingTurnSubmitCommandIds(
  items: readonly CommandOutboxItem[],
) {
  const result: Record<string, string> = {};
  for (const item of orderCommandOutbox(items)) {
    if (item.command.type !== "turn_submit") continue;
    result[item.command.session_id] = item.commandId;
  }
  return result;
}

export function loadCommandOutbox(
  storage: Pick<Storage, "length" | "key" | "getItem">,
  scope: string,
): CommandOutboxItem[] {
  const prefix = `${commandOutboxStorageKey(scope)}:`;
  const items: CommandOutboxItem[] = [];
  for (
    let index = 0;
    index < storage.length && items.length < MAX_STORED_OUTBOX_ITEMS;
    index += 1
  ) {
    const key = storage.key(index);
    if (!key?.startsWith(prefix)) continue;
    const item = parseCommandOutboxItem(storage.getItem(key));
    if (item && commandOutboxStorageKey(scope, item.commandId) === key)
      items.push(item);
  }
  return orderCommandOutbox(items);
}

export function saveCommandOutboxItem(
  storage: Pick<Storage, "setItem">,
  scope: string,
  item: CommandOutboxItem,
) {
  try {
    storage.setItem(
      commandOutboxStorageKey(scope, item.commandId),
      JSON.stringify(item),
    );
    return true;
  } catch {
    return false;
  }
}

export function removeCommandOutboxItem(
  storage: Pick<Storage, "removeItem">,
  scope: string,
  commandId: string,
) {
  try {
    storage.removeItem(commandOutboxStorageKey(scope, commandId));
    return true;
  } catch {
    return false;
  }
}
