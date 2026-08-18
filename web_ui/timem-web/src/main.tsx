import { AssistantRuntimeProvider, ThreadMessageLike, ThreadPrimitive, useExternalStoreRuntime } from "@assistant-ui/react";
import { ArrowDown, BriefcaseBusiness, Check, CheckCheck, ChevronDown, ChevronRight, ChevronUp, CircleStop, Copy, Cpu, Database, Eye, EyeOff, FolderOpen, Gauge, GripVertical, KeyRound, LoaderCircle, Menu, Palette, Paperclip, Pencil, Plug, Plus, RefreshCw, Search, Send, Sparkles, Terminal, Trash2, Wrench, X } from "lucide-react";
import { Children, Dispatch, isValidElement, memo, MutableRefObject, SetStateAction, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import rehypeHighlight from "rehype-highlight";
import remarkGfm from "remark-gfm";
import { Appearance, applyAppearance, loadAppearance } from "./appearance";
import { Activity, ChatMessage, ClientCommand, clientId, CommandWithId, Decision, McpServerConfig, McpServerReport, McpTransport, Session, Snapshot, ToolDetail, ToolSummary, WebTurn, WebTurnEvent, WireEvent, WorkerRole } from "./protocol";
import { isNearScrollBottom, preservePrependScrollTop, restoreSessionScrollTop, ScrollMetrics, SessionScrollPosition } from "./scroll";
import { activityFromTopic, appendTurnEvent, applyChatMessageDeleted, applyCoreTopicToSession, attachTurnCompletion, boundSessionHistory, clearDecisionsForWorker, coalesceActionLifecycle, compareTurnTimelineItems, composerSendDecision, decisionKey, decisionsFromSessions, draftForSession, enqueueDecision, finishSessionDraftSubmission, finishTurn, groupDecisionsBySessionTurn, hasOnlyFreeTalkActivity, manualToolGenCommand, prependHistoryRecords, pruneSessionDrafts, pruneSessionSubmissionLocks, releaseSessionDraftSubmission, removePendingAttachment, requestDecision, reserveSessionDraftSubmission, resolveActiveSessionId, runtimeConnectionLabel, sessionContextUsage, sessionCreateDecision, sessionInteractionLockReason as sessionInteractionLockReasonForState, sessionRenameDecision, sessionTurnKey, setSessionDraft, tailPath, toolDisplayName, turnLiveUsage, turnTimelinePlacement, updateSessionWorkerState, visibleRuntimeRestartMarkers, upsertSession, upsertTurn, workspacePathLabel } from "./view_model";
import { safeMarkdownUrl } from "./markdown_security";
import { createMcpTransportDrafts, maskSensitiveMcpValues, mcpTransportLabel, mergeMcpSecrets } from "./mcp";
import { reconcileRuntimeDrafts, runtimeOptionLabel, sessionRuntimeOptions, shouldAutoRevealSessionApiKey, updateRevealedSessionApiKeys } from "./runtime_settings";
import { createFrameEventQueue } from "./frame_event_queue";
import { formatTokens } from "./token_format";
import { summarizeConsecutiveToolActivities, ToolActivitySummary } from "./activity_groups";
import { applyQueuedMessagesAck, claimQueuedMessage, clearQueuedMessagesPause, COLLAPSED_QUEUE_LIMIT, loadQueuedMessages, loadQueuedMessagesPause, QueuedMessage, queuedMessageKey, QueuedMessagesPauseState, queuedMessagesPauseStorageKey, queuedMessagesStorageKey, releaseQueuedMessageClaim, releaseSessionQueuedMessageClaims, removeQueuedMessage, reorderQueuedMessages, reservedQueuedAttachmentIds, saveQueuedMessages, saveQueuedMessagesPause, selectQueuedDispatches, shouldDirectManualMessage, shouldPauseQueuedMessages, unclaimedQueuedMessages } from "./queued_messages";
import { acceptOutboxCommand, addCommandToOutbox, commandMayPersist, commandNeedsReliableDelivery, CommandOutboxItem, commandOutboxStorageKey, finishOutboxCommand, loadCommandOutbox, reliableStorageScope, removeCommandOutboxItem, saveCommandOutboxItem } from "./command_outbox";
import { classifyEventSequence, loadEventCursor, resolveHelloEventCursor, saveEventCursor } from "./event_cursor";
import { enablesSemanticDelivery, shouldReduceTopLevelWireEvent } from "./wire_delivery";
import { clipboardImageFiles } from "./clipboard_images";
import "./styles.css";
import "highlight.js/styles/github-dark.css";

const MAX_ACTIVITY_ITEMS = 300;
const STORED_HISTORY_PAGE_SIZE = 200;
const TOKEN_STORAGE_KEY = "timem-web-access-token";
const EMPTY_CHAT_MESSAGES: ChatMessage[] = [];
type ChatMessageDeleteCandidate = {
  sessionId: string;
  turnId: string;
  role: "user" | "assistant";
  roleIndex: number;
  preview: string;
};

function chatMessageDeleteKey(candidate: Pick<ChatMessageDeleteCandidate, "sessionId" | "turnId" | "role" | "roleIndex">) {
  return `${candidate.sessionId}\u0000${candidate.turnId}\u0000${candidate.role}\u0000${candidate.roleIndex}`;
}

const FOCUSABLE_DIALOG_SELECTOR = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

function useDialogFocusTrap() {
  useEffect(() => {
    const containFocus = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const activeElement = document.activeElement;
      const dialog = activeElement instanceof HTMLElement
        ? activeElement.closest<HTMLElement>('[role="dialog"][aria-modal="true"]')
        : null;
      if (!dialog || !dialog.contains(document.activeElement)) return;
      const focusable = Array.from(dialog.querySelectorAll<HTMLElement>(FOCUSABLE_DIALOG_SELECTOR)).filter((element) => element.getClientRects().length > 0);
      if (focusable.length === 0) {
        event.preventDefault();
        dialog.focus({ preventScroll: true });
        return;
      }
      const currentIndex = focusable.indexOf(document.activeElement as HTMLElement);
      const nextIndex = event.shiftKey
        ? currentIndex <= 0 ? focusable.length - 1 : currentIndex - 1
        : currentIndex === focusable.length - 1 ? 0 : currentIndex + 1;
      event.preventDefault();
      focusable[nextIndex].focus({ preventScroll: true });
    };
    document.addEventListener("keydown", containFocus, true);
    return () => document.removeEventListener("keydown", containFocus, true);
  }, []);
}

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
  return { id: id ?? `${role}-${clientId()}`, role, text, created_at_ms: Date.now() };
}

function TimemApp() {
  useDialogFocusTrap();
  const [appearance, setAppearance] = useState<Appearance>(loadAppearance);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeSessionId, setActiveSessionId] = useState("");
  const [selectedRoleIds, setSelectedRoleIds] = useState<Record<string, string[]>>({});
  const [activities, setActivities] = useState<Activity[]>([]);
  const [decisions, setDecisions] = useState<Decision[]>([]);
  const [connected, setConnected] = useState(false);
  const [snapshotReady, setSnapshotReady] = useState(false);
  const [runtimeEverConnected, setRuntimeEverConnected] = useState(false);
  const [reconnectAttempt, setReconnectAttempt] = useState(0);
  const [showToolRepo, setShowToolRepo] = useState(false);
  const [showRoles, setShowRoles] = useState(false);
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
  const [showMcp, setShowMcp] = useState(false);
  const [showNewSession, setShowNewSession] = useState(false);
  const [showMemSwitch, setShowMemSwitch] = useState(false);
  const [deleteSessionCandidate, setDeleteSessionCandidate] = useState<Session | null>(null);
  const [deleteMessageCandidate, setDeleteMessageCandidate] = useState<ChatMessageDeleteCandidate | null>(null);
  const [renamingSessionId, setRenamingSessionId] = useState("");
  const [expandedSessionIds, setExpandedSessionIds] = useState<Set<string>>(() => new Set());
  const [renameDraft, setRenameDraft] = useState("");
  const [server, setServer] = useState<Snapshot["server"] | null>(null);
  const socket = useRef<WebSocket | null>(null);
  const sessionsRef = useRef<Session[]>([]);
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
  const [pendingDeleteSessionIds, setPendingDeleteSessionIds] = useState<Set<string>>(() => new Set());
  const [pendingDeleteMessageKeys, setPendingDeleteMessageKeys] = useState<Set<string>>(() => new Set());
  const [pendingRuntimeKeys, setPendingRuntimeKeys] = useState<Set<string>>(() => new Set());
  const [pendingSessionCredentialIds, setPendingSessionCredentialIds] = useState<Set<string>>(() => new Set());
  const [pendingMcpKeys, setPendingMcpKeys] = useState<Set<string>>(() => new Set());
  const [revealedSessionApiKeys, setRevealedSessionApiKeys] = useState<Record<string, string>>({});
  const [revealedMcpSecrets, setRevealedMcpSecrets] = useState<Record<string, Record<string, string>>>({});
  const [pendingHistorySessionIds, setPendingHistorySessionIds] = useState<Set<string>>(() => new Set());
  const [pendingUploadSessionIds, setPendingUploadSessionIds] = useState<Set<string>>(() => new Set());
  const [pendingUploadFiles, setPendingUploadFiles] = useState<Record<string, { name: string; bytes: number }>>({});
  const [pendingMemSwitch, setPendingMemSwitch] = useState(false);
  const [completedTurnKey, setCompletedTurnKey] = useState("");
  const [queuePauseRequest, setQueuePauseRequest] = useState<{ key: string; reason: string } | null>(null);
  const [commandAcks, setCommandAcks] = useState<Record<string, Extract<WireEvent, { type: "command_ack" }>>>({});
  const consumeCommandAcks = useCallback((commandIds: ReadonlySet<string>) => {
    setCommandAcks((current) => Object.fromEntries(Object.entries(current).filter(([commandId]) => !commandIds.has(commandId))));
  }, []);
  const commandOutboxRef = useRef<CommandOutboxItem[]>([]);
  const commandOutboxScopeRef = useRef("");
  const eventCursorRef = useRef(0);
  const eventCursorScopeRef = useRef("");
  const semanticDeliveryRef = useRef(false);
  const creatingSessionRef = useRef(false);
  const pendingAttachmentRemoveIdsRef = useRef<Set<string>>(new Set());
  const pendingDecisionKeysRef = useRef<Set<string>>(new Set());
  const pendingRenameSessionIdsRef = useRef<Set<string>>(new Set());
  const pendingDeleteSessionIdsRef = useRef<Set<string>>(new Set());
  const pendingDeleteMessageKeysRef = useRef<Set<string>>(new Set());
  const pendingRuntimeKeysRef = useRef<Set<string>>(new Set());
  const pendingSessionCredentialIdsRef = useRef<Set<string>>(new Set());
  const pendingSessionApiKeyValuesRef = useRef<Map<string, string>>(new Map());
  const pendingMcpKeysRef = useRef<Set<string>>(new Set());
  const pendingHistorySessionIdsRef = useRef<Set<string>>(new Set());
  const pendingUploadSessionIdsRef = useRef<Set<string>>(new Set());
  const pendingToolgenRequestsRef = useRef<Set<string>>(new Set());
  const fileInput = useRef<HTMLInputElement | null>(null);
  const newSessionButtonRef = useRef<HTMLButtonElement | null>(null);
  const appearanceButtonRef = useRef<HTMLButtonElement | null>(null);
  const appearancePanelRef = useRef<HTMLElement | null>(null);
  const mcpButtonRef = useRef<HTMLButtonElement | null>(null);
  const mcpPanelRef = useRef<HTMLElement | null>(null);
  const runtimeButtonRef = useRef<HTMLButtonElement | null>(null);
  const runtimePanelRef = useRef<HTMLElement | null>(null);
  const mobileSessionButtonRef = useRef<HTMLButtonElement | null>(null);
  const mobileSidebarRef = useRef<HTMLElement | null>(null);
  const toolRepoButtonRef = useRef<HTMLButtonElement | null>(null);
  const toolRepoPanelRef = useRef<HTMLElement | null>(null);
  const memSwitchButtonRef = useRef<HTMLButtonElement | null>(null);
  const activeSession = sessions.find((session) => session.session_id === activeSessionId) ?? sessions[0];
  sessionsRef.current = sessions;
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
    pushActivity({ id: clientId(), sessionId, tone: "error", title, detail, createdAt: Date.now() });
  }, [pushActivity]);
  const closeToolRepoPanel = useCallback(() => {
    setShowToolRepo(false);
    toolRepoButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeRuntimePanel = useCallback((restoreFocus = true) => {
    setShowRuntime(false);
    setRevealedSessionApiKeys({});
    if (restoreFocus) runtimeButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeAppearancePanel = useCallback((restoreFocus = true) => {
    setShowAppearance(false);
    if (restoreFocus) appearanceButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeMcpPanel = useCallback((restoreFocus = true) => {
    setShowMcp(false);
    setRevealedMcpSecrets({});
    if (restoreFocus) mcpButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeMobileSidebar = useCallback((restoreFocus = true) => {
    setShowMobileSessions(false);
    if (restoreFocus) mobileSessionButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeMemSwitchDialog = useCallback((restoreFocus = true) => {
    setShowMemSwitch(false);
    if (restoreFocus) memSwitchButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeNewSessionDialog = useCallback((restoreFocus = true) => {
    setShowNewSession(false);
    if (!restoreFocus) return;
    const newSessionButton = newSessionButtonRef.current;
    if (newSessionButton && window.getComputedStyle(newSessionButton).visibility !== "hidden") {
      newSessionButton.focus({ preventScroll: true });
    } else {
      mobileSessionButtonRef.current?.focus({ preventScroll: true });
    }
  }, []);

  useEffect(() => {
    applyAppearance(appearance);
  }, [appearance]);

  useEffect(() => {
    setRevealedSessionApiKeys({});
    setRevealedMcpSecrets({});
  }, [activeSessionId]);

  useEffect(() => {
    if (!showRuntime) return;
    runtimePanelRef.current?.focus({ preventScroll: true });
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (runtimeButtonRef.current?.contains(target) || runtimePanelRef.current?.contains(target)) return;
      closeRuntimePanel(false);
    };
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeRuntimePanel();
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer);
    document.addEventListener("keydown", dismissOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer);
      document.removeEventListener("keydown", dismissOnEscape);
    };
  }, [closeRuntimePanel, showRuntime]);

  useEffect(() => {
    if (!showToolRepo) return;
    toolRepoPanelRef.current?.focus({ preventScroll: true });
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeToolRepoPanel();
    };
    document.addEventListener("keydown", dismissOnEscape);
    return () => document.removeEventListener("keydown", dismissOnEscape);
  }, [closeToolRepoPanel, showToolRepo]);

  useEffect(() => {
    if (!showMobileSessions) return;
    mobileSidebarRef.current?.focus({ preventScroll: true });
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeMobileSidebar();
    };
    document.addEventListener("keydown", dismissOnEscape);
    return () => document.removeEventListener("keydown", dismissOnEscape);
  }, [closeMobileSidebar, showMobileSessions]);

  useEffect(() => {
    if (!showAppearance) return;
    appearancePanelRef.current?.focus({ preventScroll: true });
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (appearanceButtonRef.current?.contains(target) || appearancePanelRef.current?.contains(target)) return;
      closeAppearancePanel(false);
    };
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeAppearancePanel();
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer);
    document.addEventListener("keydown", dismissOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer);
      document.removeEventListener("keydown", dismissOnEscape);
    };
  }, [closeAppearancePanel, showAppearance]);

  useEffect(() => {
    if (!showMcp) return;
    mcpPanelRef.current?.focus({ preventScroll: true });
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (mcpButtonRef.current?.contains(target) || mcpPanelRef.current?.contains(target)) return;
      closeMcpPanel(false);
    };
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeMcpPanel();
    };
    document.addEventListener("pointerdown", dismissOnOutsidePointer);
    document.addEventListener("keydown", dismissOnEscape);
    return () => {
      document.removeEventListener("pointerdown", dismissOnOutsidePointer);
      document.removeEventListener("keydown", dismissOnEscape);
    };
  }, [closeMcpPanel, showMcp]);

  useEffect(() => {
    if (new URLSearchParams(window.location.search).has("token")) {
      window.history.replaceState(null, "", `${window.location.pathname}${window.location.hash}`);
    }
  }, []);

  const sendCommand = useCallback((command: ClientCommand, requestedCommandId?: string) => {
    const reliable = commandNeedsReliableDelivery(command);
    let wireCommand: ClientCommand | CommandWithId = command;
    if (reliable) {
      const commandId = requestedCommandId ?? clientId("command");
      const next = addCommandToOutbox(commandOutboxRef.current, command, commandId);
      const item = next.find((candidate) => candidate.commandId === commandId);
      if (!item || (commandMayPersist(command) && !saveCommandOutboxItem(window.localStorage, commandOutboxScopeRef.current, item))) return false;
      commandOutboxRef.current = next;
      wireCommand = { ...command, command_id: commandId };
    }
    if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return reliable;
    try {
      socket.current.send(JSON.stringify(wireCommand));
      return true;
    } catch {
      return reliable;
    }
  }, [snapshotReady]);

  useEffect(() => {
    if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return;
    for (const item of commandOutboxRef.current) {
      try { socket.current.send(JSON.stringify(item.command)); } catch { break; }
    }
  }, [snapshotReady]);

  useEffect(() => {
    const syncCrossTabOutbox = (event: StorageEvent) => {
      const scope = commandOutboxScopeRef.current;
      if (!scope || !event.key?.startsWith(`${commandOutboxStorageKey(scope)}:`)) return;
      const stored = loadCommandOutbox(window.localStorage, scope);
      const memoryOnly = commandOutboxRef.current.filter((item) => !commandMayPersist(item.command));
      commandOutboxRef.current = [...stored, ...memoryOnly.filter((item) => !stored.some((candidate) => candidate.commandId === item.commandId))];
      if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return;
      for (const item of stored) {
        try { socket.current.send(JSON.stringify(item.command)); } catch { break; }
      }
    };
    window.addEventListener("storage", syncCrossTabOutbox);
    return () => window.removeEventListener("storage", syncCrossTabOutbox);
  }, [snapshotReady]);

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
    pendingDeleteSessionIdsRef.current.clear();
    pendingDeleteMessageKeysRef.current.clear();
    pendingRuntimeKeysRef.current.clear();
    pendingSessionCredentialIdsRef.current.clear();
    pendingSessionApiKeyValuesRef.current.clear();
    pendingMcpKeysRef.current.clear();
    pendingHistorySessionIdsRef.current.clear();
    pendingUploadSessionIdsRef.current.clear();
    pendingToolgenRequestsRef.current.clear();
    setCreatingSession(false);
    setCancellingSessionIdSet(new Set());
    setPendingAttachmentRemoveIds(new Set());
    setPendingDecisionKeys(new Set());
    setPendingRenameSessionIds(new Set());
    setPendingDeleteSessionIds(new Set());
    setPendingDeleteMessageKeys(new Set());
    setPendingRuntimeKeys(new Set());
    setPendingSessionCredentialIds(new Set());
    setPendingMcpKeys(new Set());
    setRevealedSessionApiKeys({});
    setRevealedMcpSecrets({});
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

  const receive = useCallback(function receiveWireEvent(event: WireEvent, fromSemantic = false) {
    if (!fromSemantic) {
      if (event.type === "hello") {
        // A reconnect may intentionally target an older Host, so Hello resets
        // rather than only ever enabling this connection-level capability.
        semanticDeliveryRef.current = enablesSemanticDelivery(event);
      } else if (enablesSemanticDelivery(event)) {
        semanticDeliveryRef.current = true;
      }
      if (!shouldReduceTopLevelWireEvent(event, semanticDeliveryRef.current)) return;
    }
    if (event.type === "semantic_event") {
      if (event.event.type === "semantic_event" || event.event.type === "hello") {
        reportUiError("Invalid runtime event", `Event ${event.event_seq} contains an invalid nested ${event.event.type} envelope.`);
        socket.current?.close();
        return;
      }
      const sequenceState = classifyEventSequence(eventCursorRef.current, event.event_seq);
      if (sequenceState === "duplicate") return;
      if (sequenceState === "gap") {
        reportUiError("Runtime event gap", `Expected event ${eventCursorRef.current + 1}, received ${event.event_seq}. Reconnecting to replay missing events.`);
        socket.current?.close();
        return;
      }
      receiveWireEvent(event.event, true);
      eventCursorRef.current = event.event_seq;
      saveEventCursor(window.sessionStorage, eventCursorScopeRef.current, event.event_seq);
      return;
    }
    if (event.type === "command_ack") {
      if (
        event.command_id.startsWith("queued-")
        || (event.command_id.startsWith("submit-") && event.status === "rejected")
      ) {
        setCommandAcks((current) => ({ ...current, [event.command_id]: event }));
      }
      if (event.status === "accepted") {
        commandOutboxRef.current = acceptOutboxCommand(commandOutboxRef.current, event.command_id);
        const accepted = commandOutboxRef.current.find((item) => item.commandId === event.command_id);
        if (accepted && commandMayPersist(accepted.command)) saveCommandOutboxItem(window.localStorage, commandOutboxScopeRef.current, accepted);
      } else {
        commandOutboxRef.current = finishOutboxCommand(commandOutboxRef.current, event.command_id);
        removeCommandOutboxItem(window.localStorage, commandOutboxScopeRef.current, event.command_id);
        if (event.status === "rejected") {
          reportUiError("Command rejected", event.error || "The runtime rejected this command.");
        }
      }
      return;
    }
    if (event.type === "hello") {
      const scope = reliableStorageScope(window.location.origin, event.snapshot.server.mem.space_dir);
      let reconnectForReplay = false;
      if (eventCursorScopeRef.current !== scope) {
        const previousScope = eventCursorScopeRef.current;
        const restoredCursor = loadEventCursor(window.sessionStorage, scope);
        const resolved = resolveHelloEventCursor(previousScope, scope, restoredCursor, event.event_cursor, event.event_replay_floor);
        eventCursorScopeRef.current = scope;
        eventCursorRef.current = resolved.cursor;
        reconnectForReplay = resolved.reconnectForReplay;
        saveEventCursor(window.sessionStorage, scope, eventCursorRef.current);
      }
      if (commandOutboxScopeRef.current !== scope) {
        commandOutboxScopeRef.current = scope;
        commandOutboxRef.current = loadCommandOutbox(window.localStorage, scope);
        setCommandAcks({});
      }
      clearAllPendingCommands();
      setDecisions(decisionsFromSessions(event.snapshot.sessions));
      applySnapshot(event.snapshot);
      setSnapshotReady(true);
      if (reconnectForReplay) queueMicrotask(() => socket.current?.close());
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
    if (event.type === "session_deleted") {
      removePendingKey(pendingDeleteSessionIdsRef, setPendingDeleteSessionIds, event.session_id);
      setDeleteSessionCandidate((current) => current?.session_id === event.session_id ? null : current);
      toolCountBySessionRef.current.delete(event.session_id);
      setExpandedSessionIds((current) => {
        const next = new Set(current);
        next.delete(event.session_id);
        return next;
      });
      setDecisions((current) => current.filter((decision) => decision.event.session_id !== event.session_id));
      setActivities((current) => current.filter((activity) => activity.sessionId !== event.session_id));
      setSessions((current) => {
        const remaining = current.filter((session) => session.session_id !== event.session_id);
        setActiveSessionId((activeId) => resolveActiveSessionId(activeId, remaining));
        return remaining;
      });
      return;
    }
    if (event.type === "worker_roles_updated") {
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, roles: event.roles }
        : session));
      setSelectedRoleIds((current) => {
        const selected = current[event.session_id] ?? [];
        const retained = selected.filter((roleId) => event.roles.some((role) => role.id === roleId));
        if (retained.length === selected.length) return current;
        if (retained.length === 0) {
          const next = { ...current };
          delete next[event.session_id];
          return next;
        }
        return { ...current, [event.session_id]: retained };
      });
      return;
    }
    if (event.type === "chat_message_deleted") {
      const key = chatMessageDeleteKey({
        sessionId: event.session_id,
        turnId: event.turn_id,
        role: event.role,
        roleIndex: event.role_index,
      });
      removePendingKey(pendingDeleteMessageKeysRef, setPendingDeleteMessageKeys, key);
      setDeleteMessageCandidate((current) => current && chatMessageDeleteKey(current) === key ? null : current);
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? applyChatMessageDeleted(session, event.turn_id, event.role, event.role_index)
        : session));
      return;
    }
    if (event.type === "session_runtime_updated") {
      removePendingKey(pendingSessionCredentialIdsRef, setPendingSessionCredentialIds, event.session_id);
      const savedApiKey = pendingSessionApiKeyValuesRef.current.get(event.session_id);
      pendingSessionApiKeyValuesRef.current.delete(event.session_id);
      setRevealedSessionApiKeys((current) => updateRevealedSessionApiKeys(current, event.session_id, savedApiKey));
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, runtime_profile: event.runtime_profile }
        : session));
      return;
    }
    if (event.type === "session_runtime_config_updated") {
      removePendingKey(pendingRuntimeKeysRef, setPendingRuntimeKeys, `${event.session_id}:${event.key}`);
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, runtime_profile: event.runtime_profile, max_llm_input_tokens: event.runtime_profile.max_llm_input_tokens }
        : session));
      const activity: Activity = { id: clientId(), sessionId: event.session_id, tone: "notice", title: "Session setting updated", detail: `${runtimeOptionLabel(event.key)}: ${event.value}`, createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    if (event.type === "session_api_key_revealed") {
      removePendingKey(pendingSessionCredentialIdsRef, setPendingSessionCredentialIds, `reveal:${event.session_id}`);
      setRevealedSessionApiKeys((current) => ({ ...current, [event.session_id]: event.api_key }));
      return;
    }
    if (event.type === "turn_started") {
      setSessions((current) => current.map((session) => {
        if (session.session_id !== event.session_id) return session;
        return updateSessionWorkerState(upsertTurn(session, event.turn), event.worker_id, "working");
      }));
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
      const activity: Activity = { id: clientId(), sessionId: "system", tone: "error", title: "Runtime error", detail: event.message, createdAt: Date.now() };
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
      const activity: Activity = { id: clientId(), sessionId: "system", tone: "notice", title: "Runtime setting updated", detail: `${event.key}: ${event.value}`, createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    if (event.type === "mcp_updated") {
      pendingMcpKeysRef.current.clear();
      setPendingMcpKeys(new Set());
      setRevealedMcpSecrets({});
      setServer((current) => current ? { ...current, mcp_servers: event.servers } : current);
      if (event.session_id) {
        setSessions((current) => current.map((session) => session.session_id === event.session_id
          ? { ...session, mcp_server_ids: event.enabled_server_ids }
          : session));
      } else {
        const available = new Set(event.servers.map((server) => server.config.id));
        setSessions((current) => current.map((session) => ({ ...session, mcp_server_ids: session.mcp_server_ids.filter((id) => available.has(id)) })));
      }
      return;
    }
    if (event.type === "mcp_server_secrets_revealed") {
      removePendingKey(pendingMcpKeysRef, setPendingMcpKeys, `reveal:${event.server_id}`);
      setRevealedMcpSecrets((current) => ({ ...current, [event.server_id]: event.values }));
      return;
    }
    if (event.type === "file_uploaded") {
      setSessions((current) => current.map((session) => session.session_id === event.session_id
        ? { ...session, attachments: [...session.attachments, event.file] }
        : session));
      const activity: Activity = { id: clientId(), sessionId: event.session_id, tone: "notice", title: "File attached", detail: `${event.file.name} · ${formatBytes(event.file.bytes)}`, createdAt: Date.now() };
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
        const activity: Activity = { id: clientId(), sessionId: event.session_id, tone: kind.includes("error") ? "error" : kind.includes("retry") ? "warning" : "notice", title: kind.replaceAll("_", " "), detail, createdAt: Date.now() };
        pushActivity(activity);
      }
      const turnEvent: WebTurnEvent = { event_id: event.turn_event_id ?? clientId(), source: "worker_activity", payload: event.event, created_at_ms: Date.now() };
      const workerState = kind === "model_request"
        ? "working"
        : kind === "model_error"
          ? "error"
          : kind === "worker_stopped"
            ? "stopped"
            : kind === "subworker_turn_finished"
              ? "ready"
              : null;
      setSessions((current) => current.map((session) => {
        if (session.session_id !== event.session_id) return session;
        const withEvent = appendTurnEvent(session, event.turn_id, turnEvent);
        return workerState ? updateSessionWorkerState(withEvent, event.worker_id, workerState) : withEvent;
      }));
      if (kind === "model_request") {
        setDecisions((current) => clearDecisionsForWorker(current, event.session_id, event.worker_id));
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
      const completedKey = `${event.session_id}:${event.turn_id ?? ""}`;
      setCompletedTurnKey(completedKey);
      const stopReason = event.outcome.completion?.stop_reason;
      if (shouldPauseQueuedMessages(stopReason)) {
        setQueuePauseRequest({ key: completedKey, reason: stopReason });
      }
      return;
    }
    if (event.type !== "core_topic") return;
    const topic = event.event;
    const activity = activityFromTopic(topic);
    if (activity) setActivities((current) => [activity, ...current.filter((item) => !(activity.kind === "toolgen" && item.kind === "toolgen" && item.sessionId === activity.sessionId))].slice(0, MAX_ACTIVITY_ITEMS));
    setSessions((current) => current.map((session) => applyCoreTopicToSession(
      appendTurnEvent(session, event.turn_id, { event_id: event.turn_event_id ?? clientId(), source: "core_topic", payload: topic as unknown as Record<string, unknown>, created_at_ms: Date.now() }),
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
          : [...current, { session_id: sessionId, display_name: displayName, ordinal, state: "ready", current_dir: "", max_llm_input_tokens: typeof topic.payload.max_llm_input_tokens === "number" ? topic.payload.max_llm_input_tokens : 0, tools: [], mcp_server_ids: [], contexts: [{ context_id: contextId, current_dir: "", worker_ids: [workerId] }], workers: [{ worker_id: workerId, context_id: contextId, display_name: displayName, ordinal, state: "ready", parent_worker_id: typeof item.parent_worker_id === "string" ? item.parent_worker_id : null }], active_context_id: contextId, primary_worker_id: workerId, attachments: [], roles: [], messages: [], turns: [], history_before_cursor: null, history_has_more: false, active_turn_id: null }]);
        setActiveSessionId((current) => current || sessionId);
      }
    }
  }, [applySnapshot, clearAllPendingCommands, pushActivity, removePendingKey, reportUiError]);

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
    if (!showToolRepo || !activeSession) return;
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
  }, [activeSession?.session_id, showToolRepo, toolSearchQuery, sendCommand, reportUiError]);

  useEffect(() => {
    const token = queryToken();
    let stopped = false;
    let retryTimer: number | undefined;
    let retryAttempt = 0;
    let hasConnectedOnce = false;
    let disconnectNoticeShown = false;
    const inboundEvents = createFrameEventQueue<WireEvent>({
      consume: (events) => events.forEach((event) => receive(event)),
    });
    const connect = () => {
      if (stopped) return;
      const scheme = window.location.protocol === "https:" ? "wss" : "ws";
      const query = new URLSearchParams();
      if (token) query.set("token", token);
      if (eventCursorRef.current > 0) query.set("last_event_seq", String(eventCursorRef.current));
      const queryString = query.size > 0 ? `?${query.toString()}` : "";
      const ws = new WebSocket(`${scheme}://${window.location.host}/ws${queryString}`);
      socket.current = ws;
      ws.onopen = () => {
        hasConnectedOnce = true;
        disconnectNoticeShown = false;
        retryAttempt = 0;
        setConnected(true);
        setReconnectAttempt(0);
        setRuntimeEverConnected(true);
        setSnapshotReady(false);
      };
      ws.onclose = () => {
        if (socket.current === ws) socket.current = null;
        setConnected(false);
        setSnapshotReady(false);
        if (!stopped) {
          const nextAttempt = retryAttempt + 1;
          retryAttempt = nextAttempt;
          setReconnectAttempt(nextAttempt);
          if (hasConnectedOnce && !disconnectNoticeShown) {
            disconnectNoticeShown = true;
            pushActivity({
              id: clientId(),
              sessionId: activeSessionIdRef.current || "system",
              tone: "notice",
              title: "Runtime disconnected",
              detail: "Timem Web lost its runtime connection. If timem-web has exited, restart it and reopen the authenticated URL.",
              createdAt: Date.now(),
            });
          }
          const delay = Math.min(10_000, 500 * 2 ** Math.min(nextAttempt - 1, 5));
          retryTimer = window.setTimeout(connect, delay);
        }
      };
      ws.onerror = () => setConnected(false);
      ws.onmessage = (message) => {
        try { inboundEvents.enqueue(JSON.parse(String(message.data)) as WireEvent); } catch { /* Ignore malformed transport data. */ }
      };
    };
    connect();
    return () => {
      stopped = true;
      inboundEvents.dispose();
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      socket.current?.close();
      socket.current = null;
    };
  }, [pushActivity, receive]);

  const sendTextForSession = useCallback((sessionId: string, text: string, commandId?: string, attachmentIds?: readonly string[], forceSupplement = false, roleIds: readonly string[] = []): boolean => {
    const targetSession = sessionsRef.current.find((session) => session.session_id === sessionId);
    const decision = composerSendDecision(
      targetSession,
      text,
      targetSession ? cancellingSessionIds.current.has(targetSession.session_id) : false,
      pendingMemSwitch,
    attachmentIds,
 forceSupplement,
 );
    if (decision.kind === "skip") {
      if (decision.reason === "cancelling" && targetSession) {
        pushActivity({ id: clientId(), sessionId: targetSession.session_id, tone: "notice", title: "Cancellation in progress", detail: "Wait for the current turn to stop before sending another message.", createdAt: Date.now() });
      } else if (decision.reason === "mem_switching") {
        pushActivity({ id: clientId(), sessionId: targetSession?.session_id ?? "system", tone: "notice", title: "Switching mem", detail: "Wait for the new mem space to load before sending another message.", createdAt: Date.now() });
      }
      return false;
    }
    const command = roleIds.length > 0 ? { ...decision.command, role_ids: [...new Set(roleIds)] } : decision.command;
    if (!sendCommand(command, commandId)) {
      pushActivity({ id: clientId(), sessionId: decision.command.session_id, tone: "error", title: "Runtime unavailable", detail: "Timem Web runtime is not connected. Restart timem-web and reopen the authenticated URL before sending another message.", createdAt: Date.now() });
      return false;
    }
    return decision.clearDraftOnSuccess;
  }, [pendingMemSwitch, pushActivity, sendCommand]);
  const sendText = useCallback((text: string, commandId?: string) => activeSession
    ? sendTextForSession(activeSession.session_id, text, commandId)
    : false, [activeSession, sendTextForSession]);

  const uploadFile = useCallback(async (file: File) => {
    if (!activeSession || pendingMemSwitch) return;
    if (!addPendingKey(pendingUploadSessionIdsRef, setPendingUploadSessionIds, activeSession.session_id)) {
      const activity: Activity = { id: clientId(), sessionId: activeSession.session_id, tone: "notice", title: "Upload already in progress", detail: "Wait for the current file upload to finish before attaching another file.", createdAt: Date.now() };
      pushActivity(activity);
      return;
    }
    setPendingUploadFiles((current) => ({ ...current, [activeSession.session_id]: { name: file.name, bytes: file.size } }));
    const token = queryToken();
    const form = new FormData();
    form.append("file", file);
    try {
      const params = new URLSearchParams({ session_id: activeSession.session_id });
      if (token) params.set("token", token);
      const response = await fetch(`/api/upload?${params.toString()}`, { method: "POST", body: form });
      if (!response.ok) throw new Error((await response.json() as { error?: string }).error ?? "upload_failed");
    } catch (error) {
      const activity: Activity = { id: clientId(), sessionId: activeSession.session_id, tone: "error", title: "File upload failed", detail: error instanceof Error ? error.message : "upload_failed", createdAt: Date.now() };
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
    if (!sendCommand({ type: "history_page", session_id: session.session_id, before_cursor: session.history_before_cursor, limit: STORED_HISTORY_PAGE_SIZE })) {
      removePendingKey(pendingHistorySessionIdsRef, setPendingHistorySessionIds, session.session_id);
      const activity: Activity = { id: clientId(), sessionId: session.session_id, tone: "error", title: "Load history failed", detail: "Reconnect to Timem Web before loading earlier history.", createdAt: Date.now() };
      pushActivity(activity);
    }
  }, [addPendingKey, pendingMemSwitch, pushActivity, removePendingKey, sendCommand]);

  const runtimeMessages = useMemo<readonly ThreadMessageLike[]>(() => activeMessages
    .filter((message): message is ChatMessage & { role: "user" | "assistant" } => message.role !== "system")
    .map((message) => ({
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
      const activity: Activity = { id: clientId(), sessionId: activeSession.session_id, tone: "error", title: "Cancel failed", detail: "Reconnect to Timem Web before cancelling this turn.", createdAt: Date.now() };
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

  const sessionDecisions = decisions.filter((decision) => decision.event.session_id === activeSession?.session_id);
  const visibleErrors = activities.filter((activity) => activity.tone === "error" && (activity.sessionId === activeSession?.session_id || activity.sessionId === "system"));
  const visibleError = visibleErrors[0];
  const visibleErrorText = visibleError ? `${visibleError.title}${visibleError.detail ? ` · ${visibleError.detail}` : ""}` : "";
  const visibleErrorCount = visibleErrors.length;
  const hiddenErrorCount = Math.max(0, visibleErrorCount - 1);
  const dismissErrorLabel = visibleError ? `Dismiss ${visibleError.title}` : "Dismiss error";
  const runtimeDisconnected = runtimeEverConnected && !connected;
  const runtimeUnavailable = runtimeDisconnected && reconnectAttempt >= 3;
  const runtimeDisconnectedTitle = runtimeUnavailable ? "Runtime unavailable" : "Connection lost";
  const runtimeDisconnectedDetail = runtimeUnavailable
    ? "Restart timem-web and reopen the authenticated URL to continue."
    : "Reconnecting to Timem runtime… sending and session changes are paused until it reconnects.";
  const sessionInteractionLockReason = sessionInteractionLockReasonForState(pendingMemSwitch, connected, runtimeEverConnected, reconnectAttempt);
  const runtimeReady = connected && snapshotReady;
  const runtimeLocked = pendingMemSwitch || !runtimeReady;
  const connectionLabel = runtimeConnectionLabel(connected, snapshotReady, runtimeEverConnected, reconnectAttempt);
  const memSwitchTitle = !runtimeReady ? "Wait for the runtime snapshot before switching mem" : pendingMemSwitch ? "Mem switch is in progress" : "Switch mem directory";
  const newSessionLabel = runtimeLocked ? "Session controls are temporarily locked" : "New session";
  const headerModelLabel = activeSession?.runtime_profile?.model ?? "";
  const appearanceLabel = showAppearance ? "Close appearance settings" : "Open appearance settings";
  const runtimeLabel = showRuntime ? "Close runtime information" : "Open runtime information";
  const activeToolCount = activeSession?.tools.length ?? 0;
  const selectedRoleIdsForSession = activeSession ? selectedRoleIds[activeSession.session_id] ?? [] : [];
  const toolRepoLabel = showToolRepo ? "Close ToolRepo" : `Open ToolRepo · ${activeToolCount} reusable tools`;
  const mobileSessionsLabel = showMobileSessions ? "Close session navigation" : "Open session navigation";
  return <AssistantRuntimeProvider runtime={runtime}>
    <div className="app-shell">
      {showMobileSessions && <button type="button" className="mobile-sidebar-backdrop" aria-label="Close session navigation" onClick={() => closeMobileSidebar()}/>}
      <aside id="session-navigation" ref={mobileSidebarRef} className={`sidebar ${showMobileSessions ? "mobile-open" : ""}`} aria-label="Session navigation" tabIndex={-1}>
        <div className="brand"><img src="/timem_logo.png" alt="Timem logo" className="brand-logo"/><span>TIMEM</span><button type="button" className="mobile-sidebar-close" title="Close sessions" aria-label="Close sessions" onClick={() => closeMobileSidebar()}><X size={17}/></button></div>
        <button type="button" ref={newSessionButtonRef} className="new-session" title={newSessionLabel} aria-label={newSessionLabel} disabled={runtimeLocked} onClick={() => { setShowNewSession(true); closeMobileSidebar(false); }}><Plus size={16}/> New session</button>
        <nav className="session-list" aria-label="Sessions">
          {sessions.map((session) => {
            const renamingSession = pendingRenameSessionIds.has(session.session_id);
            const deletingSession = pendingDeleteSessionIds.has(session.session_id);
            return <div key={session.session_id} className="session-group"><div className={`session-row ${session.session_id === activeSession?.session_id ? "active" : ""} ${session.state === "working" ? "working" : ""} ${renamingSession ? "renaming-session" : ""}`} aria-busy={renamingSession || deletingSession || undefined}>
            <button type="button" className={`session-expand ${expandedSessionIds.has(session.session_id) ? "expanded" : ""}`} title={runtimeLocked ? "Session controls are temporarily locked" : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers`} aria-label={runtimeLocked ? `Workers locked while the runtime synchronizes for ${session.display_name}` : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers for ${session.display_name}`} aria-expanded={expandedSessionIds.has(session.session_id)} disabled={runtimeLocked} onClick={() => setExpandedSessionIds((current) => {
              const next = new Set(current);
              if (next.has(session.session_id)) next.delete(session.session_id); else next.add(session.session_id);
              return next;
            })}><ChevronRight size={13}/></button>
            {renamingSessionId === session.session_id ? <input
              className="session-rename-input"
              autoFocus
              value={renameDraft}
              aria-label={`Rename ${session.display_name}`}
              disabled={runtimeLocked}
              onChange={(event) => setRenameDraft(event.target.value)}
              onBlur={() => finishRename(session.session_id)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); finishRename(session.session_id); }
                if (event.key === "Escape") { event.preventDefault(); setRenamingSessionId(""); setRenameDraft(""); }
              }}
            />: <button type="button" className={`session ${session.session_id === activeSession?.session_id ? "active" : ""}`} title={runtimeLocked ? "Session controls are temporarily locked" : session.current_dir} aria-label={runtimeLocked ? `${session.display_name} locked while the runtime synchronizes` : renamingSession ? `${session.display_name} rename is being saved` : undefined} aria-current={session.session_id === activeSession?.session_id ? "page" : undefined} disabled={runtimeLocked} onClick={() => { setActiveSessionId(session.session_id); closeMobileSidebar(); }}>
              {session.state === "working" ? <LoaderCircle className="session-working-icon" size={15} aria-label="Session working"/> : <span className={`session-dot ${session.state}`} aria-hidden="true"/>}<span className="session-identity"><span className="session-name" title={session.display_name} onDoubleClick={() => { if (!runtimeLocked && renamingSessionId !== session.session_id) beginRename(session); }}>{session.display_name}</span><span className="session-sub"><span className="session-detail session-cwd" title={session.current_dir}><FolderOpen size={11} aria-hidden="true"/><span className="path-tail">{workspacePathLabel(session.current_dir)}</span></span>{renamingSession ? <span className="session-detail session-pending">Saving name...</span> : session.runtime_profile && <span className="session-detail session-profile" title={session.runtime_profile.model}><Sparkles size={9} className="session-model-icon" aria-hidden="true"/><span>{session.runtime_profile.model}</span></span>}</span></span><span className="sr-only">Session state: {session.state}</span>
            </button>}
            <button type="button" className={`session-delete ${deletingSession ? "deleting" : ""}`} title={`Delete ${session.display_name}`} aria-label={`Delete ${session.display_name}`} disabled={runtimeLocked || deletingSession} onClick={() => { setDeleteSessionCandidate(session); closeMobileSidebar(false); }}>{deletingSession ? <LoaderCircle size={14}/> : <Trash2 size={14}/>}</button>
          </div>{expandedSessionIds.has(session.session_id) && <div className="worker-list" aria-label={`Workers for ${session.display_name}: ${session.workers.length} worker${session.workers.length === 1 ? "" : "s"}`}>{[...session.workers].sort((left, right) => left.ordinal - right.ordinal).map((worker) => <div className="worker-row" key={worker.worker_id} title={`${worker.worker_id} · ${worker.context_id}`}><span className={`worker-state-dot ${worker.state}`} aria-hidden="true"/><span className="worker-name">{worker.display_name || `ID${worker.ordinal}`}</span><span className="worker-state">{worker.state}</span></div>)}</div>}</div>;
          })}
        </nav>
        <div className="sidebar-footer">
          <div className="connection-row" role="status" aria-live="polite" title={connectionLabel}><span className={`connection ${connected ? "online" : "offline"}`}/><span className="connection-label">{connectionLabel}</span></div>
          <button type="button" ref={memSwitchButtonRef} className="mem-card" title={server?.mem?.space_dir ?? memSwitchTitle} aria-label={memSwitchTitle} disabled={!runtimeReady || pendingMemSwitch} onClick={() => setShowMemSwitch(true)}><span className="mem-card-icon" aria-hidden="true"><Database size={15}/></span><span className="mem-card-copy"><strong>Memory</strong><small dir="rtl">{pendingMemSwitch ? "Switching…" : server?.mem?.space_dir ?? "…"}</small></span><ChevronRight size={14} aria-hidden="true"/></button>
        </div>
      </aside>
      <main className="chat-shell">
        <header className="chat-header">
          <div className="header-identity"><button type="button" ref={runtimeButtonRef} className={`header-model ${showRuntime ? "selected" : ""}`} title={runtimeLabel} aria-label={`${runtimeLabel}: ${headerModelLabel}`} aria-expanded={showRuntime} aria-controls="runtime-panel" onClick={() => { setShowAppearance(false); setShowMcp(false); setShowToolRepo(false); if (showRuntime) closeRuntimePanel(); else setShowRuntime(true); }}><span title={headerModelLabel}>{headerModelLabel}</span><ChevronDown size={12} aria-hidden="true"/></button><HeaderContextUsage session={activeSession}/></div>
          <div className="header-actions">
            <button type="button" ref={mobileSessionButtonRef} title={mobileSessionsLabel} aria-label={mobileSessionsLabel} className="icon-button mobile-session-button" aria-expanded={showMobileSessions} aria-controls="session-navigation" onClick={() => setShowMobileSessions(true)}><Menu size={18}/></button>
            <button type="button" title="Open worker roles" aria-label="Open worker roles" className="icon-button mobile-role-button" aria-expanded={showRoles} aria-controls="worker-role-panel" onClick={() => setShowRoles(true)}><BriefcaseBusiness size={17}/></button>
            <button type="button" ref={appearanceButtonRef} title={appearanceLabel} aria-label={appearanceLabel} className={`icon-button ${showAppearance ? "selected" : ""}`} aria-expanded={showAppearance} aria-controls="appearance-panel" onClick={() => { setShowRuntime(false); setShowMcp(false); setShowToolRepo(false); if (showAppearance) closeAppearancePanel(); else setShowAppearance(true); }}><Palette size={17}/></button>
            <button type="button" ref={mcpButtonRef} title="Manage MCP servers" aria-label="Manage MCP servers" className={`icon-button mcp-button ${showMcp ? "selected" : ""}`} aria-expanded={showMcp} aria-controls="mcp-panel" onClick={() => { setShowAppearance(false); setShowRuntime(false); setShowToolRepo(false); if (showMcp) closeMcpPanel(); else setShowMcp(true); }}><Plug size={17}/>{(activeSession?.mcp_server_ids.length ?? 0) > 0 && <span className="mcp-enabled-dot" aria-hidden="true"/>}</button>
            <button type="button" ref={toolRepoButtonRef} title={toolRepoLabel} aria-label={toolRepoLabel} className={`icon-button toolrepo-header-button ${showToolRepo ? "selected" : ""} ${toolCountPulseSessionId === activeSession?.session_id ? "count-pulse" : ""}`} aria-expanded={showToolRepo} aria-controls="toolrepo-panel" onClick={() => { setShowAppearance(false); setShowRuntime(false); setShowMcp(false); if (showToolRepo) closeToolRepoPanel(); else setShowToolRepo(true); }}><Wrench size={17}/><span className="toolrepo-header-count" aria-hidden="true">{activeToolCount}</span></button>
          </div>
        </header>
        {showAppearance && (
          <AppearancePanel
            panelRef={appearancePanelRef}
            appearance={appearance}
            onChange={setAppearance}
            onClose={closeAppearancePanel}
          />
        )}
        {showMcp && <McpPanel
          panelRef={mcpPanelRef}
          servers={server?.mcp_servers ?? []}
          session={activeSession}
          pendingKeys={pendingMcpKeys}
          revealedSecrets={revealedMcpSecrets}
          onClose={closeMcpPanel}
          onCommand={(key, command) => {
            if (!connected || !addPendingKey(pendingMcpKeysRef, setPendingMcpKeys, key)) return;
            if (!sendCommand(command)) removePendingKey(pendingMcpKeysRef, setPendingMcpKeys, key);
          }}
        />}
        {runtimeDisconnected && <div className="runtime-disconnect-banner" role="alert">
          <strong>{runtimeDisconnectedTitle}</strong>
          <span>{runtimeDisconnectedDetail}</span>
        </div>}
        {visibleError && <div className="host-error-banner" role="alert">
          <span className="host-error-text" title={visibleErrorText}><strong>{visibleError.title}</strong>{visibleError.detail && <span className="host-error-detail"> · {visibleError.detail}</span>}{hiddenErrorCount > 0 && <em>{hiddenErrorCount} more hidden error{hiddenErrorCount === 1 ? "" : "s"}</em>}</span>
          <div className="host-error-actions">
            {hiddenErrorCount > 0 && <button type="button" className="host-error-dismiss-all" title="Dismiss all visible errors" aria-label="Dismiss all visible errors" onClick={() => setActivities((current) => current.filter((activity) => activity.tone !== "error" || (activity.sessionId !== activeSession?.session_id && activity.sessionId !== "system")))}>Dismiss all</button>}
            <button type="button" className="icon-button" title={dismissErrorLabel} aria-label={dismissErrorLabel} onClick={() => setActivities((current) => current.filter((activity) => activity.id !== visibleError.id))}><X size={15}/></button>
          </div>
        </div>}
        {showRuntime && <RuntimePanel panelRef={runtimePanelRef} server={server} session={activeSession} pendingKeys={new Set(activeSession ? Array.from(pendingRuntimeKeys).filter((key) => key.startsWith(`${activeSession.session_id}:`)).map((key) => key.slice(activeSession.session_id.length + 1)) : [])} credentialPending={!!activeSession && (pendingSessionCredentialIds.has(activeSession.session_id) || pendingSessionCredentialIds.has(`reveal:${activeSession.session_id}`))} onApiKeyUpdate={(apiKey) => {
          if (!activeSession || !addPendingKey(pendingSessionCredentialIdsRef, setPendingSessionCredentialIds, activeSession.session_id)) return;
          pendingSessionApiKeyValuesRef.current.set(activeSession.session_id, apiKey);
          if (!sendCommand({ type: "session_api_key_update", session_id: activeSession.session_id, api_key: apiKey })) {
            pendingSessionApiKeyValuesRef.current.delete(activeSession.session_id);
            removePendingKey(pendingSessionCredentialIdsRef, setPendingSessionCredentialIds, activeSession.session_id);
            reportUiError("API key update failed", "Reconnect to Timem Web before saving this Session credential.", activeSession.session_id);
          }
        }} revealedApiKey={activeSession ? revealedSessionApiKeys[activeSession.session_id] : undefined} onApiKeyReveal={() => {
          if (!activeSession) return;
          const key = `reveal:${activeSession.session_id}`;
          if (!connected || !addPendingKey(pendingSessionCredentialIdsRef, setPendingSessionCredentialIds, key)) return;
          if (!sendCommand({ type: "session_api_key_reveal", session_id: activeSession.session_id })) {
            removePendingKey(pendingSessionCredentialIdsRef, setPendingSessionCredentialIds, key);
            reportUiError("API key reveal failed", "Reconnect to Timem Web before revealing this Session credential.", activeSession.session_id);
          }
        }} onUpdate={(key, value) => {
          if (!activeSession) return;
          const pendingKey = `${activeSession.session_id}:${key}`;
          if (!addPendingKey(pendingRuntimeKeysRef, setPendingRuntimeKeys, pendingKey)) return;
          if (!sendCommand({ type: "session_runtime_update", session_id: activeSession.session_id, key, value })) {
            removePendingKey(pendingRuntimeKeysRef, setPendingRuntimeKeys, pendingKey);
            reportUiError("Runtime update failed", "Reconnect to Timem Web before applying this Session configuration.", activeSession.session_id);
          }
        }}/>}
        <TimemThread
          activeSession={activeSession}
          sessions={sessions}
          completedTurnKey={completedTurnKey}
          queuePauseRequest={queuePauseRequest}
          commandAcks={commandAcks}
          onConsumeCommandAcks={consumeCommandAcks}
          reliableStorageScope={server ? reliableStorageScope(window.location.origin, server.mem.space_dir) : ""}
          sessionIds={sessions.map((session) => session.session_id)}
          sessionInteractionLocked={runtimeLocked}
          sessionInteractionLockReason={sessionInteractionLockReason}
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
          onSendForSession={sendTextForSession}
          selectedRoleIds={selectedRoleIdsForSession}
          onRolesConsumed={(sessionId, expectedRoleIds) => setSelectedRoleIds((current) => {
    if (
      expectedRoleIds
      && JSON.stringify(current[sessionId] ?? []) !== JSON.stringify(expectedRoleIds)
    ) return current;
    return Object.fromEntries(Object.entries(current).filter(([key]) => key !== sessionId));
  })}
          pendingToolGenTurnIds={activeSession ? pendingToolgenTurnIds(pendingToolgenRequests, activeSession.session_id) : new Set()}
          toolGenSessionBusy={!!activeSession && hasPendingToolgenForSession(pendingToolgenRequests, activeSession.session_id)}
          onRequestToolGen={(turnId) => {
            if (!activeSession || activeSession.state === "working" || runtimeLocked || hasPendingToolgenForSession(pendingToolgenRequests, activeSession.session_id)) return;
            setToolgenDialog({ sessionId: activeSession.session_id, turnId });
          }}
          onRequestMessageDelete={setDeleteMessageCandidate}
          onCancel={cancelActiveTurn}
          onUpload={uploadFile}
          onRemoveAttachment={(attachmentId) => {
            if (!activeSession || runtimeLocked) return;
            const key = `${activeSession.session_id}:${attachmentId}`;
            if (!addPendingKey(pendingAttachmentRemoveIdsRef, setPendingAttachmentRemoveIds, key)) return;
            if (!sendCommand({ type: "attachment_remove", session_id: activeSession.session_id, attachment_id: attachmentId })) {
              removePendingKey(pendingAttachmentRemoveIdsRef, setPendingAttachmentRemoveIds, key);
              const activity: Activity = { id: clientId(), sessionId: activeSession.session_id, tone: "error", title: "Remove attachment failed", detail: "Reconnect to Timem Web before removing this attachment.", createdAt: Date.now() };
              pushActivity(activity);
            }
          }}
          onDecisionReply={(decision, decisionValue) => {
            if (runtimeLocked) return;
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
      {!showToolRepo && showRoles && <button type="button" className="role-panel-backdrop" aria-label="Close worker roles" onClick={() => setShowRoles(false)}/>}
      {!showToolRepo && <WorkerRolePanel
        session={activeSession}
        mobileOpen={showRoles}
        onClose={() => setShowRoles(false)}
        selectedRoleIds={selectedRoleIdsForSession}
        disabled={runtimeLocked}
        onSelect={(roleId) => {
          if (!activeSession) return;
          setSelectedRoleIds((current) => {
            const selected = current[activeSession.session_id] ?? [];
            const nextSelected = selected.includes(roleId)
              ? selected.filter((selectedId) => selectedId !== roleId)
              : [...selected, roleId];
            if (nextSelected.length === 0) {
              const next = { ...current };
              delete next[activeSession.session_id];
              return next;
            }
            return { ...current, [activeSession.session_id]: nextSelected };
          });
        }}
        onCommand={(command) => sendCommand(command)}
      />}
      {showToolRepo && <button type="button" className="side-panel-backdrop" aria-label="Close ToolRepo" onClick={closeToolRepoPanel}/>}
      {showToolRepo && <ToolRepoPanel
        panelRef={toolRepoPanelRef}
        onClose={closeToolRepoPanel}
        session={activeSession}
        searchQuery={toolSearchQuery}
        searchPending={!!activeSession && pendingToolSearchKey === `${activeSession.session_id}:${toolSearchQuery}`}
        onSearchQueryChange={setToolSearchQuery}
        tools={activeSession ? (toolSearchResults[activeSession.session_id] ?? activeSession.tools) : []}
        selectedTool={selectedTool}
        pendingToolDetailId={activeSession && pendingToolDetailKey.startsWith(`${activeSession.session_id}:`) ? pendingToolDetailKey.slice(activeSession.session_id.length + 1) : ""}
        pendingToolRenameIds={activeSession ? pendingToolIdsForSession(pendingToolRenameKeys, activeSession.session_id) : new Set()}
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
          const activity: Activity = { id: clientId(), sessionId: activeSession?.session_id ?? "system", tone: "error", title: "Tool rename failed", detail: "Reconnect to Timem Web before renaming this tool.", createdAt: Date.now() };
          pushActivity(activity);
          return false;
        }}
        onOpenTerminal={(toolId) => {
          if (activeSession && sendCommand({ type: "tool_repo_open_terminal", session_id: activeSession.session_id, tool_id: toolId })) return true;
          const activity: Activity = { id: clientId(), sessionId: activeSession?.session_id ?? "system", tone: "error", title: "Open terminal failed", detail: "Reconnect to Timem Web before opening a tool directory.", createdAt: Date.now() };
          pushActivity(activity);
          return false;
        }}
      />}
      {showNewSession && <NewSessionDialog workspaces={server?.workspace_dirs ?? []} runtimeDefaults={server?.session_env_defaults ?? {}} creating={creatingSession} memSwitching={runtimeLocked} onClose={() => { if (!creatingSessionRef.current) closeNewSessionDialog(); }} onCreate={(command) => {
        if (runtimeLocked) return;
        if (creatingSessionRef.current) return;
        creatingSessionRef.current = true;
        setCreatingSession(true);
        if (sendCommand(command)) {
          closeNewSessionDialog();
        } else {
          creatingSessionRef.current = false;
          setCreatingSession(false);
          reportUiError("Create session failed", "Reconnect to Timem Web before creating a new session.", "system");
        }
      }} />}
      {deleteSessionCandidate && <SessionDeleteDialog session={deleteSessionCandidate} pending={pendingDeleteSessionIds.has(deleteSessionCandidate.session_id)} onClose={() => {
        if (!pendingDeleteSessionIdsRef.current.has(deleteSessionCandidate.session_id)) setDeleteSessionCandidate(null);
      }} onConfirm={() => {
        const sessionId = deleteSessionCandidate.session_id;
        if (!addPendingKey(pendingDeleteSessionIdsRef, setPendingDeleteSessionIds, sessionId)) return;
        if (!sendCommand({ type: "session_delete", session_id: sessionId })) {
          removePendingKey(pendingDeleteSessionIdsRef, setPendingDeleteSessionIds, sessionId);
          reportUiError("Delete session failed", "Reconnect to Timem Web before deleting this session.", sessionId);
        }
      }} />}
      {deleteMessageCandidate && <ChatMessageDeleteDialog
        candidate={deleteMessageCandidate}
        pending={pendingDeleteMessageKeys.has(chatMessageDeleteKey(deleteMessageCandidate))}
        onClose={() => {
          if (!pendingDeleteMessageKeysRef.current.has(chatMessageDeleteKey(deleteMessageCandidate))) setDeleteMessageCandidate(null);
        }}
        onConfirm={() => {
          const key = chatMessageDeleteKey(deleteMessageCandidate);
          if (!addPendingKey(pendingDeleteMessageKeysRef, setPendingDeleteMessageKeys, key)) return;
          if (!sendCommand({
            type: "chat_message_delete",
            session_id: deleteMessageCandidate.sessionId,
            turn_id: deleteMessageCandidate.turnId,
            role: deleteMessageCandidate.role,
            role_index: deleteMessageCandidate.roleIndex,
          })) {
            removePendingKey(pendingDeleteMessageKeysRef, setPendingDeleteMessageKeys, key);
            reportUiError("Delete message failed", "Reconnect to Timem Web before deleting this message.", deleteMessageCandidate.sessionId);
          }
        }}
      />}
      {showMemSwitch && <MemSwitchDialog current={server?.mem?.space_dir ?? ""} pending={pendingMemSwitch} onClose={() => { if (!pendingMemSwitch) closeMemSwitchDialog(); }} onSwitch={(path) => {
        setRenamingSessionId("");
        setRenameDraft("");
        setPendingMemSwitch(true);
        if (sendCommand({ type: "mem_switch", path })) {
          closeMemSwitchDialog();
        } else {
          setPendingMemSwitch(false);
          reportUiError("Mem switch failed", "Reconnect to Timem Web before switching the mem directory.", "system");
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

function WorkerRolePanel({ session, selectedRoleIds, disabled, mobileOpen, onClose, onSelect, onCommand }: {
  session?: Session;
  selectedRoleIds: readonly string[];
  disabled: boolean;
  mobileOpen: boolean;
  onClose: () => void;
  onSelect: (roleId: string) => void;
  onCommand: (command: ClientCommand) => boolean;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [deleteConfirmId, setDeleteConfirmId] = useState("");
  const resetEditor = () => { setEditingId(null); setName(""); setDescription(""); };
  useEffect(() => { resetEditor(); setDeleteConfirmId(""); }, [session?.session_id]);
  const submit = () => {
    if (!session || !name.trim() || !description.trim()) return;
    const command: ClientCommand = editingId
      ? { type: "worker_role_update", session_id: session.session_id, role_id: editingId, name, description }
      : { type: "worker_role_create", session_id: session.session_id, name, description };
    if (onCommand(command)) resetEditor();
  };
  return <aside id="worker-role-panel" className={`worker-role-panel ${mobileOpen ? "mobile-open" : ""}`} aria-label="Worker roles">
    <header><span><BriefcaseBusiness size={16}/> Roles</span><div><button type="button" className="worker-role-close" title="Close roles" aria-label="Close roles" onClick={onClose}><X size={15}/></button></div></header>
    <p className="worker-role-help">为工作添加工作指导角色</p>
    <div className="worker-role-list">
      {(session?.roles ?? []).map((role) => <article className={`worker-role-item ${selectedRoleIds.includes(role.id) ? "selected" : ""}`} key={role.id}>
        <label title={`Use ${role.name} for the next message`}><input type="checkbox" checked={selectedRoleIds.includes(role.id)} disabled={disabled} onChange={() => onSelect(role.id)}/><span><strong>{role.name}</strong><small>{role.description}</small></span></label>
        <div><button type="button" disabled={disabled} title={`Edit ${role.name}`} aria-label={`Edit ${role.name}`} onClick={() => { setEditingId(role.id); setName(role.name); setDescription(role.description); setDeleteConfirmId(""); }}><Pencil size={12}/></button><button type="button" className={deleteConfirmId === role.id ? "confirm-delete" : ""} disabled={disabled} title={deleteConfirmId === role.id ? `Confirm delete ${role.name}` : `Delete ${role.name}`} aria-label={deleteConfirmId === role.id ? `Confirm delete ${role.name}` : `Delete ${role.name}`} onClick={() => {
          if (deleteConfirmId !== role.id) { setDeleteConfirmId(role.id); return; }
          if (session && onCommand({ type: "worker_role_delete", session_id: session.session_id, role_id: role.id })) setDeleteConfirmId("");
        }}>{deleteConfirmId === role.id ? <Check size={12}/> : <Trash2 size={12}/>}</button></div>
      </article>)}
      {session && session.roles.length === 0 && <div className="worker-role-empty">还没有 Role。创建一个，安排重复的工作步骤、要求。</div>}
      {!session && <div className="worker-role-empty">Select a session to manage its roles.</div>}
    </div>
    {session && <form className={`worker-role-editor ${editingId ? "editing" : "creating"}`} onSubmit={(event) => { event.preventDefault(); submit(); }}>
      <strong>{editingId ? "编辑 Role" : "新建 Role"}</strong>
      <input value={name} maxLength={80} disabled={disabled} placeholder="称呼，例如：严谨审查员" aria-label="Role name" onChange={(event) => setName(event.target.value)}/>
      <textarea value={description} maxLength={16384} disabled={disabled} placeholder="描述工作要求、步骤和约束…" aria-label="Role description" onChange={(event) => setDescription(event.target.value)}/>
      <div><button type="submit" disabled={disabled || !name.trim() || !description.trim()}>{editingId ? "保存" : "创建"}</button>{editingId && <button type="button" onClick={resetEditor}>取消</button>}</div>
    </form>}
  </aside>;
}

function ToolRepoPanel({ panelRef, onClose, session, searchQuery, searchPending, onSearchQueryChange, tools, selectedTool, pendingToolDetailId, pendingToolRenameIds, onSelectTool, onCollapseTool, onRenameTool, onOpenTerminal }: {
  panelRef: MutableRefObject<HTMLElement | null>;
  onClose: () => void;
  session: Session | undefined;
  searchQuery: string;
  searchPending: boolean;
  onSearchQueryChange: (query: string) => void;
  tools: ToolSummary[];
  selectedTool: ToolDetail | null;
  pendingToolDetailId: string;
  pendingToolRenameIds: Set<string>;
  onSelectTool: (toolId: string) => boolean;
  onCollapseTool: () => void;
  onRenameTool: (toolId: string, newName: string) => boolean;
  onOpenTerminal: (toolId: string) => boolean;
}) {
  const [sort, setSort] = useState<"time" | "type" | "language">("time");
  const [renameToolId, setRenameToolId] = useState("");
  const [renameValue, setRenameValue] = useState("");
  const [contextMenu, setContextMenu] = useState<{ toolId: string; x: number; y: number } | null>(null);
  const contextMenuActionRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    setRenameToolId("");
    setRenameValue("");
    setContextMenu(null);
  }, [session?.session_id]);
  useEffect(() => {
    setContextMenu(null);
  }, [searchQuery, sort, selectedTool?.summary.tool_id, tools.length]);
  useEffect(() => {
    if (!contextMenu) return;
    contextMenuActionRef.current?.focus({ preventScroll: true });
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
  const pendingToolDetailLabel = pendingTool ? `Loading ${pendingTool.name} tool directory` : "";
  const sortLabel = sort === "time" ? "recent update" : sort;
  const sortControlLabel = `Sort ToolRepo by ${sortLabel}`;
  return <aside id="toolrepo-panel" ref={panelRef} className="toolrepo-side-panel session-side-panel" aria-label="ToolRepo" tabIndex={-1}>
    <header className="side-panel-header"><div className="side-panel-title"><Wrench size={15}/><strong>ToolRepo</strong></div><button type="button" className="icon-button" title="Close ToolRepo" aria-label="Close ToolRepo" onClick={onClose}><X size={16}/></button></header>
    <div className="toolrepo-panel">
      <div className="toolrepo-controls"><label className={searchPending ? "searching" : ""} aria-busy={searchPending}><Search size={14}/><input value={searchQuery} disabled={!session} onChange={(event) => onSearchQueryChange(event.target.value)} onKeyDown={(event) => { if (event.key === "Escape" && searchQuery) { event.preventDefault(); event.stopPropagation(); onSearchQueryChange(""); } }} placeholder={session ? "Search names and code" : "Select a session first"} aria-label="Search ToolRepo"/>{searchPending && <span className="toolrepo-search-pending" aria-hidden="true"/>}{hasToolSearch && <button type="button" title="Clear ToolRepo search" aria-label="Clear ToolRepo search" onClick={() => onSearchQueryChange("")}><X size={13}/></button>}</label><select value={sort} disabled={!session} onChange={(event) => setSort(event.target.value as typeof sort)} title={sortControlLabel} aria-label={sortControlLabel}><option value="time">Recent</option><option value="type">Type</option><option value="language">Language</option></select></div>
      {session && <div className="toolrepo-result-count" aria-live="polite">{toolRepoResultText}</div>}
      {!sortedTools.length ? <div className={`toolrepo-empty ${searchPending ? "searching" : ""}`} aria-label={`${toolRepoEmptyTitle}. ${toolRepoEmptyText}`} aria-busy={searchPending || undefined}><Wrench size={20}/><strong>{toolRepoEmptyTitle}</strong><span>{toolRepoEmptyText}</span></div> : <div className="toolrepo-browser"><div className="toolrepo-list" role="tree">{sortedTools.map((tool) => {
        const loadingDetail = pendingToolDetailId === tool.tool_id;
        const renamingTool = pendingToolRenameIds.has(tool.tool_id);
        const expanded = selectedTool?.summary.tool_id === tool.tool_id;
        const toolToggleLabel = expanded ? `收起 ${tool.name} 详情` : `展开 ${tool.name} 详情`;
        return <div className={`toolrepo-item ${selectedTool?.summary.tool_id === tool.tool_id ? "selected" : ""} ${loadingDetail ? "loading-detail" : ""} ${renamingTool ? "renaming-tool" : ""}`} role="treeitem" tabIndex={0} aria-selected={selectedTool?.summary.tool_id === tool.tool_id} aria-expanded={expanded} aria-busy={loadingDetail || renamingTool || undefined} key={tool.tool_id} onKeyDown={(event) => {
          if (event.target instanceof HTMLElement && (event.target.closest("button, input, select, textarea") || event.target !== event.currentTarget)) return;
          if (event.key === "Enter" || event.key === " ") {
            event.preventDefault();
            if (expanded) onCollapseTool(); else onSelectTool(tool.tool_id);
          } else if (event.key === "ArrowRight" && !expanded) {
            event.preventDefault();
            onSelectTool(tool.tool_id);
          } else if (event.key === "ArrowLeft" && expanded) {
            event.preventDefault();
            onCollapseTool();
          } else if (event.key === "Escape" && expanded) {
            event.preventDefault();
            onCollapseTool();
          }
        }} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ toolId: tool.tool_id, x: Math.max(8, Math.min(event.clientX, window.innerWidth - 220)), y: Math.max(8, Math.min(event.clientY, window.innerHeight - 76)) }); }}>
        <button type="button" className="toolrepo-item-main" title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`} aria-label={toolToggleLabel} aria-expanded={expanded} onClick={() => { if (expanded) onCollapseTool(); else onSelectTool(tool.tool_id); }}><ChevronRight size={13}/><span><strong>{tool.name}</strong><small>{renamingTool ? "Renaming..." : loadingDetail ? "Loading details..." : `${tool.language} · ${tool.tool_type}`}</small><em className="toolrepo-toggle-state">{expanded ? "收起" : "展开"}</em></span></button>
        <button type="button" className="toolrepo-open" title={`Open ${tool.name} directory in terminal`} aria-label={`Open ${tool.name} directory in terminal`} onClick={() => onOpenTerminal(tool.tool_id)}><Terminal size={12}/></button>
        {renameToolId === tool.tool_id ? <input className="toolrepo-rename" autoFocus value={renameValue} aria-label={`Rename ${tool.name}`} disabled={renamingTool} onChange={(event) => setRenameValue(event.target.value)} onBlur={() => finishToolRename(tool)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); finishToolRename(tool); } if (event.key === "Escape") { event.preventDefault(); setRenameToolId(""); setRenameValue(""); } }}/> : <button type="button" className="toolrepo-edit" title={renamingTool ? `Renaming ${tool.name}` : `Rename ${tool.name}`} aria-label={renamingTool ? `Renaming ${tool.name}` : `Rename ${tool.name}`} disabled={renamingTool} onClick={() => { setRenameToolId(tool.tool_id); setRenameValue(tool.name); }}><Pencil size={12}/></button>}
      </div>})}</div>
      {pendingTool ? <section className="toolrepo-detail loading" aria-busy="true" aria-label={pendingToolDetailLabel}><header><div><strong title={pendingTool.name}>{pendingTool.name}</strong><code>Reading tool directory…</code></div><div className="toolrepo-detail-actions"><button type="button" className="toolrepo-detail-collapse" title={`Stop viewing ${pendingTool.name} details`} aria-label={`Stop viewing ${pendingTool.name} details`} onClick={onCollapseTool}>收起详情</button></div></header><div className="toolrepo-detail-loading" role="status" aria-live="polite" aria-label={pendingToolDetailLabel}><span className="toolrepo-search-pending" aria-hidden="true"/>Reading directory tree...</div></section> : selectedTool && <section className="toolrepo-detail"><header><div><strong title={selectedTool.summary.name}>{selectedTool.summary.name}</strong><code title={selectedTool.summary.synopsis}>{selectedTool.summary.synopsis}</code></div><div className="toolrepo-detail-actions"><button type="button" title="Open directory in terminal" aria-label="Open directory in terminal" onClick={() => onOpenTerminal(selectedTool.summary.tool_id)}><Terminal size={14}/></button><button type="button" className="toolrepo-detail-collapse" title="Collapse tool detail" aria-label="Collapse tool detail" onClick={onCollapseTool}>收起详情</button></div></header><div className="toolrepo-files" aria-label="Tool directory tree">{selectedTool.files.map((file) => <div key={file.path} title={`${file.path} · ${formatBytes(file.bytes)}`} style={{ paddingLeft: `${8 + Math.max(0, file.path.split("/").length - 1) * 12}px` }}><span>{file.path}</span><small>{formatBytes(file.bytes)}</small></div>)}</div></section>}
      </div>}
    </div>
    {contextMenu && <div className="toolrepo-context-menu" role="menu" aria-label="Tool actions" style={{ left: contextMenu.x, top: contextMenu.y }} onPointerDown={(event) => event.stopPropagation()} onKeyDownCapture={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setContextMenu(null); } }}><button ref={contextMenuActionRef} type="button" role="menuitem" onClick={() => { onOpenTerminal(contextMenu.toolId); setContextMenu(null); }}><Terminal size={14}/>在命令行中打开目录</button></div>}
  </aside>;
}

const MAX_RENDERED_TURN_EVENTS = 200;
const EMPTY_DECISIONS: Decision[] = [];

const VisibleTurnList = memo(function VisibleTurnList({ sessionId, turns, restartMarkers, decisionsByTurn, sessionInteractionLocked, pendingDecisionKeys, pendingToolGenTurnIds, toolGenSessionBusy, onDecisionReply, onRequestToolGen, onRequestMessageDelete }: {
  sessionId: string;
  turns: WebTurn[];
  restartMarkers: ChatMessage[];
  decisionsByTurn: ReadonlyMap<string, Decision[]>;
  sessionInteractionLocked: boolean;
  pendingDecisionKeys: Set<string>;
  pendingToolGenTurnIds: Set<string>;
  toolGenSessionBusy: boolean;
  onDecisionReply: (decision: Decision, reply: "accept" | "decline" | "always_allow") => void;
  onRequestToolGen: (turnId: string) => void;
  onRequestMessageDelete: (candidate: ChatMessageDeleteCandidate) => void;
}) {
  const timeline = [
    ...turns.map((turn) => {
      const placement = turnTimelinePlacement(turn, restartMarkers);
      return {
        type: "turn" as const,
        createdAtMs: placement.createdAtMs,
        resumedAfterRestart: placement.resumedAfterRestart,
        id: turn.turn_id,
        turn,
      };
    }),
    ...restartMarkers.map((marker) => ({
      type: "restart" as const,
      createdAtMs: marker.created_at_ms,
      resumedAfterRestart: false,
      id: marker.id,
      marker,
    })),
  ].sort(compareTurnTimelineItems);

  return timeline.map((item) => {
    if (item.type === "restart") return <RuntimeRestartDivider key={item.id} marker={item.marker}/>;
    const turn = item.turn;
    return <TurnInteraction
      key={turn.turn_id}
      sessionId={sessionId}
      turn={turn}
      decisions={decisionsByTurn.get(sessionTurnKey(sessionId, turn.turn_id)) ?? EMPTY_DECISIONS}
      sessionInteractionLocked={sessionInteractionLocked}
      pendingDecisionKeys={pendingDecisionKeys}
      toolGenPending={pendingToolGenTurnIds.has(turn.turn_id)}
      toolGenBlocked={toolGenSessionBusy && !pendingToolGenTurnIds.has(turn.turn_id)}
      onDecisionReply={onDecisionReply}
      onRequestToolGen={onRequestToolGen}
      onRequestMessageDelete={onRequestMessageDelete}
    />;
  });
});

function RuntimeRestartDivider({ marker }: { marker: ChatMessage }) {
  const restartedAt = new Date(marker.created_at_ms);
  const timeLabel = Number.isNaN(restartedAt.getTime())
    ? ""
    : restartedAt.toLocaleString([], { dateStyle: "medium", timeStyle: "medium" });
  return <div className="runtime-restart-divider" role="separator" aria-label={`${marker.text}${timeLabel ? `，${timeLabel}` : ""}`}>
    <span aria-hidden="true"/>
    <div><strong>{marker.text}</strong>{timeLabel && <time dateTime={restartedAt.toISOString()}>{timeLabel}</time>}</div>
    <span aria-hidden="true"/>
  </div>;
}

function TimemThread({ activeSession, sessions, completedTurnKey, queuePauseRequest, commandAcks, onConsumeCommandAcks, reliableStorageScope, sessionIds, sessionInteractionLocked, sessionInteractionLockReason, decisions, fileInput, isCancelling, pendingAttachmentRemoveIds, pendingDecisionKeys, uploadingAttachment, uploadingAttachmentFile, loadingHistory, pendingToolGenTurnIds, toolGenSessionBusy, selectedRoleIds, onRolesConsumed, onLoadMoreHistory, onSend, onSendForSession, onCancel, onUpload, onRemoveAttachment, onDecisionReply, onRequestToolGen, onRequestMessageDelete }: {
  activeSession: Session | undefined;
  sessions: Session[];
  completedTurnKey: string;
  queuePauseRequest: { key: string; reason: string } | null;
  commandAcks: Record<string, Extract<WireEvent, { type: "command_ack" }>>;
  onConsumeCommandAcks: (commandIds: ReadonlySet<string>) => void;
  reliableStorageScope: string;
  sessionIds: string[];
  sessionInteractionLocked: boolean;
  sessionInteractionLockReason: string;
  decisions: Decision[];
  fileInput: React.RefObject<HTMLInputElement | null>;
  isCancelling: boolean;
  pendingAttachmentRemoveIds: Set<string>;
  pendingDecisionKeys: Set<string>;
  uploadingAttachment: boolean;
  uploadingAttachmentFile?: { name: string; bytes: number };
  loadingHistory: boolean;
  pendingToolGenTurnIds: Set<string>;
  toolGenSessionBusy: boolean;
  onLoadMoreHistory: (session: Session) => void;
  onSend: (text: string, commandId?: string) => boolean;
  onSendForSession: (sessionId: string, text: string, commandId?: string, attachmentIds?: readonly string[], forceSupplement?: boolean, roleIds?: readonly string[]) => boolean;
  selectedRoleIds: readonly string[];
  onRolesConsumed: (sessionId: string, expectedRoleIds?: readonly string[]) => void;
  onCancel: () => Promise<void>;
  onUpload: (file: File) => Promise<void>;
  onRemoveAttachment: (attachmentId: string) => void;
  onDecisionReply: (decision: Decision, reply: "accept" | "decline" | "always_allow") => void;
  onRequestToolGen: (turnId: string) => void;
  onRequestMessageDelete: (candidate: ChatMessageDeleteCandidate) => void;
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const previousScrollMetrics = useRef<ScrollMetrics | null>(null);
  const sessionScrollPositionsRef = useRef<Map<string, SessionScrollPosition>>(new Map());
  const renderedSessionIdRef = useRef<string | undefined>(undefined);
  const restoredSessionIdRef = useRef<string | undefined>(undefined);
  const followThreadLatest = useRef(true);
  const [draftsBySession, setDraftsBySession] = useState<Record<string, string>>({});
  const [queuedMessagesBySession, setQueuedMessagesBySession] = useState<Record<string, QueuedMessage[]>>({});
  const queuedMessagesBySessionRef = useRef<Record<string, QueuedMessage[]>>(queuedMessagesBySession);
  const [expandedQueueSessionIds, setExpandedQueueSessionIds] = useState<Set<string>>(() => new Set());
  const [collapsedQueuePanelSessionIds, setCollapsedQueuePanelSessionIds] = useState<Set<string>>(() => new Set());
  const [draggedQueueMessageId, setDraggedQueueMessageId] = useState<string>();
  const [editingQueuedMessage, setEditingQueuedMessage] = useState<{ sessionId: string; id: string; text: string }>();
  const queuedDispatchSessionIdsRef = useRef<Set<string>>(new Set());
  const queuedMessageClaimsRef = useRef<Set<string>>(new Set());
  const [queuedMessageClaims, setQueuedMessageClaims] = useState<Set<string>>(() => new Set());
 const [queuedMessagesPause, setQueuedMessagesPause] = useState<QueuedMessagesPauseState | null>(null);
 const processedQueuePauseRequestKeyRef = useRef("");
  const submittingDraftSessionIdsRef = useRef<Set<string>>(new Set());
  const submittingDraftStartedAtRef = useRef<Map<string, number>>(new Map());
  const directSubmissionsRef = useRef<Map<string, {
    commandId: string;
    text: string;
    roleIds: string[];
  }>>(new Map());
  const [submittingDraftSessionIds, setSubmittingDraftSessionIds] = useState<Set<string>>(() => new Set());
  const updateQueuedMessages = useCallback((update: (current: Record<string, QueuedMessage[]>) => Record<string, QueuedMessage[]>) => {
    const previous = queuedMessagesBySessionRef.current;
    const next = update(previous);
    if (!reliableStorageScope || !saveQueuedMessages(window.localStorage, reliableStorageScope, next, previous)) return;
    queuedMessagesBySessionRef.current = next;
    setQueuedMessagesBySession(next);
  }, [reliableStorageScope]);
 const releaseAllQueuedDispatches = useCallback(() => {
 queuedDispatchSessionIdsRef.current.clear();
 queuedMessageClaimsRef.current.clear();
 setQueuedMessageClaims(new Set());
 setDraggedQueueMessageId(undefined);
 }, []);
 const pauseQueuedMessages = useCallback((reason: string) => {
 const pause: QueuedMessagesPauseState = { paused: true, reason, stoppedAtMs: Date.now() };
 if (reliableStorageScope) saveQueuedMessagesPause(window.localStorage, reliableStorageScope, pause);
 releaseAllQueuedDispatches();
 setQueuedMessagesPause(pause);
 }, [releaseAllQueuedDispatches, reliableStorageScope]);
 const resumeQueuedMessages = useCallback(() => {
 if (reliableStorageScope && !clearQueuedMessagesPause(window.localStorage, reliableStorageScope)) return false;
 setQueuedMessagesPause(null);
 return true;
 }, [reliableStorageScope]);
  const turns = activeSession?.turns ?? [];
  const restartMarkers = useMemo(
    () => visibleRuntimeRestartMarkers(
      turns,
      (activeSession?.messages ?? []).filter((message) => message.role === "system" && message.kind === "runtime_restart"),
    ),
    [activeSession?.messages, turns],
  );
  const activeSessionId = activeSession?.session_id;
  const draft = draftForSession(draftsBySession, activeSessionId);
  const queuedMessages = activeSessionId ? queuedMessagesBySession[activeSessionId] ?? [] : [];
  const displayQueuedMessages = activeSessionId
    ? unclaimedQueuedMessages(queuedMessages, queuedMessageClaims, activeSessionId)
    : [];
  const queueExpanded = !!activeSessionId && expandedQueueSessionIds.has(activeSessionId);
  const queuePanelCollapsed = !!activeSessionId && collapsedQueuePanelSessionIds.has(activeSessionId);
  const visibleQueuedMessages = queueExpanded ? displayQueuedMessages : displayQueuedMessages.slice(0, COLLAPSED_QUEUE_LIMIT);
  const hiddenQueuedMessageCount = Math.max(0, displayQueuedMessages.length - COLLAPSED_QUEUE_LIMIT);
 const reservedAttachmentIds = useMemo(() => reservedQueuedAttachmentIds(queuedMessages), [queuedMessages]);
 const availableAttachments = useMemo(
   () => (activeSession?.attachments ?? []).filter((attachment) => !reservedAttachmentIds.has(attachment.id)),
   [activeSession?.attachments, reservedAttachmentIds],
 );
  const selectedRoles = activeSession?.roles.filter((role) => selectedRoleIds.includes(role.id)) ?? [];
  const submittingDraft = !!activeSessionId && submittingDraftSessionIds.has(activeSessionId);
  const sendLabel = isCancelling ? "Cancellation in progress" : activeSession?.state === "working" ? "Queue message" : "Send message";
  const lockedControlHint = sessionInteractionLocked ? sessionInteractionLockReason : "";
  const missingSessionHint = activeSession ? "" : "Create a session before using Timem";
  const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";
  const composerHint = missingSessionHint || lockedControlHint || (uploadingAttachment ? `${uploadingAttachmentText} · send is paused until it finishes` : activeSession?.state === "working" ? "Enter to queue · use 立即 to send during this turn" : "Enter to send · Shift+Enter for newline");
  const attachTitle = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");
  const attachLabel = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");
  const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);
  const attachedFileCount = activeSession?.attachments.length ?? 0;
  const attachmentSummary = attachedFileCount === 1 ? "1 file attached" : `${attachedFileCount} files attached`;
  const attachmentStripLabel = uploadingAttachment
    ? `${attachmentSummary}; ${uploadingAttachmentText}`
    : `Files attached to the next message; ${attachmentSummary}`;
  const composerHintId = `composer-hint-${activeSessionId || "empty"}`;
  const canLoadStoredHistory = !!activeSession?.history_has_more && !!activeSession.history_before_cursor;
  const decisionsByTurn = useMemo(() => groupDecisionsBySessionTurn(decisions), [decisions]);
  const historyButtonLabel = sessionInteractionLocked
    ? `${sessionInteractionLockReason} · earlier history is locked`
    : loadingHistory
      ? "Loading earlier history…"
      : `Load ${STORED_HISTORY_PAGE_SIZE} older stored tasks`;
  const latestTurn = turns.at(-1);
  const latestTurnVersion = `${latestTurn?.turn_id ?? ""}:${latestTurn?.events.length ?? 0}:${latestTurn?.user_entries.length ?? 0}:${latestTurn?.final_answer?.length ?? 0}:${latestTurn?.completion ? 1 : 0}`;
  const liveSessionKey = sessionIds.join("\u0000");
  const welcomeTitle = activeSession ? "Ready when you are." : "Create a session to start.";
  const welcomeText = activeSession ? "Ask Timem to investigate, write, or work with you." : "Use New session to choose a workspace and runtime profile.";

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    renderedSessionIdRef.current = activeSessionId;
    if (!viewport || !activeSessionId) return;
    const position = sessionScrollPositionsRef.current.get(activeSessionId);
    followThreadLatest.current = position?.followLatest ?? true;
    restoredSessionIdRef.current = activeSessionId;
    const previousBehavior = viewport.style.scrollBehavior;
    viewport.style.scrollBehavior = "auto";
    viewport.scrollTop = restoreSessionScrollTop(position, viewport.scrollHeight);
    viewport.style.scrollBehavior = previousBehavior;
  }, [activeSessionId]);

 useEffect(() => {
 if (!reliableStorageScope) return;
 const restored = loadQueuedMessages(window.localStorage, reliableStorageScope);
 const restoredPause = loadQueuedMessagesPause(window.localStorage, reliableStorageScope);
 queuedMessagesBySessionRef.current = restored;
 releaseAllQueuedDispatches();
 setQueuedMessagesBySession(restored);
 setQueuedMessagesPause(restoredPause);
 }, [releaseAllQueuedDispatches, reliableStorageScope]);

 useEffect(() => {
 const syncCrossTabQueues = (event: StorageEvent) => {
 if (!reliableStorageScope || !event.key) return;
 if (event.key === queuedMessagesPauseStorageKey(reliableStorageScope)) {
 const restoredPause = loadQueuedMessagesPause(window.localStorage, reliableStorageScope);
 if (restoredPause) releaseAllQueuedDispatches();
 setQueuedMessagesPause(restoredPause);
 return;
 }
 if (!event.key.startsWith(`${queuedMessagesStorageKey(reliableStorageScope)}:`)) return;
 const restored = loadQueuedMessages(window.localStorage, reliableStorageScope);
 queuedMessagesBySessionRef.current = restored;
 for (const [sessionId, messages] of Object.entries(restored)) {
 if (messages.some((message) => message.deliveryError)) queuedDispatchSessionIdsRef.current.delete(sessionId);
 }
 for (const key of Array.from(queuedMessageClaimsRef.current)) {
 if (!Object.entries(restored).some(([sessionId, messages]) => messages.some((message) => queuedMessageKey(sessionId, message.id) === key))) {
 queuedMessageClaimsRef.current.delete(key);
 }
 }
 setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
 setQueuedMessagesBySession(restored);
 };
 window.addEventListener("storage", syncCrossTabQueues);
 return () => window.removeEventListener("storage", syncCrossTabQueues);
 }, [releaseAllQueuedDispatches, reliableStorageScope]);

  useEffect(() => {
    if (sessionIds.length === 0) return;
    setDraftsBySession((current) => pruneSessionDrafts(current, sessionIds));
    updateQueuedMessages((current) => Object.fromEntries(Object.entries(current).filter(([sessionId]) => sessionIds.includes(sessionId))));
    setExpandedQueueSessionIds((current) => new Set(Array.from(current).filter((sessionId) => sessionIds.includes(sessionId))));
    setCollapsedQueuePanelSessionIds((current) => new Set(Array.from(current).filter((sessionId) => sessionIds.includes(sessionId))));
    setEditingQueuedMessage((current) => current && sessionIds.includes(current.sessionId) ? current : undefined);
    for (const sessionId of Array.from(queuedDispatchSessionIdsRef.current)) {
      if (!sessionIds.includes(sessionId)) queuedDispatchSessionIdsRef.current.delete(sessionId);
    }
    for (const key of Array.from(queuedMessageClaimsRef.current)) {
      if (!sessionIds.some((sessionId) => key.startsWith(`${sessionId}\u0000`))) queuedMessageClaimsRef.current.delete(key);
    }
    setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
    for (const sessionId of Array.from(directSubmissionsRef.current.keys())) {
      if (!sessionIds.includes(sessionId)) directSubmissionsRef.current.delete(sessionId);
    }
    if (pruneSessionSubmissionLocks(submittingDraftSessionIdsRef, sessionIds)) {
      setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    }
  }, [liveSessionKey, updateQueuedMessages]);

  useEffect(() => {
    let nextQueues = queuedMessagesBySessionRef.current;
    const appliedCommandIds = new Set<string>();
    const matchedSessionByCommand = new Map<string, string>();
    const rejectedSessionIds = new Set<string>();
    let directSubmissionReleased = false;
    const rejectedDirectDrafts = new Map<string, string>();
    for (const ack of Object.values(commandAcks)) {
      if (ack.command_id.startsWith("submit-")) {
        appliedCommandIds.add(ack.command_id);
        for (const [sessionId, submission] of directSubmissionsRef.current) {
          if (submission.commandId !== ack.command_id) continue;
          directSubmissionsRef.current.delete(sessionId);
          submittingDraftStartedAtRef.current.delete(sessionId);
          rejectedDirectDrafts.set(sessionId, submission.text);
          directSubmissionReleased = releaseSessionDraftSubmission(
            submittingDraftSessionIdsRef,
            sessionId,
          ) || directSubmissionReleased;
          break;
        }
        continue;
      }
      if (ack.status === "accepted") continue;
      const result = applyQueuedMessagesAck(nextQueues, ack.command_id, ack.status, ack.error, clientId("queued"));
      if (!result.matchedSessionId) continue;
      appliedCommandIds.add(ack.command_id);
      matchedSessionByCommand.set(ack.command_id, result.matchedSessionId);
      if (ack.status === "rejected") rejectedSessionIds.add(result.matchedSessionId);
      nextQueues = result.queues;
    }
    if (appliedCommandIds.size === 0) return;
    const queuesChanged = matchedSessionByCommand.size > 0;
    if (
      queuesChanged
      && (!reliableStorageScope || !saveQueuedMessages(window.localStorage, reliableStorageScope, nextQueues, queuedMessagesBySessionRef.current))
    ) return;
    if (queuesChanged) {
      queuedMessagesBySessionRef.current = nextQueues;
      setQueuedMessagesBySession(nextQueues);
    }
    if (rejectedDirectDrafts.size > 0) {
      setDraftsBySession((current) => {
        let next = current;
        for (const [sessionId, rejectedText] of rejectedDirectDrafts) {
          const newerDraft = draftForSession(next, sessionId);
          const restored = newerDraft.trim()
            ? `${rejectedText}\n\n${newerDraft}`
            : rejectedText;
          next = setSessionDraft(next, sessionId, restored);
        }
        return next;
      });
    }
    if (directSubmissionReleased) {
      setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    }
    for (const [commandId, sessionId] of matchedSessionByCommand) {
      releaseQueuedMessageClaim(queuedMessageClaimsRef.current, sessionId, commandId);
    }
    setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
    for (const sessionId of rejectedSessionIds) queuedDispatchSessionIdsRef.current.delete(sessionId);
    onConsumeCommandAcks(appliedCommandIds);
  }, [commandAcks, onConsumeCommandAcks, reliableStorageScope]);

  useEffect(() => {
 const pendingPauseRequest = queuePauseRequest
 && processedQueuePauseRequestKeyRef.current !== queuePauseRequest.key;
 if (sessionInteractionLocked || queuedMessagesPause || pendingPauseRequest) return;
    for (const session of sessions) {
      if (session.state === "working") {
        queuedDispatchSessionIdsRef.current.delete(session.session_id);
      }
    }
    const dispatches = selectQueuedDispatches(sessions, queuedMessagesBySessionRef.current, queuedDispatchSessionIdsRef.current, editingQueuedMessage?.sessionId);
    for (const { sessionId, message: next } of dispatches) {
      if (!claimQueuedMessage(queuedMessageClaimsRef.current, sessionId, queuedMessagesBySessionRef.current[sessionId] ?? [], next.id)) continue;
      queuedDispatchSessionIdsRef.current.add(sessionId);
      if (!onSendForSession(sessionId, next.text, next.id, next.attachmentIds, false, next.roleIds ?? (next.roleId ? [next.roleId] : []))) {
        queuedDispatchSessionIdsRef.current.delete(sessionId);
        releaseQueuedMessageClaim(queuedMessageClaimsRef.current, sessionId, next.id);
        updateQueuedMessages((current) => ({
          ...current,
          [sessionId]: (current[sessionId] ?? []).map((message) => message.id === next.id
            ? { ...message, deliveryError: "消息尚未安全保存，请检查浏览器存储后重试" }
            : message),
        }));
      }
    }
    setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
 }, [editingQueuedMessage?.sessionId, onSendForSession, queuePauseRequest, queuedMessagesBySession, queuedMessagesPause, sessionInteractionLocked, sessions, updateQueuedMessages]);

 useEffect(() => {
 if (!queuePauseRequest || processedQueuePauseRequestKeyRef.current === queuePauseRequest.key) return;
 processedQueuePauseRequestKeyRef.current = queuePauseRequest.key;
 pauseQueuedMessages(queuePauseRequest.reason);
 }, [pauseQueuedMessages, queuePauseRequest]);

  useEffect(() => {
    if (!completedTurnKey) return;
    const sessionId = completedTurnKey.slice(0, completedTurnKey.indexOf(":"));
    if (queuedDispatchSessionIdsRef.current.delete(sessionId)) {
      setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
    }
  }, [completedTurnKey]);

  useEffect(() => {
    let changed = false;
    for (const session of sessions) {
      if (session.state !== "working") continue;
      const submission = directSubmissionsRef.current.get(session.session_id);
      if (!submission) continue;
      directSubmissionsRef.current.delete(session.session_id);
      submittingDraftStartedAtRef.current.delete(session.session_id);
      if (submission.roleIds.length > 0) {
        onRolesConsumed(session.session_id, submission.roleIds);
      }
      changed = releaseSessionDraftSubmission(
        submittingDraftSessionIdsRef,
        session.session_id,
      ) || changed;
    }
    if (changed) setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
  }, [onRolesConsumed, sessions]);

  useEffect(() => {
    if (!completedTurnKey || !activeSessionId || !completedTurnKey.startsWith(`${activeSessionId}:`)) return;
    if (releaseSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId)) {
      submittingDraftStartedAtRef.current.delete(activeSessionId);
      setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    }
  }, [activeSessionId, completedTurnKey]);

  const latestActiveTurn = activeSession?.turns.at(-1);
  useEffect(() => {
    if (!activeSessionId || !latestActiveTurn || latestActiveTurn.state === "working") return;
    const startedAt = submittingDraftStartedAtRef.current.get(activeSessionId);
    if (startedAt === undefined || latestActiveTurn.created_at_ms < startedAt) return;
    if (releaseSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId)) {
      submittingDraftStartedAtRef.current.delete(activeSessionId);
      setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    }
  }, [activeSessionId, latestActiveTurn?.created_at_ms, latestActiveTurn?.state]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const previous = previousScrollMetrics.current;
    if (!viewport || !previous) return;
    viewport.scrollTop = preservePrependScrollTop(previous, viewport.scrollHeight);
    previousScrollMetrics.current = null;
  }, [turns.length]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !latestTurn?.turn_id) return;
    if (restoredSessionIdRef.current === activeSessionId) {
      restoredSessionIdRef.current = undefined;
      return;
    }
    followThreadLatest.current = true;
    viewport.scrollTop = viewport.scrollHeight;
  }, [activeSessionId, latestTurn?.turn_id]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport || !followThreadLatest.current || previousScrollMetrics.current) return;
    viewport.scrollTop = viewport.scrollHeight;
  }, [latestTurnVersion]);

  const loadEarlierTurns = () => {
    if (sessionInteractionLocked) return;
    if (!activeSession || !canLoadStoredHistory || loadingHistory) return;
    if (viewportRef.current) {
      previousScrollMetrics.current = {
        scrollTop: viewportRef.current.scrollTop,
        scrollHeight: viewportRef.current.scrollHeight,
      };
    }
    onLoadMoreHistory(activeSession);
  };
  const submitDraft = () => {
    if (uploadingAttachment || sessionInteractionLocked) return;
    const reserved = reserveSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId, draftsBySession);
    if (reserved === null) return;
    setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    const attachmentIds = availableAttachments.map((attachment) => attachment.id);
    const existingQueue = queuedMessagesBySessionRef.current[reserved.sessionId] ?? [];
    const direct = activeSession?.session_id === reserved.sessionId
      && shouldDirectManualMessage(activeSession.state, existingQueue.length, !!queuedMessagesPause);
    let sent: boolean;
    let directCommandId: string | undefined;
    if (direct) {
      directCommandId = clientId("submit");
      sent = onSendForSession(
        reserved.sessionId,
        reserved.text,
        directCommandId,
        attachmentIds,
        false,
        selectedRoleIds,
      );
    } else {
      const nextQueues = {
        ...queuedMessagesBySessionRef.current,
        [reserved.sessionId]: [...existingQueue, {
          id: clientId("queued"),
          text: reserved.text,
          createdAtMs: Date.now(),
          attachmentIds,
          roleIds: [...selectedRoleIds],
        }],
      };
      sent = !!reliableStorageScope
        && saveQueuedMessages(window.localStorage, reliableStorageScope, nextQueues, queuedMessagesBySessionRef.current);
      if (sent) {
        // Busy, paused, and already-backed-up sessions retain durable FIFO ordering.
        updateQueuedMessages(() => nextQueues);
      }
    }
    if (sent && !directCommandId && selectedRoleIds.length > 0) {
      onRolesConsumed(reserved.sessionId);
    }
    // Release the synchronous deduplication lock before publishing the React state
    // snapshot. Calling the mutating helper inside a deferred state updater would
    // leave the next lock snapshot stale and keep the composer stuck on Sending.
    const nextDrafts = finishSessionDraftSubmission(submittingDraftSessionIdsRef, draftsBySession, reserved.sessionId, reserved.text, sent);
    if (sent && directCommandId) {
      // Keep this Session occupied until Core authoritatively starts the turn.
      // A rejected command ACK releases the same lock so the user can retry.
      directSubmissionsRef.current.set(reserved.sessionId, {
        commandId: directCommandId,
        text: reserved.text,
        roleIds: [...selectedRoleIds],
      });
      submittingDraftStartedAtRef.current.set(reserved.sessionId, Date.now());
      submittingDraftSessionIdsRef.current.add(reserved.sessionId);
    }
    setDraftsBySession(nextDrafts);
    if (!submittingDraftSessionIdsRef.current.has(reserved.sessionId)) submittingDraftStartedAtRef.current.delete(reserved.sessionId);
    setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
  };

  const submitDraftAsSupplement = () => {
 if (uploadingAttachment || sessionInteractionLocked) return;
 const reserved = reserveSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId, draftsBySession);
 if (reserved === null) return;
 setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
 const sent = onSendForSession(
 reserved.sessionId,
 reserved.text,
 clientId("supplement"),
 availableAttachments.map((attachment) => attachment.id),
 true,
 selectedRoleIds,
 );
 if (sent && selectedRoleIds.length > 0) onRolesConsumed(reserved.sessionId);
 const nextDrafts = finishSessionDraftSubmission(
 submittingDraftSessionIdsRef,
 draftsBySession,
 reserved.sessionId,
 reserved.text,
 sent,
 );
 setDraftsBySession(nextDrafts);
 if (!submittingDraftSessionIdsRef.current.has(reserved.sessionId)) submittingDraftStartedAtRef.current.delete(reserved.sessionId);
 setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
};

const toggleQueuedMessages = () => {
    if (!activeSessionId) return;
    setExpandedQueueSessionIds((current) => {
      const next = new Set(current);
      if (next.has(activeSessionId)) next.delete(activeSessionId);
      else next.add(activeSessionId);
      return next;
    });
  };

  const toggleQueuedMessagePanel = () => {
    if (!activeSessionId) return;
    setCollapsedQueuePanelSessionIds((current) => {
      const next = new Set(current);
      if (next.has(activeSessionId)) next.delete(activeSessionId);
      else next.add(activeSessionId);
      return next;
    });
  };

  const dropQueuedMessage = (targetId: string) => {
    if (!activeSessionId || !draggedQueueMessageId) return;
    updateQueuedMessages((current) => ({
      ...current,
      [activeSessionId]: reorderQueuedMessages(current[activeSessionId] ?? [], draggedQueueMessageId, targetId, queuedMessageClaimsRef.current, activeSessionId),
    }));
    setDraggedQueueMessageId(undefined);
  };

  const saveQueuedMessageEdit = () => {
    const edit = editingQueuedMessage;
    const text = edit?.text.trim();
    if (!edit || !text) return;
    updateQueuedMessages((current) => ({
      ...current,
      [edit.sessionId]: queuedMessageClaimsRef.current.has(queuedMessageKey(edit.sessionId, edit.id))
        ? current[edit.sessionId] ?? []
        : (current[edit.sessionId] ?? []).map((message) => message.id === edit.id ? { ...message, text, deliveryError: undefined } : message),
    }));
    setEditingQueuedMessage(undefined);
  };

 const cancelActiveSessionTurn = async () => {
 pauseQueuedMessages("CancelledByUser");
 setEditingQueuedMessage(undefined);
 await onCancel();
 };

 return <ThreadPrimitive.Root key={activeSessionId ?? "no-session"} className="aui-thread">
    <ThreadPrimitive.Viewport
      ref={viewportRef}
      className="chat-scroll aui-thread-viewport"
      autoScroll={false} scrollToBottomOnInitialize={false} scrollToBottomOnRunStart={false} scrollToBottomOnThreadSwitch={false}
      onScroll={(event) => {
        followThreadLatest.current = isNearScrollBottom({
          scrollTop: event.currentTarget.scrollTop,
          scrollHeight: event.currentTarget.scrollHeight,
          clientHeight: event.currentTarget.clientHeight,
        });
        if (activeSessionId) sessionScrollPositionsRef.current.set(activeSessionId, {
          scrollTop: event.currentTarget.scrollTop,
          followLatest: followThreadLatest.current,
        });
      }}
    >
      {(activeSession?.turns.length ?? 0) === 0 &&
        <div className="welcome"><Sparkles size={24}/><h2>{welcomeTitle}</h2><p>{welcomeText}</p></div>
      }
      {canLoadStoredHistory && <button type="button" className={`load-history ${loadingHistory ? "loading" : ""}`} title={historyButtonLabel} aria-label={historyButtonLabel} aria-live="polite" aria-busy={loadingHistory || undefined} disabled={loadingHistory || sessionInteractionLocked} onClick={loadEarlierTurns}>{loadingHistory && <LoaderCircle size={13} aria-hidden="true"/>}<span>{historyButtonLabel}</span></button>}
      <VisibleTurnList
        sessionId={activeSession?.session_id ?? ""}
        turns={turns}
        restartMarkers={restartMarkers}
        decisionsByTurn={decisionsByTurn}
        sessionInteractionLocked={sessionInteractionLocked}
        pendingDecisionKeys={pendingDecisionKeys}
        pendingToolGenTurnIds={pendingToolGenTurnIds}
        toolGenSessionBusy={toolGenSessionBusy}
        onDecisionReply={onDecisionReply}
        onRequestToolGen={onRequestToolGen}
        onRequestMessageDelete={onRequestMessageDelete}
      />
      <ThreadPrimitive.ViewportFooter className="composer-wrap aui-thread-footer">
        <ThreadPrimitive.ScrollToBottom asChild><button type="button" className="scroll-to-bottom" title="Scroll to latest message" aria-label="Scroll to latest message"><ArrowDown size={16} aria-hidden="true"/></button></ThreadPrimitive.ScrollToBottom>
        {!!activeSession && displayQueuedMessages.length > 0 && <section className={`queued-message-list ${queueExpanded ? "expanded" : "collapsed"} ${queuePanelCollapsed ? "summary-only" : ""} ${queuedMessagesPause ? "paused" : ""}`} aria-label={`${displayQueuedMessages.length} queued message${displayQueuedMessages.length === 1 ? "" : "s"}`} aria-live="polite"><header><span>待发送</span><small>{queuePanelCollapsed ? `${displayQueuedMessages.length} 条消息` : queuedMessagesPause ? "自动续发已暂停，手动发送仍可用" : "上一条完成后自动发送"}</small><div className="queued-message-header-actions">{!queuePanelCollapsed && queuedMessagesPause && <button type="button" className="queued-message-resume" onClick={resumeQueuedMessages}>继续发送</button>}{!queuePanelCollapsed && hiddenQueuedMessageCount > 0 && <button type="button" className="queued-message-toggle" aria-expanded={queueExpanded} title={queueExpanded ? "收起待发送消息" : `向上展开全部 ${displayQueuedMessages.length} 条待发送消息`} onClick={toggleQueuedMessages}>{queueExpanded ? <ChevronDown size={13}/> : <ChevronUp size={13}/>}<span>{queueExpanded ? "收起" : `展开 ${hiddenQueuedMessageCount} 条`}</span></button>}<button type="button" className="queued-message-panel-toggle" aria-expanded={!queuePanelCollapsed} aria-controls={`queued-message-items-${activeSession.session_id}`} title={queuePanelCollapsed ? "展开待发送队列" : "折叠待发送队列为一行"} onClick={toggleQueuedMessagePanel}>{queuePanelCollapsed ? <ChevronDown size={14}/> : <ChevronUp size={14}/>}<span>{queuePanelCollapsed ? "展开" : "折叠"}</span></button></div></header>{!queuePanelCollapsed && <div id={`queued-message-items-${activeSession.session_id}`} className="queued-message-items">{visibleQueuedMessages.map((message) => {
          const index = queuedMessages.findIndex((candidate) => candidate.id === message.id);
          const editing = editingQueuedMessage?.sessionId === activeSession.session_id && editingQueuedMessage.id === message.id;
          const claimed = queuedMessageClaims.has(queuedMessageKey(activeSession.session_id, message.id));
          const messageRoleIds = message.roleIds ?? (message.roleId ? [message.roleId] : []);
          const messageRoleNames = messageRoleIds.map((roleId) => activeSession.roles.find((role) => role.id === roleId)?.name ?? roleId);
          return <article className={`queued-message ${editing ? "editing" : ""} ${message.deliveryError ? "delivery-error" : ""} ${draggedQueueMessageId === message.id ? "dragging" : ""} ${claimed ? "sending" : ""}`} aria-busy={claimed || undefined} key={message.id} onDragOver={(event) => { if (!editing && !claimed) { event.preventDefault(); event.dataTransfer.dropEffect = "move"; } }} onDrop={(event) => { event.preventDefault(); if (!editing && !claimed) dropQueuedMessage(message.id); }}><button type="button" className="queued-message-drag" draggable={!editing && !claimed && displayQueuedMessages.length > 1} disabled={editing || claimed} title={`拖动调整第 ${index + 1} 条消息的顺序`} aria-label={`拖动调整第 ${index + 1} 条消息的顺序`} onDragStart={(event) => { event.dataTransfer.effectAllowed = "move"; event.dataTransfer.setData("text/plain", message.id); setDraggedQueueMessageId(message.id); }} onDragEnd={() => setDraggedQueueMessageId(undefined)}><GripVertical size={13}/></button><span className="queued-message-order" aria-label={`Queue position ${index + 1}`}>{index + 1}</span><div className="queued-message-preview">{messageRoleNames.length > 0 && <small className="queued-message-roles" title={messageRoleNames.join(" | ")}><BriefcaseBusiness size={11}/><span className="queued-message-role-names">{messageRoleNames.map((roleName, roleIndex) => <span className="queued-message-role" key={`${messageRoleIds[roleIndex]}-${roleIndex}`}>{roleIndex > 0 && <i className="queued-message-role-separator" aria-hidden="true">|</i>}<span>{roleName}</span></span>)}</span></small>}{editing ? <textarea className="queued-message-editor" autoFocus value={editingQueuedMessage.text} aria-label={`编辑第 ${index + 1} 条待发送消息`} onChange={(event) => setEditingQueuedMessage({ ...editingQueuedMessage, text: event.target.value })} onKeyDown={(event) => { if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); saveQueuedMessageEdit(); } if (event.key === "Escape") { event.preventDefault(); setEditingQueuedMessage(undefined); } }}/>: <p title={message.deliveryError || message.text}>{message.text}</p>}{message.attachmentIds.length > 0 && <small className="queued-message-attachments"><Paperclip size={11}/>{message.attachmentIds.length} 个附件</small>}{message.deliveryError && <small className="queued-message-error">{message.deliveryError}</small>}</div><div className="queued-message-actions">{editing ? <><button type="button" className="queued-message-edit-save" disabled={!editingQueuedMessage.text.trim() || claimed} onClick={saveQueuedMessageEdit}>保存</button><button type="button" className="queued-message-edit-cancel" disabled={claimed} onClick={() => setEditingQueuedMessage(undefined)}>取消</button></> : <><button type="button" className="queued-message-edit" title="重新编辑这条待发送消息" aria-label={`重新编辑第 ${index + 1} 条待发送消息`} disabled={claimed} onClick={() => { setEditingQueuedMessage({ sessionId: activeSession.session_id, id: message.id, text: message.text }); setExpandedQueueSessionIds((current) => new Set(current).add(activeSession.session_id)); }}><Pencil size={12}/></button><button type="button" className="queued-message-supplement" title={message.deliveryError ? "重试发送这条消息" : "立即发送为当前任务的补充"} disabled={claimed || (!message.deliveryError && activeSession.state !== "working") || sessionInteractionLocked || isCancelling} onClick={() => {
          if (!claimQueuedMessage(queuedMessageClaimsRef.current, activeSession.session_id, queuedMessagesBySession[activeSession.session_id] ?? [], message.id)) return;
          setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
          if (!onSendForSession(activeSession.session_id, message.text, message.id, message.attachmentIds, false, messageRoleIds)) {
            releaseQueuedMessageClaim(queuedMessageClaimsRef.current, activeSession.session_id, message.id);
            setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
            updateQueuedMessages((current) => ({ ...current, [activeSession.session_id]: (current[activeSession.session_id] ?? []).map((candidate) => candidate.id === message.id ? { ...candidate, deliveryError: "消息尚未安全保存，请检查浏览器存储后重试" } : candidate) }));
            return;
          }
          if (message.deliveryError) updateQueuedMessages((current) => ({ ...current, [activeSession.session_id]: (current[activeSession.session_id] ?? []).map((candidate) => candidate.id === message.id ? { ...candidate, deliveryError: undefined } : candidate) }));
        }}>{claimed ? "发送中…" : message.deliveryError ? "重试" : "立即"}</button><button type="button" className="queued-message-remove" title="Remove queued message" aria-label={`Remove queued message ${index + 1}`} disabled={claimed} onClick={() => updateQueuedMessages((current) => ({ ...current, [activeSession.session_id]: removeQueuedMessage(current[activeSession.session_id] ?? [], message.id, queuedMessageClaimsRef.current, activeSession.session_id) }))}><X size={13}/></button></>}</div></article>;
        })}</div>}</section>}
        {!!activeSession && (!!availableAttachments.length || uploadingAttachment) && <div className="attachment-strip" aria-label={attachmentStripLabel} aria-live="polite" aria-busy={uploadingAttachment || undefined}>{attachedFileCount > 0 && <div className="attachment-summary" title={attachmentSummary}><Paperclip size={13}/><span>{attachmentSummary}</span></div>}{uploadingAttachment && <div className="pending-attachment uploading" role="status" aria-label={uploadingAttachmentFile ? `${uploadingAttachmentText}, ${formatBytes(uploadingAttachmentFile.bytes)}` : uploadingAttachmentText} title={uploadingAttachmentFile?.name ?? uploadingAttachmentText}><span className="upload-dot" aria-hidden="true"/><span className="pending-attachment-name">{uploadingAttachmentFile?.name ?? "Uploading file…"}</span>{uploadingAttachmentFile && <small>{formatBytes(uploadingAttachmentFile.bytes)}</small>}</div>}{availableAttachments.map((attachment) => {
          const removing = pendingAttachmentRemoveIds.has(`${activeSession.session_id}:${attachment.id}`);
          const removeLabel = removing ? `Removing ${attachment.name}` : sessionInteractionLocked ? `${sessionInteractionLockReason} · cannot remove ${attachment.name}` : `Remove ${attachment.name}`;
          return <div className="pending-attachment" key={attachment.id} title={attachment.name}><Paperclip size={13}/><span className="pending-attachment-name">{attachment.name}</span><small>{formatBytes(attachment.bytes)}</small><button type="button" title={removeLabel} aria-label={removeLabel} aria-busy={removing || undefined} disabled={removing || sessionInteractionLocked} onClick={() => onRemoveAttachment(attachment.id)}>{removing ? "…" : <X size={13}/>}</button></div>;
        })}</div>}
        <form className="composer" onSubmit={(event) => { event.preventDefault(); submitDraft(); }}>
          <textarea
            value={draft}
            placeholder={!activeSession ? "Create a session to start…" : sessionInteractionLocked ? sessionInteractionLockReason : activeSession.state === "working" ? "继续输入…" : "Ask Timem to investigate, write, or work with you."}
            aria-label="Message Timem"
            aria-describedby={composerHintId}
            title={composerHint}
            disabled={!activeSession || sessionInteractionLocked}
            onChange={(event) => setDraftsBySession((current) => setSessionDraft(current, activeSessionId, event.target.value))}
            onKeyDown={(event) => {
 if (event.key !== "Enter" || event.nativeEvent.isComposing) return;
 if (event.metaKey || event.ctrlKey) {
 event.preventDefault();
 submitDraftAsSupplement();
 return;
 }
 if (!event.shiftKey) {
 event.preventDefault();
 submitDraft();
 }
 }}
 />
          {selectedRoles.length > 0 && activeSession && <div className="composer-role" title={selectedRoles.map((role) => `${role.name}: ${role.description}`).join("\n")}><BriefcaseBusiness size={14}/><span>本条将使用 <strong>{selectedRoles.map((role) => role.name).join("、")}</strong></span><button type="button" title="Clear roles for this message" aria-label="Clear selected roles" onClick={() => onRolesConsumed(activeSession.session_id)}><X size={13}/></button></div>}
          <div className="composer-actions"><span className="composer-cwd-inline" title={activeSession?.current_dir}>{activeSession && <><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></>}</span><span id={composerHintId} className="sr-only" role="status" aria-live="polite">{composerHint}</span><div className="composer-buttons"><button className={`attach-button ${uploadingAttachment ? "uploading" : ""}`} type="button" title={attachTitle} aria-label={attachLabel} disabled={!activeSession || uploadingAttachment || sessionInteractionLocked} onClick={() => fileInput.current?.click()}>{uploadingAttachment ? <LoaderCircle size={17}/> : <Paperclip size={17}/>}</button><input ref={fileInput} className="file-input" type="file" disabled={!activeSession || uploadingAttachment || sessionInteractionLocked} onChange={(event) => { const file = event.target.files?.[0]; event.currentTarget.value = ""; if (file) void onUpload(file); }}/><button className={`send-button ${submittingDraft ? "sending" : ""}`} type="submit" title={effectiveSendLabel} aria-label={effectiveSendLabel} disabled={!activeSession || !draft.trim() || submittingDraft || uploadingAttachment || sessionInteractionLocked}>{submittingDraft ? <LoaderCircle size={17}/> : <Send size={17}/>}</button>{activeSession?.state === "working" && <button className={`stop-button ${isCancelling ? "sending" : ""}`} type="button" title={isCancelling ? "Cancellation requested" : lockedControlHint || "Cancel current turn"} aria-label={isCancelling ? "Cancellation requested" : lockedControlHint || "Cancel current turn"} disabled={isCancelling || sessionInteractionLocked} onClick={() => void cancelActiveSessionTurn()}>{isCancelling ? <LoaderCircle size={17}/> : <CircleStop size={17}/>} {isCancelling ? "Stopping…" : "Stop"}</button>}</div></div>
        </form>
      </ThreadPrimitive.ViewportFooter>
    </ThreadPrimitive.Viewport>
  </ThreadPrimitive.Root>;
}

type TurnInteractionProps = {
  sessionId: string;
  turn: WebTurn;
  decisions: Decision[];
  sessionInteractionLocked: boolean;
  pendingDecisionKeys: Set<string>;
  toolGenPending: boolean;
  toolGenBlocked: boolean;
  onDecisionReply: (decision: Decision, reply: "accept" | "decline" | "always_allow") => void;
  onRequestToolGen: (turnId: string) => void;
  onRequestMessageDelete: (candidate: ChatMessageDeleteCandidate) => void;
};

const TurnInteraction = memo(function TurnInteraction({ sessionId, turn, decisions, sessionInteractionLocked, pendingDecisionKeys, toolGenPending, toolGenBlocked, onDecisionReply, onRequestToolGen, onRequestMessageDelete }: TurnInteractionProps) {
  const workScrollRef = useRef<HTMLDivElement | null>(null);
 const workContentRef = useRef<HTMLDivElement | null>(null);
 const followLatest = useRef(true);
  const previousUpdateCount = useRef(turn.events.length + decisions.length);
  const previousTurnState = useRef(turn.state);
  const [pendingUpdates, setPendingUpdates] = useState(0);
  const [workingElapsedMs, setWorkingElapsedMs] = useState(() => Math.max(0, Date.now() - turn.created_at_ms));
  const lifecycleEvents = useMemo(() => coalesceActionLifecycle(turn.events), [turn.events]);
 const lifecycleItems = useMemo(() => lifecycleEvents.map((event) => ({
 event,
 activity: activityFromTurnEvent(event, sessionId),
 })), [lifecycleEvents, sessionId]);
 const visibleItems = useMemo(() => lifecycleItems.slice(-MAX_RENDERED_TURN_EVENTS), [lifecycleItems]);
 const processActivities = useMemo(() => lifecycleItems
 .map(({ activity }) => activity)
 .filter((activity): activity is Activity => activity !== null), [lifecycleItems]);
 const persistentToolGenItems = useMemo(() => visibleItems.filter(({ activity }) => (
 activity?.kind === "toolgen" && activity.toolgen_phase === "published"
 )), [visibleItems]);
 const persistentToolGenEventIds = useMemo(() => new Set(
 persistentToolGenItems.map(({ event }) => event.event_id)
 ), [persistentToolGenItems]);
 const scrollItems = useMemo(() => visibleItems.filter(
 ({ event }) => !persistentToolGenEventIds.has(event.event_id)
 ), [persistentToolGenEventIds, visibleItems]);
 const toolActivityRuns = useMemo(
 () => summarizeConsecutiveToolActivities(scrollItems.map(({ activity }) => activity)),
 [scrollItems],
 );
 const toolActivityRunByStartIndex = useMemo(
 () => new Map(toolActivityRuns.map((run) => [run.startIndex, run.summary])),
 [toolActivityRuns],
 );
 const omitted = lifecycleItems.length - visibleItems.length;
 const hasVisibleProcess = processActivities.length > 0 || decisions.length > 0 || turn.state === "working";
  const hasOnlyFreeTalk = hasOnlyFreeTalkActivity(processActivities, decisions.length);
  const interruptedByUser = turn.completion?.stop_reason?.toLowerCase() === "cancelledbyuser";
  const [showCompletedWork, setShowCompletedWork] = useState(() => !interruptedByUser && (turn.state === "working" || !hasOnlyFreeTalk));
  const isToolGenTurn = turn.turn_id.startsWith("web_toolgen_turn_")
    || turn.user_entries.some((entry) => entry.kind === "toolgen_instruction")
    || turn.events.some((event) => (event.payload.topic as { name?: string } | undefined)?.name === "core.toolgen");
  const canCollapseCompletedWork = turn.state !== "working" && (!!turn.final_answer || interruptedByUser);
  const showWorkStream = !canCollapseCompletedWork || showCompletedWork;
  const workingElapsed = turn.state === "working" ? formatDuration(workingElapsedMs) : undefined;

  useEffect(() => {
    if (turn.state !== "working") return;
    const updateElapsed = () => setWorkingElapsedMs(Math.max(0, Date.now() - turn.created_at_ms));
    updateElapsed();
    const timer = window.setInterval(updateElapsed, 1_000);
    return () => window.clearInterval(timer);
  }, [turn.created_at_ms, turn.state]);
  const canDeleteConversationContent = turn.state !== "working" && !sessionInteractionLocked;

  useEffect(() => {
    const wasWorking = previousTurnState.current === "working";
    previousTurnState.current = turn.state;
    if (wasWorking && turn.state !== "working" && (hasOnlyFreeTalk || interruptedByUser)) setShowCompletedWork(false);
  }, [hasOnlyFreeTalk, interruptedByUser, turn.state]);

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
 useLayoutEffect(() => {
 const scroll = workScrollRef.current;
 const content = workContentRef.current;
 if (!scroll || !content || typeof ResizeObserver === "undefined") return;
 const observer = new ResizeObserver(() => {
 if (!followLatest.current) return;
 scroll.scrollTop = scroll.scrollHeight;
 setPendingUpdates(0);
 });
 observer.observe(content);
 return () => observer.disconnect();
 }, [showWorkStream]);


  const scrollWorkToLatest = () => {
    const scroll = workScrollRef.current;
    if (!scroll) return;
    scroll.scrollTo({ top: scroll.scrollHeight, behavior: prefersReducedMotion() ? "auto" : "smooth" });
    followLatest.current = true;
    setPendingUpdates(0);
  };

  return <article className={`turn-interaction ${turn.state === "working" ? "active" : "completed"}`} data-turn-id={turn.turn_id}>
    {!!turn.user_entries.filter((e) => e.kind !== "approval").length && <section className="turn-user-frame">
      <div className="turn-user-content">{turn.user_entries.map((entry, roleIndex) => ({ entry, roleIndex })).filter(({ entry }) => entry.kind !== "approval").map(({ entry, roleIndex }) => <div className={`turn-user-entry ${entry.kind}`} key={`${entry.created_at_ms}-${roleIndex}`}>
        <button type="button" className="chat-message-delete user-message-delete" title="Delete this message from the conversation and raw chat log" aria-label="Delete user message" disabled={!canDeleteConversationContent} onClick={() => onRequestMessageDelete({ sessionId, turnId: turn.turn_id, role: "user", roleIndex, preview: entry.text })}><Trash2 size={13}/></button>
        {entry.kind === "supplement" && <span>[补充]</span>}
        <MarkdownContent text={entry.text}/>
        {(entry.worker_roles ?? (entry.worker_role ? [entry.worker_role] : [])).length > 0 && <div className="turn-entry-roles" aria-label={`使用 Role：${(entry.worker_roles ?? (entry.worker_role ? [entry.worker_role] : [])).map((role) => role.name).join("、")}`}><BriefcaseBusiness size={12}/><span>Role</span>{(entry.worker_roles ?? (entry.worker_role ? [entry.worker_role] : [])).map((role) => <b key={role.id} title={role.description}>{role.name}</b>)}</div>}
        {!!entry.attachments?.length && <div className="turn-entry-attachments">{entry.attachments.map((attachment) => <span key={attachment.id} title={attachment.path}><Paperclip size={13}/><i aria-hidden="true">:</i><b>{attachment.name}</b><small>{formatBytes(attachment.bytes)}</small></span>)}</div>}
      </div>)}</div>
    </section>}
    {hasVisibleProcess && <section className={`turn-assistant-frame ${turn.state} ${showWorkStream ? "" : "collapsed-work"}`}>
      {(turn.state === "working" || canCollapseCompletedWork) && <div className="turn-assistant-heading">{canCollapseCompletedWork ? <button type="button" className={`working-chip work-title-chip completed-work-title work-collapse-toggle${interruptedByUser ? " interrupted-work-title" : ""}${isToolGenTurn ? " toolgen-working toolgen-completed-title" : ""}`} title={showCompletedWork ? "Hide work details" : "Show work details"} aria-label={showCompletedWork ? "Hide work details" : "Show work details"} aria-expanded={showCompletedWork} onClick={() => setShowCompletedWork((visible) => !visible)}><ChevronRight className="work-collapse-arrow" size={13} aria-hidden="true"/>{isToolGenTurn && <Wrench size={11}/>} {isToolGenTurn ? "ToolGen" : "Thought/Action"}{interruptedByUser && <span className="work-title-status">(Interrupted)</span>}</button> : <span className={`working-chip work-title-chip active-work-title${isToolGenTurn ? " toolgen-working" : ""}`} role="status" aria-live="polite">{isToolGenTurn && <Wrench size={11}/>} {isToolGenTurn ? "Generating tools…" : <span className="working-label">working</span>}{workingElapsed && <span className="working-elapsed" aria-hidden="true">{workingElapsed}</span>}</span>}</div>}
      {showWorkStream && <div className="turn-work-panel">
        <div className={`turn-work-scroll ${pendingUpdates > 0 ? "has-pending-updates" : ""}${visibleItems.length === 0 && decisions.length === 0 ? " empty" : " has-content"}`} role="region" aria-label={isToolGenTurn ? "ToolGen work stream" : "Task work stream"} ref={workScrollRef} onScroll={(event) => {
          followLatest.current = isNearScrollBottom({ scrollTop: event.currentTarget.scrollTop, scrollHeight: event.currentTarget.scrollHeight, clientHeight: event.currentTarget.clientHeight }, 36);
          if (followLatest.current) setPendingUpdates(0);
        }}>
          <div className="turn-work-content" ref={workContentRef}> {omitted > 0 && <div className="turn-events-omitted">{omitted} earlier work updates are retained by the host but not rendered.</div>} {scrollItems.map(({ event, activity }, index) => { if (activity?.tone === "action") { const summary = toolActivityRunByStartIndex.get(index); return summary ? <ToolActivityGroup key={`tool-activity-group-${event.event_id}`} summary={summary}/> : null; } return activity ? <ActivityView key={event.event_id} activity={activity}/> : null; })} {decisions.map((decision, index) => <InlineDecision key={decisionKey(decision)} decision={decision} pending={pendingDecisionKeys.has(decisionKey(decision))} locked={sessionInteractionLocked} position={index + 1} total={decisions.length} onReply={(reply) => onDecisionReply(decision, reply)} />)}
          {turn.state === "working" && <LiveTurnUsage turn={turn}/>}
          {visibleItems.length === 0 && decisions.length === 0 && turn.state === "working" && <div className={`working-indicator${isToolGenTurn ? " toolgen-working" : ""}`} role="status" aria-live="polite"><span className="pulse"/>{isToolGenTurn ? "Generating tools…" : "Waiting for the first runtime update…"}</div>} </div>
        </div>
        {pendingUpdates > 0 && <button type="button" className="turn-new-updates" title="Scroll to latest work update" aria-live="polite" aria-label={`${pendingUpdates} new work update${pendingUpdates === 1 ? "" : "s"}; scroll to latest`} onClick={scrollWorkToLatest}><ArrowDown size={13} aria-hidden="true"/>{pendingUpdates} new update{pendingUpdates === 1 ? "" : "s"}</button>}
      </div>}
    </section>}
    {persistentToolGenItems.length > 0 && <div className="turn-persistent-toolgen" aria-label="ToolGen result">{persistentToolGenItems.map(({ event, activity }) => activity ? <ActivityView key={event.event_id} activity={activity}/> : null)}</div>}
    {turn.final_answer && <FinalAnswerDelivery text={turn.final_answer} completion={turn.completion} toolGenPending={toolGenPending} toolGenBlocked={toolGenBlocked} onToolGen={isToolGenTurn ? undefined : () => onRequestToolGen(turn.turn_id)} onDelete={canDeleteConversationContent ? () => onRequestMessageDelete({ sessionId, turnId: turn.turn_id, role: "assistant", roleIndex: 0, preview: turn.final_answer ?? "" }) : undefined}/>}
    {!turn.final_answer && turn.completion && <section className="turn-completion-only"><CompletionCard completion={turn.completion}/></section>}
  </article>;
}, areTurnInteractionPropsEqual);

function areTurnInteractionPropsEqual(previous: TurnInteractionProps, next: TurnInteractionProps) {
  if (
    previous.sessionId !== next.sessionId
    || previous.turn !== next.turn
    || previous.sessionInteractionLocked !== next.sessionInteractionLocked
    || previous.toolGenPending !== next.toolGenPending
    || previous.toolGenBlocked !== next.toolGenBlocked
    || previous.onDecisionReply !== next.onDecisionReply
    || previous.onRequestToolGen !== next.onRequestToolGen
    || previous.onRequestMessageDelete !== next.onRequestMessageDelete
    || previous.decisions.length !== next.decisions.length
  ) return false;
  return previous.decisions.every((decision, index) => {
    const nextDecision = next.decisions[index];
    return decision === nextDecision
      && previous.pendingDecisionKeys.has(decisionKey(decision)) === next.pendingDecisionKeys.has(decisionKey(nextDecision));
  });
}

function FinalAnswerDelivery({ text, completion, toolGenPending, toolGenBlocked, onToolGen, onDelete }: { text: string; completion: WebTurn["completion"]; toolGenPending: boolean; toolGenBlocked: boolean; onToolGen?: () => void; onDelete?: () => void }) {
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(text, {
    idle: "Copy answer",
    copied: "Answer copied",
    failed: "Copy answer failed",
  });
  return <section className="turn-final-delivery">
    <div className="turn-final-toolbar"><button type="button" className={`final-copy ${copyClass}`} title={copyLabel} aria-label={copyLabel} onClick={() => void copy()}>{copyState === "copied" ? <CheckCheck size={13}/> : <Copy size={13}/>}<span aria-live="polite">{copyLabel}</span></button>{onDelete && <button type="button" className="chat-message-delete assistant-message-delete" title="Delete this answer from the conversation and raw chat log" aria-label="Delete assistant answer" onClick={onDelete}><Trash2 size={13}/><span>Delete</span></button>}</div>
    <div className="message-content"><MarkdownContent text={text}/></div>
    {completion && <CompletionCard completion={completion} toolGenPending={toolGenPending} toolGenBlocked={toolGenBlocked} onToolGen={onToolGen}/>}
  </section>;
}

function HeaderContextUsage({ session }: { session: Session | undefined }) {
  const usage = session ? sessionContextUsage(session) : undefined;
  const limit = session?.max_llm_input_tokens || undefined;
  const ratio = limit ? Math.min(100, Math.ceil((usage?.prompt_tokens ?? 0) * 100 / limit)) : 0;
  const level = ratio >= 90 ? "critical" : ratio >= 75 ? "warning" : "normal";
  const contextUsageLabel = limit
    ? `Context usage ${ratio}% · ${formatTokens(usage?.prompt_tokens ?? 0)} / ${formatTokens(limit)} input tokens`
    : "Context usage waiting for runtime usage";
  return <span className={`header-context ${level}`} title={contextUsageLabel} aria-label={contextUsageLabel}>
    <span aria-hidden="true">· ctx</span><span className="header-context-meter" aria-hidden="true"><span style={{ width: `${ratio}%` }}/></span><span>{limit ? `${ratio}%/${formatTokens(limit)}` : "—"}</span>
  </span>;
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

function ActivityView({ activity }: { activity: Activity }) {
 if (activity.kind === "context_compact") return <ContextCompactNotice activity={activity}/>;
 if (activity.kind === "toolgen") return <ToolGenNotice activity={activity}/>;
 if (activity.tone === "action") return <ToolActivity activity={activity}/>;
 return <div className={`turn-work-item ${activity.tone}${activity.kind === "free_talk" ? " free-talk" : ""}`}>
 <span className="activity-mark">{activity.tone === "thinking" ? <span className="activity-thinking-dot" aria-hidden="true"/> : activity.tone === "warning" ? "⚠️" : activity.tone === "error" ? "×" : "i"}</span>
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

function ToolActivityGroup({ summary }: { summary: ToolActivitySummary }) {
 const [open, setOpen] = useState(false);
 const running = summary.status === "running";
 const summaryLabel = `${open ? "收起" : "展开"}工具活动：${summary.label}，${summary.status}`;
 return <details className={`tool-activity-group ${summary.status}`} open={open} aria-busy={running || undefined} onToggle={(event) => setOpen(event.currentTarget.open)}>
 <summary aria-label={summaryLabel} title={open ? "收起工具活动" : "展开工具活动"}>
 <span className="tool-activity-group-icon tool-command-symbol" aria-hidden="true">&gt;_</span>
 <span className="tool-activity-group-counts" aria-hidden="true">{summary.counts.map(({ name, count }, index) => <span className="tool-activity-group-count" key={name}>{index > 0 && <i>|</i>}<span>{name}</span><strong>{count}</strong></span>)}</span>
 <span className="tool-activity-group-status">· {summary.status}</span>
 <ChevronRight className="tool-activity-chevron" size={14}/>
 </summary>
 <div className="tool-activity-group-body">{summary.activities.map((activity, index) => <ToolActivity key={`${activity.id}-${index}`} activity={activity}/>)}</div>
 </details>;
}

function ToolActivity({ activity }: { activity: Activity }) {
  const status = activity.tool_status || "running";
  const running = status === "running" || status === "background_running";
  const bashActivity = activity.tool_name === "run_bash";
  const [open, setOpen] = useState(false);
  const invocationPreview = toolInvocationPreview(activity);
 const detail = activity.detail?.trim();
 const code = activity.code?.trim();
 const hasExpandableDetail = !!detail || !!code;
  const toolName = toolDisplayName(activity.tool_name || activity.title);
  const summaryLabel = `${open ? "收起" : "展开"}工具详情：${toolName}`;
  const summaryContent = <>
    <span className="tool-activity-icon tool-command-symbol" aria-hidden="true">&gt;_</span>
    <b>{toolName}</b>
    <span className="tool-activity-meta"><span className="tool-activity-status">{humanizeToolStatus(status)}</span>{activity.elapsed_ms !== undefined && !running && <span className="tool-activity-duration">{formatDuration(activity.elapsed_ms)}</span>}</span>
    {invocationPreview && <code className="tool-activity-command" title={invocationPreview}>{invocationPreview}</code>}
  </>;
  if (!hasExpandableDetail) return <div className={`tool-activity tool-activity-static ${bashActivity ? "bash-activity" : ""} ${running ? "running" : "settled"}`} aria-busy={running || undefined}>
    {summaryContent}
  </div>;
  return <details className={`tool-activity ${bashActivity ? "bash-activity" : ""} ${running ? "running" : "settled"}`} aria-busy={running || undefined} open={open} onToggle={(event) => setOpen(event.currentTarget.open)}>
    <summary title={open ? "收起工具详情" : "展开工具详情"} aria-label={summaryLabel}>
      {summaryContent}
      <ChevronRight className="tool-activity-chevron" size={14}/>
    </summary>
    <div className="tool-activity-body">
      {detail && <div className="turn-work-detail"><MarkdownContent text={detail}/></div>}
      {code && (
 <MarkdownContent text={fencedCode(activity.code_language ?? "text", code)} />
 )}
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
 if (event.source === "core_topic") {
 const activity = activityFromTopic(event.payload as unknown as import("./protocol").CoreTopicEvent);
 return activity ? {
 ...activity,
 id: event.event_id,
 sessionId,
 createdAt: event.created_at_ms,
 } : null;
 }
 if (event.source !== "worker_activity") return null;
  const kind = String(event.payload.kind ?? "worker_event");
  if (kind === "model_request" || kind === "model_response") return null;
  const detail = Object.entries(event.payload)
    .filter(([key]) => !["kind", "session_id", "context_id", "worker_id"].includes(key))
    .map(([key, value]) => `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`)
    .join("\n");
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

const MarkdownContent = memo(function MarkdownContent({ text }: { text: string }) {
  return <div className="markdown-body"><ReactMarkdown
    remarkPlugins={[remarkGfm]}
    rehypePlugins={[rehypeHighlight]}
    components={{
      a: ({ node: _node, href, ...props }) => {
        const safeHref = safeMarkdownUrl(href);
        return safeHref ? <a {...props} href={safeHref} target="_blank" rel="noopener noreferrer"/> : <span {...props}/>;
      },
      img: ({ node: _node, src, alt, ...props }) => {
        const safeSrc = safeMarkdownUrl(src);
        return safeSrc ? <img {...props} src={safeSrc} alt={alt ?? ""}/> : null;
      },
      pre: CodeBlock,
      table: ({ node: _node, ...props }) => <div className="table-scroll" role="region" tabIndex={0} aria-label="Scrollable table. Use horizontal scroll to inspect all columns."><table {...props}/></div>,
    }}
  >{text}</ReactMarkdown></div>;
});

function CodeBlock({ children }: React.ComponentPropsWithoutRef<"pre">) {
  const child = Children.count(children) === 1 ? Children.only(children) : null;
  const className = isValidElement<{ className?: string }>(child) ? child.props.className ?? "" : "";
  const language = className.match(/(?:^|\s)language-([^\s]+)/)?.[1] ?? "text";
  const code = textFromNode(children).replace(/\n$/, "");
  const codeCopySubject = `${language} code`;
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(code, {
    idle: `Copy ${codeCopySubject}`,
    copied: `${codeCopySubject} copied`,
    failed: `Copy ${codeCopySubject} failed`,
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
      await copyTextToClipboard(text);
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

function prefersReducedMotion() {
  return window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
}

async function copyTextToClipboard(text: string) {
  try {
    await navigator.clipboard.writeText(text);
    return;
  } catch {
    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.left = "-9999px";
    textarea.style.top = "0";
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.select();
    try {
      if (!document.execCommand("copy")) throw new Error("execCommand copy failed");
    } finally {
      document.body.removeChild(textarea);
      window.getSelection()?.removeAllRanges();
    }
  }
}

function textFromNode(node: React.ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textFromNode).join("");
  if (isValidElement<{ children?: React.ReactNode }>(node)) return textFromNode(node.props.children);
  return "";
}

function McpPanel({ panelRef, servers, session, pendingKeys, revealedSecrets, onClose, onCommand }: {
  panelRef: MutableRefObject<HTMLElement | null>;
  servers: McpServerReport[];
  session: Session | undefined;
  pendingKeys: Set<string>;
  revealedSecrets: Record<string, Record<string, string>>;
  onClose: () => void;
  onCommand: (key: string, command: ClientCommand) => void;
}) {
  const [editing, setEditing] = useState<McpServerConfig | null>(null);
  const enabled = new Set(session?.mcp_server_ids ?? []);
  const startNew = () => setEditing({
    id: "",
    name: "",
    enabled: true,
    transport: { type: "stdio", command: "", args: [], env: {} },
    request_timeout_ms: 30000,
  });
  return <section id="mcp-panel" ref={panelRef} className="mcp-panel" role="dialog" aria-modal="false" aria-label="MCP servers" tabIndex={-1}>
    <header><div><span className="eyebrow">MCP</span><h2><strong className="mcp-session-name">{session?.display_name ?? "Current session"}</strong> 's Capabilities</h2></div><button type="button" className="icon-button" title="Close MCP panel" aria-label="Close MCP panel" onClick={onClose}><X size={16}/></button></header>
    {editing ? <McpEditor config={editing} pending={pendingKeys.has(`save:${editing.id || "new"}`)} revealPending={!!editing.id && pendingKeys.has(`reveal:${editing.id}`)} revealedSecrets={editing.id ? revealedSecrets[editing.id] : undefined} onReveal={() => editing.id && onCommand(`reveal:${editing.id}`, { type: "mcp_server_secrets_reveal", server_id: editing.id })} onCancel={() => setEditing(null)} onSave={(config) => {
      if (!session) return;
      const key = `save:${config.id || "new"}`;
      onCommand(key, { type: "mcp_server_upsert", session_id: session.session_id, config });
      setEditing(null);
    }}/> : <>
      <div className="mcp-list">{servers.length === 0 ? <div className="mcp-empty"><Plug size={20}/><strong>No MCP servers</strong><span>Add local stdio, Streamable HTTP, or legacy SSE.</span></div> : servers.map((server) => {
        const active = enabled.has(server.config.id);
        const pending = Array.from(pendingKeys).some((key) => key.endsWith(`:${server.config.id}`));
        const connectionState = !active ? "disabled" : server.state === "connected" ? "connected" : server.state === "error" || !!server.error ? "failed" : "connecting";
        const connectionLabel = connectionState === "connected" ? "Connected" : connectionState === "failed" ? "Enabled, connection failed" : connectionState === "connecting" ? "Enabled, connecting" : "Disabled";
        return <article className={`mcp-server ${connectionState}`} key={server.config.id}>
          <div className="mcp-server-main"><div><strong>{server.config.name}</strong><small>{mcpEndpoint(server.config)}</small></div><button type="button" role="switch" aria-checked={active} aria-label={`${active ? "Disable" : "Enable"} ${server.config.name} for this session`} className={`mcp-session-toggle ${connectionState}`} title={`${connectionLabel} · click to ${active ? "disable" : "enable"}`} disabled={!session || pending} onClick={() => session && onCommand(`toggle:${server.config.id}`, { type: "mcp_session_toggle", session_id: session.session_id, server_id: server.config.id, enabled: !active })}><span className="mcp-port-glyph" aria-hidden="true"><span className="mcp-port-node left"/><span className="mcp-port-link"/><span className="mcp-port-node right"/>{connectionState === "failed" && <X className="mcp-port-failure" size={10}/>}</span></button></div>
          <div className="mcp-server-meta"><span>{mcpTransportLabel(server.config.transport)}</span><span>{connectionLabel}</span><span>{server.tools.length} tool{server.tools.length === 1 ? "" : "s"}</span>{server.error && <span className="mcp-error" title={server.error}>{server.error}</span>}</div>
          <div className="mcp-server-actions"><button type="button" title="Reconnect and refresh tools" aria-label={`Reconnect ${server.config.name}`} disabled={!session || pending} onClick={() => session && onCommand(`reconnect:${server.config.id}`, { type: "mcp_server_reconnect", session_id: session.session_id, server_id: server.config.id })}><RefreshCw size={13}/></button><button type="button" title="Edit server" aria-label={`Edit ${server.config.name}`} disabled={pending} onClick={() => setEditing(server.config)}><Pencil size={13}/></button><button type="button" className="danger" title="Delete server" aria-label={`Delete ${server.config.name}`} disabled={pending} onClick={() => window.confirm(`Delete MCP server “${server.config.name}”? This removes it from every session in the current mem.`) && onCommand(`delete:${server.config.id}`, { type: "mcp_server_delete", server_id: server.config.id })}><Trash2 size={13}/></button></div>
        </article>;
      })}</div>
      <button type="button" className="mcp-add" disabled={!session} onClick={startNew}><Plus size={15}/> Add MCP server</button>
    </>}
  </section>;
}

function McpEditor({ config, pending, revealPending, revealedSecrets, onReveal, onCancel, onSave }: { config: McpServerConfig; pending: boolean; revealPending: boolean; revealedSecrets?: Record<string, string>; onReveal: () => void; onCancel: () => void; onSave: (config: McpServerConfig) => void }) {
  const [draft, setDraft] = useState(config);
  const [transportType, setTransportType] = useState<McpTransport["type"]>(config.transport.type);
  const [transportDrafts, setTransportDrafts] = useState(() => createMcpTransportDrafts(config.transport));
  const [showSecrets, setShowSecrets] = useState(false);
  const transport = transportDrafts[transportType];
  const valid = draft.name.trim() && (transport.type === "stdio" ? transport.command.trim() : transport.url.trim());
  useEffect(() => {
    if (!revealedSecrets) return;
    setTransportDrafts((current) => ({
      stdio: { ...current.stdio, env: mergeMcpSecrets(current.stdio.env, revealedSecrets) },
      streamable_http: { ...current.streamable_http, headers: mergeMcpSecrets(current.streamable_http.headers, revealedSecrets) },
      sse: { ...current.sse, headers: mergeMcpSecrets(current.sse.headers, revealedSecrets) },
    }));
    setShowSecrets(true);
  }, [revealedSecrets]);
  const toggleSecrets = () => {
    if (showSecrets) setShowSecrets(false);
    else if (revealedSecrets) setShowSecrets(true);
    else onReveal();
  };
  return <form className="mcp-editor" onSubmit={(event) => {
    event.preventDefault();
    if (!valid) return;
    const id = draft.id || draft.name.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, "_").replace(/^_+|_+$/g, "") || `server_${clientId()}`;
    onSave({ ...draft, id, transport });
  }}>
    <fieldset className="mcp-transport"><legend>Transport</legend><div>{(["stdio", "streamable_http", "sse"] as const).map((type) => <button type="button" aria-pressed={transportType === type} className={transportType === type ? "active" : ""} key={type} onClick={() => setTransportType(type)}>{mcpTransportLabel({ type } as McpTransport)}</button>)}</div><p>{transportType === "stdio" ? "Launch a local MCP process and communicate over stdin/stdout." : transportType === "streamable_http" ? "Recommended remote transport. One MCP endpoint may return JSON or an SSE stream." : "Compatibility mode for older servers with an SSE endpoint and a separate POST endpoint."}</p></fieldset>
    <label>Name<input autoFocus value={draft.name} placeholder="GitHub" onChange={(event) => setDraft({ ...draft, name: event.target.value })}/></label>
    {draft.id && <label>Server ID<input value={draft.id} disabled/></label>}
    {transport.type === "stdio" ? <>
      <label>Command<input value={transport.command} placeholder="npx" onChange={(event) => setTransportDrafts((current) => ({ ...current, stdio: { ...current.stdio, command: event.target.value } }))}/></label>
      <label>Arguments<textarea rows={3} value={transport.args.join("\n")} placeholder={"-y\n@modelcontextprotocol/server-filesystem\n/path"} onChange={(event) => setTransportDrafts((current) => ({ ...current, stdio: { ...current.stdio, args: nonemptyLines(event.target.value) } }))}/><small>One argument per line. Spaces stay inside that argument.</small></label>
      <div className="mcp-secret-field"><div className="mcp-secret-heading"><span>Environment</span>{draft.id && <button type="button" className="icon-button" title={showSecrets ? "Hide sensitive environment values" : "Reveal sensitive environment values"} aria-label={showSecrets ? "Hide sensitive environment values" : "Reveal sensitive environment values"} disabled={revealPending} onClick={toggleSecrets}>{showSecrets ? <EyeOff size={14}/> : <Eye size={14}/>}</button>}</div><textarea aria-label="Environment" rows={3} value={mapLines(showSecrets ? transport.env : maskSensitiveMcpValues(transport.env))} placeholder="KEY=value" onChange={(event) => setTransportDrafts((current) => ({ ...current, stdio: { ...current.stdio, env: parseMapLines(event.target.value) } }))}/></div>
    </> : <>
      <label>{transport.type === "sse" ? "SSE URL" : "MCP endpoint URL"}<input value={transport.url} placeholder={transport.type === "sse" ? "https://example.com/sse" : "https://example.com/mcp"} onChange={(event) => setTransportDrafts((current) => ({ ...current, [transport.type]: { ...current[transport.type], url: event.target.value } }))}/></label>
      <div className="mcp-secret-field"><div className="mcp-secret-heading"><span>Headers</span>{draft.id && <button type="button" className="icon-button" title={showSecrets ? "Hide sensitive headers" : "Reveal sensitive headers"} aria-label={showSecrets ? "Hide sensitive headers" : "Reveal sensitive headers"} disabled={revealPending} onClick={toggleSecrets}>{showSecrets ? <EyeOff size={14}/> : <Eye size={14}/>}</button>}</div><textarea aria-label="Headers" rows={3} value={mapLines(showSecrets ? transport.headers : maskSensitiveMcpValues(transport.headers))} placeholder="Authorization=Bearer ${MCP_TOKEN}" onChange={(event) => setTransportDrafts((current) => ({ ...current, [transport.type]: { ...current[transport.type], headers: parseMapLines(event.target.value) } }))}/><small>One header per line as Name=value. Environment references use ${"${NAME}"}.</small></div>
    </>}
    <label>Request timeout (ms)<input type="number" min={1} value={draft.request_timeout_ms} onChange={(event) => setDraft({ ...draft, request_timeout_ms: Math.max(1, Number(event.target.value) || 1) })}/></label>
    <div className="mcp-editor-actions"><button type="button" className="secondary" disabled={pending} onClick={onCancel}>Cancel</button><button type="submit" className="primary" disabled={!valid || pending}>{pending ? <LoaderCircle size={14}/> : <Plug size={14}/>} Save and connect</button></div>
  </form>;
}

function nonemptyLines(value: string) { return value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean); }
function mcpEndpoint(config: McpServerConfig) { return config.transport.type === "stdio" ? config.transport.command : config.transport.url; }
function parseMapLines(value: string) { return Object.fromEntries(value.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).map((line) => { const index = line.indexOf("="); return index < 0 ? [line, ""] : [line.slice(0, index).trim(), line.slice(index + 1)]; }).filter(([key]) => key)); }
function mapLines(value: Record<string, string>) { return Object.entries(value).map(([key, item]) => `${key}=${item}`).join("\n"); }

function AppearancePanel({ panelRef, appearance, onChange, onClose }: { panelRef: MutableRefObject<HTMLElement | null>; appearance: Appearance; onChange: (appearance: Appearance) => void; onClose: () => void }) {
  const update = <K extends keyof Appearance>(key: K, value: Appearance[K]) => onChange({ ...appearance, [key]: value });
  const descriptionId = "appearance-panel-description";
  return <>
    <div className="appearance-dismiss" aria-hidden="true" onClick={onClose}/>
    <section id="appearance-panel" ref={panelRef} className="appearance-panel" role="dialog" aria-modal="false" aria-label="Appearance settings" aria-describedby={descriptionId} tabIndex={-1} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); } }}>
      <header><div><span className="eyebrow">APPEARANCE</span><h2>Reading preferences</h2><p id={descriptionId}>Adjust theme, language fonts, and message text size for this browser.</p></div><button type="button" className="icon-button" aria-label="Close appearance settings" onClick={onClose}><X size={16}/></button></header>
      <fieldset><legend>Theme</legend><div className="segmented-control">{(["dark", "light"] as const).map((theme) => <button type="button" title={`Use ${theme} theme`} className={appearance.theme === theme ? "active" : ""} aria-pressed={appearance.theme === theme} key={theme} onClick={() => update("theme", theme)}>{theme === "dark" ? "Dark" : "Light"}</button>)}</div></fieldset>
      <fieldset className="appearance-role-fonts"><legend>User</legend><div className="appearance-font-selects"><label><span>汉语字体</span><select value={appearance.userChineseFont} aria-label="User Chinese font" onChange={(event) => update("userChineseFont", event.target.value as Appearance["userChineseFont"])}><option value="system">系统</option><option value="heiti">黑体</option><option value="kaiti">楷体</option><option value="songti">宋体</option></select></label><label><span>其他语言字体</span><select value={appearance.userFont} aria-label="User other language font" onChange={(event) => update("userFont", event.target.value as Appearance["userFont"])}><option value="system">System</option><option value="serif">Serif</option><option value="mono">Mono</option></select></label></div><label className="appearance-bold-option"><input type="checkbox" checked={appearance.userBold} onChange={(event) => update("userBold", event.target.checked)}/><span>粗体</span></label></fieldset>
      <fieldset className="appearance-role-fonts"><legend>Agent</legend><div className="appearance-font-selects"><label><span>汉语字体</span><select value={appearance.agentChineseFont} aria-label="Agent Chinese font" onChange={(event) => update("agentChineseFont", event.target.value as Appearance["agentChineseFont"])}><option value="system">系统</option><option value="heiti">黑体</option><option value="kaiti">楷体</option><option value="songti">宋体</option></select></label><label><span>其他语言字体</span><select value={appearance.agentFont} aria-label="Agent other language font" onChange={(event) => update("agentFont", event.target.value as Appearance["agentFont"])}><option value="system">System</option><option value="serif">Serif</option><option value="mono">Mono</option></select></label></div><label className="appearance-bold-option"><input type="checkbox" checked={appearance.agentBold} onChange={(event) => update("agentBold", event.target.checked)}/><span>粗体</span></label></fieldset>
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
  const toolGenLabel = toolGenPending ? "Starting ToolGen" : toolGenBlocked ? "ToolGen busy" : "ToolGen";
  const toolGenTitle = toolGenPending ? "ToolGen is starting for this task..." : toolGenBlocked ? "Another ToolGen task is already running in this session" : "Extract reusable tool from this task";
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
    {onToolGen && !cancelled && <button className={`completion-toolgen ${toolGenPending ? "sending" : ""}`} type="button" title={toolGenTitle} aria-label={toolGenTitle} aria-busy={toolGenPending || undefined} disabled={toolGenPending || toolGenBlocked} onClick={onToolGen}>{toolGenPending ? <LoaderCircle size={12}/> : <Wrench size={12}/>}<span aria-live="polite">{toolGenLabel}</span></button>}
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

function RuntimePanel({ panelRef, server, session, pendingKeys, credentialPending, revealedApiKey, onUpdate, onApiKeyUpdate, onApiKeyReveal }: {
  panelRef: MutableRefObject<HTMLElement | null>;
  server: Snapshot["server"] | null;
  session?: Session;
  pendingKeys: Set<string>;
  credentialPending: boolean;
  revealedApiKey?: string;
  onUpdate: (key: string, value: string) => void;
  onApiKeyUpdate: (apiKey: string) => void;
  onApiKeyReveal: () => void;
}) {
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [apiKeyDraft, setApiKeyDraft] = useState("");
  const [showApiKey, setShowApiKey] = useState(false);
  const autoRevealSessionRef = useRef("");
  const previousCredentialPending = useRef(credentialPending);
  const keyConfigured = session?.runtime_profile?.api_key_configured ?? false;
  const sessionWorking = session?.state === "working";
  const runtimeOptions = useMemo(
    () => sessionRuntimeOptions(session?.runtime_profile, server?.runtime_options ?? []),
    [server?.runtime_options, session?.runtime_profile],
  );
  useEffect(() => setDrafts({}), [session?.session_id]);
  useEffect(() => setDrafts((current) => reconcileRuntimeDrafts(current, runtimeOptions)), [runtimeOptions]);
  useEffect(() => {
    setApiKeyDraft("");
    setShowApiKey(false);
    autoRevealSessionRef.current = "";
  }, [session?.session_id]);
  useEffect(() => {
    if (revealedApiKey === undefined) return;
    setApiKeyDraft(revealedApiKey);
    setShowApiKey(false);
  }, [revealedApiKey]);
  useEffect(() => {
    if (previousCredentialPending.current && !credentialPending && revealedApiKey === undefined) {
      setApiKeyDraft("");
      setShowApiKey(false);
    }
    previousCredentialPending.current = credentialPending;
  }, [credentialPending, revealedApiKey]);
  useEffect(() => {
    const sessionId = session?.session_id;
    if (!shouldAutoRevealSessionApiKey({ sessionId, configured: keyConfigured, revealedApiKey, pending: credentialPending, requestedSessionId: autoRevealSessionRef.current })) return;
    if (!sessionId) return;
    autoRevealSessionRef.current = sessionId;
    onApiKeyReveal();
  }, [credentialPending, keyConfigured, onApiKeyReveal, revealedApiKey, session?.session_id]);
  if (!server) return <section id="runtime-panel" ref={panelRef} className="runtime-card" tabIndex={-1}><Cpu size={16}/><span>Loading runtime settings…</span></section>;
  const pendingRuntimeLabel = pendingKeys.size ? `Applying runtime setting${pendingKeys.size === 1 ? "" : "s"}: ${Array.from(pendingKeys).map(runtimeOptionLabel).join(", ")}` : "";
  const bindLabel = `${server.bind_host || "127.0.0.1"}:${server.port}`;
  const apiKeyDirty = revealedApiKey === undefined ? apiKeyDraft.length > 0 : apiKeyDraft !== revealedApiKey;
  const canSaveApiKey = !!session && apiKeyDirty && !credentialPending && !sessionWorking;
  const toggleApiKey = () => {
    if (showApiKey) setShowApiKey(false);
    else if (revealedApiKey !== undefined || !keyConfigured || apiKeyDraft.length > 0) setShowApiKey(true);
    else onApiKeyReveal();
  };
  return <section id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings" tabIndex={-1}><div className="runtime-summary"><Cpu size={16}/><span>Timem {server.version}</span><span>topic protocol v{server.protocol_version}</span><span><FolderOpen size={14}/>{bindLabel}</span>{server.public_access && <span>public · token required</span>}</div>
    <div className="session-credential-settings">
      <div className="session-credential-heading"><KeyRound size={15}/><div><strong>Session API key</strong><small>{session ? session.display_name : "Create or select a session first"}</small></div></div>
      <div className="session-credential-control"><div className="secret-input"><input type={showApiKey ? "text" : "password"} value={apiKeyDraft} autoComplete="new-password" spellCheck={false} aria-label="API key for current session" placeholder={credentialPending && keyConfigured ? "Loading API key…" : "Enter API key"} disabled={!session || credentialPending || sessionWorking} onChange={(event) => setApiKeyDraft(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing && canSaveApiKey) { event.preventDefault(); onApiKeyUpdate(apiKeyDraft); } }}/><button type="button" title={showApiKey ? "Hide API key" : "Show API key"} aria-label={showApiKey ? "Hide API key" : "Show API key"} disabled={!session || credentialPending || sessionWorking} onClick={toggleApiKey}>{showApiKey ? <EyeOff size={15}/> : <Eye size={15}/>}</button></div><button type="button" className="primary compact" disabled={!canSaveApiKey} onClick={() => onApiKeyUpdate(apiKeyDraft)}>{credentialPending ? "Working…" : "Save key"}</button></div>
      {sessionWorking && <small className="session-credential-note">Finish or stop the active task before changing credentials.</small>}
    </div>
    <p>{session ? `Runtime settings for ${session.display_name}. Changes apply only to this Session.` : "Create or select a Session to configure its runtime."}</p><div className="runtime-options">{runtimeOptions.map((option) => {
    const value = drafts[option.key] ?? option.value;
    const pending = pendingKeys.has(option.key);
    const dirty = value !== option.value;
    const optionLabel = runtimeOptionLabel(option.key);
    const inputLabel = `${optionLabel} current value`;
    const applyLabel = pending ? `Applying ${optionLabel}` : dirty ? `Apply ${optionLabel}` : `${optionLabel} has no changes`;
    const resetDraft = () => setDrafts((current) => { const { [option.key]: _removed, ...rest } = current; return rest; });
    const options = runtimeSelectOptions(option.key);
    return <label key={option.key}><span>{optionLabel}</span><div>{options ? <select value={value} title={inputLabel} aria-label={inputLabel} disabled={pending} onChange={(event) => setDrafts((current) => ({ ...current, [option.key]: event.target.value }))} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing && dirty && !pending) { event.preventDefault(); onUpdate(option.key, value); } if (event.key === "Escape" && dirty) { event.preventDefault(); resetDraft(); } }}>{options.map((choice) => <option value={choice} key={choice}>{choice === "unlimited" ? "Unlimited" : choice}</option>)}</select> : <input value={value} title={inputLabel} aria-label={inputLabel} disabled={pending} onChange={(event) => setDrafts((current) => ({ ...current, [option.key]: event.target.value }))} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing && dirty && !pending) { event.preventDefault(); onUpdate(option.key, value); } if (event.key === "Escape" && dirty) { event.preventDefault(); resetDraft(); } }}/>} {dirty && <button type="button" className="secondary compact runtime-reset" title={`Reset ${optionLabel} to current value`} aria-label={`Reset ${optionLabel} to current value`} disabled={pending} onClick={resetDraft}>Reset</button>}<button type="button" className="secondary compact" title={applyLabel} aria-label={applyLabel} disabled={pending || !dirty} onClick={() => onUpdate(option.key, value)}>{pending ? "Applying…" : "Apply"}</button></div></label>;
  })}</div>{(pendingRuntimeLabel || credentialPending) && <p className="runtime-pending-status" role="status" aria-live="polite">{credentialPending ? "Saving the Session API key…" : pendingRuntimeLabel}</p>}</section>;
}

function runtimeSelectOptions(key: string): readonly string[] | null {
  switch (key) {
    case "TIMEM_API_PROTOCOL":
      return ["openai-compatible", "openai-responses", "anthropic"];
    case "TIMEM_RESPONSE_PROTOCOL":
      return ["xml", "json", "markdown"];
    case "TIMEM_BASH_APPROVAL":
      return ["approve", "ask"];
    case "TIMEM_WORK_INSTRUCTIONS":
      return ["silent", "ask", "off"];
    case "TIMEM_MAX_ROUNDS":
      return ["50", "200", "500", "unlimited"];
    case "TIMEM_ENABLE_THINKING":
    case "TIMEM_STREAM":
      return ["true", "false"];
    default:
      return null;
  }
}

const SESSION_RUNTIME_FIELDS = [
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
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss create session" onClick={closeIfIdle}><section className="decision-modal session-modal" role="dialog" aria-modal="true" aria-label="Create session" aria-describedby={describedBy} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeIfIdle(); } }}><div className="modal-titlebar"><div><span className="eyebrow">NEW SESSION</span><h2>Start a session</h2></div><button type="button" className="icon-button" title="Close create session" aria-label="Close create session" disabled={creating} onClick={closeIfIdle}><X size={16}/></button></div><p id={descriptionId}>Choose a workspace and optional runtime overrides for this session.</p>{creating && <p id={statusId} className="mem-validation" role="status" aria-live="polite">Creating session…</p>}<div className="session-modal-scroll"><label>Display name<input autoFocus value={displayName} placeholder="Optional name" disabled={creating} onChange={(event) => setDisplayName(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); submit(); } }}/></label><label>Workspace<select value={workspaceDir} disabled={creating || workspaces.length === 0} onChange={(event) => setWorkspaceDir(event.target.value)}>{workspaces.length === 0 ? <option value="">No workspace available</option> : workspaces.map((workspace) => <option value={workspace} key={workspace} title={workspace}>{tailPath(workspace, 64)}</option>)}</select></label>{workspaces.length === 0 && <p className="mem-hint">No workspace is available from the runtime snapshot. Reconnect Timem Web or check the host workspace configuration.</p>}<details className="session-runtime-overrides"><summary>Runtime environment</summary><div className="session-runtime-grid">{SESSION_RUNTIME_FIELDS.map(([key, label, kind]) => <label key={key}><span>{label}<small>{key}</small></span><div className="session-runtime-control">{kind === "api_protocol" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "default"}</option><option value="openai-compatible">openai-compatible</option><option value="openai-responses">openai-responses</option><option value="anthropic">anthropic</option></select> : kind === "response_protocol" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "xml"}</option><option value="xml">xml</option><option value="json">json</option><option value="markdown">markdown</option></select> : kind === "bash_approval" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "ask"}</option><option value="ask">ask</option><option value="approve">approve</option></select> : kind === "work_instructions" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "silent"}</option><option value="silent">silent</option><option value="ask">ask</option><option value="off">off</option></select> : kind === "boolean" ? <select value={env[key] ?? ""} disabled={creating} onChange={(event) => updateEnv(key, event.target.value)}><option value="">Inherit · {runtimeDefaults[key] ?? "false"}</option><option value="true">true</option><option value="false">false</option></select> : <input type={kind} value={env[key] ?? ""} min={kind === "number" ? 1 : undefined} disabled={creating} autoComplete={kind === "password" ? "new-password" : undefined} placeholder={kind === "password" ? "Optional session-only key" : `Inherit · ${runtimeDefaults[key] ?? "default"}`} onChange={(event) => updateEnv(key, event.target.value)}/>} {env[key] !== undefined && <button type="button" className="session-runtime-reset" title={`Reset ${label} to inherited value`} aria-label={`Reset ${label} to inherited value`} disabled={creating} onClick={() => resetEnv(key)}>Reset</button>}</div></label>)}</div></details></div><div className="decision-actions"><button type="button" className="secondary" disabled={creating} onClick={closeIfIdle}>Cancel</button><button type="button" className={`primary ${creating ? "sending" : ""}`} disabled={!canCreateSession} onClick={submit}>{creating ? <LoaderCircle size={16}/> : <Plus size={16}/>} {creating ? "Creating…" : "Create session"}</button></div></section></div>;
}

function SessionDeleteDialog({ session, pending, onClose, onConfirm }: {
  session: Session;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const descriptionId = "delete-session-dialog-description";
  const statusId = "delete-session-dialog-status";
  const closeIfIdle = () => { if (!pending) onClose(); };
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss delete session confirmation" onClick={closeIfIdle}><section className="decision-modal session-delete-dialog" role="dialog" aria-modal="true" aria-label={`Delete ${session.display_name}`} aria-describedby={pending ? `${descriptionId} ${statusId}` : descriptionId} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeIfIdle(); } }}><div className="modal-titlebar"><div><span className="eyebrow">DELETE SESSION</span><h2>Delete “{session.display_name}”?</h2></div><button type="button" className="icon-button" title="Close delete session confirmation" aria-label="Close delete session confirmation" disabled={pending} onClick={closeIfIdle}><X size={16}/></button></div><p id={descriptionId}>This permanently deletes the session, its stored task history, settings, and session tools. {session.state === "working" && "Current work will be stopped."} This cannot be undone.</p>{pending && <p id={statusId} className="session-delete-status" role="status" aria-live="polite">Stopping workers and deleting session…</p>}<div className="decision-actions"><button type="button" className="secondary" disabled={pending} onClick={closeIfIdle}>Cancel</button><button type="button" className={`danger ${pending ? "sending" : ""}`} disabled={pending} onClick={onConfirm}>{pending ? <LoaderCircle size={16}/> : <Trash2 size={15}/>} {pending ? "Deleting…" : "Delete session"}</button></div></section></div>;
}

function ChatMessageDeleteDialog({ candidate, pending, onClose, onConfirm }: {
  candidate: ChatMessageDeleteCandidate;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const descriptionId = "chat-message-delete-description";
  const statusId = "chat-message-delete-status";
  const closeIfIdle = () => { if (!pending) onClose(); };
  const roleLabel = candidate.role === "user" ? "user message" : "assistant answer";
  const normalizedPreview = candidate.preview.trim().replace(/\s+/g, " ");
  const preview = normalizedPreview.slice(0, 180);
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss delete message confirmation" onClick={closeIfIdle}>
    <section className="decision-modal chat-message-delete-dialog" role="dialog" aria-modal="true" aria-label={`Delete ${roleLabel}`} aria-describedby={pending ? `${descriptionId} ${statusId}` : descriptionId} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeIfIdle(); } }}>
      <div className="modal-titlebar"><div><span className="eyebrow">DELETE MESSAGE</span><h2>Delete this {roleLabel}?</h2></div><button type="button" className="icon-button" title="Close delete message confirmation" aria-label="Close delete message confirmation" disabled={pending} onClick={closeIfIdle}><X size={16}/></button></div>
      <p id={descriptionId}>This permanently removes the content from the conversation and its raw chat log. Runtime activity records for the task are retained. This cannot be undone.</p>
      {preview && <blockquote className="chat-message-delete-preview">{preview}{normalizedPreview.length > 180 ? "…" : ""}</blockquote>}
      {pending && <p id={statusId} className="session-delete-status" role="status" aria-live="polite">Deleting message and rewriting raw chat history…</p>}
      <div className="decision-actions"><button type="button" className="secondary" disabled={pending} onClick={closeIfIdle}>Cancel</button><button type="button" className={`danger ${pending ? "sending" : ""}`} disabled={pending} onClick={onConfirm}>{pending ? <LoaderCircle size={16}/> : <Trash2 size={15}/>} {pending ? "Deleting…" : "Delete message"}</button></div>
    </section>
  </div>;
}

function ToolGenDialog({ pending, onClose, onSubmit }: { pending: boolean; onClose: () => void; onSubmit: (text: string) => void }) {
  const [instruction, setInstruction] = useState("");
  const closeIfIdle = () => { if (!pending) onClose(); };
  const submit = () => { if (!pending) onSubmit(instruction.trim()); };
  const descriptionId = "toolgen-dialog-description";
  const statusId = "toolgen-dialog-status";
  const describedBy = pending ? `${descriptionId} ${statusId}` : descriptionId;
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss ToolGen dialog" onClick={closeIfIdle}><section className="decision-modal toolgen-dialog" role="dialog" aria-modal="true" aria-label="Generate reusable tool" aria-describedby={describedBy} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeIfIdle(); } }}><div className="modal-titlebar"><div><span className="eyebrow">TOOLGEN</span><h2>Extract reusable tool</h2></div><button type="button" className="icon-button" title="Close ToolGen dialog" aria-label="Close ToolGen dialog" disabled={pending} onClick={closeIfIdle}><X size={16}/></button></div><p id={descriptionId}>Timem will preserve reusable work from the completed task as one or more standalone script tools. Add optional guidance below.</p>{pending && <p id={statusId} className="toolgen-dialog-status" role="status" aria-live="polite">Starting ToolGen and opening a generating-tools task…</p>}<label>Additional guidance<textarea autoFocus value={instruction} disabled={pending} placeholder="Optional: preferred interface, language, scope, or reusable workflow…" onChange={(event) => setInstruction(event.target.value)} onKeyDown={(event) => { if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); submit(); } }}/><small className="toolgen-dialog-hint">Cmd/Ctrl+Enter to generate; Escape closes before it starts.</small></label><div className="decision-actions"><button type="button" className="secondary" disabled={pending} onClick={closeIfIdle}>Cancel</button><button type="button" className={`primary ${pending ? "sending" : ""}`} disabled={pending} onClick={submit}>{pending ? <LoaderCircle size={16}/> : <Wrench size={15}/>} {pending ? "Starting…" : "Generate tool"}</button></div></section></div>;
}

function MemSwitchDialog({ current, pending, onClose, onSwitch }: {
  current: string;
  pending: boolean;
  onClose: () => void;
  onSwitch: (path: string) => void;
}) {
  const [path, setPath] = useState(current);
  const cleaned = path.trim();
  const invalid = !cleaned;
  const validationText = pending
    ? "Switching mem directory…"
    : invalid
      ? "Enter an absolute mem directory path on the Timem host."
      : cleaned === current
        ? "This is the current mem directory."
        : "";
  const closeIfIdle = () => { if (!pending) onClose(); };
  const descriptionId = "mem-switch-dialog-description";
  const statusId = "mem-switch-dialog-status";
  const describedBy = validationText ? `${descriptionId} ${statusId}` : descriptionId;
  return <div className="modal-backdrop" role="presentation" aria-label="Dismiss mem switch" onClick={closeIfIdle}><section className="decision-modal session-modal mem-switch-modal" role="dialog" aria-modal="true" aria-label="Switch memory directory" aria-describedby={describedBy} onClick={(event) => event.stopPropagation()} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeIfIdle(); } }}><div className="modal-titlebar"><div><span className="eyebrow">MEM DIRECTORY</span><h2>Switch mem directory</h2></div><button type="button" className="icon-button" title="Close mem switch" aria-label="Close mem switch" disabled={pending} onClick={closeIfIdle}><X size={16}/></button></div><p id={descriptionId}>Switching mem stops current workers, swaps out current sessions, then loads sessions from the selected directory.</p><label>Mem directory<input autoFocus value={path} disabled={pending} placeholder="/absolute/path/to/.test_mem" onChange={(event) => setPath(event.target.value)} onKeyDown={(event) => {
    if (event.key === "Enter" && !event.nativeEvent.isComposing && !pending && !invalid) { event.preventDefault(); onSwitch(cleaned); }
  }}/></label><p className="mem-hint">Use an absolute directory path on the machine running Timem Web.</p>{validationText && <p id={statusId} className="mem-validation" role="status" aria-live="polite">{validationText}</p>}<div className="decision-actions"><button type="button" className="secondary" disabled={pending} onClick={closeIfIdle}>Cancel</button><button type="button" className={`primary ${pending ? "sending" : ""}`} disabled={pending || invalid || cleaned === current} title={validationText || "Switch mem"} aria-label={validationText || "Switch mem"} onClick={() => onSwitch(cleaned)}>{pending && <LoaderCircle size={16}/>} {pending ? "Switching…" : "Switch mem"}</button></div></section></div>;
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

function InlineDecision({ decision, pending, locked, position, total, onReply }: { decision: Decision; pending: boolean; locked: boolean; position: number; total: number; onReply: (decision: "accept" | "decline" | "always_allow") => void }) {
  const disabled = pending || locked;
  const status = pending ? "Sending decision…" : locked ? "Session interaction is temporarily locked." : "";
  const canAlwaysAllow = decision.event.topic.name === "core.user.approval.request";
  const denyLabel = pending ? "Waiting for the current reply to finish" : locked ? "Decision is locked while the session changes" : "Deny this runtime request";
  const allowLabel = pending ? "Sending decision" : locked ? "Decision is locked while the session changes" : "Allow this runtime request";
  const alwaysAllowLabel = pending ? "Sending decision" : locked ? "Decision is locked while the session changes" : "Allow and stop asking for this session";
  return <section className="inline-decision" aria-label="Decision required" aria-busy={pending}>
    <div className="inline-decision-heading"><span className="eyebrow">RUNTIME REQUEST{total > 1 ? ` · ${position} OF ${total}` : ""}</span><h2>{decision.title}</h2></div>
    <pre>{decision.detail}</pre>
    {status && <span className="inline-decision-status" role="status" aria-live="polite">{status}</span>}
    <div className="decision-actions"><button type="button" className="secondary" title={denyLabel} aria-label={denyLabel} disabled={disabled} onClick={() => onReply("decline")}>Deny</button><button type="button" className="primary" title={allowLabel} aria-label={allowLabel} disabled={disabled} onClick={() => onReply("accept")}>Allow</button>{canAlwaysAllow && <button type="button" className="primary always-allow" title={alwaysAllowLabel} aria-label={alwaysAllowLabel} disabled={disabled} onClick={() => onReply("always_allow")}>Always Allow</button>}</div>
  </section>;
}

export default function Root() { return <TimemApp/>; }

import { createRoot } from "react-dom/client";
createRoot(document.getElementById("root")!).render(<Root/>);
