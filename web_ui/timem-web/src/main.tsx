import { AssistantRuntimeProvider, ComposerPrimitive, ThreadMessageLike, ThreadPrimitive, useExternalStoreRuntime } from "@assistant-ui/react";
import { ArrowDown, Check, CheckCheck, ChevronRight, CircleStop, Copy, Cpu, FolderOpen, Gauge, LoaderCircle, Menu, Palette, Paperclip, PanelRight, Pencil, Plus, Send, Settings2, Sparkles, Terminal, Wrench, X } from "lucide-react";
import { Children, isValidElement, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { Appearance, applyAppearance, loadAppearance } from "./appearance";
import { Activity, ChatMessage, ClientCommand, Decision, Session, Snapshot, WebTurn, WebTurnEvent, WireEvent } from "./protocol";
import { isNearScrollBottom, preservePrependScrollTop, ScrollMetrics } from "./scroll";
import { activityFromTopic, appendTurnEvent, applyCoreTopicToSession, attachTurnCompletion, boundSessionHistory, clearDecisionsForWorker, coalesceActionLifecycle, enqueueDecision, finishTurn, removePendingAttachment, requestDecision, sessionContextUsage, tailPath, turnLiveUsage, updateSessionWorkerState, upsertSession, upsertTurn } from "./view_model";
import "./styles.css";
import "highlight.js/styles/github-dark.css";

const MAX_ACTIVITY_ITEMS = 300;
const TOKEN_STORAGE_KEY = "timem-web-access-token";

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
  const [showMobileSessions, setShowMobileSessions] = useState(false);
  const [showRuntime, setShowRuntime] = useState(false);
  const [showAppearance, setShowAppearance] = useState(false);
  const [showNewSession, setShowNewSession] = useState(false);
  const [renamingSessionId, setRenamingSessionId] = useState("");
  const [expandedSessionIds, setExpandedSessionIds] = useState<Set<string>>(() => new Set());
  const [renameDraft, setRenameDraft] = useState("");
  const [server, setServer] = useState<Snapshot["server"] | null>(null);
  const socket = useRef<WebSocket | null>(null);
  const fileInput = useRef<HTMLInputElement | null>(null);
  const activeSession = sessions.find((session) => session.session_id === activeSessionId) ?? sessions[0];
  const activeMessages = activeSession?.messages ?? [];

  useEffect(() => {
    applyAppearance(appearance);
  }, [appearance]);

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

  const beginRename = useCallback((session: Session) => {
    setRenamingSessionId(session.session_id);
    setRenameDraft(session.display_name);
  }, []);

  const finishRename = useCallback((sessionId: string) => {
    const displayName = renameDraft.trim();
    if (displayName && sendCommand({ type: "session_rename", session_id: sessionId, display_name: displayName })) {
      setSessions((current) => current.map((session) => session.session_id === sessionId ? { ...session, display_name: displayName } : session));
    }
    setRenamingSessionId("");
    setRenameDraft("");
  }, [renameDraft, sendCommand]);

  const applySnapshot = useCallback((snapshot: Snapshot) => {
    setServer(snapshot.server);
    setSessions(snapshot.sessions.map(boundSessionHistory));
    setActiveSessionId((current) => current || snapshot.sessions[0]?.session_id || "");
  }, []);

  const receive = useCallback((event: WireEvent) => {
    if (event.type === "hello") {
      applySnapshot(event.snapshot);
      return;
    }
    if (event.type === "session_created") {
      setSessions((current) => upsertSession(current, event.session));
      setActiveSessionId(event.session.session_id);
      return;
    }
    if (event.type === "session_renamed") {
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
      const activity: Activity = { id: crypto.randomUUID(), sessionId: "system", tone: "error", title: "Runtime error", detail: event.message, createdAt: Date.now() };
      setActivities((current) => [activity, ...current].slice(0, MAX_ACTIVITY_ITEMS));
      return;
    }
    if (event.type === "host_config_updated") {
      setServer((current) => current ? {
        ...current,
        runtime_options: current.runtime_options.map((option) => option.key === event.key ? { ...option, value: event.value } : option),
        session_env_defaults: event.session_env_defaults,
      } : current);
      const activity: Activity = { id: crypto.randomUUID(), sessionId: "system", tone: "notice", title: "Runtime setting updated", detail: `${event.key}: ${event.value}`, createdAt: Date.now() };
      setActivities((current) => [activity, ...current].slice(0, MAX_ACTIVITY_ITEMS));
      return;
    }
    if (event.type === "file_uploaded") {
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, attachments: [...session.attachments, event.file] }
        : session));
      const activity: Activity = { id: crypto.randomUUID(), sessionId: event.session_id, tone: "notice", title: "File attached", detail: `${event.file.name} · ${formatBytes(event.file.bytes)}`, createdAt: Date.now() };
      setActivities((current) => [activity, ...current].slice(0, MAX_ACTIVITY_ITEMS));
      return;
    }
    if (event.type === "attachment_removed") {
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? removePendingAttachment(session, event.attachment_id)
        : session));
      return;
    }
    if (event.type === "worker_activity") {
      const kind = String(event.event.kind ?? "worker_event");
      if (kind !== "model_request" && kind !== "model_response") {
        const detail = Object.entries(event.event).filter(([key]) => !["kind", "session_id", "context_id", "worker_id"].includes(key)).map(([key, value]) => `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`).join("\n");
        const activity: Activity = { id: crypto.randomUUID(), sessionId: event.session_id, tone: kind.includes("error") ? "error" : kind.includes("retry") ? "warning" : "notice", title: kind.replaceAll("_", " "), detail, createdAt: Date.now() };
        setActivities((current) => [activity, ...current].slice(0, MAX_ACTIVITY_ITEMS));
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
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? finishTurn(attachTurnCompletion(session, event.outcome.message_id, event.outcome.completion ?? {}), event.turn_id, event.outcome.completion ?? {})
        : session));
      return;
    }
    if (event.type !== "core_topic") return;
    const topic = event.event;
    const activity = activityFromTopic(topic);
    if (activity) setActivities((current) => [activity, ...current].slice(0, MAX_ACTIVITY_ITEMS));
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
          : [...current, { session_id: sessionId, display_name: displayName, ordinal, state: "ready", current_dir: "", max_llm_input_tokens: typeof topic.payload.max_llm_input_tokens === "number" ? topic.payload.max_llm_input_tokens : 0, contexts: [{ context_id: contextId, current_dir: "", worker_ids: [workerId] }], workers: [{ worker_id: workerId, context_id: contextId, display_name: displayName, ordinal, state: "ready", parent_worker_id: typeof item.parent_worker_id === "string" ? item.parent_worker_id : null }], active_context_id: contextId, primary_worker_id: workerId, attachments: [], messages: [], turns: [], active_turn_id: null }]);
        setActiveSessionId((current) => current || sessionId);
      }
    }
  }, [applySnapshot]);

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

  const sendText = useCallback(async (text: string) => {
    if (!activeSession || !text.trim()) return;
    const command: ClientCommand = activeSession.state === "working"
      ? { type: "turn_supplement", session_id: activeSession.session_id, text: text.trim() }
      : { type: "turn_submit", session_id: activeSession.session_id, text: text.trim() };
    if (!sendCommand(command)) {
      const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "error", title: "Not connected", detail: "Reconnect to Timem Web before sending another message.", createdAt: Date.now() };
      setActivities((current) => [activity, ...current]);
      return;
    }
  }, [activeSession, sendCommand]);

  const uploadFile = useCallback(async (file: File) => {
    if (!activeSession) return;
    const token = queryToken();
    if (!token) return;
    const form = new FormData();
    form.append("file", file);
    try {
      const response = await fetch(`/api/upload?token=${encodeURIComponent(token)}&session_id=${encodeURIComponent(activeSession.session_id)}`, { method: "POST", body: form });
      if (!response.ok) throw new Error((await response.json() as { error?: string }).error ?? "upload_failed");
    } catch (error) {
      const activity: Activity = { id: crypto.randomUUID(), sessionId: activeSession.session_id, tone: "error", title: "File upload failed", detail: error instanceof Error ? error.message : "upload_failed", createdAt: Date.now() };
      setActivities((current) => [activity, ...current].slice(0, MAX_ACTIVITY_ITEMS));
    }
  }, [activeSession]);

  const runtimeMessages = useMemo<readonly ThreadMessageLike[]>(() => activeMessages.map((message) => ({
    id: message.id,
    role: message.role,
    content: [{ type: "text" as const, text: message.text }],
  })), [activeMessages]);
  const [auiMessages, setAuiMessages] = useState<readonly ThreadMessageLike[]>(runtimeMessages);
  useEffect(() => setAuiMessages(runtimeMessages), [runtimeMessages]);
  const runtime = useExternalStoreRuntime<ThreadMessageLike>({
    messages: auiMessages,
    setMessages: setAuiMessages,
    convertMessage: (message) => message,
    onNew: async (message) => {
      const first = message.content[0];
      if (first?.type === "text") await sendText(first.text);
    },
    onCancel: async () => { if (activeSession) sendCommand({ type: "turn_cancel", session_id: activeSession.session_id }); },
  });

  const sessionActivities = activities.filter((activity) => activity.sessionId === activeSession?.session_id);
  const sessionDecisions = decisions.filter((decision) => decision.event.session_id === activeSession?.session_id);
  const visibleError = activities.find((activity) => activity.tone === "error" && (activity.sessionId === activeSession?.session_id || activity.sessionId === "system"));
  return <AssistantRuntimeProvider runtime={runtime}>
    <div className="app-shell">
      {showMobileSessions && <button className="mobile-sidebar-backdrop" aria-label="Close session navigation" onClick={() => setShowMobileSessions(false)}/>}
      <aside className={`sidebar ${showMobileSessions ? "mobile-open" : ""}`}>
        <div className="brand"><Sparkles size={18}/><span>Timem</span><small>local</small><button className="mobile-sidebar-close" title="Close sessions" aria-label="Close sessions" onClick={() => setShowMobileSessions(false)}><X size={17}/></button></div>
        <button className="new-session" onClick={() => { setShowNewSession(true); setShowMobileSessions(false); }}><Plus size={16}/> New session</button>
        <nav className="session-list" aria-label="Sessions">
          {sessions.map((session) => <div key={session.session_id} className="session-group"><div className={`session-row ${session.session_id === activeSession?.session_id ? "active" : ""} ${session.state === "working" ? "working" : ""}`}>
            <button className={`session-expand ${expandedSessionIds.has(session.session_id) ? "expanded" : ""}`} title={`${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers`} aria-label={`${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers for ${session.display_name}`} aria-expanded={expandedSessionIds.has(session.session_id)} onClick={() => setExpandedSessionIds((current) => {
              const next = new Set(current);
              if (next.has(session.session_id)) next.delete(session.session_id); else next.add(session.session_id);
              return next;
            })}><ChevronRight size={13}/></button>
            {renamingSessionId === session.session_id ? <input
              className="session-rename-input"
              autoFocus
              value={renameDraft}
              aria-label={`Rename ${session.display_name}`}
              onChange={(event) => setRenameDraft(event.target.value)}
              onBlur={() => finishRename(session.session_id)}
              onKeyDown={(event) => {
                if (event.key === "Enter") finishRename(session.session_id);
                if (event.key === "Escape") { setRenamingSessionId(""); setRenameDraft(""); }
              }}
            /> : <button className="session" title={session.current_dir} onClick={() => { setActiveSessionId(session.session_id); setShowMobileSessions(false); }}>
              {session.state === "working" ? <LoaderCircle className="session-working-icon" size={15} aria-label="Agent working"/> : <span className={`session-dot ${session.state}`}/>}<span className="session-identity"><span className="session-name">{session.display_name}</span><span className="session-cwd">{tailPath(session.current_dir)}</span>{session.runtime_profile && <span className="session-profile">{session.runtime_profile.provider}:{session.runtime_profile.model}</span>}</span><span className="session-state">{session.state === "working" ? "busy" : ""}</span>
            </button>}
            {renamingSessionId !== session.session_id && <button className="session-rename" title={`Rename ${session.display_name}`} aria-label={`Rename ${session.display_name}`} onClick={() => beginRename(session)}><Pencil size={13}/></button>}
          </div>{expandedSessionIds.has(session.session_id) && <div className="worker-list" aria-label={`Workers for ${session.display_name}`}>{[...session.workers].sort((left, right) => left.ordinal - right.ordinal).map((worker) => <div className="worker-row" key={worker.worker_id} title={`${worker.worker_id} · ${worker.context_id}`}><span className={`worker-state-dot ${worker.state}`}/><span className="worker-name">{worker.display_name || `ID${worker.ordinal}`}</span><span className="worker-state">{worker.state}</span></div>)}</div>}</div>)}
        </nav>
        <div className="sidebar-footer"><span className={`connection ${connected ? "online" : "offline"}`}/>{connected ? "Local runtime connected" : "Reconnecting…"}</div>
      </aside>
      <main className="chat-shell">
        <header className="chat-header">
          <span className="header-model">{activeSession?.runtime_profile ? `${activeSession.runtime_profile.provider}:${activeSession.runtime_profile.model}` : ""}</span>
          <div className="header-actions">
            <button title="Sessions" aria-label="Sessions" className="icon-button mobile-session-button" onClick={() => setShowMobileSessions(true)}><Menu size={18}/></button>
            <button title="Appearance" aria-label="Appearance" className={`icon-button ${showAppearance ? "selected" : ""}`} onClick={() => setShowAppearance((visible) => !visible)}><Palette size={17}/></button>
            <button title="Runtime information" className="icon-button" onClick={() => setShowRuntime((visible) => !visible)}><Settings2 size={17}/></button>
            <button title="Activity panel" className={`icon-button ${showActivity ? "selected" : ""}`} onClick={() => setShowActivity((visible) => !visible)}><PanelRight size={17}/></button>
          </div>
        </header>
        {showAppearance && <AppearancePanel appearance={appearance} onChange={setAppearance} onClose={() => setShowAppearance(false)}/>}
        {visibleError && <div className="host-error-banner" role="alert"><span><strong>{visibleError.title}</strong>{visibleError.detail && ` · ${visibleError.detail}`}</span><button className="icon-button" title="Dismiss error" onClick={() => setActivities((current) => current.filter((activity) => activity.id !== visibleError.id))}><X size={15}/></button></div>}
        {showRuntime && <RuntimePanel server={server} onUpdate={(key, value) => sendCommand({ type: "runtime_update", key, value })}/>}
        <ContextUsageBar session={activeSession}/>
        <TimemThread
          activeSession={activeSession}
          decisions={sessionDecisions}
          fileInput={fileInput}
          onUpload={uploadFile}
          onRemoveAttachment={(attachmentId) => {
            if (!activeSession) return;
            sendCommand({ type: "attachment_remove", session_id: activeSession.session_id, attachment_id: attachmentId });
          }}
          onDecisionReply={(decision, decisionValue) => {
            const event = decision.event;
            if (sendCommand({ type: "topic_reply", session_id: event.session_id, worker_id: event.worker_id ?? undefined, topic_name: event.topic.name, request_id: typeof event.payload.request_id === "string" ? event.payload.request_id : undefined, decision: decisionValue, payload: { summary: decision.detail } })) {
              setDecisions((current) => current.filter((candidate) => candidate !== decision));
            }
          }}
        />
      </main>
      {showActivity && <aside className="activity-panel">
        <header><button className="icon-button" title="Close activity panel" aria-label="Close activity panel" onClick={() => setShowActivity(false)}><X size={16}/></button></header>
        <div className="activity-list">{sessionActivities.map((activity) => <div className={`activity ${activity.tone}`} key={activity.id}><span className="activity-mark">{activity.tone === "thinking" ? "✦" : activity.tone === "action" ? "↳" : activity.tone === "warning" ? "!" : activity.tone === "error" ? "×" : "i"}</span><div>{activity.title && <strong>{activity.title}</strong>}{activity.detail && <div className="activity-detail"><MarkdownContent text={activity.detail}/></div>}{activity.code && <MarkdownContent text={fencedCode(activity.code_language ?? "text", activity.code)} />}</div></div>)}</div>
      </aside>}
      {showNewSession && <NewSessionDialog workspaces={server?.workspace_dirs ?? []} runtimeDefaults={server?.session_env_defaults ?? {}} onClose={() => setShowNewSession(false)} onCreate={(displayName, workspaceDir, env) => { if (sendCommand({ type: "session_create", display_name: displayName || undefined, workspace_dir: workspaceDir || undefined, env })) setShowNewSession(false); }} />}
    </div>
  </AssistantRuntimeProvider>;
}

const MAX_RENDERED_TURN_EVENTS = 200;
const INITIAL_RENDERED_TURNS = 24;
const TURN_HISTORY_PAGE_SIZE = 24;

function TimemThread({ activeSession, decisions, fileInput, onUpload, onRemoveAttachment, onDecisionReply }: {
  activeSession: Session | undefined;
  decisions: Decision[];
  fileInput: React.RefObject<HTMLInputElement | null>;
  onUpload: (file: File) => Promise<void>;
  onRemoveAttachment: (attachmentId: string) => void;
  onDecisionReply: (decision: Decision, reply: "accept" | "decline") => void;
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const previousScrollMetrics = useRef<ScrollMetrics | null>(null);
  const followThreadLatest = useRef(true);
  const [visibleTurnCount, setVisibleTurnCount] = useState(INITIAL_RENDERED_TURNS);
  const turns = activeSession?.turns ?? [];
  const hiddenTurnCount = Math.max(0, turns.length - visibleTurnCount);
  const visibleTurns = hiddenTurnCount > 0 ? turns.slice(-visibleTurnCount) : turns;
  const latestTurn = turns.at(-1);
  const latestTurnVersion = `${latestTurn?.turn_id ?? ""}:${latestTurn?.events.length ?? 0}:${latestTurn?.user_entries.length ?? 0}:${latestTurn?.final_answer?.length ?? 0}:${latestTurn?.completion ? 1 : 0}`;

  useEffect(() => setVisibleTurnCount(INITIAL_RENDERED_TURNS), [activeSession?.session_id]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const previous = previousScrollMetrics.current;
    if (!viewport || !previous) return;
    viewport.scrollTop = preservePrependScrollTop(previous, viewport.scrollHeight);
    previousScrollMetrics.current = null;
  }, [visibleTurnCount]);

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
    if (hiddenTurnCount === 0) return;
    if (viewportRef.current) {
      previousScrollMetrics.current = {
        scrollTop: viewportRef.current.scrollTop,
        scrollHeight: viewportRef.current.scrollHeight,
      };
    }
    setVisibleTurnCount((count) => Math.min(turns.length, count + TURN_HISTORY_PAGE_SIZE));
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
        if (event.currentTarget.scrollTop <= 48 && hiddenTurnCount > 0) loadEarlierTurns();
      }}
    >
      {(activeSession?.turns.length ?? 0) === 0 &&
        <div className="welcome"><Sparkles size={24}/><h2>Ready when you are.</h2><p>Ask Timem to investigate, write, or work with your local environment.</p></div>
      }
      {hiddenTurnCount > 0 && <button className="load-history" onClick={loadEarlierTurns}>Load {Math.min(TURN_HISTORY_PAGE_SIZE, hiddenTurnCount)} earlier tasks</button>}
      {visibleTurns.map((turn) => <TurnInteraction
        key={turn.turn_id}
        turn={turn}
        decisions={decisions.filter((decision) => decision.turnId === turn.turn_id)}
        onDecisionReply={onDecisionReply}
      />)}
      <ThreadPrimitive.ViewportFooter className="composer-wrap aui-thread-footer">
        <ThreadPrimitive.ScrollToBottom asChild><button className="scroll-to-bottom" title="Scroll to latest" aria-label="Scroll to latest"><ArrowDown size={16}/></button></ThreadPrimitive.ScrollToBottom>
        {!!activeSession?.attachments.length && <div className="attachment-strip" aria-label="Files attached to the next message">{activeSession.attachments.map((attachment) => <div className="pending-attachment" key={attachment.id} title={attachment.name}><Paperclip size={13}/><span className="pending-attachment-name">{attachment.name}</span><small>{formatBytes(attachment.bytes)}</small><button type="button" title={`Remove ${attachment.name}`} aria-label={`Remove ${attachment.name}`} onClick={() => onRemoveAttachment(attachment.id)}><X size={13}/></button></div>)}</div>}
        {activeSession && <div className="composer-cwd" title={activeSession.current_dir}><FolderOpen size={13}/><span>{activeSession.current_dir}</span></div>}
        <ComposerPrimitive.Root className="composer">
          <ComposerPrimitive.Input placeholder={activeSession?.state === "working" ? "继续输入…" : "Ask Timem anything about this workspace…"} aria-label="Message Timem" />
          <div className="composer-actions"><span>Enter to send · Shift+Enter for newline</span><div className="composer-buttons"><button className="attach-button" type="button" title="Attach a file" onClick={() => fileInput.current?.click()}><Paperclip size={17}/></button><input ref={fileInput} className="file-input" type="file" onChange={(event) => { const file = event.target.files?.[0]; event.currentTarget.value = ""; if (file) void onUpload(file); }}/><ComposerPrimitive.Send asChild><button className="send-button" title="Send message" aria-label="Send message"><Send size={17}/></button></ComposerPrimitive.Send>{activeSession?.state === "working" && <ComposerPrimitive.Cancel asChild><button className="stop-button" title="Cancel current turn"><CircleStop size={17}/> Stop</button></ComposerPrimitive.Cancel>}</div></div>
        </ComposerPrimitive.Root>
      </ThreadPrimitive.ViewportFooter>
    </ThreadPrimitive.Viewport>
  </ThreadPrimitive.Root>;
}

function TurnInteraction({ turn, decisions, onDecisionReply }: { turn: WebTurn; decisions: Decision[]; onDecisionReply: (decision: Decision, reply: "accept" | "decline") => void }) {
  const workScrollRef = useRef<HTMLDivElement | null>(null);
  const followLatest = useRef(true);
  const previousUpdateCount = useRef(turn.events.length + decisions.length);
  const [pendingUpdates, setPendingUpdates] = useState(0);
  const lifecycleEvents = coalesceActionLifecycle(turn.events);
  const visibleEvents = lifecycleEvents.slice(-MAX_RENDERED_TURN_EVENTS);
  const omitted = lifecycleEvents.length - visibleEvents.length;
  const hasVisibleProcess = visibleEvents.some((event) => activityFromTurnEvent(event, turn.turn_id) !== null) || decisions.length > 0 || turn.state === "working";

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
    <section className="turn-user-frame">
      <div className="turn-user-content">{turn.user_entries.map((entry, index) => <div className={`turn-user-entry ${entry.kind}`} key={`${entry.created_at_ms}-${index}`}>
        {entry.kind === "supplement" && <span>[补充]</span>}
        {entry.kind === "approval" && <span>[审批]</span>}
        <MarkdownContent text={entry.text}/>
        {!!entry.attachments?.length && <div className="turn-entry-attachments">{entry.attachments.map((attachment) => <span key={attachment.id} title={attachment.path}><Paperclip size={13}/><i aria-hidden="true">:</i><b>{attachment.name}</b><small>{formatBytes(attachment.bytes)}</small></span>)}</div>}
      </div>)}</div>
    </section>
    {hasVisibleProcess && <section className={`turn-assistant-frame ${turn.state}`}>
      {turn.state === "working" && <div className="turn-assistant-heading"><span className="working-chip"><span className="pulse"/> working</span></div>}
      <div className="turn-work-scroll" ref={workScrollRef} onScroll={(event) => {
        const remaining = event.currentTarget.scrollHeight - event.currentTarget.scrollTop - event.currentTarget.clientHeight;
        followLatest.current = remaining < 36;
        if (followLatest.current) setPendingUpdates(0);
      }}>
        {omitted > 0 && <div className="turn-events-omitted">{omitted} earlier work updates are retained by the host but not rendered.</div>}
        {visibleEvents.map((event) => <TurnEventView key={event.event_id} event={event} sessionId={turn.turn_id}/>)}
        {decisions.map((decision, index) => <InlineDecision key={decisionKey(decision)} decision={decision} position={index + 1} total={decisions.length} onReply={(reply) => onDecisionReply(decision, reply)} />)}
        {turn.state === "working" && <LiveTurnUsage turn={turn}/>}
        {visibleEvents.length === 0 && decisions.length === 0 && turn.state === "working" && <div className="working-indicator"><span className="pulse"/> Waiting for the first runtime update…</div>}
      </div>
      {pendingUpdates > 0 && <button className="turn-new-updates" onClick={scrollWorkToLatest}><ArrowDown size={13}/>{pendingUpdates} new update{pendingUpdates === 1 ? "" : "s"}</button>}
    </section>}
    {turn.final_answer && <section className="turn-final-delivery"><div className="message-content"><MarkdownContent text={turn.final_answer}/></div>{turn.completion && <CompletionCard completion={turn.completion}/>}</section>}
    {!turn.final_answer && turn.completion && <section className="turn-completion-only"><CompletionCard completion={turn.completion}/></section>}
  </article>;
}

function ContextUsageBar({ session }: { session: Session | undefined }) {
  const usage = session ? sessionContextUsage(session) : undefined;
  const limit = session?.max_llm_input_tokens || undefined;
  const ratio = usage && limit ? Math.min(100, Math.ceil((usage.prompt_tokens ?? 0) * 100 / limit)) : 0;
  return <section className="context-usage-bar" aria-label="Context usage">
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
  if (activity.tone === "action") return <ToolActivity activity={activity}/>;
  return <div className={`turn-work-item ${activity.tone}`}>
    <span className="activity-mark">{activity.tone === "thinking" ? "💡" : activity.tone === "warning" ? "!" : activity.tone === "error" ? "×" : "i"}</span>
    <div>{activity.title && <strong>{activity.title}</strong>}{activity.detail && <div className="turn-work-detail"><MarkdownContent text={activity.detail}/></div>}{activity.code && <MarkdownContent text={fencedCode(activity.code_language ?? "text", activity.code)}/>}</div>
  </div>;
}

function ToolActivity({ activity }: { activity: Activity }) {
  const status = activity.tool_status || "running";
  const running = status === "running" || status === "background_running";
  const commandPreview = activity.code?.split("\n", 1)[0]?.trim();
  return <details className={`tool-activity ${running ? "running" : "settled"}`}>
    <summary>
      <span className="tool-activity-icon">{activity.tool_name === "run_bash" ? <Terminal size={14}/> : <Wrench size={14}/>}</span>
      <b>{activity.tool_name || activity.title}</b>
      <span className="tool-activity-status">{humanizeToolStatus(status)}</span>
      {commandPreview && <code>{commandPreview}</code>}
      <ChevronRight className="tool-activity-chevron" size={14}/>
    </summary>
    <div className="tool-activity-body">
      {activity.detail && <div className="turn-work-detail"><MarkdownContent text={activity.detail}/></div>}
      {activity.code && <MarkdownContent text={fencedCode(activity.code_language ?? "text", activity.code)}/>}
    </div>
  </details>;
}

function humanizeToolStatus(status: string) {
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
      table: ({ node: _node, ...props }) => <div className="table-scroll"><table {...props}/></div>,
    }}
  >{text}</ReactMarkdown></div>;
}

function CodeBlock({ children }: React.ComponentPropsWithoutRef<"pre">) {
  const [copied, setCopied] = useState(false);
  const child = Children.count(children) === 1 ? Children.only(children) : null;
  const className = isValidElement<{ className?: string }>(child) ? child.props.className ?? "" : "";
  const language = className.match(/(?:^|\s)language-([^\s]+)/)?.[1] ?? "text";
  const code = textFromNode(children).replace(/\n$/, "");
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1400);
    } catch {
      setCopied(false);
    }
  };
  return <figure className="code-block">
    <figcaption><span>{language}</span><button type="button" onClick={() => void copy()} aria-label="Copy code">{copied ? <CheckCheck size={14}/> : <Copy size={14}/>}<span>{copied ? "Copied" : "Copy"}</span></button></figcaption>
    <pre>{children}</pre>
  </figure>;
}

function textFromNode(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textFromNode).join("");
  if (isValidElement<{ children?: React.ReactNode }>(node)) return textFromNode(node.props.children);
  return "";
}

function AppearancePanel({ appearance, onChange, onClose }: { appearance: Appearance; onChange: (appearance: Appearance) => void; onClose: () => void }) {
  const update = <K extends keyof Appearance>(key: K, value: Appearance[K]) => onChange({ ...appearance, [key]: value });
  return <>
    <div className="appearance-dismiss" aria-hidden="true" onClick={onClose}/>
    <section className="appearance-panel" role="dialog" aria-modal="false" aria-label="Appearance settings">
      <header><div><span className="eyebrow">APPEARANCE</span><h2>Reading preferences</h2></div><button className="icon-button" aria-label="Close appearance settings" onClick={onClose}><X size={16}/></button></header>
      <fieldset><legend>Theme</legend><div className="segmented-control">{(["dark", "light"] as const).map((theme) => <button className={appearance.theme === theme ? "active" : ""} key={theme} onClick={() => update("theme", theme)}>{theme === "dark" ? "Dark" : "Light"}</button>)}</div></fieldset>
      <fieldset><legend>Font</legend><div className="appearance-options">{(["system", "serif", "mono"] as const).map((font) => <button className={`${font}-sample ${appearance.font === font ? "active" : ""}`} key={font} onClick={() => update("font", font)}>{font === "system" ? "System" : font === "serif" ? "Serif" : "Mono"}<small>Aa</small></button>)}</div></fieldset>
      <fieldset><legend>Text size</legend><div className="segmented-control text-size-control">{(["small", "medium", "large"] as const).map((size) => <button className={appearance.textSize === size ? "active" : ""} key={size} onClick={() => update("textSize", size)}>{size === "small" ? "Small" : size === "medium" ? "Default" : "Large"}</button>)}</div></fieldset>
    </section>
  </>;
}

function fencedCode(language: string, code: string) {
  let fence = "```";
  while (code.includes(fence)) fence += "`";
  return `${fence}${language}\n${code}\n${fence}`;
}

function CompletionCard({ completion }: { completion: NonNullable<ChatMessage["completion"]> }) {
  const stats = completion.stats ?? {};
  const facts = [
    ["Completed", formatDuration(completion.elapsed_ms)],
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
    {facts.map(([label, value]) => <span key={label}><b>{label}</b> {value}</span>)}
    {isNotableStopReason(completion.stop_reason) && <span className="completion-status"><b>Status</b> {completion.stop_reason}</span>}
    {completion.repair_issue && <span className="completion-status warning"><b>Last repair</b> {completion.repair_issue}</span>}
  </div>;
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

function RuntimePanel({ server, onUpdate }: { server: Snapshot["server"] | null; onUpdate: (key: string, value: string) => void }) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  if (!server) return <section className="runtime-card"><Cpu size={16}/><span>Loading runtime settings…</span></section>;
  return <section className="runtime-card runtime-settings"><div className="runtime-summary"><Cpu size={16}/><span>Timem {server.version}</span><span>topic protocol v{server.protocol_version}</span><span><FolderOpen size={14}/> localhost:{server.port}</span></div><p>Changes apply to newly created sessions. Existing sessions retain their current runtime configuration.</p><div className="runtime-options">{server.runtime_options.map((option) => <label key={option.key}><span>{option.key}</span><div><input value={drafts[option.key] ?? option.value} onChange={(event) => setDrafts((current) => ({ ...current, [option.key]: event.target.value }))}/><button className="secondary compact" disabled={(drafts[option.key] ?? option.value) === option.value} onClick={() => onUpdate(option.key, drafts[option.key] ?? option.value)}>Apply</button></div></label>)}</div></section>;
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
] as const;

function NewSessionDialog({ workspaces, runtimeDefaults, onClose, onCreate }: {
  workspaces: string[];
  runtimeDefaults: Snapshot["server"]["session_env_defaults"];
  onClose: () => void;
  onCreate: (displayName: string, workspaceDir: string, env: Record<string, string>) => void;
}) {
  const [displayName, setDisplayName] = useState("");
  const [workspaceDir, setWorkspaceDir] = useState(workspaces[0] ?? "");
  const [env, setEnv] = useState<Record<string, string>>({});
  const updateEnv = (key: string, value: string) => setEnv((current) => ({ ...current, [key]: value }));
  const cleanedEnv = () => Object.fromEntries(Object.entries(env).map(([key, value]) => [key, value.trim()]).filter(([, value]) => value));
  return <div className="modal-backdrop" role="presentation"><section className="decision-modal session-modal" role="dialog" aria-modal="true" aria-label="Create session"><span className="eyebrow">NEW SESSION</span><h2>Start a session</h2><div className="session-modal-scroll"><label>Display name<input autoFocus value={displayName} placeholder="Optional name" onChange={(event) => setDisplayName(event.target.value)}/></label><label>Workspace<select value={workspaceDir} onChange={(event) => setWorkspaceDir(event.target.value)}>{workspaces.map((workspace) => <option value={workspace} key={workspace}>{workspace}</option>)}</select></label><details className="session-runtime-overrides"><summary>Runtime environment</summary><div className="session-runtime-grid">{SESSION_RUNTIME_FIELDS.map(([key, label, kind]) => <label key={key}><span>{label}<small>{key}</small></span>{kind === "api_protocol" ? <select value={env[key] ?? ""} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "default"}</option><option value="openai-compatible">openai-compatible</option><option value="openai-responses">openai-responses</option><option value="anthropic">anthropic</option></select> : kind === "response_protocol" ? <select value={env[key] ?? ""} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "xml"}</option><option value="xml">xml</option><option value="json">json</option><option value="markdown">markdown</option></select> : kind === "bash_approval" ? <select value={env[key] ?? ""} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "ask"}</option><option value="ask">ask</option><option value="approve">approve</option></select> : kind === "work_instructions" ? <select value={env[key] ?? ""} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "silent"}</option><option value="silent">silent</option><option value="ask">ask</option><option value="off">off</option></select> : <input type={kind} value={env[key] ?? ""} min={kind === "number" ? 1 : undefined} autoComplete={kind === "password" ? "new-password" : undefined} placeholder={kind === "password" ? "Optional session-only key" : `Inherit · ${runtimeDefaults[key] ?? "default"}`} onChange={(event) => updateEnv(key, event.target.value)}/>}</label>)}</div></details></div><div className="decision-actions"><button className="secondary" onClick={onClose}>Cancel</button><button className="primary" onClick={() => onCreate(displayName.trim(), workspaceDir, cleanedEnv())}><Plus size={16}/> Create session</button></div></section></div>;
}

function decisionKey(decision: Decision) {
  return `${decision.event.session_id}:${decision.event.topic.name}:${String(decision.event.payload.request_id ?? "")}`;
}

function InlineDecision({ decision, position, total, onReply }: { decision: Decision; position: number; total: number; onReply: (decision: "accept" | "decline") => void }) {
  return <section className="inline-decision" aria-label="Decision required">
    <div className="inline-decision-heading"><span className="eyebrow">RUNTIME REQUEST{total > 1 ? ` · ${position} OF ${total}` : ""}</span><h2>{decision.title}</h2></div>
    <pre>{decision.detail}</pre>
    <div className="decision-actions"><button className="secondary" onClick={() => onReply("decline")}>Decline</button><button className="primary" onClick={() => onReply("accept")}><Check size={16}/> Continue</button></div>
  </section>;
}

export default function Root() { return <TimemApp/>; }

import { createRoot } from "react-dom/client";
createRoot(document.getElementById("root")!).render(<Root/>);
