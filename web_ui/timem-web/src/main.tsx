import { AssistantRuntimeProvider, ThreadMessageLike, ThreadPrimitive, useExternalStoreRuntime } from "@assistant-ui/react";
import { ArrowDown, Check, CheckCheck, ChevronRight, CircleStop, Copy, Cpu, FolderOpen, FolderTree, Gauge, LoaderCircle, Menu, Palette, Paperclip, PanelRight, Pencil, Plus, Search, Send, Settings2, Sparkles, Terminal, Wrench, X } from "lucide-react";
import { Children, Dispatch, isValidElement, MutableRefObject, SetStateAction, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { Appearance, applyAppearance, loadAppearance } from "./appearance";
import { Activity, ChatMessage, ClientCommand, Decision, Session, Snapshot, ToolDetail, ToolSummary, WebTurn, WebTurnEvent, WireEvent } from "./protocol";
import { isNearScrollBottom, preservePrependScrollTop, ScrollMetrics } from "./scroll";
import { activityFromTopic, appendTurnEvent, applyCoreTopicToSession, attachTurnCompletion, boundSessionHistory, clearDecisionsForWorker, coalesceActionLifecycle, composerSendDecision, draftForSession, enqueueDecision, finishSessionDraftSubmission, finishTurn, manualToolGenCommand, prependHistoryRecords, pruneSessionDrafts, pruneSessionSubmissionLocks, removePendingAttachment, requestDecision, reserveSessionDraftSubmission, resolveActiveSessionId, sessionContextUsage, sessionCreateDecision, sessionRenameDecision, setSessionDraft, tailPath, toolDisplayName, turnLiveUsage, updateSessionWorkerState, upsertSession, upsertTurn } from "./view_model";
import "./styles.css";
import "highlight.js/styles/github-dark.css";

const MAX_ACTIVITY_ITEMS = 300;
const TOKEN_STORAGE_KEY = "timem-web-access-token";
const EMPTY_CHAT_MESSAGES: ChatMessage[] = [];

function initialAccessToken() {
  const query = new URLSearchParams(window.location.search).get("token") ?? "";
  if (query) {
    try { window.sessionStorage.setItem(TOKEN_STORAGE_KEY, query); } catch { /* Keep the in-memory token. */ }
    return query;
  }
  try { return window.sessionStorage.getItem(TOKEN_STORAGE_KEY) ?? ""; } catch { return ""; }
}

const accessToken = initialAccessToken();

function queryToken() {
  return accessToken;
}

function makeMessage(role: ChatMessage["role"], text: string, id?: string): ChatMessage {
  return { id: id ?? `${role}-${crypto.randomUUID()}`, role, text, created_at_ms: Date.now() };
}

function TimemApp() {
  const [appearance, setAppearance] = useState<Appearance>(loadAppearance);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeSessionId, setActiveSessionId] = useState("");
  const [activities, setActivities] = useState<Activity[]>([]);
  const [decisions, setDecisions] = useState<Decision[]>([]);
  const [connected, setConnected] = useState(false);
  // The activity feed is a diagnostic view. Keep the normal chat workspace focused.
  const [showActivity, setShowActivity] = useState(false);
  const [sidePanelTab, setSidePanelTab] = useState<"tools" | "activity">("tools");
  const [toolSearchQuery, setToolSearchQuery] = useState("");
  const [toolSearchResults, setToolSearchResults] = useState<Record<string, ToolSummary[]>>({});
  const [pendingToolSearchKey, setPendingToolSearchKey] = useState("");
  const [pendingToolDetailKey, setPendingToolDetailKey] = useState("");
  const [pendingToolRenameKeys, setPendingToolRenameKeys] = useState<Set<string>>(() => new Set());
  const [selectedTool, setSelectedTool] = useState<ToolDetail | null>(null);
  const [toolCountPulseSessionId, setToolCountPulseSessionId] = useState("");
  const [pendingToolgenRequests, setPendingToolgenRequests] = useState<Set<string>>(() => new Set());
  const [toolgenDialog, setToolgenDialog] = useState<{ sessionId: string; turnId: string } | null>(null);
  const [showMobileSessions, setShowMobileSessions] = useState(false);
  const [showRuntime, setShowRuntime] = useState(false);
  const [showAppearance, setShowAppearance] = useState(false);
  const [showNewSession, setShowNewSession] = useState(false);
  const [showMemSwitch, setShowMemSwitch] = useState(false);
  const [renamingSessionId, setRenamingSessionId] = useState("");
  const [expandedSessionIds, setExpandedSessionIds] = useState<Set<string>>(() => new Set());
  const [renameDraft, setRenameDraft] = useState("");
  const [server, setServer] = useState<Snapshot["server"] | null>(null);
  const socket = useRef<WebSocket | null>(null);
  const activeSessionIdRef = useRef("");
  const toolSearchQueryRef = useRef("");
  const selectedToolRef = useRef<ToolDetail | null>(null);
  const toolCountBySessionRef = useRef<Map<string, number>>(new Map());
  const cancellingSessionIds = useRef<Set<string>>(new Set());
  const [cancellingSessionIdSet, setCancellingSessionIdSet] = useState<Set<string>>(() => new Set());
  const [creatingSession, setCreatingSession] = useState(false);
  const [pendingAttachmentRemoveIds, setPendingAttachmentRemoveIds] = useState<Set<string>>(() => new Set());
  const [pendingDecisionKeys, setPendingDecisionKeys] = useState<Set<string>>(() => new Set());
  const [pendingRenameSessionIds, setPendingRenameSessionIds] = useState<Set<string>>(() => new Set());
  const [pendingRuntimeKeys, setPendingRuntimeKeys] = useState<Set<string>>(() => new Set());
  const [pendingHistorySessionIds, setPendingHistorySessionIds] = useState<Set<string>>(() => new Set());
  const [pendingUploadSessionIds, setPendingUploadSessionIds] = useState<Set<string>>(() => new Set());
  const [pendingUploadFiles, setPendingUploadFiles] = useState<Record<string, { name: string; bytes: number }>>({});
  const [pendingMemSwitch, setPendingMemSwitch] = useState(false);
  const creatingSessionRef = useRef(false);
  const pendingAttachmentRemoveIdsRef = useRef<Set<string>>(new Set());
  const pendingDecisionKeysRef = useRef<Set<string>>(new Set());
  const pendingRenameSessionIdsRef = useRef<Set<string>>(new Set());
  const pendingRuntimeKeysRef = useRef<Set<string>>(new Set());
  const pendingHistorySessionIdsRef = useRef<Set<string>>(new Set());
  const pendingUploadSessionIdsRef = useRef<Set<string>>(new Set());
  const pendingToolgenRequestsRef = useRef<Set<string>>(new Set());
  const fileInput = useRef<HTMLInputElement | null>(null);
  const appearanceButtonRef = useRef<HTMLButtonElement | null>(null);
  const appearancePanelRef = useRef<HTMLElement | null>(null);
  const runtimeButtonRef = useRef<HTMLButtonElement | null>(null);
  const runtimePanelRef = useRef<HTMLElement | null>(null);
  const activeSession = sessions.find((session) => session.session_id === activeSessionId) ?? sessions[0];
  const activeMessages = activeSession?.messages ?? EMPTY_CHAT_MESSAGES;
  const pushActivity = useCallback((activity: Activity) => {
    setActivities((current) => {
      const existingIndex = current.findIndex((candidate) =>
        candidate.sessionId === activity.sessionId &&
        candidate.tone === activity.tone &&
        candidate.title === activity.title &&
        candidate.detail === activity.detail
      );
      const withoutExisting = existingIndex >= 0
        ? current.filter((_, index) => index !== existingIndex)
        : current;
      const merged = existingIndex >= 0 ? { ...activity, id: current[existingIndex].id } : activity;
      return [merged, ...withoutExisting].slice(0, MAX_ACTIVITY_ITEMS);
    });
  }, []);
  const reportUiError = useCallback((title: string, detail: string, sessionId = activeSessionIdRef.current || "system") => {
    pushActivity({ id: crypto.randomUUID(), sessionId, tone: "error", title, detail, createdAt: Date.now() });
  }, [pushActivity]);

  useEffect(() => {
    applyAppearance(appearance);
  }, [appearance]);

  useEffect(() => {
    if (!showRuntime) return;
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (runtimeButtonRef.current?.contains(target) || runtimePanelRef.current?.contains(target)) return;
      setShowRuntime(false);
    };
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setShowRuntime(false);
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer);
    document.addEventListener("keydown", dismissOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer);
      document.removeEventListener("keydown", dismissOnEscape);
    };
  }, [showRuntime]);

  useEffect(() => {
    if (!showActivity) return;
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setShowActivity(false);
    };
    document.addEventListener("keydown", dismissOnEscape);
    return () => document.removeEventListener("keydown", dismissOnEscape);
  }, [showActivity]);

  useEffect(() => {
    if (!showMobileSessions) return;
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setShowMobileSessions(false);
    };
    document.addEventListener("keydown", dismissOnEscape);
    return () => document.removeEventListener("keydown", dismissOnEscape);
  }, [showMobileSessions]);

  useEffect(() => {
    if (!showAppearance) return;
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (appearanceButtonRef.current?.contains(target) || appearancePanelRef.current?.contains(target)) return;
      setShowAppearance(false);
    };
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setShowAppearance(false);
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer);
    document.addEventListener("keydown", dismissOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer);
      document.removeEventListener("keydown", dismissOnEscape);
    };
  }, [showAppearance]);

  useEffect(() => {
    if (new URLSearchParams(window.location.search).has("token")) {
      window.history.replaceState(null, "", `${window.location.pathname}${window.location.hash}`);
    }
  }, []);

  const sendCommand = useCallback((command: ClientCommand) => {
    if (socket.current?.readyState !== WebSocket.OPEN) return false;
    socket.current.send(JSON.stringify(command));
    return true;
  }, []);

  const addPendingKey = useCallback((ref: MutableRefObject<Set<string>>, setState: Dispatch<SetStateAction<Set<string>>>, key: string) => {
    if (ref.current.has(key)) return false;
    ref.current.add(key);
    setState((current) => new Set(current).add(key));
    return true;
  }, []);

  const removePendingKey = useCallback((ref: MutableRefObject<Set<string>>, setState: Dispatch<SetStateAction<Set<string>>>, key: string) => {
    ref.current.delete(key);
    setState((current) => {
      const next = new Set(current);
      next.delete(key);
      return next;
    });
  }, []);

  const clearAllPendingCommands = useCallback(() => {
    creatingSessionRef.current = false;
    cancellingSessionIds.current.clear();
    pendingAttachmentRemoveIdsRef.current.clear();
    pendingDecisionKeysRef.current.clear();
    pendingRenameSessionIdsRef.current.clear();
    pendingRuntimeKeysRef.current.clear();
    pendingHistorySessionIdsRef.current.clear();
    pendingUploadSessionIdsRef.current.clear();
    pendingToolgenRequestsRef.current.clear();
    setCreatingSession(false);
    setCancellingSessionIdSet(new Set());
    setPendingAttachmentRemoveIds(new Set());
    setPendingDecisionKeys(new Set());
    setPendingRenameSessionIds(new Set());
    setPendingRuntimeKeys(new Set());
    setPendingHistorySessionIds(new Set());
    setPendingUploadSessionIds(new Set());
    setPendingUploadFiles({});
    setPendingToolSearchKey("");
    setPendingToolDetailKey("");
    setPendingToolRenameKeys(new Set());
    setSelectedTool(null);
    setPendingToolgenRequests(new Set());
    setPendingMemSwitch(false);
  }, []);

  useEffect(() => {
    const workingIds = new Set(sessions.filter((session) => session.state === "working").map((session) => session.session_id));
    let changed = false;
    for (const sessionId of Array.from(cancellingSessionIds.current)) {
      if (!workingIds.has(sessionId)) {
        cancellingSessionIds.current.delete(sessionId);
        changed = true;
      }
    }
    if (changed) setCancellingSessionIdSet(new Set(cancellingSessionIds.current));
  }, [sessions]);

  const beginRename = useCallback((session: Session) => {
    setRenamingSessionId(session.session_id);
    setRenameDraft(session.display_name);
  }, []);

  const finishRename = useCallback((sessionId: string) => {
    const decision = sessionRenameDecision(
      sessionId,
      renameDraft,
      pendingRenameSessionIdsRef.current,
      pendingMemSwitch,
    );
    if (decision.kind === "skip") {
      setRenamingSessionId("");
      setRenameDraft("");
      return;
    }
    if (addPendingKey(pendingRenameSessionIdsRef, setPendingRenameSessionIds, sessionId)) {
      if (!sendCommand(decision.command)) {
        removePendingKey(pendingRenameSessionIdsRef, setPendingRenameSessionIds, sessionId);
        setRenamingSessionId("");
        setRenameDraft("");
        reportUiError("Rename session failed", "Reconnect to Timem Web before renaming this session.", sessionId);
        return;
      }
      setSessions((current) => current.map((session) => session.session_id === sessionId ? { ...session, display_name: decision.displayName } : session));
    }
    setRenamingSessionId("");
    setRenameDraft("");
  }, [addPendingKey, pendingMemSwitch, removePendingKey, renameDraft, reportUiError, sendCommand]);

  const applySnapshot = useCallback((snapshot: Snapshot) => {
    toolCountBySessionRef.current = new Map(snapshot.sessions.map((session) => [session.session_id, session.tools.length]));
    setServer(snapshot.server);
    setSessions(snapshot.sessions.map(boundSessionHistory));
    setActiveSessionId((current) => resolveActiveSessionId(current, snapshot.sessions));
  }, []);

  const receive = useCallback((event: WireEvent) => {
    if (event.type === "hello") {
      clearAllPendingCommands();
      setDecisions([]);
      applySnapshot(event.snapshot);
      return;
    }
    if (event.type === "session_created") {
      creatingSessionRef.current = false;
      setCreatingSession(false);
      setSessions((current) => upsertSession(current, event.session));
      toolCountBySessionRef.current.set(event.session.session_id, event.session.tools.length);
      setActiveSessionId(event.session.session_id);
      return;
    }
    if (event.type === "session_renamed") {
      removePendingKey(pendingRenameSessionIdsRef, setPendingRenameSessionIds, event.session_id);
      setSessions((current) => current.map((session) => session.session_id === event.session_id ? { ...session, display_name: event.display_name } : session));
      return;
    }
    if (event.type === "turn_updated") {
      const consumedAttachmentIds = new Set(event.turn.user_entries.flatMap((entry) => entry.attachments ?? []).map((attachment) => attachment.id));
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...upsertTurn(session, event.turn), attachments: session.attachments.filter((attachment) => !consumedAttachmentIds.has(attachment.id)) }
        : session));
      return;
    }
    if (event.type === "host_error") {
      clearAllPendingCommands();
      const activity: Activity = { id: crypto.randomUUID(), sessionId: "system", tone: "error", title: "Runtime error", detail: event.message, createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    if (event.type === "host_config_updated") {
      removePendingKey(pendingRuntimeKeysRef, setPendingRuntimeKeys, event.key);
      setServer((current) => current ? {
        ...current,
        runtime_options: current.runtime_options.map((option) => option.key === event.key ? { ...option, value: event.value } : option),
        session_env_defaults: event.session_env_defaults,
      } : current);
      const activity: Activity = { id: crypto.randomUUID(), sessionId: "system", tone: "notice", title: "Runtime setting updated", detail: `${event.key}: ${event.value}`, createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    if (event.type === "file_uploaded") {
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, attachments: [...session.attachments, event.file] }
        : session));
      const activity: Activity = { id: crypto.randomUUID(), sessionId: event.session_id, tone: "notice", title: "File attached", detail: `${event.file.name} · ${formatBytes(event.file.bytes)}`, createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    if (event.type === "attachment_removed") {
      removePendingKey(pendingAttachmentRemoveIdsRef, setPendingAttachmentRemoveIds, `${event.session_id}:${event.attachment_id}`);
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? removePendingAttachment(session, event.attachment_id)
        : session));
      return;
    }
    if (event.type === "history_page") {
      removePendingKey(pendingHistorySessionIdsRef, setPendingHistorySessionIds, event.session_id);
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? {
            ...prependHistoryRecords(session, event.records),
            history_before_cursor: event.before_cursor ?? null,
            history_has_more: event.has_more,
          }
        : session));
      return;
    }
    if (event.type === "tool_repo_updated") {
      const previousCount = toolCountBySessionRef.current.get(event.session_id) ?? 0;
      toolCountBySessionRef.current.set(event.session_id, event.tools.length);
      if (event.tools.length > previousCount) {
        setToolCountPulseSessionId(event.session_id);
        window.setTimeout(() => setToolCountPulseSessionId((value) => value === event.session_id ? "" : value), 2400);
      }
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, tools: event.tools }
        : session));
      setToolSearchResults((current) => {
        if (event.session_id === activeSessionIdRef.current && toolSearchQueryRef.current.trim()) return current;
        return { ...current, [event.session_id]: event.tools };
      });
      const selected = selectedToolRef.current;
      if (selected && !event.tools.some((tool) => tool.tool_id === selected.summary.tool_id)) setSelectedTool(null);
      setPendingToolRenameKeys((current) => removeToolKeysForSession(current, event.session_id));
      return;
    }
    if (event.type === "tool_repo_search_result") {
      if (event.session_id !== activeSessionIdRef.current || event.query !== toolSearchQueryRef.current) return;
      setPendingToolSearchKey((key) => key === `${event.session_id}:${event.query}` ? "" : key);
      setToolSearchResults((current) => ({ ...current, [event.session_id]: event.tools }));
      const selected = selectedToolRef.current;
      if (selected && !event.tools.some((tool) => tool.tool_id === selected.summary.tool_id)) setSelectedTool(null);
      return;
    }
    if (event.type === "tool_repo_detail") {
      if (event.session_id === activeSessionIdRef.current) {
        setPendingToolDetailKey((key) => key === `${event.session_id}:${event.detail.summary.tool_id}` ? "" : key);
        setSelectedTool(event.detail);
      }
      return;
    }
    if (event.type === "worker_activity") {
      const kind = String(event.event.kind ?? "worker_event");
      if (kind !== "model_request" && kind !== "model_response") {
        const detail = Object.entries(event.event).filter(([key]) => !["kind", "session_id", "context_id", "worker_id"].includes(key)).map(([key, value]) => `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`).join("\n");
        const activity: Activity = { id: crypto.randomUUID(), sessionId: event.session_id, tone: kind.includes("error") ? "error" : kind.includes("retry") ? "warning" : "notice", title: kind.replaceAll("_", " "), detail, createdAt: Date.now() };
        pushActivity(activity);
      }
      const turnEvent: WebTurnEvent = { event_id: event.turn_event_id ?? crypto.randomUUID(), source: "worker_activity", payload: event.event, created_at_ms: Date.now() };
      setSessions((current) => current.map((session) => session.session_id === event.session_id ? appendTurnEvent(session, event.turn_id, turnEvent) : session));
      if (kind === "model_request") {
        setSessions((current) => current.map((session) => session.session_id === event.session_id ? updateSessionWorkerState(session, event.worker_id, "working") : session));
        setDecisions((current) => clearDecisionsForWorker(current, event.session_id, event.worker_id));
      } else if (kind === "model_error") {
        setSessions((current) => current.map((session) => session.session_id === event.session_id ? updateSessionWorkerState(session, event.worker_id, "error") : session));
      } else if (kind === "worker_stopped") {
        setSessions((current) => current.map((session) => session.session_id === event.session_id ? updateSessionWorkerState(session, event.worker_id, "stopped") : session));
      } else if (kind === "subworker_turn_finished") {
        setSessions((current) => current.map((session) => session.session_id === event.session_id ? updateSessionWorkerState(session, event.worker_id, "ready") : session));
      }
      return;
    }
    if (event.type === "turn_finished") {
      pendingToolgenRequestsRef.current = removeToolgenRequestsForSession(pendingToolgenRequestsRef.current, event.session_id);
      setPendingToolgenRequests(new Set(pendingToolgenRequestsRef.current));
      cancellingSessionIds.current.delete(event.session_id);
      setCancellingSessionIdSet(new Set(cancellingSessionIds.current));
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? finishTurn(attachTurnCompletion(session, event.outcome.message_id, event.outcome.completion ?? {}), event.turn_id, event.outcome.completion ?? {})
        : session));
      return;
    }
    if (event.type !== "core_topic") return;
    const topic = event.event;
    const activity = activityFromTopic(topic);
    if (activity) setActivities((current) => [activity, ...current.filter((item) => !(activity.kind === "toolgen" && item.kind === "toolgen" && item.sessionId === activity.sessionId))].slice(0, MAX_ACTIVITY_ITEMS));
    setSessions((current) => current.map((session) => applyCoreTopicToSession(
      appendTurnEvent(session, event.turn_id, { event_id: event.turn_event_id ?? crypto.randomUUID(), source: "core_topic", payload: topic as unknown as Record<string, unknown>, created_at_ms: Date.now() }),
      topic,
      (text) => makeMessage("assistant", text),
    )));
    const pendingDecision = requestDecision(topic, event.turn_id);
    if (pendingDecision) setDecisions((current) => enqueueDecision(current, pendingDecision));
    if (topic.topic.name === "core.lifecycle") {
      const worker = topic.payload.worker;
      if (worker && typeof worker === "object") {
        const item = worker as Record<string, unknown>;
        const sessionId = typeof item.session_id === "string" ? item.session_id : topic.session_id;
        const contextId = typeof item.context_id === "string" ? item.context_id : topic.context_id ?? "context_0";
        const workerId = typeof item.worker_id === "string" ? item.worker_id : topic.worker_id ?? sessionId;
        const displayName = typeof item.display_name === "string" ? item.display_name : sessionId;
        const ordinal = typeof item.ordinal === "number" ? item.ordinal : 0;
        setSessions((current) => current.some((session) => session.session_id === sessionId)
          ? current
          : [...current, { session_id: sessionId, display_name: displayName, ordinal, state: "ready", current_dir: "", max_llm_input_tokens: typeof topic.payload.max_llm_input_tokens === "number" ? topic.payload.max_llm_input_tokens : 0, tools: [], contexts: [{ context_id: contextId, current_dir: "", worker_ids: [workerId] }], workers: [{ worker_id: workerId, context_id: contextId, display_name: displayName, ordinal, state: "ready", parent_worker_id: typeof item.parent_worker_id === "string" ? item.parent_worker_id : null }], active_context_id: contextId, primary_worker_id: workerId, attachments: [], messages: [], turns: [], history_before_cursor: null, history_has_more: false, active_turn_id: null }]);
        setActiveSessionId((current) => current || sessionId);
      }
    }
  }, [applySnapshot, clearAllPendingCommands, pushActivity, removePendingKey]);

  useEffect(() => {
    activeSessionIdRef.current = activeSessionId;
  }, [activeSessionId]);

  useEffect(() => {
    toolSearchQueryRef.current = toolSearchQuery;
  }, [toolSearchQuery]);

  useEffect(() => {
    selectedToolRef.current = selectedTool;
  }, [selectedTool]);

  useEffect(() => {
    setSelectedTool(null);
    setToolSearchQuery("");
    setPendingToolSearchKey("");
    setPendingToolDetailKey("");
    setPendingToolRenameKeys(new Set());
  }, [activeSessionId]);

  useEffect(() => {
    if (!showActivity || sidePanelTab !== "tools" || !activeSession) return;
    const query = toolSearchQuery.trim();
    const searchKey = query ? `${activeSession.session_id}:${toolSearchQuery}` : "";
    setPendingToolSearchKey(searchKey);
    const timer = window.setTimeout(() => {
      if (!sendCommand({ type: "tool_repo_search", session_id: activeSession.session_id, query: toolSearchQuery, limit: 200 })) {
        setPendingToolSearchKey((key) => key === searchKey ? "" : key);
        reportUiError("ToolRepo search failed", "Reconnect to Timem Web before searching saved tools.", activeSession.session_id);
      }
    }, 180);
    return () => window.clearTimeout(timer);
  }, [activeSession?.session_id, showActivity, sidePanelTab, toolSearchQuery, sendCommand, reportUiError]);

  useEffect(() => {
    const token = queryToken();
    if (!token) {
      setActivities([{ id: crypto.randomUUID(), sessionId: "system", tone: "error", title: "Access token missing", detail: "Open Timem Web using the authenticated URL printed by the local host.", createdAt: Date.now() }]);
      return;
    }
    let stopped = false;
    let retryTimer: number | undefined;
    let retryAttempt = 0;
    const connect = () => {
      if (stopped) return;
      const scheme = window.location.protocol === "https:" ? "wss" : "ws";
      const ws = new WebSocket(`${scheme}://${window.location.host}/ws?token=${encodeURIComponent(token)}`);
      socket.current = ws;
      ws.onopen = () => { retryAttempt = 0; setConnected(true); };
      ws.onclose = () => {
        if (socket.current === ws) socket.current = null;
        setConnected(false);
        if (!stopped) {
          const delay = Math.min(10_000, 500 * 2 ** Math.min(retryAttempt, 5));
          retryAttempt += 1;
          retryTimer = window.setTimeout(connect, delay);
        }
      };
      ws.onerror = () => setConnected(false);
      ws.onmessage = (message) => {
        try { receive(JSON.parse(String(message.data)) as WireEvent); } catch { /* Ignore malformed transport data. */ }
      };
    };
    connect();
    return () => {
      stopped = true;
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      socket.current?.close();
      socket.current = null;
    };
  }, [receive]);

  const sendText = useCallback(async (text: string): Promise<boolean> => {
    const decision = composerSendDecision(
      activeSession,
      text,
      activeSession ? cancellingSessionIds.current.has(activeSession.session_id) : false,
      pendingMemSwitch,
    );
    if (decision.kind === "skip") {
      if (decision.reason === "cancelling" && activeSession) {
        pushActivity({ id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "notice", title: "Cancellation in progress", detail: "Wait for the current turn to stop before sending another message.", createdAt: Date.now() });
      } else if (decision.reason === "mem_switching") {
        pushActivity({ id: crypto.randomUUID(), sessionId: activeSession?.session_id ?? "system", tone: "notice", title: "Switching mem", detail: "Wait for the new mem space to load before sending another message.", createdAt: Date.now() });
      }
      return false;
    }
    if (!sendCommand(decision.command)) {
      pushActivity({ id: crypto.randomUUID(), sessionId: decision.command.session_id, tone: "error", title: "Not connected", detail: "Reconnect to Timem Web before sending another message.", createdAt: Date.now() });
      return false;
    }
    return decision.clearDraftOnSuccess;
  }, [activeSession, pendingMemSwitch, pushActivity, sendCommand]);

  const uploadFile = useCallback(async (file: File) => {
    if (!activeSession || pendingMemSwitch) return;
    if (!addPendingKey(pendingUploadSessionIdsRef, setPendingUploadSessionIds, activeSession.session_id)) {
      const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "notice", title: "Upload already in progress", detail: "Wait for the current file upload to finish before attaching another file.", createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    setPendingUploadFiles((current) => ({ ...current, [activeSession.session_id]: { name: file.name, bytes: file.size } }));
    const token = queryToken();
    if (!token) {
      reportUiError("File upload failed", "Open Timem Web using the authenticated URL before attaching files.", activeSession.session_id);
      removePendingKey(pendingUploadSessionIdsRef, setPendingUploadSessionIds, activeSession.session_id);
      setPendingUploadFiles((current) => {
        const next = { ...current };
        delete next[activeSession.session_id];
        return next;
      });
      return;
    }
    const form = new FormData();
    form.append("file", file);
    try {
      const response = await fetch(`/api/upload?token=${encodeURIComponent(token)}&session_id=${encodeURIComponent(activeSession.session_id)}`, { method: "POST", body: form });
      if (!response.ok) throw new Error((await response.json() as { error?: string }).error ?? "upload_failed");
    } catch (error) {
      const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "error", title: "File upload failed", detail: error instanceof Error ? error.message : "upload_failed", createdAt: Date.now() };
      pushActivity(activity);
    } finally {
      removePendingKey(pendingUploadSessionIdsRef, setPendingUploadSessionIds, activeSession.session_id);
      setPendingUploadFiles((current) => {
        const next = { ...current };
        delete next[activeSession.session_id];
        return next;
      });
    }
  }, [activeSession, addPendingKey, pendingMemSwitch, pushActivity, removePendingKey, reportUiError]);

  const loadMoreHistory = useCallback((session: Session) => {
    if (pendingMemSwitch) return;
    if (!session.history_has_more || !session.history_before_cursor) return;
    if (!addPendingKey(pendingHistorySessionIdsRef, setPendingHistorySessionIds, session.session_id)) return;
    if (!sendCommand({ type: "history_page", session_id: session.session_id, before_cursor: session.history_before_cursor, limit: 200 })) {
      removePendingKey(pendingHistorySessionIdsRef, setPendingHistorySessionIds, session.session_id);
      const activity: Activity = { id: crypto.randomUUID(), sessionId: session.session_id, tone: "error", title: "Load history failed", detail: "Reconnect to Timem Web before loading earlier history.", createdAt: Date.now() };
      pushActivity(activity);
    }
  }, [addPendingKey, pendingMemSwitch, pushActivity, removePendingKey, sendCommand]);

  const runtimeMessages = useMemo<readonly ThreadMessageLike[]>(() => activeMessages.map((message) => ({
    id: message.id,
    role: message.role,
    content: [{ type: "text" as const, text: message.text }],
  })), [activeMessages]);
  const [auiMessages, setAuiMessages] = useState<readonly ThreadMessageLike[]>(runtimeMessages);
  useEffect(() => setAuiMessages(runtimeMessages), [runtimeMessages]);
  const cancelActiveTurn = useCallback(async () => {
    if (!activeSession || activeSession.state !== "working" || pendingMemSwitch) return;
    if (cancellingSessionIds.current.has(activeSession.session_id)) return;
    cancellingSessionIds.current.add(activeSession.session_id);
    setCancellingSessionIdSet(new Set(cancellingSessionIds.current));
    if (!sendCommand({ type: "turn_cancel", session_id: activeSession.session_id })) {
      cancellingSessionIds.current.delete(activeSession.session_id);
      setCancellingSessionIdSet(new Set(cancellingSessionIds.current));
      const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "error", title: "Cancel failed", detail: "Reconnect to Timem Web before cancelling this turn.", createdAt: Date.now() };
      pushActivity(activity);
    }
  }, [activeSession, pendingMemSwitch, pushActivity, sendCommand]);
  const runtime = useExternalStoreRuntime<ThreadMessageLike>({
    messages: auiMessages,
    setMessages: setAuiMessages,
    convertMessage: (message) => message,
    isRunning: activeSession?.state === "working",
    onNew: async (message) => {
      const first = message.content[0];
      if (first?.type === "text") await sendText(first.text);
    },
    onCancel: cancelActiveTurn,
  });

  const sessionActivities = activities.filter((activity) => activity.sessionId === activeSession?.session_id || activity.sessionId === "system");
  const sessionActivityCount = sessionActivities.length;
  const sessionDecisions = decisions.filter((decision) => decision.event.session_id === activeSession?.session_id);
  const visibleErrors = activities.filter((activity) => activity.tone === "error" && (activity.sessionId === activeSession?.session_id || activity.sessionId === "system"));
  const visibleError = visibleErrors[0];
  const visibleErrorText = visibleError ? `${visibleError.title}${visibleError.detail ? ` · ${visibleError.detail}` : ""}` : "";
  const visibleErrorCount = visibleErrors.length;
  const hiddenErrorCount = Math.max(0, visibleErrorCount - 1);
  const errorDetailsLabel = visibleErrorCount === 1 ? "Show this error in Activity" : `Show ${visibleErrorCount} errors in Activity`;
  const dismissErrorLabel = visibleError ? `Dismiss ${visibleError.title}` : "Dismiss error";
  const connectionLabel = connected ? "Runtime connected" : "Reconnecting to runtime…";
  const memSwitchTitle = !connected ? "Reconnect before switching mem" : pendingMemSwitch ? "Mem switch is in progress" : "Switch mem space";
  const newSessionLabel = pendingMemSwitch ? "New session is locked while switching mem" : "New session";
  const headerModelLabel = activeSession?.runtime_profile ? `${activeSession.runtime_profile.provider}:${activeSession.runtime_profile.model}` : "";
  const appearanceLabel = showAppearance ? "Close appearance settings" : "Open appearance settings";
  const runtimeLabel = showRuntime ? "Close runtime information" : "Open runtime information";
  const sidePanelLabel = `${showActivity ? "Close" : "Open"} session tools and activity${sessionActivityCount ? `, ${sessionActivityCount} updates` : ""}`;
  const mobileSessionsLabel = showMobileSessions ? "Close session navigation" : "Open session navigation";
  return <AssistantRuntimeProvider runtime={runtime}>
    <div className="app-shell">
      {showMobileSessions && <button type="button" className="mobile-sidebar-backdrop" aria-label="Close session navigation" onClick={() => setShowMobileSessions(false)}/>}
      <aside id="session-navigation" className={`sidebar ${showMobileSessions ? "mobile-open" : ""}`}>
        <div className="brand"><Sparkles size={18}/><span>Timem</span><button type="button" className="mobile-sidebar-close" title="Close sessions" aria-label="Close sessions" onClick={() => setShowMobileSessions(false)}><X size={17}/></button></div>
        <button type="button" className="new-session" title={newSessionLabel} aria-label={newSessionLabel} disabled={pendingMemSwitch} onClick={() => { setShowNewSession(true); setShowMobileSessions(false); }}><Plus size={16}/> New session</button>
        <nav className="session-list" aria-label="Sessions">
          {sessions.map((session) => <div key={session.session_id} className="session-group"><div className={`session-row ${session.session_id === activeSession?.session_id ? "active" : ""} ${session.state === "working" ? "working" : ""}`}>
            <button type="button" className={`session-expand ${expandedSessionIds.has(session.session_id) ? "expanded" : ""}`} title={pendingMemSwitch ? "Mem switch is in progress" : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers`} aria-label={pendingMemSwitch ? `Workers locked while switching mem for ${session.display_name}` : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers for ${session.display_name}`} aria-expanded={expandedSessionIds.has(session.session_id)} disabled={pendingMemSwitch} onClick={() => setExpandedSessionIds((current) => {
              const next = new Set(current);
              if (next.has(session.session_id)) next.delete(session.session_id); else next.add(session.session_id);
              return next;
            })}><ChevronRight size={13}/></button>
            {renamingSessionId === session.session_id ? <input
              className="session-rename-input"
              autoFocus
              value={renameDraft}
              aria-label={`Rename ${session.display_name}`}
              disabled={pendingMemSwitch}
              onChange={(event) => setRenameDraft(event.target.value)}
              onBlur={() => finishRename(session.session_id)}
              onKeyDown={(event) => {
                if (event.key === "Enter") finishRename(session.session_id);
                if (event.key === "Escape") { setRenamingSessionId(""); setRenameDraft(""); }
              }}
            /> : <button type="button" className={`session ${session.session_id === activeSession?.session_id ? "active" : ""}`} title={pendingMemSwitch ? "Mem switch is in progress" : session.current_dir} aria-label={pendingMemSwitch ? `${session.display_name} locked while switching mem` : undefined} aria-current={session.session_id === activeSession?.session_id ? "page" : undefined} disabled={pendingMemSwitch} onClick={() => { setActiveSessionId(session.session_id); setShowMobileSessions(false); }}>
              {session.state === "working" ? <LoaderCircle className="session-working-icon" size={15} aria-label="Session working"/> : <span className={`session-dot ${session.state}`} aria-hidden="true"/>}<span className="session-identity"><span className="session-name" title={session.display_name}>{session.display_name}</span><span className="session-cwd" title={session.current_dir}>{tailPath(session.current_dir)}</span>{session.runtime_profile && <span className="session-profile" title={`${session.runtime_profile.provider}:${session.runtime_profile.model}`}>{session.runtime_profile.provider}:{session.runtime_profile.model}</span>}</span><span className="sr-only">Session state: {session.state}</span>
            </button>}
            {renamingSessionId !== session.session_id && <button type="button" className="session-rename" title={`Rename ${session.display_name}`} aria-label={`Rename ${session.display_name}`} disabled={pendingMemSwitch} onClick={() => beginRename(session)}><Pencil size={13}/></button>}
          </div>{expandedSessionIds.has(session.session_id) && <div className="worker-list" aria-label={`Workers for ${session.display_name}: ${session.workers.length} worker${session.workers.length === 1 ? "" : "s"}`}>{[...session.workers].sort((left, right) => left.ordinal - right.ordinal).map((worker) => <div className="worker-row" key={worker.worker_id} title={`${worker.worker_id} · ${worker.context_id}`}><span className={`worker-state-dot ${worker.state}`} aria-hidden="true"/><span className="worker-name">{worker.display_name || `ID${worker.ordinal}`}</span><span className="worker-state">{worker.state}</span></div>)}</div>}</div>)}
        </nav>
        <div className="sidebar-footer">
          <div className="connection-row" role="status" aria-live="polite" title={connectionLabel}><span className={`connection ${connected ? "online" : "offline"}`}/><span className="connection-label">{connectionLabel}</span></div>
          <div className="mem-row" title={server?.mem?.memory_dir ?? ""}><span>mem</span><code>{server?.mem?.space ?? "…"}</code><button type="button" className="mem-switch-button" title={memSwitchTitle} aria-label={memSwitchTitle} disabled={!connected || pendingMemSwitch} onClick={() => setShowMemSwitch(true)}>{pendingMemSwitch ? "Switching…" : "Switch"}</button></div>
        </div>
      </aside>
      <main className="chat-shell">
        <header className="chat-header">
          <span className="header-model" title={headerModelLabel}>{headerModelLabel}</span>
          <div className="header-actions">
            <button type="button" title={mobileSessionsLabel} aria-label={mobileSessionsLabel} className="icon-button mobile-session-button" aria-expanded={showMobileSessions} aria-controls="session-navigation" onClick={() => setShowMobileSessions(true)}><Menu size={18}/></button>
            <button type="button" ref={appearanceButtonRef} title={appearanceLabel} aria-label={appearanceLabel} className={`icon-button ${showAppearance ? "selected" : ""}`} aria-expanded={showAppearance} aria-controls="appearance-panel" onClick={() => { setShowRuntime(false); setShowActivity(false); setShowAppearance((visible) => !visible); }}><Palette size={17}/></button>
            <button type="button" ref={runtimeButtonRef} title={runtimeLabel} aria-label={runtimeLabel} className={`icon-button ${showRuntime ? "selected" : ""}`} aria-expanded={showRuntime} aria-controls="runtime-panel" onClick={() => { setShowAppearance(false); setShowActivity(false); setShowRuntime((visible) => !visible); }}><Settings2 size={17}/></button>
            <button type="button" title={sidePanelLabel} aria-label={sidePanelLabel} className={`icon-button side-panel-button ${showActivity ? "selected" : ""}`} aria-expanded={showActivity} aria-controls="session-side-panel" onClick={() => { setShowAppearance(false); setShowRuntime(false); setShowActivity((visible) => !visible); }}><PanelRight size={17}/>{sessionActivityCount > 0 && <span className="activity-count-badge">{sessionActivityCount > 99 ? "99+" : sessionActivityCount}</span>}</button>
          </div>
        </header>
        {showAppearance && <AppearancePanel panelRef={appearancePanelRef} appearance={appearance} onChange={setAppearance} onClose={() => setShowAppearance(false)}/>}
        {visibleError && <div className="host-error-banner" role="alert">
          <span className="host-error-text" title={visibleErrorText}><strong>{visibleError.title}</strong>{visibleError.detail && <span className="host-error-detail"> · {visibleError.detail}</span>}{hiddenErrorCount > 0 && <em>{hiddenErrorCount} more hidden error{hiddenErrorCount === 1 ? "" : "s"}</em>}</span>
          <div className="host-error-actions">
            <button type="button" className="host-error-details" title={errorDetailsLabel} aria-label={errorDetailsLabel} aria-controls="session-side-panel" aria-expanded={showActivity && sidePanelTab === "activity"} onClick={() => { setShowAppearance(false); setShowRuntime(false); setSidePanelTab("activity"); setShowActivity(true); }}>Details</button>
            {hiddenErrorCount > 0 && <button type="button" className="host-error-dismiss-all" title="Dismiss all visible errors" aria-label="Dismiss all visible errors" onClick={() => setActivities((current) => current.filter((activity) => activity.tone !== "error" || (activity.sessionId !== activeSession?.session_id && activity.sessionId !== "system")))}>Dismiss all</button>}
            <button type="button" className="icon-button" title={dismissErrorLabel} aria-label={dismissErrorLabel} onClick={() => setActivities((current) => current.filter((activity) => activity.id !== visibleError.id))}><X size={15}/></button>
          </div>
        </div>}
        {showRuntime && <RuntimePanel panelRef={runtimePanelRef} server={server} pendingKeys={pendingRuntimeKeys} onUpdate={(key, value) => {
          if (!addPendingKey(pendingRuntimeKeysRef, setPendingRuntimeKeys, key)) return;
          if (!sendCommand({ type: "runtime_update", key, value })) {
            removePendingKey(pendingRuntimeKeysRef, setPendingRuntimeKeys, key);
            reportUiError("Runtime update failed", "Reconnect to Timem Web before applying runtime configuration.");
          }
        }}/>}
        <ContextUsageBar session={activeSession}/>
        <TimemThread
          activeSession={activeSession}
          sessionIds={sessions.map((session) => session.session_id)}
          sessionInteractionLocked={pendingMemSwitch}
          decisions={sessionDecisions}
          fileInput={fileInput}
          isCancelling={!!activeSession && cancellingSessionIdSet.has(activeSession.session_id)}
          pendingAttachmentRemoveIds={pendingAttachmentRemoveIds}
          pendingDecisionKeys={pendingDecisionKeys}
          uploadingAttachment={!!activeSession && pendingUploadSessionIds.has(activeSession.session_id)}
          uploadingAttachmentFile={activeSession ? pendingUploadFiles[activeSession.session_id] : undefined}
          loadingHistory={activeSession ? pendingHistorySessionIds.has(activeSession.session_id) : false}
          onLoadMoreHistory={loadMoreHistory}
          onSend={sendText}
          toolCount={activeSession?.tools.length ?? 0}
          toolCountPulse={toolCountPulseSessionId === activeSession?.session_id}
          pendingToolGenTurnIds={activeSession ? pendingToolgenTurnIds(pendingToolgenRequests, activeSession.session_id) : new Set()}
          toolGenSessionBusy={!!activeSession && hasPendingToolgenForSession(pendingToolgenRequests, activeSession.session_id)}
          onOpenToolRepo={() => { setShowAppearance(false); setShowRuntime(false); setSidePanelTab("tools"); setShowActivity(true); }}
          onRequestToolGen={(turnId) => {
            if (!activeSession || activeSession.state === "working" || pendingMemSwitch || hasPendingToolgenForSession(pendingToolgenRequests, activeSession.session_id)) return;
            setToolgenDialog({ sessionId: activeSession.session_id, turnId });
          }}
          onCancel={cancelActiveTurn}
          onUpload={uploadFile}
          onRemoveAttachment={(attachmentId) => {
            if (!activeSession || pendingMemSwitch) return;
            const key = `${activeSession.session_id}:${attachmentId}`;
            if (!addPendingKey(pendingAttachmentRemoveIdsRef, setPendingAttachmentRemoveIds, key)) return;
            if (!sendCommand({ type: "attachment_remove", session_id: activeSession.session_id, attachment_id: attachmentId })) {
              removePendingKey(pendingAttachmentRemoveIdsRef, setPendingAttachmentRemoveIds, key);
              const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "error", title: "Remove attachment failed", detail: "Reconnect to Timem Web before removing this attachment.", createdAt: Date.now() };
              pushActivity(activity);
            }
          }}
          onDecisionReply={(decision, decisionValue) => {
            if (pendingMemSwitch) return;
            const key = decisionKey(decision);
            if (!addPendingKey(pendingDecisionKeysRef, setPendingDecisionKeys, key)) return;
            const event = decision.event;
            if (sendCommand({ type: "topic_reply", session_id: event.session_id, worker_id: event.worker_id ?? undefined, topic_name: event.topic.name, request_id: typeof event.payload.request_id === "string" ? event.payload.request_id : undefined, decision: decisionValue, payload: { summary: decision.detail } })) {
              setDecisions((current) => current.filter((candidate) => candidate !== decision));
            } else {
              removePendingKey(pendingDecisionKeysRef, setPendingDecisionKeys, key);
              reportUiError("Decision reply failed", "Reconnect to Timem Web before replying to this runtime request.", event.session_id);
            }
          }}
        />
      </main>
      {showActivity && <button type="button" className="side-panel-backdrop" aria-label="Close session tools and activity" onClick={() => setShowActivity(false)}/>}
      {showActivity && <SessionSidePanel
        tab={sidePanelTab}
        onTabChange={setSidePanelTab}
        onClose={() => setShowActivity(false)}
        session={activeSession}
        activities={sessionActivities}
        searchQuery={toolSearchQuery}
        searchPending={!!activeSession && pendingToolSearchKey === `${activeSession.session_id}:${toolSearchQuery}`}
        onSearchQueryChange={setToolSearchQuery}
        tools={activeSession ? (toolSearchResults[activeSession.session_id] ?? activeSession.tools) : []}
        selectedTool={selectedTool}
        pendingToolDetailId={activeSession && pendingToolDetailKey.startsWith(`${activeSession.session_id}:`) ? pendingToolDetailKey.slice(activeSession.session_id.length + 1) : ""}
        pendingToolRenameIds={activeSession ? pendingToolIdsForSession(pendingToolRenameKeys, activeSession.session_id) : new Set()}
        onClearActivities={() => {
          const sessionId = activeSession?.session_id;
          if (!sessionId) return;
          setActivities((current) => current.filter((activity) => activity.sessionId !== sessionId));
        }}
        onSelectTool={(toolId) => {
          if (selectedTool?.summary.tool_id === toolId) {
            setSelectedTool(null);
            setPendingToolDetailKey("");
            return true;
          }
          if (!activeSession) return false;
          setPendingToolDetailKey(`${activeSession.session_id}:${toolId}`);
          if (sendCommand({ type: "tool_repo_detail", session_id: activeSession.session_id, tool_id: toolId })) return true;
          setPendingToolDetailKey("");
          reportUiError("Tool detail failed", "Reconnect to Timem Web before opening tool details.", activeSession.session_id);
          return false;
        }}
        onCollapseTool={() => { setSelectedTool(null); setPendingToolDetailKey(""); }}
        onRenameTool={(toolId, newName) => {
          if (activeSession) {
            const renameKey = toolKey(activeSession.session_id, toolId);
            setPendingToolRenameKeys((current) => new Set(current).add(renameKey));
            if (sendCommand({ type: "tool_repo_rename", session_id: activeSession.session_id, tool_id: toolId, new_name: newName })) return true;
            setPendingToolRenameKeys((current) => { const next = new Set(current); next.delete(renameKey); return next; });
          }
          const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession?.session_id ?? "system", tone: "error", title: "Tool rename failed", detail: "Reconnect to Timem Web before renaming this tool.", createdAt: Date.now() };
          pushActivity(activity);
          return false;
        }}
        onOpenTerminal={(toolId) => {
          if (activeSession && sendCommand({ type: "tool_repo_open_terminal", session_id: activeSession.session_id, tool_id: toolId })) return true;
          const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession?.session_id ?? "system", tone: "error", title: "Open terminal failed", detail: "Reconnect to Timem Web before opening a tool directory.", createdAt: Date.now() };
          pushActivity(activity);
          return false;
        }}
      />}
      {showNewSession && <NewSessionDialog workspaces={server?.workspace_dirs ?? []} runtimeDefaults={server?.session_env_defaults ?? {}} creating={creatingSession} memSwitching={pendingMemSwitch} onClose={() => { if (!creatingSessionRef.current) setShowNewSession(false); }} onCreate={(command) => {
        if (pendingMemSwitch) return;
        if (creatingSessionRef.current) return;
        creatingSessionRef.current = true;
        setCreatingSession(true);
        if (sendCommand(command)) {
          setShowNewSession(false);
        } else {
          creatingSessionRef.current = false;
          setCreatingSession(false);
          reportUiError("Create session failed", "Reconnect to Timem Web before creating a new session.", "system");
        }
      }} />}
      {showMemSwitch && <MemSwitchDialog current={server?.mem?.space ?? ""} pending={pendingMemSwitch} onClose={() => { if (!pendingMemSwitch) setShowMemSwitch(false); }} onSwitch={(space) => {
        setRenamingSessionId("");
        setRenameDraft("");
        setPendingMemSwitch(true);
        if (sendCommand({ type: "mem_switch", space })) {
          setShowMemSwitch(false);
        } else {
          setPendingMemSwitch(false);
          reportUiError("Mem switch failed", "Reconnect to Timem Web before switching memory space.", "system");
        }
      }}
      />}
      {toolgenDialog && <ToolGenDialog
        key={`${toolgenDialog.sessionId}:${toolgenDialog.turnId}`}
        pending={pendingToolgenRequests.has(toolgenRequestKey(toolgenDialog.sessionId, toolgenDialog.turnId))}
        onClose={() => { if (!pendingToolgenRequests.has(toolgenRequestKey(toolgenDialog.sessionId, toolgenDialog.turnId))) setToolgenDialog(null); }}
        onSubmit={(text) => {
          const request = toolgenDialog;
          const requestKey = toolgenRequestKey(request.sessionId, request.turnId);
          if (pendingToolgenRequestsRef.current.has(requestKey)) return;
          pendingToolgenRequestsRef.current.add(requestKey);
          setPendingToolgenRequests(new Set(pendingToolgenRequestsRef.current));
          if (sendCommand(manualToolGenCommand(request.sessionId, request.turnId, text))) {
            setToolgenDialog(null);
          } else {
            pendingToolgenRequestsRef.current.delete(requestKey);
            setPendingToolgenRequests(new Set(pendingToolgenRequestsRef.current));
            reportUiError("ToolGen start failed", "Reconnect to Timem Web before generating a reusable tool.", request.sessionId);
          }
        }}
      />}
    </div>
  </AssistantRuntimeProvider>;
}

function SessionSidePanel({ tab, onTabChange, onClose, session, activities, searchQuery, searchPending, onSearchQueryChange, tools, selectedTool, pendingToolDetailId, pendingToolRenameIds, onClearActivities, onSelectTool, onCollapseTool, onRenameTool, onOpenTerminal }: {
  tab: "tools" | "activity";
  onTabChange: (tab: "tools" | "activity") => void;
  onClose: () => void;
  session: Session | undefined;
  activities: Activity[];
  searchQuery: string;
  searchPending: boolean;
  onSearchQueryChange: (query: string) => void;
  tools: ToolSummary[];
  selectedTool: ToolDetail | null;
  pendingToolDetailId: string;
  pendingToolRenameIds: Set<string>;
  onClearActivities: () => void;
  onSelectTool: (toolId: string) => boolean;
  onCollapseTool: () => void;
  onRenameTool: (toolId: string, newName: string) => boolean;
  onOpenTerminal: (toolId: string) => boolean;
}) {
  const [sort, setSort] = useState<"time" | "type" | "language">("time");
  const [renameToolId, setRenameToolId] = useState("");
  const [renameValue, setRenameValue] = useState("");
  const [contextMenu, setContextMenu] = useState<{ toolId: string; x: number; y: number } | null>(null);
  const toolsTabRef = useRef<HTMLButtonElement>(null);
  const activityTabRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setRenameToolId("");
    setRenameValue("");
    setContextMenu(null);
  }, [session?.session_id, tab]);
  useEffect(() => {
    if (!contextMenu) return;
    const dismiss = () => setContextMenu(null);
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setContextMenu(null);
    };
    window.addEventListener("pointerdown", dismiss);
    window.addEventListener("keydown", dismissOnEscape);
    return () => {
      window.removeEventListener("pointerdown", dismiss);
      window.removeEventListener("keydown", dismissOnEscape);
    };
  }, [contextMenu]);
  const sortedTools = useMemo(() => [...tools].sort((left, right) => {
    if (sort === "type") return left.tool_type.localeCompare(right.tool_type) || left.name.localeCompare(right.name);
    if (sort === "language") return left.language.localeCompare(right.language) || left.name.localeCompare(right.name);
    return right.updated_at_ms - left.updated_at_ms || left.name.localeCompare(right.name);
  }), [sort, tools]);
  const pendingTool = pendingToolDetailId ? sortedTools.find((tool) => tool.tool_id === pendingToolDetailId) : undefined;
  const finishToolRename = (tool: ToolSummary) => {
    const name = renameValue.trim();
    if (name && name !== tool.name && !onRenameTool(tool.tool_id, name)) return;
    setRenameToolId("");
    setRenameValue("");
  };
  const hasToolSearch = searchQuery.trim().length > 0;
  const toolRepoResultText = !session
    ? ""
    : searchPending
      ? "Searching..."
    : hasToolSearch
      ? `${sortedTools.length} of ${session.tools.length} tools`
      : `${sortedTools.length} tool${sortedTools.length === 1 ? "" : "s"}`;
  const toolRepoEmptyTitle = !session ? "No active session" : searchPending ? "Searching ToolRepo…" : hasToolSearch ? "No matching tools" : "No reusable tools yet";
  const toolRepoEmptyText = !session
    ? "Select or create a session to browse its ToolRepo."
    : searchPending
      ? "Searching tool names and file contents. Results will update automatically."
    : hasToolSearch
      ? "Try a different keyword, or clear search to show all saved tools."
      : "Use ToolGen on a completed task to preserve a reusable script here.";
  const activityEmptyTitle = session ? "No activity yet" : "No active session";
  const activityEmptyText = session
    ? "Runtime updates will appear here while this session works."
    : "Select or create a session to inspect runtime activity.";
  const activityTabCount = activities.length > 99 ? "99+" : String(activities.length);
  const pendingToolDetailLabel = pendingTool ? `Loading ${pendingTool.name} tool directory` : "";
  const sortLabel = sort === "time" ? "recent update" : sort;
  const sortControlLabel = `Sort ToolRepo by ${sortLabel}`;
  const switchSidePanelTabFromKeyboard = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "ArrowLeft" || event.key === "Home") {
      event.preventDefault();
      onTabChange("tools");
      toolsTabRef.current?.focus();
    } else if (event.key === "ArrowRight" || event.key === "End") {
      event.preventDefault();
      onTabChange("activity");
      activityTabRef.current?.focus();
    }
  };
  return <aside id="session-side-panel" className="activity-panel session-side-panel" aria-label="Session tools and activity panel">
    <header className="side-panel-header"><div className="side-panel-tabs" role="tablist" aria-label="Session side panel sections" onKeyDown={switchSidePanelTabFromKeyboard}><button ref={toolsTabRef} type="button" id="side-panel-tab-tools" role="tab" aria-label={`ToolRepo, ${session?.tools.length ?? 0} tools`} aria-controls="side-panel-tools" aria-selected={tab === "tools"} tabIndex={tab === "tools" ? 0 : -1} className={tab === "tools" ? "active" : ""} onClick={() => onTabChange("tools")}><FolderTree size={15}/>ToolRepo{session && <> <small>{session.tools.length}</small></>}</button><button ref={activityTabRef} type="button" id="side-panel-tab-activity" role="tab" aria-label={`Activity, ${activities.length} updates`} aria-controls="side-panel-activity" aria-selected={tab === "activity"} tabIndex={tab === "activity" ? 0 : -1} className={tab === "activity" ? "active" : ""} onClick={() => onTabChange("activity")}>Activity<small>{activityTabCount}</small></button></div><div className="side-panel-header-actions">{tab === "activity" && activities.length > 0 && <button type="button" className="side-panel-clear" title={`Clear ${activities.length} current session activity updates`} aria-label={`Clear ${activities.length} current session activity updates`} onClick={onClearActivities}>Clear</button>}<button type="button" className="icon-button" title="Close side panel" aria-label="Close side panel" onClick={onClose}><X size={16}/></button></div></header>
    {tab === "activity" ? <div id="side-panel-activity" className="activity-list" role="tabpanel" aria-labelledby="side-panel-tab-activity">{activities.length === 0 ? <div className="activity-empty" aria-label={`${activityEmptyTitle}. ${activityEmptyText}`}><strong>{activityEmptyTitle}</strong><span>{activityEmptyText}</span></div> : activities.map((activity) => <ActivityListItem activity={activity} key={activity.id}/>)}</div> : <div id="side-panel-tools" className="toolrepo-panel" role="tabpanel" aria-labelledby="side-panel-tab-tools">
      <div className="toolrepo-controls"><label className={searchPending ? "searching" : ""} aria-busy={searchPending}><Search size={14}/><input value={searchQuery} disabled={!session} onChange={(event) => onSearchQueryChange(event.target.value)} onKeyDown={(event) => { if (event.key === "Escape" && searchQuery) { event.stopPropagation(); onSearchQueryChange(""); } }} placeholder={session ? "Search names and code" : "Select a session first"} aria-label="Search ToolRepo"/>{searchPending && <span className="toolrepo-search-pending" aria-hidden="true"/>}{hasToolSearch && <button type="button" title="Clear ToolRepo search" aria-label="Clear ToolRepo search" onClick={() => onSearchQueryChange("")}><X size={13}/></button>}</label><select value={sort} disabled={!session} onChange={(event) => setSort(event.target.value as typeof sort)} title={sortControlLabel} aria-label={sortControlLabel}><option value="time">Recent</option><option value="type">Type</option><option value="language">Language</option></select></div>
      {session && <div className="toolrepo-result-count" aria-live="polite">{toolRepoResultText}</div>}
      {!sortedTools.length ? <div className={`toolrepo-empty ${searchPending ? "searching" : ""}`} aria-label={`${toolRepoEmptyTitle}. ${toolRepoEmptyText}`} aria-busy={searchPending || undefined}><Wrench size={20}/><strong>{toolRepoEmptyTitle}</strong><span>{toolRepoEmptyText}</span></div> : <div className="toolrepo-browser"><div className="toolrepo-list" role="tree">{sortedTools.map((tool) => {
        const loadingDetail = pendingToolDetailId === tool.tool_id;
        const renamingTool = pendingToolRenameIds.has(tool.tool_id);
        const expanded = selectedTool?.summary.tool_id === tool.tool_id;
        const toolToggleLabel = expanded ? `收起 ${tool.name} 详情` : `展开 ${tool.name} 详情`;
        return <div className={`toolrepo-item ${selectedTool?.summary.tool_id === tool.tool_id ? "selected" : ""} ${loadingDetail ? "loading-detail" : ""} ${renamingTool ? "renaming-tool" : ""}`} role="treeitem" aria-selected={selectedTool?.summary.tool_id === tool.tool_id} aria-expanded={expanded} aria-busy={loadingDetail || renamingTool || undefined} key={tool.tool_id} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ toolId: tool.tool_id, x: Math.max(8, Math.min(event.clientX, window.innerWidth - 220)), y: Math.max(8, Math.min(event.clientY, window.innerHeight - 76)) }); }}>
        <button type="button" className="toolrepo-item-main" title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`} aria-label={toolToggleLabel} aria-expanded={expanded} onClick={() => { if (expanded) onCollapseTool(); else onSelectTool(tool.tool_id); }}><ChevronRight size={13}/><span><strong>{tool.name}</strong><small>{renamingTool ? "Renaming..." : loadingDetail ? "Loading details..." : `${tool.language} · ${tool.tool_type}`}</small><em className="toolrepo-toggle-state">{expanded ? "收起" : "展开"}</em></span></button>
        <button type="button" className="toolrepo-open" title={`Open ${tool.name} directory in terminal`} aria-label={`Open ${tool.name} directory in terminal`} onClick={() => onOpenTerminal(tool.tool_id)}><Terminal size={12}/></button>
        {renameToolId === tool.tool_id ? <input className="toolrepo-rename" autoFocus value={renameValue} aria-label={`Rename ${tool.name}`} disabled={renamingTool} onChange={(event) => setRenameValue(event.target.value)} onBlur={() => finishToolRename(tool)} onKeyDown={(event) => { if (event.key === "Enter") finishToolRename(tool); if (event.key === "Escape") { setRenameToolId(""); setRenameValue(""); } }}/> : <button type="button" className="toolrepo-edit" title={renamingTool ? `Renaming ${tool.name}` : `Rename ${tool.name}`} aria-label={renamingTool ? `Renaming ${tool.name}` : `Rename ${tool.name}`} disabled={renamingTool} onClick={() => { setRenameToolId(tool.tool_id); setRenameValue(tool.name); }}><Pencil size={12}/></button>}
      </div>})}</div>
      {pendingTool ? <section className="toolrepo-detail loading" aria-busy="true" aria-label={pendingToolDetailLabel}><header><div><strong title={pendingTool.name}>{pendingTool.name}</strong><code>Reading tool directory…</code></div><div className="toolrepo-detail-actions"><button type="button" className="toolrepo-detail-collapse" title={`Stop viewing ${pendingTool.name} details`} aria-label={`Stop viewing ${pendingTool.name} details`} onClick={onCollapseTool}>收起详情</button></div></header><div className="toolrepo-detail-loading" role="status" aria-live="polite" aria-label={pendingToolDetailLabel}><span className="toolrepo-search-pending" aria-hidden="true"/>Reading directory tree...</div></section> : selectedTool && <section className="toolrepo-detail"><header><div><strong title={selectedTool.summary.name}>{selectedTool.summary.name}</strong><code title={selectedTool.summary.synopsis}>{selectedTool.summary.synopsis}</code></div><div className="toolrepo-detail-actions"><button type="button" title="Open directory in terminal" aria-label="Open directory in terminal" onClick={() => onOpenTerminal(selectedTool.summary.tool_id)}><Terminal size={14}/></button><button type="button" className="toolrepo-detail-collapse" title="Collapse tool detail" aria-label="Collapse tool detail" onClick={onCollapseTool}>收起详情</button></div></header><div className="toolrepo-files" aria-label="Tool directory tree">{selectedTool.files.map((file) => <div key={file.path} title={`${file.path} · ${formatBytes(file.bytes)}`} style={{ paddingLeft: `${8 + Math.max(0, file.path.split("/").length - 1) * 12}px` }}><span>{file.path}</span><small>{formatBytes(file.bytes)}</small></div>)}</div></section>}
      </div>}
    </div>}
    {contextMenu && <div className="toolrepo-context-menu" role="menu" aria-label="Tool actions" style={{ left: contextMenu.x, top: contextMenu.y }} onPointerDown={(event) => event.stopPropagation()}><button type="button" role="menuitem" onClick={() => { onOpenTerminal(contextMenu.toolId); setContextMenu(null); }}><Terminal size={14}/>在命令行中打开目录</button></div>}
  </aside>;
}

function ActivityListItem({ activity }: { activity: Activity }) {
  const [open, setOpen] = useState(false);
  const mark = activity.tone === "thinking" ? "✦" : activity.tone === "action" ? "↳" : activity.tone === "warning" ? "⚠️" : activity.tone === "error" ? "×" : "i";
  const hasExpandableDetail = !!activity.detail?.trim() || !!activity.code?.trim();
  if (!hasExpandableDetail) return <div className={`activity ${activity.tone}`}><span className="activity-mark">{mark}</span><div>{activity.title && <strong>{activity.title}</strong>}</div></div>;
  const collapse = () => setOpen(false);
  const summaryLabel = `${open ? "收起" : "展开"} Activity 详情${activity.title ? `：${activity.title}` : ""}`;
  return <details className={`activity ${activity.tone}`} open={open} onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary title={open ? "收起详情" : "展开详情"} aria-label={summaryLabel}><span className="activity-mark">{mark}</span><div>{activity.title && <strong>{activity.title}</strong>}<span className="activity-expand-label">{open ? "收起" : "展开"}</span></div></summary>
    <div className="activity-body"><button type="button" className="activity-collapse top" title="Collapse activity details" aria-label="Collapse activity details" onClick={collapse}>收起详情</button>{activity.detail && <div className="activity-detail"><MarkdownContent text={activity.detail}/></div>}{activity.code && <MarkdownContent text={fencedCode(activity.code_language ?? "text", activity.code)}/>}<button type="button" className="activity-collapse" title="Collapse activity details" aria-label="Collapse activity details" onClick={collapse}>收起详情</button></div>
  </details>;
}

const MAX_RENDERED_TURN_EVENTS = 200;
const INITIAL_RENDERED_TURNS = 24;
const TURN_HISTORY_PAGE_SIZE = 24;

function TimemThread({ activeSession, sessionIds, sessionInteractionLocked, decisions, fileInput, isCancelling, pendingAttachmentRemoveIds, pendingDecisionKeys, uploadingAttachment, uploadingAttachmentFile, loadingHistory, toolCount, toolCountPulse, pendingToolGenTurnIds, toolGenSessionBusy, onLoadMoreHistory, onSend, onCancel, onUpload, onRemoveAttachment, onDecisionReply, onOpenToolRepo, onRequestToolGen }: {
  activeSession: Session | undefined;
  sessionIds: string[];
  sessionInteractionLocked: boolean;
  decisions: Decision[];
  fileInput: React.RefObject<HTMLInputElement | null>;
  isCancelling: boolean;
  pendingAttachmentRemoveIds: Set<string>;
  pendingDecisionKeys: Set<string>;
  uploadingAttachment: boolean;
  uploadingAttachmentFile?: { name: string; bytes: number };
  loadingHistory: boolean;
  toolCount: number;
  toolCountPulse: boolean;
  pendingToolGenTurnIds: Set<string>;
  toolGenSessionBusy: boolean;
  onLoadMoreHistory: (session: Session) => void;
  onSend: (text: string) => Promise<boolean>;
  onCancel: () => Promise<void>;
  onUpload: (file: File) => Promise<void>;
  onRemoveAttachment: (attachmentId: string) => void;
  onDecisionReply: (decision: Decision, reply: "accept" | "decline") => void;
  onOpenToolRepo: () => void;
  onRequestToolGen: (turnId: string) => void;
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const previousScrollMetrics = useRef<ScrollMetrics | null>(null);
  const followThreadLatest = useRef(true);
  const [visibleTurnCount, setVisibleTurnCount] = useState(INITIAL_RENDERED_TURNS);
  const [draftsBySession, setDraftsBySession] = useState<Record<string, string>>({});
  const submittingDraftSessionIdsRef = useRef<Set<string>>(new Set());
  const [submittingDraftSessionIds, setSubmittingDraftSessionIds] = useState<Set<string>>(() => new Set());
  const turns = activeSession?.turns ?? [];
  const activeSessionId = activeSession?.session_id;
  const draft = draftForSession(draftsBySession, activeSessionId);
  const submittingDraft = !!activeSessionId && submittingDraftSessionIds.has(activeSessionId);
  const sendLabel = isCancelling ? "Cancellation in progress" : activeSession?.state === "working" ? "Send supplement" : "Send message";
  const lockedControlHint = sessionInteractionLocked ? "Mem switch is in progress" : "";
  const missingSessionHint = activeSession ? "" : "Create a session before using Timem";
  const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";
  const composerHint = missingSessionHint || lockedControlHint || (uploadingAttachment ? `${uploadingAttachmentText} · send is paused until it finishes` : activeSession?.state === "working" ? "Enter to add supplement · Shift+Enter for newline" : "Enter to send · Shift+Enter for newline");
  const attachTitle = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");
  const attachLabel = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");
  const toolRepoTitle = missingSessionHint || lockedControlHint || `Open ToolRepo · ${toolCount} tools`;
  const toolRepoLabel = missingSessionHint || lockedControlHint || `Open ToolRepo with ${toolCount} tools`;
  const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);
  const attachedFileCount = activeSession?.attachments.length ?? 0;
  const attachmentSummary = attachedFileCount === 1 ? "1 file attached" : `${attachedFileCount} files attached`;
  const attachmentStripLabel = uploadingAttachment
    ? `${attachmentSummary}; ${uploadingAttachmentText}`
    : `Files attached to the next message; ${attachmentSummary}`;
  const composerHintId = `composer-hint-${activeSessionId || "empty"}`;
  const hiddenTurnCount = Math.max(0, turns.length - visibleTurnCount);
  const canLoadStoredHistory = !!activeSession?.history_has_more && !!activeSession.history_before_cursor;
  const visibleTurns = hiddenTurnCount > 0 ? turns.slice(-visibleTurnCount) : turns;
  const historyButtonLabel = sessionInteractionLocked
    ? "Earlier history is locked while switching mem"
    : loadingHistory
      ? "Loading earlier history…"
      : hiddenTurnCount > 0
        ? `Load ${Math.min(TURN_HISTORY_PAGE_SIZE, hiddenTurnCount)} earlier tasks`
        : "Load earlier history";
  const latestTurn = turns.at(-1);
  const latestTurnVersion = `${latestTurn?.turn_id ?? ""}:${latestTurn?.events.length ?? 0}:${latestTurn?.user_entries.length ?? 0}:${latestTurn?.final_answer?.length ?? 0}:${latestTurn?.completion ? 1 : 0}`;
  const liveSessionKey = sessionIds.join("\u0000");
  const welcomeTitle = activeSession ? "Ready when you are." : "Create a session to start.";
  const welcomeText = activeSession ? "Ask Timem to investigate, write, or work with you." : "Use New session to choose a workspace and runtime profile.";

  useEffect(() => setVisibleTurnCount(INITIAL_RENDERED_TURNS), [activeSession?.session_id]);

  useEffect(() => {
    setDraftsBySession((current) => pruneSessionDrafts(current, sessionIds));
    if (pruneSessionSubmissionLocks(submittingDraftSessionIdsRef, sessionIds)) {
      setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    }
  }, [liveSessionKey]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const previous = previousScrollMetrics.current;
    if (!viewport || !previous) return;
    viewport.scrollTop = preservePrependScrollTop(previous, viewport.scrollHeight);
    previousScrollMetrics.current = null;
  }, [visibleTurnCount, turns.length]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !latestTurn?.turn_id) return;
    followThreadLatest.current = true;
    viewport.scrollTop = viewport.scrollHeight;
  }, [latestTurn?.turn_id]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !followThreadLatest.current || previousScrollMetrics.current) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [latestTurnVersion]);

  const loadEarlierTurns = () => {
    if (sessionInteractionLocked) return;
    if (hiddenTurnCount === 0 && (!activeSession || !canLoadStoredHistory || loadingHistory)) return;
    if (viewportRef.current) {
      previousScrollMetrics.current = {
        scrollTop: viewportRef.current.scrollTop,
        scrollHeight: viewportRef.current.scrollHeight,
      };
    }
    if (hiddenTurnCount > 0) {
      setVisibleTurnCount((count) => Math.min(turns.length, count + TURN_HISTORY_PAGE_SIZE));
    } else if (activeSession) {
      setVisibleTurnCount((count) => count + TURN_HISTORY_PAGE_SIZE);
      onLoadMoreHistory(activeSession);
    }
  };
  const submitDraft = async () => {
    const reserved = reserveSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId, draftsBySession);
    if (reserved === null) return;
    setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    let sent = false;
    try {
      sent = await onSend(reserved.text);
    } finally {
      setDraftsBySession((current) => finishSessionDraftSubmission(submittingDraftSessionIdsRef, current, reserved.sessionId, reserved.text, sent));
      setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    }
  };

  return <ThreadPrimitive.Root className="aui-thread">
    <ThreadPrimitive.Viewport
      ref={viewportRef}
      className="chat-scroll aui-thread-viewport"
      autoScroll
      scrollToBottomOnInitialize
      scrollToBottomOnThreadSwitch
      onScroll={(event) => {
        followThreadLatest.current = isNearScrollBottom({
          scrollTop: event.currentTarget.scrollTop,
          scrollHeight: event.currentTarget.scrollHeight,
          clientHeight: event.currentTarget.clientHeight,
        });
        if (!sessionInteractionLocked && event.currentTarget.scrollTop <= 48 && (hiddenTurnCount > 0 || canLoadStoredHistory)) loadEarlierTurns();
      }}
    >
      {(activeSession?.turns.length ?? 0) === 0 &&
        <div className="welcome"><Sparkles size={24}/><h2>{welcomeTitle}</h2><p>{welcomeText}</p></div>
      }
      {(hiddenTurnCount > 0 || canLoadStoredHistory) && <button type="button" className={`load-history ${loadingHistory ? "loading" : ""}`} title={historyButtonLabel} aria-label={historyButtonLabel} aria-live="polite" aria-busy={loadingHistory || undefined} disabled={loadingHistory || sessionInteractionLocked} onClick={loadEarlierTurns}>{loadingHistory && <LoaderCircle size={13} aria-hidden="true"/>}<span>{historyButtonLabel}</span></button>}
      {visibleTurns.map((turn) => <TurnInteraction
        key={turn.turn_id}
        sessionId={activeSession?.session_id ?? ""}
        turn={turn}
        decisions={decisions.filter((decision) => decision.turnId === turn.turn_id)}
        sessionInteractionLocked={sessionInteractionLocked}
        pendingDecisionKeys={pendingDecisionKeys}
        toolGenPending={pendingToolGenTurnIds.has(turn.turn_id)}
        toolGenBlocked={toolGenSessionBusy && !pendingToolGenTurnIds.has(turn.turn_id)}
        onDecisionReply={onDecisionReply}
        onRequestToolGen={onRequestToolGen}
      />)}
      <ThreadPrimitive.ViewportFooter className="composer-wrap aui-thread-footer">
        <ThreadPrimitive.ScrollToBottom asChild><button type="button" className="scroll-to-bottom" title="Scroll to latest message" aria-label="Scroll to latest message"><ArrowDown size={16} aria-hidden="true"/></button></ThreadPrimitive.ScrollToBottom>
        {!!activeSession && (!!activeSession.attachments.length || uploadingAttachment) && <div className="attachment-strip" aria-label={attachmentStripLabel} aria-live="polite" aria-busy={uploadingAttachment || undefined}>{attachedFileCount > 0 && <div className="attachment-summary" title={attachmentSummary}><Paperclip size={13}/><span>{attachmentSummary}</span></div>}{uploadingAttachment && <div className="pending-attachment uploading" role="status" aria-label={uploadingAttachmentFile ? `${uploadingAttachmentText}, ${formatBytes(uploadingAttachmentFile.bytes)}` : uploadingAttachmentText} title={uploadingAttachmentFile?.name ?? uploadingAttachmentText}><span className="upload-dot" aria-hidden="true"/><span className="pending-attachment-name">{uploadingAttachmentFile?.name ?? "Uploading file…"}</span>{uploadingAttachmentFile && <small>{formatBytes(uploadingAttachmentFile.bytes)}</small>}</div>}{activeSession.attachments.map((attachment) => {
          const removing = pendingAttachmentRemoveIds.has(`${activeSession.session_id}:${attachment.id}`);
          const removeLabel = removing ? `Removing ${attachment.name}` : sessionInteractionLocked ? `Cannot remove ${attachment.name} while session is switching mem` : `Remove ${attachment.name}`;
          return <div className="pending-attachment" key={attachment.id} title={attachment.name}><Paperclip size={13}/><span className="pending-attachment-name">{attachment.name}</span><small>{formatBytes(attachment.bytes)}</small><button type="button" title={removeLabel} aria-label={removeLabel} aria-busy={removing || undefined} disabled={removing || sessionInteractionLocked} onClick={() => onRemoveAttachment(attachment.id)}>{removing ? "…" : <X size={13}/>}</button></div>;
        })}</div>}
        {activeSession && <div className="composer-cwd" title={activeSession.current_dir} aria-label={`Current working directory: ${activeSession.current_dir}`}><FolderOpen size={13} aria-hidden="true"/><span>{tailPath(activeSession.current_dir, 64)}</span></div>}
        <form className="composer" onSubmit={(event) => { event.preventDefault(); void submitDraft(); }}>
          <textarea
            value={draft}
            placeholder={!activeSession ? "Create a session to start…" : sessionInteractionLocked ? "Switching mem…" : activeSession.state === "working" ? "继续输入…" : "Ask Timem to investigate, write, or work with you."}
            aria-label="Message Timem"
            aria-describedby={composerHintId}
            title={composerHint}
            disabled={!activeSession || sessionInteractionLocked}
            onChange={(event) => setDraftsBySession((current) => setSessionDraft(current, activeSessionId, event.target.value))}
            onKeyDown={(event) => {
              if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                event.preventDefault();
                void submitDraft();
              }
            }}
          />
          <div className="composer-actions"><span id={composerHintId}>{composerHint}</span><div className="composer-buttons"><button className={`attach-button ${uploadingAttachment ? "uploading" : ""}`} type="button" title={attachTitle} aria-label={attachLabel} disabled={!activeSession || uploadingAttachment || sessionInteractionLocked} onClick={() => fileInput.current?.click()}>{uploadingAttachment ? <LoaderCircle size={17}/> : <Paperclip size={17}/>}</button><input ref={fileInput} className="file-input" type="file" disabled={!activeSession || uploadingAttachment || sessionInteractionLocked} onChange={(event) => { const file = event.target.files?.[0]; event.currentTarget.value = ""; if (file) void onUpload(file); }}/><button className={`toolrepo-toggle ${toolCountPulse ? "count-pulse" : ""}`} type="button" title={toolRepoTitle} aria-label={toolRepoLabel} disabled={!activeSession || sessionInteractionLocked} onClick={onOpenToolRepo}><Wrench size={17}/><span>{toolCount}</span></button><button className={`send-button ${submittingDraft ? "sending" : ""}`} type="submit" title={effectiveSendLabel} aria-label={effectiveSendLabel} disabled={!activeSession || !draft.trim() || submittingDraft || uploadingAttachment || sessionInteractionLocked}>{submittingDraft ? <LoaderCircle size={17}/> : <Send size={17}/>}</button>{activeSession?.state === "working" && <button className={`stop-button ${isCancelling ? "sending" : ""}`} type="button" title={isCancelling ? "Cancellation requested" : lockedControlHint || "Cancel current turn"} aria-label={isCancelling ? "Cancellation requested" : lockedControlHint || "Cancel current turn"} disabled={isCancelling || sessionInteractionLocked} onClick={() => void onCancel()}>{isCancelling ? <LoaderCircle size={17}/> : <CircleStop size={17}/>} {isCancelling ? "Stopping…" : "Stop"}</button>}</div></div>
        </form>
      </ThreadPrimitive.ViewportFooter>
    </ThreadPrimitive.Viewport>
  </ThreadPrimitive.Root>;
}

function TurnInteraction({ sessionId, turn, decisions, sessionInteractionLocked, pendingDecisionKeys, toolGenPending, toolGenBlocked, onDecisionReply, onRequestToolGen }: { sessionId: string; turn: WebTurn; decisions: Decision[]; sessionInteractionLocked: boolean; pendingDecisionKeys: Set<string>; toolGenPending: boolean; toolGenBlocked: boolean; onDecisionReply: (decision: Decision, reply: "accept" | "decline") => void; onRequestToolGen: (turnId: string) => void }) {
  const workScrollRef = useRef<HTMLDivElement | null>(null);
  const followLatest = useRef(true);
  const previousUpdateCount = useRef(turn.events.length + decisions.length);
  const [pendingUpdates, setPendingUpdates] = useState(0);
  const [showCompletedWork, setShowCompletedWork] = useState(true);
  const lifecycleEvents = coalesceActionLifecycle(turn.events);
  const visibleEvents = lifecycleEvents.slice(-MAX_RENDERED_TURN_EVENTS);
  const omitted = lifecycleEvents.length - visibleEvents.length;
  const hasVisibleProcess = visibleEvents.some((event) => activityFromTurnEvent(event, turn.turn_id) !== null) || decisions.length > 0 || turn.state === "working";
  const isToolGenTurn = turn.turn_id.startsWith("web_toolgen_turn_")
    || turn.user_entries.some((entry) => entry.kind === "toolgen_instruction")
    || turn.events.some((event) => event.source === "core_topic" && (event.payload.topic as { name?: string } | undefined)?.name === "core.toolgen");
  const canCollapseCompletedWork = turn.state !== "working" && !!turn.final_answer;
  const showWorkStream = !canCollapseCompletedWork || showCompletedWork;

  useLayoutEffect(() => {
    const scroll = workScrollRef.current;
    const updateCount = turn.events.length + decisions.length;
    const added = Math.max(0, updateCount - previousUpdateCount.current);
    previousUpdateCount.current = updateCount;
    if (!scroll) return;
    if (followLatest.current) {
      scroll.scrollTop = scroll.scrollHeight;
      setPendingUpdates(0);
    } else if (added > 0) {
      setPendingUpdates((count) => count + added);
    }
  }, [turn.events.length, decisions.length]);

  const scrollWorkToLatest = () => {
    const scroll = workScrollRef.current;
    if (!scroll) return;
    scroll.scrollTo({ top: scroll.scrollHeight, behavior: "smooth" });
    followLatest.current = true;
    setPendingUpdates(0);
  };

  return <article className="turn-interaction" data-turn-id={turn.turn_id}>
    {!!turn.user_entries.length && <section className="turn-user-frame">
      <div className="turn-user-content">{turn.user_entries.map((entry, index) => <div className={`turn-user-entry ${entry.kind}`} key={`${entry.created_at_ms}-${index}`}>
        {entry.kind === "supplement" && <span>[补充]</span>}
        {entry.kind === "approval" && <span>[审批]</span>}
        <MarkdownContent text={entry.text}/>
        {!!entry.attachments?.length && <div className="turn-entry-attachments">{entry.attachments.map((attachment) => <span key={attachment.id} title={attachment.path}><Paperclip size={13}/><i aria-hidden="true">:</i><b>{attachment.name}</b><small>{formatBytes(attachment.bytes)}</small></span>)}</div>}
      </div>)}</div>
    </section>}
    {hasVisibleProcess && <section className={`turn-assistant-frame ${turn.state} ${showWorkStream ? "" : "collapsed-work"}`}>
      {(turn.state === "working" || canCollapseCompletedWork) && <div className="turn-assistant-heading"><span className={`working-chip${isToolGenTurn ? " toolgen-working" : ""}`} role={turn.state === "working" ? "status" : undefined} aria-live={turn.state === "working" ? "polite" : undefined}>{turn.state === "working" ? isToolGenTurn ? <Wrench size={11}/> : <span className="pulse"/> : <CheckCheck size={11}/>} {turn.state === "working" ? isToolGenTurn ? "Generating tools…" : "working" : "work details"}</span>{canCollapseCompletedWork && <button type="button" className="work-collapse-toggle" title={showCompletedWork ? "Hide completed work details" : "Show completed work details"} aria-label={showCompletedWork ? "Hide completed work details" : "Show completed work details"} aria-expanded={showCompletedWork} onClick={() => setShowCompletedWork((visible) => !visible)}>{showCompletedWork ? "Hide" : "Show"}</button>}</div>}
      {showWorkStream && <div className={`turn-work-scroll ${pendingUpdates > 0 ? "has-pending-updates" : ""}`} role="region" aria-label={isToolGenTurn ? "ToolGen work stream" : "Task work stream"} ref={workScrollRef} onScroll={(event) => {
        const remaining = event.currentTarget.scrollHeight - event.currentTarget.scrollTop - event.currentTarget.clientHeight;
        followLatest.current = remaining < 36;
        if (followLatest.current) setPendingUpdates(0);
      }}>
        {omitted > 0 && <div className="turn-events-omitted">{omitted} earlier work updates are retained by the host but not rendered.</div>}
        {visibleEvents.map((event) => <TurnEventView key={event.event_id} event={event} sessionId={sessionId}/>)}
        {decisions.map((decision, index) => <InlineDecision key={decisionKey(decision)} decision={decision} pending={pendingDecisionKeys.has(decisionKey(decision))} locked={sessionInteractionLocked} position={index + 1} total={decisions.length} onReply={(reply) => onDecisionReply(decision, reply)} />)}
        {turn.state === "working" && <LiveTurnUsage turn={turn}/>}
        {visibleEvents.length === 0 && decisions.length === 0 && turn.state === "working" && <div className={`working-indicator${isToolGenTurn ? " toolgen-working" : ""}`} role="status" aria-live="polite"><span className="pulse"/>{isToolGenTurn ? "Generating tools…" : "Waiting for the first runtime update…"}</div>}
      </div>}
      {showWorkStream && pendingUpdates > 0 && <button type="button" className="turn-new-updates" title="Scroll to latest work update" aria-live="polite" aria-label={`${pendingUpdates} new work update${pendingUpdates === 1 ? "" : "s"}; scroll to latest`} onClick={scrollWorkToLatest}><ArrowDown size={13}/>{pendingUpdates} new update{pendingUpdates === 1 ? "" : "s"}</button>}
    </section>}
    {turn.final_answer && <FinalAnswerDelivery text={turn.final_answer} completion={turn.completion} toolGenPending={toolGenPending} toolGenBlocked={toolGenBlocked} onToolGen={isToolGenTurn ? undefined : () => onRequestToolGen(turn.turn_id)}/>}
    {!turn.final_answer && turn.completion && <section className="turn-completion-only"><CompletionCard completion={turn.completion}/></section>}
  </article>;
}

function FinalAnswerDelivery({ text, completion, toolGenPending, toolGenBlocked, onToolGen }: { text: string; completion: WebTurn["completion"]; toolGenPending: boolean; toolGenBlocked: boolean; onToolGen?: () => void }) {
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(text, {
    idle: "Copy answer",
    copied: "Answer copied",
    failed: "Copy answer failed",
  });
  return <section className="turn-final-delivery">
    <div className="turn-final-toolbar"><button type="button" className={`final-copy ${copyClass}`} title={copyLabel} aria-label={copyLabel} onClick={() => void copy()}>{copyState === "copied" ? <CheckCheck size={13}/> : <Copy size={13}/>}<span aria-live="polite">{copyLabel}</span></button></div>
    <div className="message-content"><MarkdownContent text={text}/></div>
    {completion && <CompletionCard completion={completion} toolGenPending={toolGenPending} toolGenBlocked={toolGenBlocked} onToolGen={onToolGen}/>}
  </section>;
}

function ContextUsageBar({ session }: { session: Session | undefined }) {
  const usage = session ? sessionContextUsage(session) : undefined;
  const limit = session?.max_llm_input_tokens || undefined;
  const ratio = usage && limit ? Math.min(100, Math.ceil((usage.prompt_tokens ?? 0) * 100 / limit)) : 0;
  const level = ratio >= 90 ? "critical" : ratio >= 75 ? "warning" : "normal";
  const contextUsageLabel = usage && limit
    ? `Context usage ${ratio}% · ${formatTokens(usage.prompt_tokens)} / ${formatTokens(limit)} input tokens`
    : "Context usage waiting for runtime usage";
  return <section className={`context-usage-bar ${level}`} title={contextUsageLabel} aria-label={contextUsageLabel}>
    <span>Context</span><strong>{formatTokens(usage?.prompt_tokens) ?? "—"}{limit ? ` / ${formatTokens(limit)}` : ""}</strong>
    <div className="context-usage-meter" aria-hidden="true"><span style={{ width: `${ratio}%` }}/></div><small>{usage && limit ? `${ratio}%` : "waiting for usage"}</small>
  </section>;
}

function LiveTurnUsage({ turn }: { turn: WebTurn }) {
  const usage = turnLiveUsage(turn);
  if (!usage) return null;
  return <div className="live-turn-usage" aria-label="Current task token usage">
    <span><b>Task</b> ▲{formatTokens(usage.total.prompt_tokens) ?? "0"} ▼{formatTokens(usage.total.completion_tokens) ?? "0"}</span>
    <span><b>Latest</b> △{formatTokens(usage.latest.prompt_tokens) ?? "0"} ▽{formatTokens(usage.latest.completion_tokens) ?? "0"}</span>
    {!!usage.total.cached_tokens && <span><b>KVC</b> {formatTokens(usage.total.cached_tokens)}</span>}
  </div>;
}

function TurnEventView({ event, sessionId }: { event: WebTurnEvent; sessionId: string }) {
  const activity = activityFromTurnEvent(event, sessionId);
  if (!activity) return null;
  if (activity.kind === "context_compact") return <ContextCompactNotice activity={activity}/>;
  if (activity.kind === "toolgen") return <ToolGenNotice activity={activity}/>;
  if (activity.tone === "action") return <ToolActivity activity={activity}/>;
  return <div className={`turn-work-item ${activity.tone}`}>
    <span className="activity-mark">{activity.tone === "thinking" ? "💡" : activity.tone === "warning" ? "⚠️" : activity.tone === "error" ? "×" : "i"}</span>
    <div>{activity.title && <strong>{activity.title}</strong>}{activity.detail && <div className="turn-work-detail"><MarkdownContent text={activity.detail}/></div>}{activity.code && <MarkdownContent text={fencedCode(activity.code_language ?? "text", activity.code)}/>}</div>
  </div>;
}

function ToolGenNotice({ activity }: { activity: Activity }) {
  const [open, setOpen] = useState(false);
  const hasDetail = !!activity.detail?.trim();
  if (!hasDetail) return <blockquote className={`toolgen-notice ${activity.toolgen_phase ?? ""}`}><span>{activity.title}</span></blockquote>;
  const collapse = () => setOpen(false);
  const summaryLabel = `${open ? "收起" : "展开"} ToolGen 详情${activity.title ? `：${activity.title}` : ""}`;
  return <details className={`toolgen-notice ${activity.toolgen_phase ?? ""}`} open={open} onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary title={open ? "收起 ToolGen 详情" : "展开 ToolGen 详情"} aria-label={summaryLabel}><ChevronRight size={13}/><span>{activity.title}</span></summary>
    <div><button type="button" className="toolgen-collapse top" title="Collapse ToolGen details" aria-label="Collapse ToolGen details" onClick={collapse}>收起详情</button><MarkdownContent text={activity.detail ?? ""}/><button type="button" className="toolgen-collapse" title="Collapse ToolGen details" aria-label="Collapse ToolGen details" onClick={collapse}>收起详情</button></div>
  </details>;
}

function ToolActivity({ activity }: { activity: Activity }) {
  const [open, setOpen] = useState(false);
  const status = activity.tool_status || "running";
  const running = status === "running" || status === "background_running";
  const invocationPreview = toolInvocationPreview(activity);
  const hasExpandableDetail = !!activity.detail?.trim() || !!activity.code?.trim();
  const collapse = () => setOpen(false);
  const toolName = toolDisplayName(activity.tool_name || activity.title);
  const summaryLabel = `${open ? "收起" : "展开"}工具详情：${toolName}`;
  const summaryContent = <>
    <span className="tool-activity-icon">{activity.tool_name === "run_bash" ? <Terminal size={14}/> : <Wrench size={14}/>}</span>
    <b>{toolName}</b>
    <span className="tool-activity-status">{humanizeToolStatus(status)}</span>
    {invocationPreview && <code title={invocationPreview}>{invocationPreview}</code>}
  </>;
  if (!hasExpandableDetail) return <div className={`tool-activity tool-activity-static ${running ? "running" : "settled"}`} aria-busy={running || undefined}>
    {summaryContent}
  </div>;
  return <details className={`tool-activity ${running ? "running" : "settled"}`} aria-busy={running || undefined} open={open} onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary title={open ? "收起工具详情" : "展开工具详情"} aria-label={summaryLabel}>
      {summaryContent}
      <ChevronRight className="tool-activity-chevron" size={14}/>
    </summary>
    <div className="tool-activity-body">
      <button type="button" className="tool-activity-collapse top" title={`Collapse ${toolName} details`} aria-label={`Collapse ${toolName} details`} onClick={collapse}>收起详情</button>
      {activity.detail && <div className="turn-work-detail"><MarkdownContent text={activity.detail}/></div>}
      {activity.code && <MarkdownContent text={fencedCode(activity.code_language ?? "text", activity.code)}/>}
      <button type="button" className="tool-activity-collapse" title={`Collapse ${toolName} details`} aria-label={`Collapse ${toolName} details`} onClick={collapse}>收起详情</button>
    </div>
  </details>;
}

function toolInvocationPreview(activity: Activity) {
  const code = activity.code?.split("\n", 1)[0]?.trim();
  if (code) return code;
  return activity.detail?.split("\n", 1)[0]?.trim();
}

function humanizeToolStatus(status: string) {
  if (status === "background_running") return "background running";
  if (status === "timeout") return "timed out";
  return status.replaceAll("_", " ");
}

function activityFromTurnEvent(event: WebTurnEvent, sessionId: string): Activity | null {
  if (event.source === "core_topic") return activityFromTopic(event.payload as unknown as import("./protocol").CoreTopicEvent);
  if (event.source !== "worker_activity") return null;
  const kind = String(event.payload.kind ?? "worker_event");
  if (kind === "model_request" || kind === "model_response") return null;
  const detail = Object.entries(event.payload).filter(([key]) => key !== "kind").map(([key, value]) => `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`).join("\n");
  return { id: event.event_id, sessionId, tone: kind.includes("error") ? "error" : kind.includes("retry") || kind.includes("discarded") ? "warning" : "notice", title: kind.replaceAll("_", " "), detail, createdAt: event.created_at_ms };
}

function ContextCompactNotice({ activity }: { activity: Activity }) {
  const before = activity.before_tokens;
  const after = activity.after_tokens;
  const ratio = before && after !== undefined ? Math.max(6, Math.min(100, (after / before) * 100)) : 36;
  return <section className="context-compact-notice" aria-label="Context compacted">
    <div className="compact-icon"><Gauge size={17}/></div>
    <div className="compact-copy"><span>Context compacted</span><strong>{formatTokens(before) ?? "?"} → {formatTokens(after) ?? "?"}</strong></div>
    <div className="compact-meter" aria-hidden="true"><span className="compact-before"/><span className="compact-after" style={{ width: `${ratio}%` }}/></div>
  </section>;
}

function MarkdownContent({ text }: { text: string }) {
  return <div className="markdown-body"><ReactMarkdown
    remarkPlugins={[remarkGfm]}
    rehypePlugins={[rehypeHighlight]}
    components={{
      a: ({ node: _node, ...props }) => <a {...props} target="_blank" rel="noreferrer"/>,
      pre: CodeBlock,
      table: ({ node: _node, ...props }) => <div className="table-scroll" role="region" tabIndex={0} aria-label="Scrollable table. Use horizontal scroll to inspect all columns."><table {...props}/></div>,
    }}
  >{text}</ReactMarkdown></div>;
}

function CodeBlock({ children }: React.ComponentPropsWithoutRef<"pre">) {
  const child = Children.count(children) === 1 ? Children.only(children) : null;
  const className = isValidElement<{ className?: string }>(child) ? child.props.className ?? "" : "";
  const language = className.match(/(?:^|\s)language-([^\s]+)/)?.[1] ?? "text";
  const code = textFromNode(children).replace(/\n$/, "");
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(code, {
    idle: "Copy code",
    copied: "Code copied",
    failed: "Copy code failed",
  });
  return <figure className="code-block">
    <figcaption><span title={language}>{language}</span><button type="button" className={copyClass} onClick={() => void copy()} title={copyLabel} aria-label={copyLabel}>{copyState === "copied" ? <CheckCheck size={14}/> : <Copy size={14}/>}<span aria-live="polite">{copyLabel}</span></button></figcaption>
    <pre>{children}</pre>
  </figure>;
}

function useTimedClipboardCopy(text: string, labels: { idle: string; copied: string; failed: string }) {
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const resetTimerRef = useRef<number | null>(null);
  useEffect(() => () => {
    if (resetTimerRef.current !== null) window.clearTimeout(resetTimerRef.current);
  }, []);
  useEffect(() => {
    if (resetTimerRef.current !== null) {
      window.clearTimeout(resetTimerRef.current);
      resetTimerRef.current = null;
    }
    setCopyState("idle");
  }, [text]);
  const copy = async () => {
    if (resetTimerRef.current !== null) window.clearTimeout(resetTimerRef.current);
    try {
      await navigator.clipboard.writeText(text);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
    resetTimerRef.current = window.setTimeout(() => {
      setCopyState("idle");
      resetTimerRef.current = null;
    }, 1400);
  };
  const copyLabel = copyState === "copied" ? labels.copied : copyState === "failed" ? labels.failed : labels.idle;
  const copyClass = copyState === "copied" ? "copy-success" : copyState === "failed" ? "copy-failed" : "";
  return { copyState, copy, copyLabel, copyClass };
}

function textFromNode(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textFromNode).join("");
  if (isValidElement<{ children?: React.ReactNode }>(node)) return textFromNode(node.props.children);
  return "";
}

function AppearancePanel({ panelRef, appearance, onChange, onClose }: { panelRef: MutableRefObject<HTMLElement | null>; appearance: Appearance; onChange: (appearance: Appearance) => void; onClose: () => void }) {
  const update = <K extends keyof Appearance>(key: K, value: Appearance[K]) => onChange({ ...appearance, [key]: value });
  const descriptionId = "appearance-panel-description";
  return <>
    <div className="appearance-dismiss" aria-hidden="true" onClick={onClose}/>
    <section id="appearance-panel" ref={panelRef} className="appearance-panel" role="dialog" aria-modal="false" aria-label="Appearance settings" aria-describedby={descriptionId} onKeyDown={(event) => { if (event.key === "Escape") onClose(); }}>
      <header><div><span className="eyebrow">APPEARANCE</span><h2>Reading preferences</h2><p id={descriptionId}>Adjust theme, font, and message text size for this browser.</p></div><button type="button" className="icon-button" aria-label="Close appearance settings" onClick={onClose}><X size={16}/></button></header>
      <fieldset><legend>Theme</legend><div className="segmented-control">{(["dark", "light"] as const).map((theme) => <button type="button" title={`Use ${theme} theme`} className={appearance.theme === theme ? "active" : ""} aria-pressed={appearance.theme === theme} key={theme} onClick={() => update("theme", theme)}>{theme === "dark" ? "Dark" : "Light"}</button>)}</div></fieldset>
      <fieldset><legend>Font</legend><div className="appearance-options">{(["system", "serif", "mono"] as const).map((font) => <button type="button" title={`Use ${font} font for chat reading`} className={`${font}-sample ${appearance.font === font ? "active" : ""}`} aria-pressed={appearance.font === font} key={font} onClick={() => update("font", font)}>{font === "system" ? "System" : font === "serif" ? "Serif" : "Mono"}<small>Aa</small></button>)}</div></fieldset>
      <fieldset><legend>Text size</legend><div className="segmented-control text-size-control">{(["small", "medium", "large"] as const).map((size) => <button type="button" title={`Use ${size === "medium" ? "default" : size} text size`} className={appearance.textSize === size ? "active" : ""} aria-pressed={appearance.textSize === size} key={size} onClick={() => update("textSize", size)}>{size === "small" ? "Small" : size === "medium" ? "Default" : "Large"}</button>)}</div></fieldset>
    </section>
  </>;
}

function fencedCode(language: string, code: string) {
  let fence = "```";
  while (code.includes(fence)) fence += "`";
  return `${fence}${language}\n${code}\n${fence}`;
}

function CompletionCard({ completion, toolGenPending = false, toolGenBlocked = false, onToolGen }: { completion: NonNullable<ChatMessage["completion"]>; toolGenPending?: boolean; toolGenBlocked?: boolean; onToolGen?: () => void }) {
  const stats = completion.stats ?? {};
  const cancelled = completion.stop_reason?.toLowerCase() === "cancelledbyuser";
  const facts = [
    [cancelled ? "Cancelled" : "Completed", formatDuration(completion.elapsed_ms)],
    ["LLM", stats.llm_calls],
    ["Input", formatOptionalTokens(stats.prompt_tokens)],
    ["Output", formatOptionalTokens(stats.completion_tokens)],
    ["KVC read", formatOptionalTokens(stats.cached_tokens)],
    ["KVC created", formatOptionalTokens(stats.cache_created_tokens)],
    ["Tools", stats.tool_calls],
    ["Repair", stats.repair_calls],
    ["Memory", formatMemoryOps(stats.mem_reads, stats.mem_writes)],
    ["Compact", formatOptionalTokens(stats.shrunk_tokens)],
  ].filter(([, value]) => value !== undefined && value !== null && value !== "" && value !== 0) as Array<[string, string | number]>;
  return <div className="completion-card" aria-label="Turn completion statistics">
    {facts.map(([label, value]) => <span key={label} title={completionFactTitle(label, completion, stats) ?? `${label}: ${value}`}><b>{label}</b> {value}</span>)}
    {!cancelled && isNotableStopReason(completion.stop_reason) && <span className="completion-status"><b>Status</b> {completion.stop_reason}</span>}
    {completion.repair_issue && <span className="completion-status warning"><b>Last repair</b> {completion.repair_issue}</span>}
    {onToolGen && !cancelled && <button className={`completion-toolgen ${toolGenPending ? "sending" : ""}`} type="button" title={toolGenPending ? "ToolGen is starting for this task…" : toolGenBlocked ? "Another ToolGen task is already running in this session" : "Extract reusable tool from this task"} aria-label={toolGenPending ? "ToolGen is starting for this task" : toolGenBlocked ? "Another ToolGen task is already running in this session" : "Extract reusable tool from this task"} aria-busy={toolGenPending || undefined} disabled={toolGenPending || toolGenBlocked} onClick={onToolGen}>{toolGenPending ? <LoaderCircle size={12}/> : <Wrench size={12}/>}{toolGenPending ? "Starting…" : toolGenBlocked ? "ToolGen busy" : "ToolGen"}</button>}
  </div>;
}

function completionFactTitle(label: string, completion: NonNullable<ChatMessage["completion"]>, stats: Record<string, number | undefined>) {
  if (label === "Completed" || label === "Cancelled") return completion.elapsed_ms === undefined ? undefined : `${completion.elapsed_ms} ms`;
  if (label === "Input") return stats.prompt_tokens === undefined ? undefined : `${stats.prompt_tokens} input tokens`;
  if (label === "Output") return stats.completion_tokens === undefined ? undefined : `${stats.completion_tokens} output tokens`;
  if (label === "KVC read") return stats.cached_tokens === undefined ? undefined : `${stats.cached_tokens} cached input tokens`;
  if (label === "KVC created") return stats.cache_created_tokens === undefined ? undefined : `${stats.cache_created_tokens} cache-created input tokens`;
  if (label === "Compact") return stats.shrunk_tokens === undefined ? undefined : `${stats.shrunk_tokens} compacted tokens`;
  if (label === "Memory") return `${stats.mem_reads ?? 0} memory reads / ${stats.mem_writes ?? 0} memory writes`;
  return undefined;
}

function isNotableStopReason(reason: string | null | undefined) {
  if (!reason) return false;
  return !["finished", "completed", "all_finished", "final_answer"].includes(reason.toLowerCase());
}

function formatTokens(value: number | undefined) {
  if (!value) return value === 0 ? "0" : undefined;
  return value >= 1000 ? `${(value / 1000).toFixed(value >= 10_000 ? 0 : 1)}K` : String(value);
}

function formatOptionalTokens(value: number | undefined) {
  return value ? formatTokens(value) : undefined;
}

function formatDuration(elapsedMs: number | undefined) {
  if (elapsedMs === undefined) return undefined;
  const seconds = Math.max(0, Math.round(elapsedMs / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m${String(seconds % 60).padStart(2, "0")}s`;
}

function formatMemoryOps(reads: number | undefined, writes: number | undefined) {
  if (!reads && !writes) return undefined;
  return `${reads ?? 0}R/${writes ?? 0}W`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function RuntimePanel({ panelRef, server, pendingKeys, onUpdate }: { panelRef: MutableRefObject<HTMLElement | null>; server: Snapshot["server"] | null; pendingKeys: Set<string>; onUpdate: (key: string, value: string) => void }) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  useEffect(() => setDrafts({}), [server?.runtime_options]);
  if (!server) return <section id="runtime-panel" ref={panelRef} className="runtime-card"><Cpu size={16}/><span>Loading runtime settings…</span></section>;
  return <section id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings"><div className="runtime-summary"><Cpu size={16}/><span>Timem {server.version}</span><span>topic protocol v{server.protocol_version}</span><span><FolderOpen size={14}/> localhost:{server.port}</span></div><p>Changes apply to newly created sessions. Existing sessions retain their current runtime configuration.</p><div className="runtime-options">{server.runtime_options.map((option) => {
    const value = drafts[option.key] ?? option.value;
    const pending = pendingKeys.has(option.key);
    const dirty = value !== option.value;
    const inputLabel = `${option.key} current value`;
    const applyLabel = pending ? `Applying ${option.key}` : dirty ? `Apply ${option.key}` : `${option.key} has no changes`;
    return <label key={option.key}><span>{option.key}</span><div><input value={value} title={inputLabel} aria-label={inputLabel} disabled={pending} onChange={(event) => setDrafts((current) => ({ ...current, [option.key]: event.target.value }))}/>{dirty && <button type="button" className="secondary compact runtime-reset" title={`Reset ${option.key} to current value`} aria-label={`Reset ${option.key} to current value`} disabled={pending} onClick={() => setDrafts((current) => { const { [option.key]: _removed, ...rest } = current; return rest; })}>Reset</button>}<button type="button" className="secondary compact" title={applyLabel} aria-label={applyLabel} disabled={pending || !dirty} onClick={() => onUpdate(option.key, value)}>{pending ? "Applying…" : "Apply"}</button></div></label>;
  })}</div></section>;
}

const SESSION_RUNTIME_FIELDS = [
  ["TIMEM_GATEWAY_PROVIDER", "Provider", "text"],
  ["TIMEM_MODEL", "Model", "text"],
  ["TIMEM_API_PROTOCOL", "API protocol", "api_protocol"],
  ["TIMEM_RESPONSE_PROTOCOL", "Response protocol", "response_protocol"],
  ["TIMEM_BASE_URL", "Base URL", "text"],
  ["TIMEM_API_KEY", "API key", "password"],
  ["TIMEM_TIMEOUT", "Timeout (seconds)", "number"],
  ["TIMEM_MAX_LLM_INPUT", "Max input tokens", "text"],
  ["TIMEM_MAX_LLM_OUTPUT", "Max output tokens", "text"],
  ["TIMEM_BASH_APPROVAL", "Bash approval", "bash_approval"],
  ["TIMEM_WORK_INSTRUCTIONS", "AGENTS/CLAUDE loading", "work_instructions"],
  ["TIMEM_ENABLE_THINKING", "Enable thinking", "boolean"],
  ["TIMEM_REASONING_EFFORT", "Reasoning effort", "text"],
  ["TIMEM_STREAM", "Stream response", "boolean"],
] as const;

function NewSessionDialog({ workspaces, runtimeDefaults, creating, memSwitching, onClose, onCreate }: {
  workspaces: string[];
  runtimeDefaults: Snapshot["server"]["session_env_defaults"];
  creating: boolean;
  memSwitching: boolean;
  onClose: () => void;
  onCreate: (command: Extract<ClientCommand, { type: "session_create" }>) => void;
}) {
  const [displayName, setDisplayName] = useState("");
  const [workspaceDir, setWorkspaceDir] = useState(workspaces[0] ?? "");
  const [env, setEnv] = useState<Record<string, string>>({});
  const updateEnv = (key: string, value: string) => setEnv((current) => ({ ...current, [key]: value }));
  const resetEnv = (key: string) => setEnv((current) => { const { [key]: _removed, ...rest } = current; return rest; });
  const createDecision = sessionCreateDecision(displayName, workspaceDir, env, creating, memSwitching);
  const canCreateSession = createDecision.kind === "send";
  const closeIfIdle = () => { if (!creating) onClose(); };
  const submit = () => { if (createDecision.kind === "send") onCreate(createDecision.command); };
  const descriptionId = "new-session-dialog-description";
  const statusId = "new-session-dialog-status";
  const describedBy = creating ? `${descriptionId} ${statusId}` : descriptionId;
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss create session" onClick={closeIfIdle}><section className="decision-modal session-modal" role="dialog" aria-modal="true" aria-label="Create session" aria-describedby={describedBy} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") closeIfIdle(); }}><div className="modal-titlebar"><div><span className="eyebrow">NEW SESSION</span><h2>Start a session</h2></div><button type="button" className="icon-button" title="Close create session" aria-label="Close create session" disabled={creating} onClick={closeIfIdle}><X size={16}/></button></div><p id={descriptionId}>Choose a workspace and optional runtime overrides for this session.</p>{creating && <p id={statusId} className="mem-validation" role="status" aria-live="polite">Creating session…</p>}<div className="session-modal-scroll"><label>Display name<input autoFocus value={displayName} placeholder="Optional name" disabled={creating} onChange={(event) => setDisplayName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); submit(); } }}/></label><label>Workspace<select value={workspaceDir} disabled={creating || workspaces.length === 0} onChange={(event) => setWorkspaceDir(event.target.value)}>{workspaces.length === 0 ? <option value="">No workspace available</option> : workspaces.map((workspace) => <option value={workspace} key={workspace} title={workspace}>{tailPath(workspace, 64)}</option>)}</select></label>{workspaces.length === 0 && <p className="mem-hint">No workspace is available from the runtime snapshot. Reconnect Timem Web or check the host workspace configuration.</p>}<details className="session-runtime-overrides"><summary>Runtime environment</summary><div className="session-runtime-grid">{SESSION_RUNTIME_FIELDS.map(([key, label, kind]) => <label key={key}><span>{label}<small>{key}</small></span><div className="session-runtime-control">{kind === "api_protocol" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "default"}</option><option value="openai-compatible">openai-compatible</option><option value="openai-responses">openai-responses</option><option value="anthropic">anthropic</option></select> : kind === "response_protocol" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "xml"}</option><option value="xml">xml</option><option value="json">json</option><option value="markdown">markdown</option></select> : kind === "bash_approval" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "ask"}</option><option value="ask">ask</option><option value="approve">approve</option></select> : kind === "work_instructions" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "silent"}</option><option value="silent">silent</option><option value="ask">ask</option><option value="off">off</option></select> : kind === "boolean" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "false"}</option><option value="true">true</option><option value="false">false</option></select> : <input type={kind} value={env[key] ?? ""} min={kind === "number" ? 1 : undefined} disabled={creating} autoComplete={kind === "password" ? "new-password" : undefined} placeholder={kind === "password" ? "Optional session-only key" : `Inherit · ${runtimeDefaults[key] ?? "default"}`} onChange={(event) => updateEnv(key, event.target.value)}/>} {env[key] !== undefined && <button type="button" className="session-runtime-reset" title={`Reset ${label} to inherited value`} aria-label={`Reset ${label} to inherited value`} disabled={creating} onClick={() => resetEnv(key)}>Reset</button>}</div></label>)}</div></details></div><div className="decision-actions"><button type="button" className="secondary" disabled={creating} onClick={closeIfIdle}>Cancel</button><button type="button" className={`primary ${creating ? "sending" : ""}`} disabled={!canCreateSession} onClick={submit}>{creating ? <LoaderCircle size={16}/> : <Plus size={16}/>} {creating ? "Creating…" : "Create session"}</button></div></section></div>;
}

function ToolGenDialog({ pending, onClose, onSubmit }: { pending: boolean; onClose: () => void; onSubmit: (text: string) => void }) {
  const [instruction, setInstruction] = useState("");
  const closeIfIdle = () => { if (!pending) onClose(); };
  const submit = () => { if (!pending) onSubmit(instruction.trim()); };
  const descriptionId = "toolgen-dialog-description";
  const statusId = "toolgen-dialog-status";
  const describedBy = pending ? `${descriptionId} ${statusId}` : descriptionId;
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss ToolGen dialog" onClick={closeIfIdle}><section className="decision-modal toolgen-dialog" role="dialog" aria-modal="true" aria-label="Generate reusable tool" aria-describedby={describedBy} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") closeIfIdle(); }}><div className="modal-titlebar"><div><span className="eyebrow">TOOLGEN</span><h2>Extract reusable tool</h2></div><button type="button" className="icon-button" title="Close ToolGen dialog" aria-label="Close ToolGen dialog" disabled={pending} onClick={closeIfIdle}><X size={16}/></button></div><p id={descriptionId}>Timem will preserve reusable work from the completed task as one or more standalone script tools. Add optional guidance below.</p>{pending && <p id={statusId} className="toolgen-dialog-status" role="status" aria-live="polite">Starting ToolGen and opening a generating-tools task…</p>}<label>Additional guidance<textarea autoFocus value={instruction} disabled={pending} placeholder="Optional: preferred interface, language, scope, or reusable workflow…" onChange={(event) => setInstruction(event.target.value)} onKeyDown={(event) => { if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); submit(); } }}/><small className="toolgen-dialog-hint">Cmd/Ctrl+Enter to generate; Escape closes before it starts.</small></label><div className="decision-actions"><button type="button" className="secondary" disabled={pending} onClick={closeIfIdle}>Cancel</button><button type="button" className={`primary ${pending ? "sending" : ""}`} disabled={pending} onClick={submit}>{pending ? <LoaderCircle size={16}/> : <Wrench size={15}/>} {pending ? "Starting…" : "Generate tool"}</button></div></section></div>;
}

function MemSwitchDialog({ current, pending, onClose, onSwitch }: {
  current: string;
  pending: boolean;
  onClose: () => void;
  onSwitch: (space: string) => void;
}) {
  const [space, setSpace] = useState(current);
  const cleaned = space.trim();
  const invalid = !cleaned || cleaned === "." || cleaned === ".." || cleaned.includes("/") || cleaned.includes("\\") || cleaned.includes("..");
  const validationText = pending
    ? "Switching mem space…"
    : invalid
      ? "Use a simple mem space name without slashes or '..'."
      : cleaned === current
        ? "This is the current mem space."
        : "";
  const closeIfIdle = () => { if (!pending) onClose(); };
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss mem switch" onClick={closeIfIdle}><section className="decision-modal session-modal mem-switch-modal" role="dialog" aria-modal="true" aria-label="Switch memory space" onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") closeIfIdle(); }}><div className="modal-titlebar"><div><span className="eyebrow">MEM SPACE</span><h2>Switch memory space</h2></div><button type="button" className="icon-button" title="Close mem switch" aria-label="Close mem switch" disabled={pending} onClick={closeIfIdle}><X size={16}/></button></div><p>Switching mem stops current workers, swaps out current sessions, then loads sessions from the selected mem space.</p><label>Mem space<input autoFocus value={space} disabled={pending} placeholder=".test_mem" onChange={(event) => setSpace(event.target.value)} onKeyDown={(event) => {
    if (event.key === "Enter" && !pending && !invalid) onSwitch(cleaned);
  }}/></label><p className="mem-hint">Use a space name, not a filesystem path. Examples: <code>.test_mem</code>, <code>project_a</code>.</p>{validationText && <p className="mem-validation" role="status" aria-live="polite">{validationText}</p>}<div className="decision-actions"><button type="button" className="secondary" disabled={pending} onClick={closeIfIdle}>Cancel</button><button type="button" className={`primary ${pending ? "sending" : ""}`} disabled={pending || invalid || cleaned === current} title={validationText || "Switch mem"} aria-label={validationText || "Switch mem"} onClick={() => onSwitch(cleaned)}>{pending && <LoaderCircle size={16}/>} {pending ? "Switching…" : "Switch mem"}</button></div></section></div>;
}

function decisionKey(decision: Decision) {
  return `${decision.event.session_id}:${decision.event.topic.name}:${String(decision.event.payload.request_id ?? "")}`;
}

function toolKey(sessionId: string, toolId: string) {
  return `${sessionId}:${toolId}`;
}

function pendingToolIdsForSession(pending: ReadonlySet<string>, sessionId: string) {
  const prefix = `${sessionId}:`;
  return new Set(Array.from(pending)
    .filter((key) => key.startsWith(prefix))
    .map((key) => key.slice(prefix.length)));
}

function removeToolKeysForSession(pending: ReadonlySet<string>, sessionId: string) {
  const prefix = `${sessionId}:`;
  return new Set(Array.from(pending).filter((key) => !key.startsWith(prefix)));
}

function toolgenRequestKey(sessionId: string, turnId: string) {
  return `${sessionId}:${turnId}`;
}

function hasPendingToolgenForSession(pending: ReadonlySet<string>, sessionId: string) {
  const prefix = `${sessionId}:`;
  return Array.from(pending).some((key) => key.startsWith(prefix));
}

function pendingToolgenTurnIds(pending: ReadonlySet<string>, sessionId: string) {
  const prefix = `${sessionId}:`;
  return new Set(Array.from(pending)
    .filter((key) => key.startsWith(prefix))
    .map((key) => key.slice(prefix.length)));
}

function removeToolgenRequestsForSession(pending: ReadonlySet<string>, sessionId: string) {
  const prefix = `${sessionId}:`;
  return new Set(Array.from(pending).filter((key) => !key.startsWith(prefix)));
}

function InlineDecision({ decision, pending, locked, position, total, onReply }: { decision: Decision; pending: boolean; locked: boolean; position: number; total: number; onReply: (decision: "accept" | "decline") => void }) {
  const disabled = pending || locked;
  const status = pending ? "Sending decision…" : locked ? "Session interaction is temporarily locked." : "";
  const declineLabel = pending ? "Decline is waiting for the current reply to finish" : locked ? "Decision is locked while the session changes" : "Decline this runtime request";
  const acceptLabel = pending ? "Sending decision" : locked ? "Decision is locked while the session changes" : "Accept this runtime request";
  return <section className="inline-decision" aria-label="Decision required" aria-busy={pending}>
    <div className="inline-decision-heading"><span className="eyebrow">RUNTIME REQUEST{total > 1 ? ` · ${position} OF ${total}` : ""}</span><h2>{decision.title}</h2></div>
    <pre>{decision.detail}</pre>
    {status && <span className="inline-decision-status" role="status" aria-live="polite">{status}</span>}
    <div className="decision-actions"><button type="button" className="secondary" title={declineLabel} aria-label={declineLabel} disabled={disabled} onClick={() => onReply("decline")}>Decline</button><button type="button" className={`primary ${pending ? "sending" : ""}`} title={acceptLabel} aria-label={acceptLabel} disabled={disabled} onClick={() => onReply("accept")}>{pending ? <LoaderCircle size={16}/> : <Check size={16}/>} {pending ? "Sending…" : "Continue"}</button></div>
  </section>;
}

export default function Root() { return <TimemApp/>; }

import { createRoot } from "react-dom/client";
createRoot(document.getElementById("root")!).render(<Root/>);
