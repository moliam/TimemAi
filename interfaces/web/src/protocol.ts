export type ChatMessage = {
  id: string;
  role: "user" | "assistant" | "system";
  text: string;
  created_at_ms: number;
  kind?: string;
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

export type WorkerRole = { id: string; name: string; description: string };
export type WorkerRoleGroup = { id: string; name: string; role_ids: string[] };
export type WorkerRoleLibrary = {
  roles: WorkerRole[];
  groups: WorkerRoleGroup[];
};
export type SessionGroup = { id: string; name: string };

export type TurnToken = {
  session_id: string;
  turn_id: string;
  epoch: number;
};

export type TurnActivity =
  | { kind: "running" }
  | { kind: "waiting_model"; round: number }
  | { kind: "waiting_user" }
  | { kind: "running_tools" };

export type TurnProjectionOutcome =
  | { kind: "completed" }
  | { kind: "cancelled" }
  | { kind: "failed"; code: string }
  | { kind: "interrupted"; code: string };

export type TurnProjection =
  | {
      state: "active";
      token: TurnToken;
      stop_requested: boolean;
      input_admission: "open" | "closed";
      activity: TurnActivity;
    }
  | {
      state: "finished";
      token: TurnToken;
      outcome: TurnProjectionOutcome;
    };

export type VersionedTurnProjection = {
  revision: number;
  projection: TurnProjection;
};

export type MessageQueueBlockReason =
  "user_cancelled" | "turn_failed" | "turn_interrupted" | "session_stopped";

export type MessageQueueContinuation =
  | { state: "awaiting_normal_completion" }
  | { state: "granted" }
  | { state: "blocked"; reason: MessageQueueBlockReason };

export type MessageQueuePayload = {
  turn_id: string;
  created_at_ms: number;
  text: string;
  attachments: Attachment[];
  worker_roles: WorkerRole[];
};

export type MessageQueueProjection = {
  revision: number;
  items: Array<{
    command_id: string;
    enqueue_seq: number;
    payload: MessageQueuePayload;
  }>;
  auto_send_enabled: boolean;
  continuation: MessageQueueContinuation;
  dispatching_command_id?: string | null;
};

export type Session = {
  session_id: string;
  display_name: string;
  group_id?: string | null;
  ordinal: number;
  state: "ready" | "working" | "interrupted" | "error" | "stopped" | string;
  current_dir: string;
  restart_cwd_decision?: {
    runtime_cwd: string;
    session_cwd: string;
    session_cwd_available: boolean;
  } | null;
  debug_dir?: string | null;
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
    stream: boolean;
    max_rounds: string;
    bash_approval: string;
    work_instructions: string;
    api_key_configured: boolean;
  };
  contexts: SessionContext[];
  workers: SessionWorker[];
  active_context_id: string;
  primary_worker_id: string;
  attachments: Attachment[];
  roles: WorkerRole[];
  messages: ChatMessage[];
  turns: WebTurn[];
  history_before_cursor?: string | null;
  history_has_more?: boolean;
  active_turn_id?: string | null;
  /** Current Turn after the Host accepted cancellation. */
  cancelling_turn_id?: string | null;
  /** Durable Host intent recorded before Core emits TurnStarted. */
  pending_turn_id?: string | null;
  /** Exact Core lifecycle projection plus HTTP/WebSocket delivery revision. */
  turn_projection?: VersionedTurnProjection | null;
  /** Authoritative Session-owned future-message queue. */
  message_queue: MessageQueueProjection;
};

export type McpTransport =
  | {
      type: "stdio";
      command: string;
      args: string[];
      env: Record<string, string>;
    }
  | { type: "streamable_http"; url: string; headers: Record<string, string> }
  | { type: "sse"; url: string; headers: Record<string, string> };

export type McpServerConfig = {
  id: string;
  name: string;
  enabled: boolean;
  transport: McpTransport;
  request_timeout_ms: number;
};

export type McpTool = {
  server_id: string;
  server_name: string;
  name: string;
  action_name: string;
  description: string;
  input_schema: unknown;
};
export type McpServerReport = {
  config: McpServerConfig;
  state: string;
  error?: string | null;
  tools: McpTool[];
};

export type SessionContext = {
  context_id: string;
  current_dir: string;
  worker_ids: string[];
};

export type SessionWorker = {
  worker_id: string;
  context_id: string;
  display_name: string;
  group_id?: string | null;
  ordinal: number;
  state: "ready" | "working" | "error" | "stopped" | string;
  parent_worker_id?: string | null;
};

export type WebTurn = {
  turn_id: string;
  state: string;
  created_at_ms: number;
  interrupted_at_ms?: number | null;
  user_entries: WebTurnUserEntry[];
  events: WebTurnEvent[];
  sub_answers: WebSubAnswer[];
  final_answer?: string | null;
  completion?: TurnCompletion | null;
};

export type WebSubAnswer = {
  sub_answer_id: string;
  ordinal: number;
  task: string;
  answer: string;
  created_at_ms: number;
};

export type WebTurnUserEntry = {
  command_id?: string;
  kind: "task" | "supplement" | "approval" | string;
  text: string;
  attachments?: Attachment[];
  worker_roles?: WorkerRole[];
  /** Legacy history compatibility. */ worker_role?: WorkerRole;
  created_at_ms: number;
};
export type WebTurnEvent = {
  event_id: string;
  source: "core_topic" | "worker_activity" | string;
  payload: Record<string, unknown>;
  created_at_ms: number;
};

export type Attachment = {
  id: string;
  name: string;
  path: string;
  bytes: number;
};

export type ChatHistoryRecord =
  | {
      type: "message";
      role: "user" | "assistant" | "system";
      turn_id: string;
      created_at_ms: number;
      content: string;
      kind?: WebTurnUserEntry["kind"];
    }
  | {
      type: "event";
      role: "user" | "assistant" | "system";
      turn_id: string;
      created_at_ms: number;
      kind: string;
      content: string;
      [key: string]: unknown;
    };

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
  tool_mode?: string;
  elapsed_ms?: number;
  timeout_ms?: number;
  loop_timeout_ms?: number;
  interval_ms?: number;
  pid?: number;
  execution_started?: boolean;
  kind?: "context_compact" | "toolgen" | "free_talk" | "user_supplement";
  toolgen_phase?: string;
  before_tokens?: number;
  after_tokens?: number;
  text_before_tokens?: number;
  text_after_tokens?: number;
  native_before_tokens?: number;
  native_after_tokens?: number;
  createdAt: number;
};

export type Decision = {
  event: CoreTopicEvent;
  turnId?: string;
  title: string;
  detail: string;
};

export type ModelEndpoint = {
  id: string;
  name: string;
  model: string;
  api_protocol: string;
  response_protocol: string;
  base_url: string;
  max_llm_input_tokens: number;
  max_llm_output_tokens: number;
  stream: boolean;
  api_key_configured: boolean;
  http_headers: Record<string, string>;
  request_fields: Record<string, unknown>;
};

export type MemTemporaryItem = {
  id: string;
  path: string;
  kind: "shell_job" | "temporary_file" | string;
  bytes: number;
  modified_at_ms: number;
  deletable?: boolean;
  delete_reason?: string;
};

export type ChatSearchHit = {
  source_key: string;
  session_id: string;
  session_display_name: string;
  turn_id: string;
  role: "user" | "assistant";
  content: string;
  created_at_ms: number;
  favorite_id?: string | null;
};

export type ChatLibraryCapacity = {
  used_bytes: number;
  limit_bytes?: number | null;
  used_percent?: number | null;
};

export type ChatFavorite = {
  id: string;
  source_key: string;
  session_id: string;
  session_display_name: string;
  turn_id: string;
  content_snapshot: string;
  title: string;
  source_created_at_ms: number;
  created_at_ms: number;
  updated_at_ms: number;
  version: number;
  deleted?: boolean;
};

export type Snapshot = {
  server: {
    version: string;
    protocol_version: number;
    port: number;
    bind_host: string;
    public_access: boolean;
    debug_mode: boolean;
    performance_trace: boolean;
    mem: {
      space: string;
      data_dir: string;
      space_dir: string;
      memory_dir: string;
      temporary_retention_days: 1 | 5 | 10 | null;
      temporary_capacity_bytes: number | null;
      conversation_capacity_bytes: number | null;
      claude_codex_tool_discovery: boolean;
    };
    runtime_options: Array<{
      key: string;
      value: string;
      applies_to: "new_sessions" | string;
    }>;
    session_env_defaults: Record<string, string>;
    workspace_dirs: string[];
    mcp_servers: McpServerReport[];
    model_endpoints: ModelEndpoint[];
  };
  sessions: Session[];
  role_library: WorkerRoleLibrary;
  session_groups: SessionGroup[];
};

export type WireEvent =
  | {
      type: "hello";
      snapshot: Snapshot;
      event_cursor?: number;
      event_replay_floor?: number;
    }
  | { type: "semantic_event"; event_seq: number; event: WireEvent }
  | {
      type: "command_ack";
      command_id: string;
      status: "accepted" | "committed" | "rejected";
      error?: string;
    }
  | { type: "session_created"; session: Session }
  | { type: "session_renamed"; session_id: string; display_name: string }
  | { type: "session_restart_cwd_resolved"; session: Session }
  | { type: "session_deleted"; session_id: string }
  | { type: "session_groups_updated"; groups: SessionGroup[] }
  | {
      type: "session_order_updated";
      group_id?: string | null;
      session_ids: string[];
    }
  | {
      type: "session_group_changed";
      session_id: string;
      group_id?: string | null;
    }
  | { type: "worker_roles_updated"; session_id: string; roles: WorkerRole[] }
  | {
      type: "worker_role_library_updated";
      library: WorkerRoleLibrary;
      command_id?: string;
    }
  | {
      type: "chat_message_deleted";
      session_id: string;
      turn_id: string;
      role: "user" | "assistant";
      role_index: number;
    }
  | { type: "chat_search_result"; query: string; hits: ChatSearchHit[] }
  | {
      type: "favorites_list";
      favorites: ChatFavorite[];
      capacity: ChatLibraryCapacity;
    }
  | {
      type: "favorite_created";
      favorite: ChatFavorite;
      capacity: ChatLibraryCapacity;
      nearing_limit: boolean;
    }
  | { type: "favorite_capacity_reached"; capacity: ChatLibraryCapacity }
  | { type: "favorite_capacity_updated"; capacity: ChatLibraryCapacity }
  | { type: "favorite_deleted"; favorite_id: string }
  | {
      type: "session_runtime_updated";
      session_id: string;
      runtime_profile: NonNullable<Session["runtime_profile"]>;
    }
  | {
      type: "session_runtime_config_updated";
      session_id: string;
      key: string;
      value: string;
      runtime_profile: NonNullable<Session["runtime_profile"]>;
    }
  | { type: "session_api_key_revealed"; session_id: string; api_key: string }
  | {
      type: "core_topic";
      turn_id?: string | null;
      turn_event_id?: string | null;
      event: CoreTopicEvent;
    }
  | {
      type: "worker_activity";
      session_id: string;
      context_id: string;
      worker_id: string;
      turn_id?: string | null;
      turn_event_id?: string | null;
      event: Record<string, unknown>;
    }
  | {
      type: "turn_finished";
      session_id: string;
      turn_id?: string | null;
      outcome: {
        text?: string;
        message_id?: string | null;
        completion?: TurnCompletion;
      };
    }
  | {
      type: "turn_projection";
      session_id: string;
      projection: VersionedTurnProjection;
    }
  | {
      type: "turn_cancelling";
      session: Session;
      target_command_id?: string | null;
    }
  | {
      type: "turn_started";
      session_id: string;
      context_id: string;
      worker_id: string;
      turn: WebTurn;
    }
  | { type: "turn_updated"; session_id: string; turn: WebTurn }
  | {
      type: "message_queue_updated";
      session_id: string;
      message_queue: MessageQueueProjection;
    }
  | { type: "host_error"; message: string }
  | {
      type: "runtime_notice";
      session_id: string;
      level: "notice" | "warning" | "error" | string;
      title: string;
      message: string;
    }
  | {
      type: "host_config_updated";
      key: string;
      value: string;
      session_env_defaults: Record<string, string>;
    }
  | {
      type: "mem_settings_updated";
      temporary_retention_days: 1 | 5 | 10 | null;
      temporary_capacity_bytes: number | null;
      conversation_capacity_bytes: number | null;
      claude_codex_tool_discovery: boolean;
    }
  | { type: "mem_temporary_items"; items: MemTemporaryItem[]; error?: string }
  | { type: "file_uploaded"; session_id: string; file: Attachment }
  | { type: "attachment_removed"; session_id: string; attachment_id: string }
  | {
      type: "history_page";
      session_id: string;
      records: ChatHistoryRecord[];
      before_cursor?: string | null;
      has_more: boolean;
    }
  | { type: "tool_repo_updated"; session_id: string; tools: ToolSummary[] }
  | {
      type: "tool_repo_search_result";
      session_id: string;
      query: string;
      tools: ToolSummary[];
    }
  | { type: "tool_repo_detail"; session_id: string; detail: ToolDetail }
  | {
      type: "mcp_updated";
      session_id?: string | null;
      servers: McpServerReport[];
      enabled_server_ids: string[];
    }
  | {
      type: "mcp_server_secrets_revealed";
      server_id: string;
      values: Record<string, string>;
    }
  | { type: "model_endpoints_updated"; endpoints: ModelEndpoint[] }
  | {
      type: "model_endpoint_secret_revealed";
      endpoint_id: string;
      api_key: string;
      http_headers: Record<string, string>;
      request_fields: Record<string, unknown>;
    };

export type ClientCommand =
  | {
      type: "session_create";
      display_name?: string;
      workspace_dir?: string;
      group_id?: string | null;
      env?: Record<string, string>;
    }
  | { type: "session_rename"; session_id: string; display_name: string }
  | { type: "session_group_create"; name: string }
  | { type: "session_group_update"; group_id: string; name: string }
  | { type: "session_group_delete"; group_id: string }
  | { type: "session_groups_reorder"; groups: SessionGroup[] }
  | {
      type: "session_reorder";
      group_id?: string | null;
      session_ids: string[];
    }
  | { type: "session_group_move"; session_id: string; group_id?: string | null }
  | { type: "session_api_key_update"; session_id: string; api_key: string }
  | { type: "session_api_key_reveal"; session_id: string }
  | { type: "session_stop"; session_id: string }
  | { type: "session_delete"; session_id: string }
  | {
      type: "chat_message_delete";
      session_id: string;
      turn_id: string;
      role: "user" | "assistant";
      role_index: number;
    }
  | { type: "chat_search"; query: string; session_id?: string; limit?: number }
  | { type: "favorites_list" }
  | { type: "favorite_create"; session_id: string; turn_id: string }
  | { type: "favorite_delete"; favorite_id: string }
  | { type: "favorite_capacity_update"; max_bytes?: number | null }
  | {
      type: "worker_role_create";
      session_id: string;
      role_id?: string;
      name: string;
      description: string;
    }
  | {
      type: "worker_role_update";
      session_id: string;
      role_id: string;
      name: string;
      description: string;
    }
  | { type: "worker_role_delete"; session_id: string; role_id: string }
  | { type: "worker_role_group_create"; session_id: string; name: string }
  | {
      type: "worker_role_group_update";
      session_id: string;
      group_id: string;
      name: string;
    }
  | { type: "worker_role_group_delete"; session_id: string; group_id: string }
  | {
      type: "worker_role_library_reorder";
      session_id: string;
      groups: WorkerRoleGroup[];
      ungrouped_role_ids: string[];
    }
  | {
      type: "turn_submit";
      session_id: string;
      text: string;
      attachment_ids?: string[];
      role_ids?: string[];
      input_kind?: "toolgen";
      source_turn_id?: string;
    }
  | {
      type: "turn_supplement";
      session_id: string;
      text: string;
      attachment_ids?: string[];
      role_ids?: string[];
    }
  | {
      type: "session_restart_cwd_resolve";
      session_id: string;
      decision: "use_runtime" | "keep_session";
    }
  | { type: "turn_cancel"; session_id: string; target_command_id?: string }
  | {
      type: "message_queue_update";
      session_id: string;
      queued_command_id: string;
      text: string;
    }
  | {
      type: "message_queue_remove";
      session_id: string;
      queued_command_id: string;
    }
  | {
      type: "message_queue_reorder";
      session_id: string;
      command_ids: string[];
    }
  | {
      type: "message_queue_auto_send_set";
      session_id: string;
      enabled: boolean;
    }
  | {
      type: "message_queue_send_now";
      session_id: string;
      queued_command_id: string;
    }
  | { type: "attachment_remove"; session_id: string; attachment_id: string }
  | {
      type: "history_page";
      session_id: string;
      before_cursor?: string | null;
      /** Maximum complete tasks (turns), not JSONL records. */ limit?: number;
    }
  | {
      type: "tool_repo_search";
      session_id: string;
      query: string;
      limit?: number;
    }
  | { type: "tool_repo_detail"; session_id: string; tool_id: string }
  | {
      type: "tool_repo_rename";
      session_id: string;
      tool_id: string;
      new_name: string;
    }
  | { type: "tool_repo_open_terminal"; session_id: string; tool_id: string }
  | { type: "runtime_update"; key: string; value: string }
  | {
      type: "session_runtime_update";
      session_id: string;
      key: string;
      value: string;
    }
  | {
      type: "model_endpoint_upsert";
      endpoint: {
        id?: string;
        name: string;
        model: string;
        api_protocol: string;
        response_protocol: string;
        base_url: string;
        max_llm_input_tokens: number;
        max_llm_output_tokens: number;
        stream: boolean;
        api_key?: string;
        http_headers: Record<string, string>;
        request_fields: Record<string, unknown>;
      };
    }
  | { type: "model_endpoint_delete"; endpoint_id: string }
  | { type: "model_endpoint_apply"; session_id: string; endpoint_id: string }
  | { type: "model_endpoint_secret_reveal"; endpoint_id: string }
  | { type: "mcp_server_upsert"; session_id: string; config: McpServerConfig }
  | { type: "mcp_server_delete"; server_id: string }
  | {
      type: "mcp_session_toggle";
      session_id: string;
      server_id: string;
      enabled: boolean;
    }
  | { type: "mcp_server_reconnect"; session_id: string; server_id: string }
  | { type: "mcp_server_secrets_reveal"; server_id: string }
  | { type: "mem_switch"; path: string; stop_running: boolean }
  | {
      type: "mem_temporary_retention_update";
      days: 1 | 5 | 10 | null;
      max_bytes: number | null;
    }
  | { type: "mem_conversation_capacity_update"; max_bytes: number | null }
  | { type: "beta_claude_codex_tool_discovery_update"; enabled: boolean }
  | { type: "mem_temporary_items_list" }
  | { type: "mem_temporary_items_delete"; ids: string[] }
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
