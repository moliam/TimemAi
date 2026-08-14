export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  text: string;
  created_at_ms: number;
  completion?: TurnCompletion;
};

let clientIdSequence = 0;

export function clientId(prefix = "client") {
  const randomUuid = globalThis.crypto?.randomUUID;
  if (typeof randomUuid === "function") {
    return `${prefix}-${randomUuid.call(globalThis.crypto)}`;
  }

  const random = new Uint32Array(2);
  if (globalThis.crypto?.getRandomValues) {
    globalThis.crypto.getRandomValues(random);
  } else {
    random[0] = Math.floor(Math.random() * 0x1_0000_0000);
    random[1] = Math.floor(Math.random() * 0x1_0000_0000);
  }
  clientIdSequence = (clientIdSequence + 1) >>> 0;
  return `${prefix}-${Date.now().toString(36)}-${random[0].toString(36)}${random[1].toString(36)}-${clientIdSequence.toString(36)}`;
}

export type UsageStats = {
  llm_calls?: number;
  repair_calls?: number;
  tool_calls?: number;
  mem_reads?: number;
  mem_writes?: number;
  prompt_tokens?: number;
  completion_tokens?: number;
  total_tokens?: number;
  cached_tokens?: number;
  cache_created_tokens?: number;
  shrunk_tokens?: number;
};

export type TurnCompletion = {
  stats?: UsageStats;
  latest_usage?: UsageStats | null;
  elapsed_ms?: number;
  repair_issue?: string | null;
  stop_reason?: string | null;
  toolgen_retrospect?: string | null;
};

export type ToolSummary = {
  tool_id: string;
  name: string;
  tool_type: string;
  language: string;
  synopsis: string;
  entrypoint: string;
  path: string;
  updated_at_ms: number;
  status: "ready" | string;
};

export type ToolDetail = {
  summary: ToolSummary;
  readme: string;
  files: Array<{ path: string; bytes: number }>;
};

export type Session = {
  session_id: string;
  display_name: string;
  ordinal: number;
  state: "ready" | "working" | "error" | "stopped" | string;
  current_dir: string;
  max_llm_input_tokens: number;
  tools: ToolSummary[];
  mcp_server_ids: string[];
  runtime_profile?: {
    model: string;
    api_protocol: string;
    response_protocol: string;
    base_url: string;
    timeout_secs: number;
    max_llm_input_tokens: number;
    max_llm_output_tokens: number;
    bash_approval: string;
    work_instructions: string;
    api_key_configured: boolean;
  };
  contexts: SessionContext[];
  workers: SessionWorker[];
  active_context_id: string;
  primary_worker_id: string;
  attachments: Attachment[];
  messages: ChatMessage[];
  turns: WebTurn[];
  history_before_cursor?: string | null;
  history_has_more?: boolean;
  active_turn_id?: string | null;
};

export type McpTransport =
  | { type: "stdio"; command: string; args: string[]; env: Record<string, string> }
  | { type: "streamable_http"; url: string; headers: Record<string, string> }
  | { type: "sse"; url: string; headers: Record<string, string> };

export type McpServerConfig = {
  id: string;
  name: string;
  enabled: boolean;
  transport: McpTransport;
  request_timeout_ms: number;
};

export type McpTool = { server_id: string; server_name: string; name: string; action_name: string; description: string; input_schema: unknown };
export type McpServerReport = { config: McpServerConfig; state: string; error?: string | null; tools: McpTool[] };

export type SessionContext = {
  context_id: string;
  current_dir: string;
  worker_ids: string[];
};

export type SessionWorker = {
  worker_id: string;
  context_id: string;
  display_name: string;
  ordinal: number;
  state: "ready" | "working" | "error" | "stopped" | string;
  parent_worker_id?: string | null;
};

export type WebTurn = {
  turn_id: string;
  state: string;
  created_at_ms: number;
  user_entries: WebTurnUserEntry[];
  events: WebTurnEvent[];
  final_answer?: string | null;
  completion?: TurnCompletion | null;
};

export type WebTurnUserEntry = { kind: "task" | "supplement" | "approval" | string; text: string; attachments?: Attachment[]; created_at_ms: number };
export type WebTurnEvent = { event_id: string; source: "core_topic" | "worker_activity" | string; payload: Record<string, unknown>; created_at_ms: number };

export type Attachment = { id: string; name: string; path: string; bytes: number };

export type ChatHistoryRecord =
  | { type: "message"; role: "user" | "assistant" | "system"; turn_id: string; created_at_ms: number; content: string; kind?: WebTurnUserEntry["kind"] }
  | { type: "event"; role: "user" | "assistant" | "system"; turn_id: string; created_at_ms: number; kind: string; content: string; [key: string]: unknown };

export type CoreTopicEvent = {
  session_id: string;
  context_id?: string | null;
  worker_id?: string | null;
  topic: { name: string; attributes?: Record<string, unknown> };
  state: { name: string; timeout_ms?: number };
  payload: Record<string, unknown>;
};

export type Activity = {
  id: string;
  sessionId: string;
  tone: "thinking" | "action" | "notice" | "warning" | "error";
  title: string;
  detail?: string;
  code?: string;
  code_language?: string;
  tool_name?: string;
  tool_status?: string;
  elapsed_ms?: number;
  kind?: "context_compact" | "toolgen";
  toolgen_phase?: string;
  before_tokens?: number;
  after_tokens?: number;
  createdAt: number;
};

export type Decision = {
  event: CoreTopicEvent;
  turnId?: string;
  title: string;
  detail: string;
};

export type Snapshot = {
  server: {
    version: string;
    protocol_version: number;
    port: number;
    bind_host: string;
    public_access: boolean;
    mem: {
      space: string;
      data_dir: string;
      space_dir: string;
      memory_dir: string;
    };
    runtime_options: Array<{ key: string; value: string; applies_to: "new_sessions" | string }>;
    session_env_defaults: Record<string, string>;
    workspace_dirs: string[];
    mcp_servers: McpServerReport[];
  };
  sessions: Session[];
};

export type WireEvent =
  | { type: "hello"; snapshot: Snapshot; event_cursor?: number; event_replay_floor?: number }
  | { type: "semantic_event"; event_seq: number; event: WireEvent }
  | { type: "command_ack"; command_id: string; status: "accepted" | "committed" | "rejected"; error?: string }
  | { type: "session_created"; session: Session }
  | { type: "session_renamed"; session_id: string; display_name: string }
  | { type: "session_deleted"; session_id: string }
  | { type: "session_runtime_updated"; session_id: string; runtime_profile: NonNullable<Session["runtime_profile"]> }
  | { type: "session_runtime_config_updated"; session_id: string; key: string; value: string; runtime_profile: NonNullable<Session["runtime_profile"]> }
  | { type: "session_api_key_revealed"; session_id: string; api_key: string }
  | { type: "core_topic"; turn_id?: string | null; turn_event_id?: string | null; event: CoreTopicEvent }
  | { type: "worker_activity"; session_id: string; context_id: string; worker_id: string; turn_id?: string | null; turn_event_id?: string | null; event: Record<string, unknown> }
  | { type: "turn_finished"; session_id: string; turn_id?: string | null; outcome: { text?: string; message_id?: string | null; completion?: TurnCompletion } }
  | { type: "turn_updated"; session_id: string; turn: WebTurn }
  | { type: "host_error"; message: string }
  | { type: "host_config_updated"; key: string; value: string; session_env_defaults: Record<string, string> }
  | { type: "file_uploaded"; session_id: string; file: Attachment }
  | { type: "attachment_removed"; session_id: string; attachment_id: string }
  | { type: "history_page"; session_id: string; records: ChatHistoryRecord[]; before_cursor?: string | null; has_more: boolean }
  | { type: "tool_repo_updated"; session_id: string; tools: ToolSummary[] }
  | { type: "tool_repo_search_result"; session_id: string; query: string; tools: ToolSummary[] }
  | { type: "tool_repo_detail"; session_id: string; detail: ToolDetail }
  | { type: "mcp_updated"; session_id?: string | null; servers: McpServerReport[]; enabled_server_ids: string[] }
  | { type: "mcp_server_secrets_revealed"; server_id: string; values: Record<string, string> };

export type ClientCommand =
  | { type: "session_create"; display_name?: string; workspace_dir?: string; env?: Record<string, string> }
  | { type: "session_rename"; session_id: string; display_name: string }
  | { type: "session_api_key_update"; session_id: string; api_key: string }
  | { type: "session_api_key_reveal"; session_id: string }
  | { type: "session_stop"; session_id: string }
  | { type: "session_delete"; session_id: string }
  | { type: "turn_submit"; session_id: string; text: string; input_kind?: "toolgen"; source_turn_id?: string }
  | { type: "turn_supplement"; session_id: string; text: string }
  | { type: "turn_cancel"; session_id: string }
  | { type: "attachment_remove"; session_id: string; attachment_id: string }
  | { type: "history_page"; session_id: string; before_cursor?: string | null; /** Maximum complete tasks (turns), not JSONL records. */ limit?: number }
  | { type: "tool_repo_search"; session_id: string; query: string; limit?: number }
  | { type: "tool_repo_detail"; session_id: string; tool_id: string }
  | { type: "tool_repo_rename"; session_id: string; tool_id: string; new_name: string }
  | { type: "tool_repo_open_terminal"; session_id: string; tool_id: string }
  | { type: "runtime_update"; key: string; value: string }
  | { type: "session_runtime_update"; session_id: string; key: string; value: string }
  | { type: "mcp_server_upsert"; session_id: string; config: McpServerConfig }
  | { type: "mcp_server_delete"; server_id: string }
  | { type: "mcp_session_toggle"; session_id: string; server_id: string; enabled: boolean }
  | { type: "mcp_server_reconnect"; session_id: string; server_id: string }
  | { type: "mcp_server_secrets_reveal"; server_id: string }
  | { type: "mem_switch"; path: string }
  | {
      type: "topic_reply";
      session_id: string;
      worker_id?: string;
      topic_name: string;
      request_id?: string;
      decision: "accept" | "decline" | "always_allow";
      payload?: Record<string, unknown>;
    };

export type CommandWithId = ClientCommand & { command_id: string };
