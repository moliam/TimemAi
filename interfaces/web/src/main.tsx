import {
  AssistantRuntimeProvider,
  ThreadMessageLike,
  ThreadPrimitive,
  useExternalStoreRuntime,
} from "@assistant-ui/react";
import {
  closestCenter,
  DndContext,
  DragEndEvent,
  DragOverlay,
  DragOverEvent,
  KeyboardSensor,
  PointerSensor,
  useDroppable,
  useSensor,
  useSensors,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import {
  ArrowDown,
  ArrowDownToLine,
  ArrowLeftRight,
  BriefcaseBusiness,
  BookText,
  Check,
  CheckCheck,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  ChevronUp,
  CircleStop,
  Clock3,
  Copy,
  CornerUpLeft,
  Cpu,
  Database,
  Eye,
  EyeOff,
  Folder,
  FolderOpen,
  FolderPlus,
  Gauge,
  GripVertical,
  KeyRound,
  LoaderCircle,
  Maximize2,
  Menu,
  Minimize2,
  Palette,
  Paperclip,
  Pencil,
  Plug,
  Plus,
  RefreshCw,
  Search,
  Send,
  Settings,
  Sparkles,
  Star,
  Terminal,
  TriangleAlert,
  Trash2,
  Wrench,
  X,
} from "lucide-react";
import {
  createContext,
  CSSProperties,
  Dispatch,
  memo,
  MutableRefObject,
  ReactNode,
  SetStateAction,
  useCallback,
  useContext,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useId,
} from "react";
import { createPortal } from "react-dom";
import { Appearance, applyAppearance, loadAppearance } from "./appearance";
import { loadToolGenEnabled, saveToolGenEnabled } from "./beta_features";
import {
  Activity,
  ChatFavorite,
  ChatLibraryCapacity,
  ChatMessage,
  ChatSearchHit,
  ClientCommand,
  clientId,
  CommandWithId,
  Decision,
  McpServerConfig,
  McpServerReport,
  McpTransport,
  MemTemporaryItem,
  ModelEndpoint,
  Session,
  Snapshot,
  ToolDetail,
  ToolSummary,
  WebTurn,
  WebTurnEvent,
  WireEvent,
  WorkerRole,
  WorkerRoleGroup,
  WorkerRoleLibrary,
  SessionGroup,
} from "./protocol";
import {
  applyWorkerRoleMutation,
  isOptimisticWorkerRoleMutation,
  replayWorkerRoleMutations,
  WorkerRoleMutation,
} from "./worker_roles_ui";
import {
  adjacentUserMessageIndex,
  canScrollInDirection,
  isNearScrollBottom,
  preservePrependScrollTop,
  restoreSessionScrollTop,
  scrollEdgeFades,
  ScrollMetrics,
  SessionScrollPosition,
  UserMessageNavigationDirection,
  wheelDeltaPixels,
} from "./scroll";
import { newestInterimAnswersFirst } from "./interim_answers";
import {
  applyTurnProjection,
  activeModelRetryStatus,
  activityFromTopic,
  applySessionRuntimeProfile,
  appendActivityToCurrentTurn,
  appendTurnEvent,
  applyChatMessageDeleted,
  applyCoreTopicToSession,
  attachTurnCompletion,
  boundSessionHistory,
  clearDecisionsForWorker,
  coalesceActionLifecycle,
  compareTurnTimelineItems,
  composerPrimaryAction,
  composerSendDecision,
  turnCommandId,
  turnInteractionPhase,
  decisionKey,
  decisionsFromSessions,
  draftForSession,
  enqueueDecision,
  finishSessionDraftSubmission,
  finishTurn,
  groupDecisionsBySessionTurn,
  manualToolGenCommand,
  normalizeCopiedUserMessageText,
  prependHistoryRecords,
  pruneSessionDrafts,
  pruneSessionSubmissionLocks,
  releaseSessionDraftSubmission,
  removePendingAttachment,
  requestDecision,
  reserveSessionDraftSubmission,
  resolveActiveSessionId,
  runtimeConnectionLabel,
  sessionCacheHitPercent,
  sessionCancellationApplies,
  shouldRenderTurnWorkFrame,
  sessionContextUsage,
  sessionCreateDecision,
  sessionInteractionLockReason as sessionInteractionLockReasonForState,
  sessionRenameDecision,
  sessionTurnKey,
  sessionVisuallyWorking,
  sessionWorkerTreeRows,
  setSessionDraft,
  tailPath,
  toolActivityDisplayName,
  toolDisplayName,
  turnElapsedMs,
  turnLiveUsage,
  turnShouldRenderInTimeline,
  turnTimelinePlacement,
  updateSessionWorkerState,
  visibleRuntimeRestartMarkers,
  upsertSession,
  upsertTurn,
} from "./view_model";
import {
  extractMarkdownOutline,
  finalAnswerNeedsOutline,
  markdownHeadingId,
  markdownFloatingNavigationLayout,
  markdownOutlineActiveId,
  markdownOutlineAnimationPosition,
  markdownOutlineRailScrollTop,
  MARKDOWN_OUTLINE_START_ID,
  markdownOutlineTargetScrollTop,
  MarkdownOutlineItem,
} from "./markdown_outline";
import {
  createMcpTransportDrafts,
  mcpTransportLabel,
  mergeMcpSecrets,
} from "./mcp";
import {
  reconcileRuntimeDrafts,
  runtimeOptionLabel,
  sessionRuntimeOptions,
  shouldAutoRevealSessionApiKey,
  updateRevealedSessionApiKeys,
} from "./runtime_settings";
import {
  commandSessionId,
  isModelSubmissionCommand,
  modelDisplayName,
  modelServiceIssue,
  NO_MODEL_ENDPOINTS_ISSUE,
  UNCONFIGURED_MODEL_LABEL,
} from "./model_service_ui";
import {
  endpointDraftValid,
  endpointMatchesProfile,
  endpointNameForProfile,
  MODEL_CONTEXT_WINDOW_OPTIONS,
  MODEL_OUTPUT_TOKEN_OPTIONS,
  ModelEndpointDraft,
} from "./model_endpoints";
import { createFrameEventQueue } from "./frame_event_queue";
import { formatTokens } from "./token_format";
import {
  summarizeConsecutiveToolActivities,
  ToolActivitySummary,
} from "./activity_groups";
import {
  applyQueuedMessagesAck,
  claimQueuedMessage,
  clearQueuedMessagesPause,
  COLLAPSED_QUEUE_LIMIT,
  loadQueuedMessages,
  loadQueuedMessagesPause,
  QueuedMessage,
  queuedMessageKey,
  QueuedMessagesPauseSource,
  QueuedMessagesPauseState,
  queuedMessagesPauseSessionId,
  queuedMessagesPauseStorageKey,
  queuedMessagesStorageKey,
  releaseQueuedMessageClaim,
  releaseSessionQueuedMessageClaims,
  removeQueuedMessage,
  reorderQueuedMessages,
  reservedQueuedAttachmentIds,
  saveQueuedMessages,
  saveQueuedMessagesPause,
  selectQueuedDispatches,
  shouldDirectManualMessage,
  stopQueuedAutoSend,
  unclaimedQueuedMessages,
} from "./queued_messages";
import {
  acceptOutboxCommand,
  addCommandToOutbox,
  commandMayPersist,
  commandNeedsReliableDelivery,
  CommandOutboxItem,
  commandOutboxStorageKey,
  finishOutboxCommand,
  loadCommandOutbox,
  orderCommandOutbox,
  pendingTurnCancelTargetCommandIds,
  pendingTurnSubmitCommandIds,
  reliableStorageScope,
  removeCommandOutboxItem,
  saveCommandOutboxItem,
} from "./command_outbox";
import { classifyEventSequence } from "./event_cursor";
import {
  enablesSemanticDelivery,
  shouldReduceTopLevelWireEvent,
} from "./wire_delivery";
import { clipboardImageFiles } from "./clipboard_images";
import {
  humanizeToolStatus,
  isToolActivityRunning,
  TOOL_STATUS_RUNNING,
} from "./tool_status";
import { MarkdownContent } from "./markdown_render";
import { BrowserPerformanceTrace } from "./performance_trace";
import { createFrameTask, FrameTask } from "./frame_task";
import { reconcileSessionTimelineCache } from "./session_timeline_cache";
import { requestTimelineNavigationWork } from "./timeline_navigation_work";
import { useTimedClipboardCopy } from "./clipboard_copy";
import "./styles.css";
import "highlight.js/styles/github-dark.css";
import "katex/dist/katex.min.css";

const STORED_HISTORY_PAGE_SIZE = 200;
const MAX_MOUNTED_SESSION_TIMELINES = 2;
const TOKEN_STORAGE_KEY = "timem-web-access-token";
const EMPTY_CHAT_MESSAGES: ChatMessage[] = [];
const SESSION_API_KEY_SAVE_TIMEOUT_MS = 20_000;
const TURN_CANCEL_UI_TIMEOUT_MS = 15_000;
type PendingSessionApiKeyCommand = {
  sessionId: string;
  timeoutId: number;
};
type MemSwitchCandidate = {
  path: string;
  runningSessionCount: number;
};
function memSwitchRunningSessionCount(sessions: Session[]) {
  return sessions.filter(
    (session) =>
      session.state === "working" ||
      !!session.active_turn_id ||
      session.workers.some((worker) => worker.state === "working"),
  ).length;
}
function shellQuoteCommandArgument(value: string) {
  return `'${value.replaceAll("'", `'"'"'`)}'`;
}
type ChatMessageDeleteCandidate = {
  sessionId: string;
  turnId: string;
  role: "user" | "assistant";
  roleIndex: number;
  preview: string;
};

function chatMessageDeleteKey(
  candidate: Pick<
    ChatMessageDeleteCandidate,
    "sessionId" | "turnId" | "role" | "roleIndex"
  >,
) {
  return `${candidate.sessionId}\u0000${candidate.turnId}\u0000${candidate.role}\u0000${candidate.roleIndex}`;
}

const FOCUSABLE_DIALOG_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), summary, [tabindex]:not([tabindex="-1"])';

function useDialogFocusTrap() {
  useEffect(() => {
    const containFocus = (event: KeyboardEvent) => {
      if (event.key !== "Tab") return;
      const activeElement = document.activeElement;
      const dialog =
        activeElement instanceof HTMLElement
          ? activeElement.closest<HTMLElement>(
              '[role="dialog"][aria-modal="true"]',
            )
          : null;
      if (!dialog || !dialog.contains(document.activeElement)) return;
      const focusable = Array.from(
        dialog.querySelectorAll<HTMLElement>(FOCUSABLE_DIALOG_SELECTOR),
      ).filter((element) => element.getClientRects().length > 0);
      if (focusable.length === 0) {
        event.preventDefault();
        dialog.focus({ preventScroll: true });
        return;
      }
      const currentIndex = focusable.indexOf(
        document.activeElement as HTMLElement,
      );
      const nextIndex = event.shiftKey
        ? currentIndex <= 0
          ? focusable.length - 1
          : currentIndex - 1
        : currentIndex === focusable.length - 1
          ? 0
          : currentIndex + 1;
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
    try {
      window.sessionStorage.setItem(TOKEN_STORAGE_KEY, query);
    } catch {
      /* Keep the in-memory token. */
    }
    return query;
  }
  try {
    return window.sessionStorage.getItem(TOKEN_STORAGE_KEY) ?? "";
  } catch {
    return "";
  }
}

const accessToken = initialAccessToken();

function isLiveTurnProgressEvent(event: WireEvent): boolean {
  if (event.type === "semantic_event")
    return isLiveTurnProgressEvent(event.event);
  if (event.type !== "core_topic") return false;
  const topicName = event.event.topic?.name;
  if (topicName === "core.model.response")
    return event.event.payload?.continue_work === true;
  if (topicName === "core.sub_answer") return true;
  return topicName === "core.action" && event.event.payload?.event === "start";
}

function queryToken() {
  return accessToken;
}

function makeMessage(
  role: ChatMessage["role"],
  text: string,
  id?: string,
): ChatMessage {
  return {
    id: id ?? `${role}-${clientId()}`,
    role,
    text,
    created_at_ms: Date.now(),
  };
}

type SortableSessionRenderState = {
  setNodeRef: (node: HTMLElement | null) => void;
  style: CSSProperties;
  attributes: ReturnType<typeof useSortable>["attributes"];
  listeners: ReturnType<typeof useSortable>["listeners"];
  isDragging: boolean;
};

function SortableSession({
  id,
  disabled,
  children,
}: {
  id: string;
  disabled: boolean;
  children: (state: SortableSessionRenderState) => ReactNode;
}) {
  const sortable = useSortable({
    id: `session:${id}`,
    disabled,
    transition: { duration: 180, easing: "cubic-bezier(.2, .8, .2, 1)" },
  });
  return children({
    setNodeRef: sortable.setNodeRef,
    style: {
      transform: CSS.Transform.toString(sortable.transform),
      transition: sortable.transition,
      zIndex: sortable.isDragging ? 4 : undefined,
    },
    attributes: sortable.attributes,
    listeners: sortable.listeners,
    isDragging: sortable.isDragging,
  });
}

function SessionDropGroup({
  id,
  sessionIds,
  className,
  children,
}: {
  id: string;
  sessionIds: string[];
  className: string;
  children: ReactNode;
}) {
  const droppable = useDroppable({ id: `session-group:${id}` });
  return (
    <section
      ref={droppable.setNodeRef}
      className={`${className} ${droppable.isOver ? "drop-target" : ""}`}
    >
      <SortableContext
        items={sessionIds.map((sessionId) => `session:${sessionId}`)}
        strategy={verticalListSortingStrategy}
      >
        {children}
      </SortableContext>
    </section>
  );
}

const SIDEBAR_LAYOUT_STORAGE_KEY = "timem.sidebar-layout.v1";
const LEFT_SIDEBAR_MIN_WIDTH = 180;
const LEFT_SIDEBAR_MAX_WIDTH = 380;
const RIGHT_SIDEBAR_MIN_WIDTH = 240;
const RIGHT_SIDEBAR_MAX_WIDTH = 480;

type SidebarLayout = {
  leftWidth: number;
  rightWidth: number;
  leftCollapsed: boolean;
  rightCollapsed: boolean;
};

function clampSidebarWidth(value: number, minimum: number, maximum: number) {
  return Math.min(maximum, Math.max(minimum, Math.round(value)));
}

function loadSidebarLayout(): SidebarLayout {
  const fallback = {
    leftWidth: 220,
    rightWidth: 286,
    leftCollapsed: false,
    rightCollapsed: false,
  };
  try {
    const stored = JSON.parse(
      window.localStorage.getItem(SIDEBAR_LAYOUT_STORAGE_KEY) ?? "null",
    ) as Partial<SidebarLayout> | null;
    if (!stored) return fallback;
    return {
      leftWidth: clampSidebarWidth(
        Number(stored.leftWidth) || fallback.leftWidth,
        LEFT_SIDEBAR_MIN_WIDTH,
        LEFT_SIDEBAR_MAX_WIDTH,
      ),
      rightWidth: clampSidebarWidth(
        Number(stored.rightWidth) || fallback.rightWidth,
        RIGHT_SIDEBAR_MIN_WIDTH,
        RIGHT_SIDEBAR_MAX_WIDTH,
      ),
      leftCollapsed: stored.leftCollapsed === true,
      rightCollapsed: stored.rightCollapsed === true,
    };
  } catch {
    return fallback;
  }
}

function saveSidebarLayout(layout: SidebarLayout) {
  try {
    window.localStorage.setItem(
      SIDEBAR_LAYOUT_STORAGE_KEY,
      JSON.stringify(layout),
    );
  } catch {
    /* Storage can be unavailable in private or restricted contexts. */
  }
}

function TimemApp() {
  useDialogFocusTrap();
  const [appearance, setAppearance] = useState<Appearance>(loadAppearance);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [roleLibrary, setRoleLibrary] = useState<WorkerRoleLibrary>({
    roles: [],
    groups: [],
  });
  const [sessionGroups, setSessionGroups] = useState<SessionGroup[]>([]);
  const [collapsedSessionGroupIds, setCollapsedSessionGroupIds] = useState<
    Set<string>
  >(() => new Set());
  const [sessionGroupEditor, setSessionGroupEditor] = useState<{
    id?: string;
    name: string;
  } | null>(null);
  const [sessionGroupDeleteConfirmId, setSessionGroupDeleteConfirmId] =
    useState("");
  const [draggedSessionId, setDraggedSessionId] = useState("");
  const authoritativeRoleLibraryRef = useRef<WorkerRoleLibrary>({
    roles: [],
    groups: [],
  });
  const pendingWorkerRoleMutationsRef = useRef<Map<string, WorkerRoleMutation>>(
    new Map(),
  );
  const [activeSessionId, setActiveSessionId] = useState("");
  const [selectedRoleIds, setSelectedRoleIds] = useState<
    Record<string, string[]>
  >({});
  const [decisions, setDecisions] = useState<Decision[]>([]);
  const [connected, setConnected] = useState(false);
  const [snapshotReady, setSnapshotReady] = useState(false);
  const [runtimeEverConnected, setRuntimeEverConnected] = useState(false);
  const [reconnectAttempt, setReconnectAttempt] = useState(0);
  const [
    runtimeUnavailableDialogDismissed,
    setRuntimeUnavailableDialogDismissed,
  ] = useState(false);
  const [showToolRepo, setShowToolRepo] = useState(false);
  const [chatLibraryMode, setChatLibraryMode] = useState<
    "search" | "favorites" | null
  >(null);
  const [chatSearchQuery, setChatSearchQuery] = useState("");
  const [chatSearchScope, setChatSearchScope] = useState<
    "all" | "session" | "favorites"
  >("all");
  const [chatSearchResults, setChatSearchResults] = useState<ChatSearchHit[]>(
    [],
  );
  const [chatSearchPending, setChatSearchPending] = useState(false);
  const [favorites, setFavorites] = useState<ChatFavorite[]>([]);
  const [favoritesLoading, setFavoritesLoading] = useState(false);
  const [pendingFavoriteSourceKeys, setPendingFavoriteSourceKeys] = useState<
    Set<string>
  >(() => new Set());
  const [favoriteCapacity, setFavoriteCapacity] = useState<ChatLibraryCapacity>(
    { used_bytes: 0, limit_bytes: 256 * 1024 * 1024, used_percent: 0 },
  );
  const [favoriteCapacityNotice, setFavoriteCapacityNotice] = useState<{
    capacity: ChatLibraryCapacity;
    full: boolean;
  } | null>(null);
  const [favoriteCapacityUpdating, setFavoriteCapacityUpdating] =
    useState(false);
  const [showRoles, setShowRoles] = useState(false);
  const [sidebarLayout, setSidebarLayout] =
    useState<SidebarLayout>(loadSidebarLayout);
  const [toolSearchQuery, setToolSearchQuery] = useState("");
  const [toolSearchResults, setToolSearchResults] = useState<
    Record<string, ToolSummary[]>
  >({});
  const [pendingToolSearchKey, setPendingToolSearchKey] = useState("");
  const [pendingToolDetailKey, setPendingToolDetailKey] = useState("");
  const [pendingToolRenameKeys, setPendingToolRenameKeys] = useState<
    Set<string>
  >(() => new Set());
  const [selectedTool, setSelectedTool] = useState<ToolDetail | null>(null);
  const [toolCountPulseSessionId, setToolCountPulseSessionId] = useState("");
  const [pendingToolgenRequests, setPendingToolgenRequests] = useState<
    Set<string>
  >(() => new Set());
  const [toolGenEnabled, setToolGenEnabled] = useState(loadToolGenEnabled);
  const [toolgenDialog, setToolgenDialog] = useState<{
    sessionId: string;
    turnId: string;
  } | null>(null);
  const [showMobileSessions, setShowMobileSessions] = useState(false);
  const [showRuntime, setShowRuntime] = useState(false);
  const [endpointEditor, setEndpointEditor] = useState<
    ModelEndpoint | "new" | null
  >(null);
  const [deleteEndpointCandidate, setDeleteEndpointCandidate] =
    useState<ModelEndpoint | null>(null);
  const [revealedEndpointApiKeys, setRevealedEndpointApiKeys] = useState<
    Record<string, string>
  >({});
  const [revealedEndpointHeaders, setRevealedEndpointHeaders] = useState<
    Record<string, Record<string, string>>
  >({});
  const [revealedEndpointRequestFields, setRevealedEndpointRequestFields] =
    useState<Record<string, Record<string, unknown>>>({});
  const [showAppearance, setShowAppearance] = useState(false);
  const [settingsSection, setSettingsSection] =
    useState<SettingsSection>("appearance");
  const [memTemporaryItems, setMemTemporaryItems] = useState<
    MemTemporaryItem[]
  >([]);
  const [memTemporaryItemsLoading, setMemTemporaryItemsLoading] =
    useState(false);
  const [memTemporaryItemsDeleting, setMemTemporaryItemsDeleting] =
    useState(false);
  const [memTemporaryItemsError, setMemTemporaryItemsError] = useState("");
  const [showMcp, setShowMcp] = useState(false);
  const [showNewSession, setShowNewSession] = useState(false);
  const [deleteSessionCandidate, setDeleteSessionCandidate] =
    useState<Session | null>(null);
  const [sessionDeleteMode, setSessionDeleteMode] = useState(false);
  const [selectedDeleteSessionId, setSelectedDeleteSessionId] = useState("");
  const [deleteMessageCandidate, setDeleteMessageCandidate] =
    useState<ChatMessageDeleteCandidate | null>(null);
  const [renamingSessionId, setRenamingSessionId] = useState("");
  const [expandedSessionIds, setExpandedSessionIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [unreadCompletedSessionIds, setUnreadCompletedSessionIds] = useState<
    Set<string>
  >(() => new Set());
  const [renameDraft, setRenameDraft] = useState("");
  const [server, setServer] = useState<Snapshot["server"] | null>(null);
  const socket = useRef<WebSocket | null>(null);
  const performanceTraceRef = useRef(new BrowserPerformanceTrace());
  const sessionsRef = useRef<Session[]>([]);
  const previousSessionStatesRef = useRef<Map<string, string> | null>(null);
  const activeSessionIdRef = useRef("");
  const memTemporaryItemsLoadedForRef = useRef("");
  const toolSearchQueryRef = useRef("");
  const selectedToolRef = useRef<ToolDetail | null>(null);
  const toolCountBySessionRef = useRef<Map<string, number>>(new Map());
  const cancellingSessionIds = useRef<Set<string>>(new Set());
  const cancellingSessionCommandIds = useRef<Map<string, string>>(new Map());
  const cancellingSessionTimeouts = useRef<Map<string, number>>(new Map());
  const [creatingSession, setCreatingSession] = useState(false);
  const [pendingAttachmentRemoveIds, setPendingAttachmentRemoveIds] = useState<
    Set<string>
  >(() => new Set());
  const [pendingDecisionKeys, setPendingDecisionKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [pendingRenameSessionIds, setPendingRenameSessionIds] = useState<
    Set<string>
  >(() => new Set());
  const [pendingDeleteSessionIds, setPendingDeleteSessionIds] = useState<
    Set<string>
  >(() => new Set());
  const [pendingDeleteMessageKeys, setPendingDeleteMessageKeys] = useState<
    Set<string>
  >(() => new Set());
  const [pendingRuntimeKeys, setPendingRuntimeKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [pendingSessionCredentialIds, setPendingSessionCredentialIds] =
    useState<Set<string>>(() => new Set());
  const [pendingMcpKeys, setPendingMcpKeys] = useState<Set<string>>(
    () => new Set(),
  );
  const [revealedSessionApiKeys, setRevealedSessionApiKeys] = useState<
    Record<string, string>
  >({});
  const [revealedMcpSecrets, setRevealedMcpSecrets] = useState<
    Record<string, Record<string, string>>
  >({});
  const [pendingHistorySessionIds, setPendingHistorySessionIds] = useState<
    Set<string>
  >(() => new Set());
  const [pendingUploadSessionIds, setPendingUploadSessionIds] = useState<
    Set<string>
  >(() => new Set());
  const [pendingUploadFiles, setPendingUploadFiles] = useState<
    Record<string, { name: string; bytes: number }>
  >({});
  const [pendingMemSwitch, setPendingMemSwitch] = useState(false);
  const [memSwitchCandidate, setMemSwitchCandidate] =
    useState<MemSwitchCandidate | null>(null);
  const [pendingMemRetention, setPendingMemRetention] = useState(false);
  const [pendingMemConversationCapacity, setPendingMemConversationCapacity] =
    useState(false);
  const [completedTurnsBySession, setCompletedTurnsBySession] = useState<
    Record<
      string,
      { key: string; continuation: "normal" | "cancelled" | "blocked" }
    >
  >({});
  const [commandAcks, setCommandAcks] = useState<
    Record<string, Extract<WireEvent, { type: "command_ack" }>>
  >({});
  const consumeCommandAcks = useCallback((commandIds: ReadonlySet<string>) => {
    setCommandAcks((current) =>
      Object.fromEntries(
        Object.entries(current).filter(
          ([commandId]) => !commandIds.has(commandId),
        ),
      ),
    );
  }, []);
  const commandOutboxRef = useRef<CommandOutboxItem[]>([]);
  const [persistedSubmitCommandIds, setPersistedSubmitCommandIds] = useState<
    Record<string, string>
  >({});
  const [persistedCancelTargetCommandIds, setPersistedCancelTargetCommandIds] =
    useState<Record<string, string>>({});
  const commandOutboxScopeRef = useRef("");
  const eventCursorRef = useRef(0);
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
  const pendingSessionApiKeyCommandsRef = useRef<
    Map<string, PendingSessionApiKeyCommand>
  >(new Map());
  const pendingSessionApiKeyCommandIdsRef = useRef<Map<string, string>>(
    new Map(),
  );
  const pendingMcpKeysRef = useRef<Set<string>>(new Set());
  const pendingHistorySessionIdsRef = useRef<Set<string>>(new Set());
  const pendingUploadSessionIdsRef = useRef<Set<string>>(new Set());
  const pendingToolgenRequestsRef = useRef<Set<string>>(new Set());
  const fileInput = useRef<HTMLInputElement | null>(null);
  const newSessionButtonRef = useRef<HTMLButtonElement | null>(null);
  const appearancePanelRef = useRef<HTMLElement | null>(null);
  const mcpButtonRef = useRef<HTMLButtonElement | null>(null);
  const mcpPanelRef = useRef<HTMLElement | null>(null);
  const runtimeButtonRef = useRef<HTMLButtonElement | null>(null);
  const runtimePanelRef = useRef<HTMLElement | null>(null);
  const mobileSessionButtonRef = useRef<HTMLButtonElement | null>(null);
  const mobileSidebarRef = useRef<HTMLElement | null>(null);
  const toolRepoButtonRef = useRef<HTMLButtonElement | null>(null);
  const toolRepoPanelRef = useRef<HTMLElement | null>(null);
  const chatLibraryPanelRef = useRef<HTMLElement | null>(null);
  const chatLibraryTriggerRef = useRef<HTMLButtonElement | null>(null);
  const settingsButtonRef = useRef<HTMLButtonElement | null>(null);
  const activeSession =
    sessions.find((session) => session.session_id === activeSessionId) ??
    sessions[0];
  sessionsRef.current = sessions;
  const activeMessages = activeSession?.messages ?? EMPTY_CHAT_MESSAGES;
  const pushActivity = useCallback((activity: Activity) => {
    const requestedSessionId =
      activity.sessionId === "system"
        ? activeSessionIdRef.current
        : activity.sessionId;
    if (!requestedSessionId) return;
    setSessions((current) =>
      current.map((session) =>
        session.session_id === requestedSessionId
          ? appendActivityToCurrentTurn(session, {
              ...activity,
              sessionId: requestedSessionId,
            })
          : session,
      ),
    );
  }, []);
  const reportUiError = useCallback(
    (
      title: string,
      detail: string,
      sessionId = activeSessionIdRef.current || "system",
    ) => {
      pushActivity({
        id: clientId(),
        sessionId,
        tone: "error",
        title,
        detail,
        createdAt: Date.now(),
      });
    },
    [pushActivity],
  );
  const closeChatLibrary = useCallback(() => {
    setChatLibraryMode(null);
    window.requestAnimationFrame(() =>
      chatLibraryTriggerRef.current?.focus({ preventScroll: true }),
    );
  }, []);
  const closeToolRepoPanel = useCallback(() => {
    setShowToolRepo(false);
    toolRepoButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeRuntimePanel = useCallback((restoreFocus = true) => {
    setShowRuntime(false);
    setRevealedSessionApiKeys({});
    setRevealedEndpointApiKeys({});
    setRevealedEndpointHeaders({});
    if (restoreFocus) runtimeButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeAppearancePanel = useCallback((restoreFocus = true) => {
    setShowAppearance(false);
    if (restoreFocus)
      window.requestAnimationFrame(() =>
        settingsButtonRef.current?.focus({ preventScroll: true }),
      );
  }, []);
  const closeMcpPanel = useCallback((restoreFocus = true) => {
    setShowMcp(false);
    setRevealedMcpSecrets({});
    if (restoreFocus) mcpButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeMobileSidebar = useCallback((restoreFocus = true) => {
    setShowMobileSessions(false);
    if (restoreFocus)
      mobileSessionButtonRef.current?.focus({ preventScroll: true });
  }, []);
  const closeNewSessionDialog = useCallback((restoreFocus = true) => {
    setShowNewSession(false);
    if (!restoreFocus) return;
    const newSessionButton = newSessionButtonRef.current;
    if (
      newSessionButton &&
      window.getComputedStyle(newSessionButton).visibility !== "hidden"
    ) {
      newSessionButton.focus({ preventScroll: true });
    } else {
      mobileSessionButtonRef.current?.focus({ preventScroll: true });
    }
  }, []);

  useEffect(() => {
    applyAppearance(appearance);
  }, [appearance]);

  useEffect(() => {
    saveToolGenEnabled(toolGenEnabled);
    if (!toolGenEnabled) {
      setShowToolRepo(false);
      if (
        toolgenDialog &&
        !pendingToolgenRequests.has(
          toolgenRequestKey(toolgenDialog.sessionId, toolgenDialog.turnId),
        )
      )
        setToolgenDialog(null);
    }
  }, [pendingToolgenRequests, toolGenEnabled, toolgenDialog]);

  useEffect(() => {
    saveSidebarLayout(sidebarLayout);
  }, [sidebarLayout]);

  const startSidebarResize = useCallback(
    (side: "left" | "right", event: React.PointerEvent<HTMLButtonElement>) => {
      if (window.matchMedia("(max-width: 1050px)").matches) return;
      event.preventDefault();
      const startX = event.clientX;
      const startWidth =
        side === "left" ? sidebarLayout.leftWidth : sidebarLayout.rightWidth;
      const minimum =
        side === "left" ? LEFT_SIDEBAR_MIN_WIDTH : RIGHT_SIDEBAR_MIN_WIDTH;
      const maximum =
        side === "left" ? LEFT_SIDEBAR_MAX_WIDTH : RIGHT_SIDEBAR_MAX_WIDTH;
      const move = (moveEvent: PointerEvent) => {
        const delta = (moveEvent.clientX - startX) * (side === "left" ? 1 : -1);
        const width = clampSidebarWidth(startWidth + delta, minimum, maximum);
        setSidebarLayout((current) =>
          side === "left"
            ? { ...current, leftWidth: width }
            : { ...current, rightWidth: width },
        );
      };
      const stop = () => {
        document.body.classList.remove("resizing-sidebar");
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", stop);
        window.removeEventListener("pointercancel", stop);
      };
      document.body.classList.add("resizing-sidebar");
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", stop);
      window.addEventListener("pointercancel", stop);
    },
    [sidebarLayout.leftWidth, sidebarLayout.rightWidth],
  );

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
      if (
        runtimeButtonRef.current?.contains(target) ||
        runtimePanelRef.current?.contains(target)
      )
        return;
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
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !pendingMemSwitch && !pendingMemRetention)
        closeAppearancePanel();
    };
    document.addEventListener("keydown", dismissOnEscape);
    return () => document.removeEventListener("keydown", dismissOnEscape);
  }, [
    closeAppearancePanel,
    pendingMemRetention,
    pendingMemSwitch,
    showAppearance,
  ]);

  useEffect(() => {
    if (!chatLibraryMode) return;
    chatLibraryPanelRef.current?.focus({ preventScroll: true });
    const dismissOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeChatLibrary();
    };
    document.addEventListener("keydown", dismissOnEscape);
    return () => document.removeEventListener("keydown", dismissOnEscape);
  }, [chatLibraryMode, closeChatLibrary]);

  useEffect(() => {
    if (!showMcp) return;
    mcpPanelRef.current?.focus({ preventScroll: true });
    const dismissOnOutsidePointer = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (
        mcpButtonRef.current?.contains(target) ||
        mcpPanelRef.current?.contains(target)
      )
        return;
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
      window.history.replaceState(
        null,
        "",
        `${window.location.pathname}${window.location.hash}`,
      );
    }
  }, []);

  const sendCommand = useCallback(
    (command: ClientCommand, requestedCommandId?: string) => {
      const reliable = commandNeedsReliableDelivery(command);
      let wireCommand: ClientCommand | CommandWithId = command;
      if (reliable) {
        const commandId = requestedCommandId ?? clientId("command");
        const next = addCommandToOutbox(
          commandOutboxRef.current,
          command,
          commandId,
        );
        const item = next.find(
          (candidate) => candidate.commandId === commandId,
        );
        if (
          !item ||
          (commandMayPersist(command) &&
            !saveCommandOutboxItem(
              window.localStorage,
              commandOutboxScopeRef.current,
              item,
            ))
        )
          return false;
        commandOutboxRef.current = next;
        setPersistedSubmitCommandIds(pendingTurnSubmitCommandIds(next));
        setPersistedCancelTargetCommandIds(
          pendingTurnCancelTargetCommandIds(next),
        );
        wireCommand = { ...command, command_id: commandId };
      }
      if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady)
        return reliable;
      const tracedCommand =
        performanceTraceRef.current.instrumentCommand(wireCommand);
      try {
        socket.current.send(JSON.stringify(tracedCommand));
        return true;
      } catch {
        return reliable;
      }
    },
    [snapshotReady],
  );

  const closeSettingsCenter = useCallback(() => {
    if (
      !pendingMemRetention &&
      !pendingMemConversationCapacity &&
      !favoriteCapacityUpdating &&
      !pendingMemSwitch &&
      !memTemporaryItemsDeleting
    )
      closeAppearancePanel();
  }, [
    closeAppearancePanel,
    favoriteCapacityUpdating,
    memTemporaryItemsDeleting,
    pendingMemConversationCapacity,
    pendingMemRetention,
    pendingMemSwitch,
  ]);
  const refreshMemTemporaryItems = useCallback(() => {
    setMemTemporaryItemsLoading(true);
    setMemTemporaryItemsError("");
    if (!sendCommand({ type: "mem_temporary_items_list" }))
      setMemTemporaryItemsLoading(false);
  }, [sendCommand]);
  const deleteMemTemporaryItems = useCallback(
    (ids: string[]) => {
      setMemTemporaryItemsDeleting(true);
      if (!sendCommand({ type: "mem_temporary_items_delete", ids }))
        setMemTemporaryItemsDeleting(false);
    },
    [sendCommand],
  );
  const revealModelEndpoint = useCallback(
    (endpointId: string) => {
      sendCommand({
        type: "model_endpoint_secret_reveal",
        endpoint_id: endpointId,
      });
    },
    [sendCommand],
  );
  const saveModelEndpoint = useCallback(
    (endpoint: ModelEndpointDraft) => {
      sendCommand({ type: "model_endpoint_upsert", endpoint });
    },
    [sendCommand],
  );
  const saveMemTemporaryPolicy = useCallback(
    (days: 1 | 5 | 10 | null, maxBytes: number | null) => {
      setPendingMemRetention(true);
      if (
        !sendCommand({
          type: "mem_temporary_retention_update",
          days,
          max_bytes: maxBytes,
        })
      ) {
        setPendingMemRetention(false);
        reportUiError(
          "Mem settings failed",
          "Reconnect to Timem Web before updating temporary-data policy.",
          "system",
        );
      }
    },
    [reportUiError, sendCommand],
  );
  const saveMemConversationCapacity = useCallback(
    (maxBytes: number | null) => {
      setPendingMemConversationCapacity(true);
      if (
        !sendCommand({
          type: "mem_conversation_capacity_update",
          max_bytes: maxBytes,
        })
      ) {
        setPendingMemConversationCapacity(false);
        reportUiError(
          "Mem settings failed",
          "Reconnect to Timem Web before updating conversation capacity.",
          "system",
        );
      }
    },
    [reportUiError, sendCommand],
  );
  const saveMemFavoriteCapacity = useCallback(
    (maxBytes: number | null) => {
      setFavoriteCapacityUpdating(true);
      if (
        !sendCommand({ type: "favorite_capacity_update", max_bytes: maxBytes })
      )
        setFavoriteCapacityUpdating(false);
    },
    [sendCommand],
  );
  const switchMemWorkspace = useCallback(
    (path: string) => {
      const runningSessionCount = memSwitchRunningSessionCount(
        sessionsRef.current,
      );
      if (runningSessionCount > 0) {
        setMemSwitchCandidate({ path, runningSessionCount });
        return;
      }
      setRenamingSessionId("");
      setRenameDraft("");
      setPendingMemSwitch(true);
      if (!sendCommand({ type: "mem_switch", path, stop_running: false })) {
        setPendingMemSwitch(false);
        reportUiError(
          "Mem switch failed",
          "Reconnect to Timem Web before switching the mem directory.",
          "system",
        );
      }
    },
    [reportUiError, sendCommand],
  );

  const toggleFavorite = useCallback(
    (
      sessionId: string,
      turnId: string,
      favoriteId?: string,
      sourceKeyOverride?: string,
    ) => {
      const sourceKey =
        sourceKeyOverride ?? `legacy:${sessionId}:${turnId}:assistant:0`;
      setPendingFavoriteSourceKeys((current) =>
        new Set(current).add(sourceKey),
      );
      const sent = sendCommand(
        favoriteId
          ? { type: "favorite_delete", favorite_id: favoriteId }
          : { type: "favorite_create", session_id: sessionId, turn_id: turnId },
      );
      if (!sent)
        setPendingFavoriteSourceKeys((current) => {
          const next = new Set(current);
          next.delete(sourceKey);
          return next;
        });
      return sent;
    },
    [sendCommand],
  );

  useEffect(() => {
    if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return;
    for (const item of commandOutboxRef.current) {
      try {
        socket.current.send(JSON.stringify(item.command));
      } catch {
        break;
      }
    }
  }, [snapshotReady]);

  useEffect(() => {
    const syncCrossTabOutbox = (event: StorageEvent) => {
      const scope = commandOutboxScopeRef.current;
      if (
        !scope ||
        !event.key?.startsWith(`${commandOutboxStorageKey(scope)}:`)
      )
        return;
      const stored = loadCommandOutbox(window.localStorage, scope);
      const memoryOnly = commandOutboxRef.current.filter(
        (item) => !commandMayPersist(item.command),
      );
      commandOutboxRef.current = orderCommandOutbox([
        ...stored,
        ...memoryOnly.filter(
          (item) =>
            !stored.some((candidate) => candidate.commandId === item.commandId),
        ),
      ]);
      setPersistedSubmitCommandIds(
        pendingTurnSubmitCommandIds(commandOutboxRef.current),
      );
      setPersistedCancelTargetCommandIds(
        pendingTurnCancelTargetCommandIds(commandOutboxRef.current),
      );
      if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady)
        return;
      for (const item of stored) {
        try {
          socket.current.send(JSON.stringify(item.command));
        } catch {
          break;
        }
      }
    };
    window.addEventListener("storage", syncCrossTabOutbox);
    return () => window.removeEventListener("storage", syncCrossTabOutbox);
  }, [snapshotReady]);

  const addPendingKey = useCallback(
    (
      ref: MutableRefObject<Set<string>>,
      setState: Dispatch<SetStateAction<Set<string>>>,
      key: string,
    ) => {
      if (ref.current.has(key)) return false;
      ref.current.add(key);
      setState((current) => new Set(current).add(key));
      return true;
    },
    [],
  );

  const removePendingKey = useCallback(
    (
      ref: MutableRefObject<Set<string>>,
      setState: Dispatch<SetStateAction<Set<string>>>,
      key: string,
    ) => {
      ref.current.delete(key);
      setState((current) => {
        const next = new Set(current);
        next.delete(key);
        return next;
      });
    },
    [],
  );

  const finishPendingSessionApiKeyCommand = useCallback(
    (sessionId: string, commandId?: string, committed = false) => {
      const activeCommandId =
        pendingSessionApiKeyCommandIdsRef.current.get(sessionId);
      if (commandId && activeCommandId !== commandId) return false;
      if (activeCommandId) {
        const pending =
          pendingSessionApiKeyCommandsRef.current.get(activeCommandId);
        if (pending) window.clearTimeout(pending.timeoutId);
        pendingSessionApiKeyCommandsRef.current.delete(activeCommandId);
        pendingSessionApiKeyCommandIdsRef.current.delete(sessionId);
        commandOutboxRef.current = finishOutboxCommand(
          commandOutboxRef.current,
          activeCommandId,
        );
        setPersistedSubmitCommandIds(
          pendingTurnSubmitCommandIds(commandOutboxRef.current),
        );
        setPersistedCancelTargetCommandIds(
          pendingTurnCancelTargetCommandIds(commandOutboxRef.current),
        );
        removeCommandOutboxItem(
          window.localStorage,
          commandOutboxScopeRef.current,
          activeCommandId,
        );
      }
      const savedApiKey = pendingSessionApiKeyValuesRef.current.get(sessionId);
      pendingSessionApiKeyValuesRef.current.delete(sessionId);
      removePendingKey(
        pendingSessionCredentialIdsRef,
        setPendingSessionCredentialIds,
        sessionId,
      );
      if (committed && savedApiKey !== undefined) {
        setRevealedSessionApiKeys((current) =>
          updateRevealedSessionApiKeys(current, sessionId, savedApiKey),
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === sessionId && session.runtime_profile
              ? {
                  ...session,
                  runtime_profile: {
                    ...session.runtime_profile,
                    api_key_configured: savedApiKey.length > 0,
                  },
                }
              : session,
          ),
        );
      }
      return true;
    },
    [removePendingKey],
  );

  const cancelAllPendingSessionApiKeyCommands = useCallback(
    (detail?: string) => {
      const sessionIds = Array.from(
        pendingSessionApiKeyCommandIdsRef.current.keys(),
      );
      for (const sessionId of sessionIds) {
        finishPendingSessionApiKeyCommand(sessionId);
        if (detail)
          reportUiError("API key update interrupted", detail, sessionId);
      }
    },
    [finishPendingSessionApiKeyCommand, reportUiError],
  );

  const clearAllPendingCommands = useCallback(() => {
    creatingSessionRef.current = false;
    cancellingSessionIds.current.clear();
    cancellingSessionCommandIds.current.clear();
    for (const timeoutId of cancellingSessionTimeouts.current.values())
      window.clearTimeout(timeoutId);
    cancellingSessionTimeouts.current.clear();
    pendingAttachmentRemoveIdsRef.current.clear();
    pendingDecisionKeysRef.current.clear();
    pendingRenameSessionIdsRef.current.clear();
    pendingDeleteSessionIdsRef.current.clear();
    pendingDeleteMessageKeysRef.current.clear();
    pendingRuntimeKeysRef.current.clear();
    for (const pending of pendingSessionApiKeyCommandsRef.current.values()) {
      window.clearTimeout(pending.timeoutId);
    }
    pendingSessionCredentialIdsRef.current.clear();
    pendingSessionApiKeyValuesRef.current.clear();
    pendingSessionApiKeyCommandsRef.current.clear();
    pendingSessionApiKeyCommandIdsRef.current.clear();
    pendingMcpKeysRef.current.clear();
    pendingHistorySessionIdsRef.current.clear();
    pendingUploadSessionIdsRef.current.clear();
    pendingToolgenRequestsRef.current.clear();
    setCreatingSession(false);
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
    setMemSwitchCandidate(null);
  }, []);

  useEffect(() => {
    const liveSessionIds = new Set(
      sessions.map((session) => session.session_id),
    );
    for (const sessionId of Array.from(cancellingSessionIds.current)) {
      if (!liveSessionIds.has(sessionId)) {
        cancellingSessionIds.current.delete(sessionId);
        cancellingSessionCommandIds.current.delete(sessionId);
        const timeoutId = cancellingSessionTimeouts.current.get(sessionId);
        if (timeoutId !== undefined) window.clearTimeout(timeoutId);
        cancellingSessionTimeouts.current.delete(sessionId);
      }
    }
  }, [sessions]);

  useEffect(() => {
    const previous = previousSessionStatesRef.current;
    const next = new Map(
      sessions.map((session) => [session.session_id, session.state]),
    );
    previousSessionStatesRef.current = next;
    if (!previous) return;
    const completedAway = sessions
      .filter(
        (session) =>
          previous.get(session.session_id) === "working" &&
          session.state !== "working" &&
          session.session_id !== activeSessionIdRef.current,
      )
      .map((session) => session.session_id);
    setUnreadCompletedSessionIds((current) => {
      const live = new Set(sessions.map((session) => session.session_id));
      const updated = new Set(
        Array.from(current).filter((sessionId) => live.has(sessionId)),
      );
      for (const sessionId of completedAway) updated.add(sessionId);
      return updated.size === current.size &&
        Array.from(updated).every((sessionId) => current.has(sessionId))
        ? current
        : updated;
    });
  }, [sessions]);

  useEffect(() => {
    if (!activeSessionId) return;
    setUnreadCompletedSessionIds((current) => {
      if (!current.has(activeSessionId)) return current;
      const next = new Set(current);
      next.delete(activeSessionId);
      return next;
    });
  }, [activeSessionId]);

  const beginRename = useCallback((session: Session) => {
    setRenamingSessionId(session.session_id);
    setRenameDraft(session.display_name);
  }, []);

  const finishRename = useCallback(
    (sessionId: string) => {
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
      if (
        addPendingKey(
          pendingRenameSessionIdsRef,
          setPendingRenameSessionIds,
          sessionId,
        )
      ) {
        if (!sendCommand(decision.command)) {
          removePendingKey(
            pendingRenameSessionIdsRef,
            setPendingRenameSessionIds,
            sessionId,
          );
          setRenamingSessionId("");
          setRenameDraft("");
          reportUiError(
            "Rename session failed",
            "Reconnect to Timem Web before renaming this session.",
            sessionId,
          );
          return;
        }
        setSessions((current) =>
          current.map((session) =>
            session.session_id === sessionId
              ? { ...session, display_name: decision.displayName }
              : session,
          ),
        );
      }
      setRenamingSessionId("");
      setRenameDraft("");
    },
    [
      addPendingKey,
      pendingMemSwitch,
      removePendingKey,
      renameDraft,
      reportUiError,
      sendCommand,
    ],
  );

  const applySnapshot = useCallback((snapshot: Snapshot) => {
    previousSessionStatesRef.current = null;
    setUnreadCompletedSessionIds(new Set());
    toolCountBySessionRef.current = new Map(
      snapshot.sessions.map((session) => [
        session.session_id,
        session.tools.length,
      ]),
    );
    setServer(snapshot.server);
    performanceTraceRef.current.setEnabled(snapshot.server.performance_trace);
    if (!snapshot.server.debug_mode) setExpandedSessionIds(new Set());
    const authoritativeRoleLibrary = snapshot.role_library ?? {
      roles: snapshot.sessions[0]?.roles ?? [],
      groups: [],
    };
    authoritativeRoleLibraryRef.current = authoritativeRoleLibrary;
    const visibleRoleLibrary = replayWorkerRoleMutations(
      authoritativeRoleLibrary,
      pendingWorkerRoleMutationsRef.current.values(),
    );
    setRoleLibrary(visibleRoleLibrary);
    setSessionGroups(snapshot.session_groups ?? []);
    setSessions(
      snapshot.sessions.map((session) =>
        boundSessionHistory({ ...session, roles: visibleRoleLibrary.roles }),
      ),
    );
    setActiveSessionId((current) =>
      resolveActiveSessionId(current, snapshot.sessions),
    );
  }, []);

  const receive = useCallback(
    function receiveWireEvent(event: WireEvent, fromSemantic = false) {
      if (!fromSemantic) {
        if (event.type === "hello") {
          // A reconnect may intentionally target an older Host, so Hello resets
          // rather than only ever enabling this connection-level capability.
          semanticDeliveryRef.current = enablesSemanticDelivery(event);
        } else if (enablesSemanticDelivery(event)) {
          semanticDeliveryRef.current = true;
        }
        if (!shouldReduceTopLevelWireEvent(event, semanticDeliveryRef.current))
          return;
      }
      if (event.type === "semantic_event") {
        if (
          event.event.type === "semantic_event" ||
          event.event.type === "hello"
        ) {
          reportUiError(
            "Invalid runtime event",
            `Event ${event.event_seq} contains an invalid nested ${event.event.type} envelope.`,
          );
          socket.current?.close();
          return;
        }
        const sequenceState = classifyEventSequence(
          eventCursorRef.current,
          event.event_seq,
        );
        if (sequenceState === "duplicate") return;
        if (sequenceState === "gap") {
          reportUiError(
            "Runtime event gap",
            `Expected event ${eventCursorRef.current + 1}, received ${event.event_seq}. Reconnecting to reload the authoritative state.`,
          );
          socket.current?.close();
          return;
        }
        receiveWireEvent(event.event, true);
        eventCursorRef.current = event.event_seq;
        return;
      }
      if (event.type === "command_ack") {
        if (
          event.command_id.startsWith("queued-") ||
          (event.command_id.startsWith("submit-") &&
            event.status === "rejected")
        ) {
          setCommandAcks((current) => ({
            ...current,
            [event.command_id]: event,
          }));
        }
        if (event.status === "accepted") {
          commandOutboxRef.current = acceptOutboxCommand(
            commandOutboxRef.current,
            event.command_id,
          );
          const accepted = commandOutboxRef.current.find(
            (item) => item.commandId === event.command_id,
          );
          if (accepted && commandMayPersist(accepted.command))
            saveCommandOutboxItem(
              window.localStorage,
              commandOutboxScopeRef.current,
              accepted,
            );
        } else {
          const completed = commandOutboxRef.current.find(
            (item) => item.commandId === event.command_id,
          );
          const pendingCredential = pendingSessionApiKeyCommandsRef.current.get(
            event.command_id,
          );
          commandOutboxRef.current = finishOutboxCommand(
            commandOutboxRef.current,
            event.command_id,
          );
          setPersistedSubmitCommandIds(
            pendingTurnSubmitCommandIds(commandOutboxRef.current),
          );
          setPersistedCancelTargetCommandIds(
            pendingTurnCancelTargetCommandIds(commandOutboxRef.current),
          );
          removeCommandOutboxItem(
            window.localStorage,
            commandOutboxScopeRef.current,
            event.command_id,
          );
          if (pendingCredential) {
            finishPendingSessionApiKeyCommand(
              pendingCredential.sessionId,
              event.command_id,
              event.status === "committed",
            );
          }
          if (event.status === "rejected") {
            if (
              pendingWorkerRoleMutationsRef.current.delete(event.command_id)
            ) {
              const visibleLibrary = replayWorkerRoleMutations(
                authoritativeRoleLibraryRef.current,
                pendingWorkerRoleMutationsRef.current.values(),
              );
              setRoleLibrary(visibleLibrary);
              setSessions((current) =>
                current.map((session) => ({
                  ...session,
                  roles: visibleLibrary.roles,
                })),
              );
            }
            const sessionId =
              pendingCredential?.sessionId ??
              commandSessionId(completed?.command) ??
              (activeSessionIdRef.current || "system");
            if (
              completed?.command.type === "turn_cancel" &&
              sessionId !== "system" &&
              cancellingSessionCommandIds.current.get(sessionId) ===
                event.command_id
            ) {
              cancellingSessionIds.current.delete(sessionId);
              cancellingSessionCommandIds.current.delete(sessionId);
              const timeoutId =
                cancellingSessionTimeouts.current.get(sessionId);
              if (timeoutId !== undefined) window.clearTimeout(timeoutId);
              cancellingSessionTimeouts.current.delete(sessionId);
            }
            if (completed?.command.type === "mem_temporary_retention_update") {
              setPendingMemRetention(false);
            }
            if (
              completed?.command.type === "mem_conversation_capacity_update"
            ) {
              setPendingMemConversationCapacity(false);
            }
            const memSwitchNeedsConfirmation =
              completed?.command.type === "mem_switch" &&
              !completed.command.stop_running &&
              event.error ===
                "mem_switch_active_sessions_confirmation_required";
            if (completed?.command.type === "mem_switch") {
              setPendingMemSwitch(false);
              if (memSwitchNeedsConfirmation) {
                setMemSwitchCandidate({
                  path: completed.command.path,
                  runningSessionCount: Math.max(
                    1,
                    memSwitchRunningSessionCount(sessionsRef.current),
                  ),
                });
              } else {
                setMemSwitchCandidate(null);
              }
            }
            if (completed?.command.type === "mem_temporary_items_list") {
              setMemTemporaryItemsLoading(false);
            }
            if (completed?.command.type === "mem_temporary_items_delete") {
              setMemTemporaryItemsDeleting(false);
            }
            if (memSwitchNeedsConfirmation) {
              // The authoritative runtime found live work after the latest browser snapshot.
              // The confirmation dialog is the actionable response; avoid a redundant error toast.
            } else if (pendingCredential) {
              reportUiError(
                "API key update rejected",
                event.error ||
                  "The runtime rejected this Session credential. Check the value and try again.",
                sessionId,
              );
            } else if (isModelSubmissionCommand(completed?.command)) {
              const issue = modelServiceIssue(
                event.error || "The runtime rejected this model request.",
              );
              reportUiError(issue.title, issue.detail, sessionId);
            } else if (completed?.command.type === "favorite_capacity_update") {
              setFavoriteCapacityUpdating(false);
              reportUiError("无法调整收藏夹空间", "请稍后重试。", sessionId);
            } else if (
              completed?.command.type === "favorite_create" ||
              completed?.command.type === "favorite_delete" ||
              completed?.command.type === "favorites_list"
            ) {
              setFavoritesLoading(false);
              setPendingFavoriteSourceKeys(new Set());
              reportUiError("收藏夹暂时不可用", "请稍后重试。", sessionId);
            } else if (completed?.command.type === "chat_search") {
              setChatSearchPending(false);
              reportUiError("搜索暂时不可用", "请稍后重试。", sessionId);
            } else {
              reportUiError(
                "Command rejected",
                event.error || "The runtime rejected this command.",
                sessionId,
              );
            }
          }
        }
        return;
      }
      if (event.type === "mem_settings_updated") {
        setPendingMemRetention(false);
        setPendingMemConversationCapacity(false);
        setServer((current) =>
          current
            ? {
                ...current,
                mem: {
                  ...current.mem,
                  temporary_retention_days: event.temporary_retention_days,
                  temporary_capacity_bytes: event.temporary_capacity_bytes,
                  conversation_capacity_bytes:
                    event.conversation_capacity_bytes,
                },
              }
            : current,
        );
        return;
      }
      if (event.type === "mem_temporary_items") {
        setMemTemporaryItems(event.items);
        setMemTemporaryItemsError(event.error ?? "");
        setMemTemporaryItemsLoading(false);
        setMemTemporaryItemsDeleting(false);
        return;
      }
      if (event.type === "hello") {
        const scope = reliableStorageScope(
          window.location.origin,
          event.snapshot.server.mem.space_dir,
        );
        // Hello always carries a complete authoritative snapshot. Its cursor is
        // the baseline for this connection; old browser cursors are deliberately
        // not persisted or replayed across reconnects.
        eventCursorRef.current =
          Number.isSafeInteger(event.event_cursor) &&
          (event.event_cursor ?? 0) >= 0
            ? (event.event_cursor ?? 0)
            : 0;
        if (commandOutboxScopeRef.current !== scope) {
          commandOutboxScopeRef.current = scope;
          commandOutboxRef.current = loadCommandOutbox(
            window.localStorage,
            scope,
          );
          setPersistedSubmitCommandIds(
            pendingTurnSubmitCommandIds(commandOutboxRef.current),
          );
          setPersistedCancelTargetCommandIds(
            pendingTurnCancelTargetCommandIds(commandOutboxRef.current),
          );
          setCommandAcks({});
        }
        const durableCommandIds = new Set(
          commandOutboxRef.current.map((item) => item.commandId),
        );
        for (const commandId of pendingWorkerRoleMutationsRef.current.keys()) {
          if (!durableCommandIds.has(commandId))
            pendingWorkerRoleMutationsRef.current.delete(commandId);
        }
        clearAllPendingCommands();
        setDecisions(decisionsFromSessions(event.snapshot.sessions));
        applySnapshot(event.snapshot);
        setFavorites([]);
        setChatSearchResults([]);
        setFavoriteCapacityNotice(null);
        setFavoriteCapacityUpdating(false);
        setFavoritesLoading(true);
        setSnapshotReady(true);
        queueMicrotask(() => {
          if (!sendCommand({ type: "favorites_list" }))
            setFavoritesLoading(false);
        });
        return;
      }
      if (event.type === "session_created") {
        creatingSessionRef.current = false;
        setCreatingSession(false);
        setSessions((current) => upsertSession(current, event.session));
        toolCountBySessionRef.current.set(
          event.session.session_id,
          event.session.tools.length,
        );
        setActiveSessionId(event.session.session_id);
        return;
      }
      if (event.type === "session_renamed") {
        removePendingKey(
          pendingRenameSessionIdsRef,
          setPendingRenameSessionIds,
          event.session_id,
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? { ...session, display_name: event.display_name }
              : session,
          ),
        );
        return;
      }
      if (event.type === "session_deleted") {
        removePendingKey(
          pendingDeleteSessionIdsRef,
          setPendingDeleteSessionIds,
          event.session_id,
        );
        setDeleteSessionCandidate((current) =>
          current?.session_id === event.session_id ? null : current,
        );
        toolCountBySessionRef.current.delete(event.session_id);
        setExpandedSessionIds((current) => {
          const next = new Set(current);
          next.delete(event.session_id);
          return next;
        });
        setDecisions((current) =>
          current.filter(
            (decision) => decision.event.session_id !== event.session_id,
          ),
        );
        setSessions((current) => {
          const remaining = current.filter(
            (session) => session.session_id !== event.session_id,
          );
          setActiveSessionId((activeId) =>
            resolveActiveSessionId(activeId, remaining),
          );
          return remaining;
        });
        return;
      }
      if (event.type === "session_groups_updated") {
        setSessionGroups(event.groups);
        setSessionGroupEditor(null);
        setSessionGroupDeleteConfirmId("");
        return;
      }
      if (event.type === "session_group_changed") {
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? { ...session, group_id: event.group_id ?? null }
              : session,
          ),
        );
        return;
      }
      if (event.type === "worker_roles_updated") {
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? { ...session, roles: event.roles }
              : session,
          ),
        );
        setSelectedRoleIds((current) => {
          const selected = current[event.session_id] ?? [];
          const retained = selected.filter((roleId) =>
            event.roles.some((role) => role.id === roleId),
          );
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
      if (event.type === "worker_role_library_updated") {
        authoritativeRoleLibraryRef.current = event.library;
        if (event.command_id)
          pendingWorkerRoleMutationsRef.current.delete(event.command_id);
        const visibleLibrary = replayWorkerRoleMutations(
          event.library,
          pendingWorkerRoleMutationsRef.current.values(),
        );
        setRoleLibrary(visibleLibrary);
        setSessions((current) =>
          current.map((session) => ({
            ...session,
            roles: visibleLibrary.roles,
          })),
        );
        setSelectedRoleIds((current) => {
          let changed = false;
          const next: Record<string, string[]> = {};
          for (const [sessionId, selected] of Object.entries(current)) {
            const retained = selected.filter((roleId) =>
              visibleLibrary.roles.some((role) => role.id === roleId),
            );
            if (retained.length > 0) next[sessionId] = retained;
            if (retained.length !== selected.length) changed = true;
          }
          return changed ? next : current;
        });
        return;
      }
      if (event.type === "chat_search_result") {
        setChatSearchPending(false);
        setChatSearchResults(event.hits);
        return;
      }
      if (event.type === "favorites_list") {
        setFavoritesLoading(false);
        setFavorites(event.favorites);
        setFavoriteCapacity(event.capacity);
        return;
      }
      if (event.type === "favorite_created") {
        setPendingFavoriteSourceKeys((current) => {
          const next = new Set(current);
          next.delete(event.favorite.source_key);
          return next;
        });
        setFavoritesLoading(false);
        setFavorites((current) => [
          event.favorite,
          ...current.filter(
            (item) =>
              item.id !== event.favorite.id &&
              item.source_key !== event.favorite.source_key,
          ),
        ]);
        setChatSearchResults((current) =>
          current.map((hit) =>
            hit.source_key === event.favorite.source_key
              ? { ...hit, favorite_id: event.favorite.id }
              : hit,
          ),
        );
        setFavoriteCapacity(event.capacity);
        if (event.nearing_limit)
          setFavoriteCapacityNotice({ capacity: event.capacity, full: false });
        return;
      }
      if (event.type === "favorite_capacity_reached") {
        setPendingFavoriteSourceKeys(new Set());
        setFavoriteCapacity(event.capacity);
        setFavoriteCapacityNotice({ capacity: event.capacity, full: true });
        return;
      }
      if (event.type === "favorite_capacity_updated") {
        setFavoriteCapacityUpdating(false);
        setFavoriteCapacity(event.capacity);
        setFavoriteCapacityNotice(null);
        return;
      }
      if (event.type === "favorite_deleted") {
        setFavorites((current) => {
          const deleted = current.find((item) => item.id === event.favorite_id);
          if (deleted)
            setPendingFavoriteSourceKeys((pending) => {
              const next = new Set(pending);
              next.delete(deleted.source_key);
              return next;
            });
          return current.filter((item) => item.id !== event.favorite_id);
        });
        setChatSearchResults((current) =>
          current.map((hit) =>
            hit.favorite_id === event.favorite_id
              ? { ...hit, favorite_id: null }
              : hit,
          ),
        );
        return;
      }
      if (event.type === "chat_message_deleted") {
        const key = chatMessageDeleteKey({
          sessionId: event.session_id,
          turnId: event.turn_id,
          role: event.role,
          roleIndex: event.role_index,
        });
        removePendingKey(
          pendingDeleteMessageKeysRef,
          setPendingDeleteMessageKeys,
          key,
        );
        setDeleteMessageCandidate((current) =>
          current && chatMessageDeleteKey(current) === key ? null : current,
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? applyChatMessageDeleted(
                  session,
                  event.turn_id,
                  event.role,
                  event.role_index,
                )
              : session,
          ),
        );
        return;
      }
      if (event.type === "session_runtime_updated") {
        finishPendingSessionApiKeyCommand(event.session_id, undefined, true);
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? applySessionRuntimeProfile(session, event.runtime_profile)
              : session,
          ),
        );
        return;
      }
      if (event.type === "session_runtime_config_updated") {
        removePendingKey(
          pendingRuntimeKeysRef,
          setPendingRuntimeKeys,
          `${event.session_id}:${event.key}`,
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? applySessionRuntimeProfile(session, event.runtime_profile)
              : session,
          ),
        );
        const activity: Activity = {
          id: clientId(),
          sessionId: event.session_id,
          tone: "notice",
          title: "Session setting updated",
          detail: `${runtimeOptionLabel(event.key)}: ${event.value}`,
          createdAt: Date.now(),
        };
        pushActivity(activity);
        return;
      }
      if (event.type === "session_api_key_revealed") {
        removePendingKey(
          pendingSessionCredentialIdsRef,
          setPendingSessionCredentialIds,
          `reveal:${event.session_id}`,
        );
        setRevealedSessionApiKeys((current) => ({
          ...current,
          [event.session_id]: event.api_key,
        }));
        return;
      }
      if (event.type === "turn_projection") {
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? applyTurnProjection(session, event.projection)
              : session,
          ),
        );
        return;
      }
      if (event.type === "turn_cancelling") {
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session.session_id
              ? event.session
              : session,
          ),
        );
        return;
      }
      if (event.type === "turn_started") {
        setSessions((current) =>
          current.map((session) => {
            if (session.session_id !== event.session_id) return session;
            return updateSessionWorkerState(
              upsertTurn(session, event.turn),
              event.worker_id,
              "working",
              event.turn.turn_id,
            );
          }),
        );
        return;
      }
      if (event.type === "turn_updated") {
        performanceTraceRef.current.observeTurnUpdated(
          event.session_id,
          event.turn,
        );
        const consumedAttachmentIds = new Set(
          event.turn.user_entries
            .flatMap((entry) => entry.attachments ?? [])
            .map((attachment) => attachment.id),
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? {
                  ...upsertTurn(session, event.turn),
                  attachments: session.attachments.filter(
                    (attachment) => !consumedAttachmentIds.has(attachment.id),
                  ),
                }
              : session,
          ),
        );
        return;
      }
      if (event.type === "host_error") {
        clearAllPendingCommands();
        pushActivity({
          id: clientId(),
          sessionId: activeSessionIdRef.current || "system",
          tone: "error",
          title: "Runtime error",
          detail: event.message,
          createdAt: Date.now(),
        });
        return;
      }
      if (event.type === "runtime_notice") {
        pushActivity({
          id: clientId(),
          sessionId: event.session_id,
          tone:
            event.level === "error"
              ? "error"
              : event.level === "notice"
                ? "notice"
                : "warning",
          title: event.title,
          detail: event.message,
          createdAt: Date.now(),
        });
        return;
      }
      if (event.type === "host_config_updated") {
        removePendingKey(
          pendingRuntimeKeysRef,
          setPendingRuntimeKeys,
          event.key,
        );
        setServer((current) =>
          current
            ? {
                ...current,
                runtime_options: current.runtime_options.map((option) =>
                  option.key === event.key
                    ? { ...option, value: event.value }
                    : option,
                ),
                session_env_defaults: event.session_env_defaults,
              }
            : current,
        );
        const activity: Activity = {
          id: clientId(),
          sessionId: "system",
          tone: "notice",
          title: "Runtime setting updated",
          detail: `${event.key}: ${event.value}`,
          createdAt: Date.now(),
        };
        pushActivity(activity);
        return;
      }
      if (event.type === "mcp_updated") {
        pendingMcpKeysRef.current.clear();
        setPendingMcpKeys(new Set());
        setRevealedMcpSecrets({});
        setServer((current) =>
          current ? { ...current, mcp_servers: event.servers } : current,
        );
        if (event.session_id) {
          setSessions((current) =>
            current.map((session) =>
              session.session_id === event.session_id
                ? { ...session, mcp_server_ids: event.enabled_server_ids }
                : session,
            ),
          );
        } else {
          const available = new Set(
            event.servers.map((server) => server.config.id),
          );
          setSessions((current) =>
            current.map((session) => ({
              ...session,
              mcp_server_ids: session.mcp_server_ids.filter((id) =>
                available.has(id),
              ),
            })),
          );
        }
        return;
      }
      if (event.type === "mcp_server_secrets_revealed") {
        removePendingKey(
          pendingMcpKeysRef,
          setPendingMcpKeys,
          `reveal:${event.server_id}`,
        );
        setRevealedMcpSecrets((current) => ({
          ...current,
          [event.server_id]: event.values,
        }));
        return;
      }
      if (event.type === "model_endpoints_updated") {
        setServer((current) =>
          current ? { ...current, model_endpoints: event.endpoints } : current,
        );
        setEndpointEditor(null);
        setDeleteEndpointCandidate(null);
        setRevealedEndpointApiKeys({});
        setRevealedEndpointHeaders({});
        setRevealedEndpointRequestFields({});
        return;
      }
      if (event.type === "model_endpoint_secret_revealed") {
        setRevealedEndpointApiKeys((current) => ({
          ...current,
          [event.endpoint_id]: event.api_key,
        }));
        setRevealedEndpointHeaders((current) => ({
          ...current,
          [event.endpoint_id]: event.http_headers,
        }));
        setRevealedEndpointRequestFields((current) => ({
          ...current,
          [event.endpoint_id]: event.request_fields,
        }));
        return;
      }
      if (event.type === "file_uploaded") {
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? {
                  ...session,
                  attachments: [...session.attachments, event.file],
                }
              : session,
          ),
        );
        const activity: Activity = {
          id: clientId(),
          sessionId: event.session_id,
          tone: "notice",
          title: "File attached",
          detail: `${event.file.name} · ${formatBytes(event.file.bytes)}`,
          createdAt: Date.now(),
        };
        pushActivity(activity);
        return;
      }
      if (event.type === "attachment_removed") {
        removePendingKey(
          pendingAttachmentRemoveIdsRef,
          setPendingAttachmentRemoveIds,
          `${event.session_id}:${event.attachment_id}`,
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? removePendingAttachment(session, event.attachment_id)
              : session,
          ),
        );
        return;
      }
      if (event.type === "history_page") {
        removePendingKey(
          pendingHistorySessionIdsRef,
          setPendingHistorySessionIds,
          event.session_id,
        );
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? {
                  ...prependHistoryRecords(session, event.records),
                  history_before_cursor: event.before_cursor ?? null,
                  history_has_more: event.has_more,
                }
              : session,
          ),
        );
        return;
      }
      if (event.type === "tool_repo_updated") {
        const previousCount =
          toolCountBySessionRef.current.get(event.session_id) ?? 0;
        toolCountBySessionRef.current.set(event.session_id, event.tools.length);
        if (event.tools.length > previousCount) {
          setToolCountPulseSessionId(event.session_id);
          window.setTimeout(
            () =>
              setToolCountPulseSessionId((value) =>
                value === event.session_id ? "" : value,
              ),
            2400,
          );
        }
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? { ...session, tools: event.tools }
              : session,
          ),
        );
        setToolSearchResults((current) => {
          if (
            event.session_id === activeSessionIdRef.current &&
            toolSearchQueryRef.current.trim()
          )
            return current;
          return { ...current, [event.session_id]: event.tools };
        });
        const selected = selectedToolRef.current;
        if (
          selected &&
          !event.tools.some((tool) => tool.tool_id === selected.summary.tool_id)
        )
          setSelectedTool(null);
        setPendingToolRenameKeys((current) =>
          removeToolKeysForSession(current, event.session_id),
        );
        return;
      }
      if (event.type === "tool_repo_search_result") {
        if (
          event.session_id !== activeSessionIdRef.current ||
          event.query !== toolSearchQueryRef.current
        )
          return;
        setPendingToolSearchKey((key) =>
          key === `${event.session_id}:${event.query}` ? "" : key,
        );
        setToolSearchResults((current) => ({
          ...current,
          [event.session_id]: event.tools,
        }));
        const selected = selectedToolRef.current;
        if (
          selected &&
          !event.tools.some((tool) => tool.tool_id === selected.summary.tool_id)
        )
          setSelectedTool(null);
        return;
      }
      if (event.type === "tool_repo_detail") {
        if (event.session_id === activeSessionIdRef.current) {
          setPendingToolDetailKey((key) =>
            key === `${event.session_id}:${event.detail.summary.tool_id}`
              ? ""
              : key,
          );
          setSelectedTool(event.detail);
        }
        return;
      }
      if (event.type === "worker_activity") {
        const kind = String(event.event.kind ?? "worker_event");
        const turnEvent: WebTurnEvent = {
          event_id: event.turn_event_id ?? clientId(),
          source: "worker_activity",
          payload: event.event,
          created_at_ms: Date.now(),
        };
        const workerState =
          kind === "model_request"
            ? "working"
            : kind === "model_error"
              ? "error"
              : kind === "worker_stopped"
                ? "stopped"
                : kind === "subworker_turn_finished"
                  ? "ready"
                  : null;
        setSessions((current) =>
          current.map((session) => {
            if (session.session_id !== event.session_id) return session;
            const withEvent = appendTurnEvent(
              session,
              event.turn_id,
              turnEvent,
            );
            return workerState
              ? updateSessionWorkerState(
                  withEvent,
                  event.worker_id,
                  workerState,
                  event.turn_id,
                )
              : withEvent;
          }),
        );
        if (kind === "model_request") {
          setDecisions((current) =>
            clearDecisionsForWorker(current, event.session_id, event.worker_id),
          );
        }
        return;
      }
      if (event.type === "turn_finished") {
        pendingToolgenRequestsRef.current = removeToolgenRequestsForSession(
          pendingToolgenRequestsRef.current,
          event.session_id,
        );
        setPendingToolgenRequests(new Set(pendingToolgenRequestsRef.current));
        cancellingSessionIds.current.delete(event.session_id);
        cancellingSessionCommandIds.current.delete(event.session_id);
        const cancellationTimeoutId = cancellingSessionTimeouts.current.get(
          event.session_id,
        );
        if (cancellationTimeoutId !== undefined)
          window.clearTimeout(cancellationTimeoutId);
        cancellingSessionTimeouts.current.delete(event.session_id);
        setSessions((current) =>
          current.map((session) =>
            session.session_id === event.session_id
              ? finishTurn(
                  attachTurnCompletion(
                    session,
                    event.outcome.message_id,
                    event.outcome.completion ?? {},
                  ),
                  event.turn_id,
                  event.outcome.completion ?? {},
                )
              : session,
          ),
        );
        const completedKey = event.turn_id
          ? `${event.session_id}:${event.turn_id}`
          : clientId(`turn-finished-${event.session_id}`);
        setCompletedTurnsBySession((current) => ({
          ...current,
          [event.session_id]: {
            key: completedKey,
            continuation: !event.outcome.completion?.stop_reason
              ? "normal"
              : event.outcome.completion.stop_reason === "CancelledByUser"
                ? "cancelled"
                : "blocked",
          },
        }));
        return;
      }
      if (event.type !== "core_topic") return;
      const topic = event.event;
      setSessions((current) => {
        const sessionIndex = current.findIndex(
          (session) => session.session_id === topic.session_id,
        );
        if (sessionIndex < 0) return current;
        const session = current[sessionIndex];
        const nextSession = applyCoreTopicToSession(
          appendTurnEvent(session, event.turn_id, {
            event_id: event.turn_event_id ?? clientId(),
            source: "core_topic",
            payload: topic as unknown as Record<string, unknown>,
            created_at_ms: Date.now(),
          }),
          topic,
          (text) => makeMessage("assistant", text),
          event.turn_id,
        );
        if (nextSession === session) return current;
        const next = [...current];
        next[sessionIndex] = nextSession;
        return next;
      });
      const pendingDecision = requestDecision(topic, event.turn_id);
      if (pendingDecision)
        setDecisions((current) => enqueueDecision(current, pendingDecision));
      if (topic.topic.name === "core.lifecycle") {
        const worker = topic.payload.worker;
        if (worker && typeof worker === "object") {
          const item = worker as Record<string, unknown>;
          const sessionId =
            typeof item.session_id === "string"
              ? item.session_id
              : topic.session_id;
          const contextId =
            typeof item.context_id === "string"
              ? item.context_id
              : (topic.context_id ?? "context_0");
          const workerId =
            typeof item.worker_id === "string"
              ? item.worker_id
              : (topic.worker_id ?? sessionId);
          const displayName =
            typeof item.display_name === "string"
              ? item.display_name
              : sessionId;
          const ordinal = typeof item.ordinal === "number" ? item.ordinal : 0;
          setSessions((current) =>
            current.some((session) => session.session_id === sessionId)
              ? current
              : [
                  ...current,
                  {
                    session_id: sessionId,
                    display_name: displayName,
                    ordinal,
                    state: "ready",
                    current_dir: "",
                    max_llm_input_tokens:
                      typeof topic.payload.max_llm_input_tokens === "number"
                        ? topic.payload.max_llm_input_tokens
                        : 0,
                    tools: [],
                    mcp_server_ids: [],
                    contexts: [
                      {
                        context_id: contextId,
                        current_dir: "",
                        worker_ids: [workerId],
                      },
                    ],
                    workers: [
                      {
                        worker_id: workerId,
                        context_id: contextId,
                        display_name: displayName,
                        ordinal,
                        state: "ready",
                        parent_worker_id:
                          typeof item.parent_worker_id === "string"
                            ? item.parent_worker_id
                            : null,
                      },
                    ],
                    active_context_id: contextId,
                    primary_worker_id: workerId,
                    attachments: [],
                    roles: [],
                    messages: [],
                    turns: [],
                    history_before_cursor: null,
                    history_has_more: false,
                    active_turn_id: null,
                  },
                ],
          );
          setActiveSessionId((current) => current || sessionId);
        }
      }
    },
    [
      applySnapshot,
      clearAllPendingCommands,
      finishPendingSessionApiKeyCommand,
      pushActivity,
      removePendingKey,
      reportUiError,
    ],
  );

  useEffect(() => {
    activeSessionIdRef.current = activeSessionId;
    if (activeSessionId)
      performanceTraceRef.current.observeSessionPainted(activeSessionId);
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
    const searchKey = query
      ? `${activeSession.session_id}:${toolSearchQuery}`
      : "";
    setPendingToolSearchKey(searchKey);
    const timer = window.setTimeout(() => {
      if (
        !sendCommand({
          type: "tool_repo_search",
          session_id: activeSession.session_id,
          query: toolSearchQuery,
          limit: 200,
        })
      ) {
        setPendingToolSearchKey((key) => (key === searchKey ? "" : key));
        reportUiError(
          "ToolRepo search failed",
          "Reconnect to Timem Web before searching saved tools.",
          activeSession.session_id,
        );
      }
    }, 180);
    return () => window.clearTimeout(timer);
  }, [
    activeSession?.session_id,
    showToolRepo,
    toolSearchQuery,
    sendCommand,
    reportUiError,
  ]);

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
      const queryString = query.size > 0 ? `?${query.toString()}` : "";
      const ws = new WebSocket(
        `${scheme}://${window.location.host}/ws${queryString}`,
      );
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
        if (stopped) return;
        setConnected(false);
        setSnapshotReady(false);
        cancelAllPendingSessionApiKeyCommands(
          "The runtime connection closed before the credential update completed. Your input was kept; reconnect and try again.",
        );
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
            detail:
              "Timem Web lost its runtime connection. If Timem has exited, restart `timem` and reopen the authenticated URL.",
            createdAt: Date.now(),
          });
        }
        const delay = Math.min(10_000, 500 * 2 ** Math.min(nextAttempt - 1, 5));
        retryTimer = window.setTimeout(connect, delay);
      };
      ws.onerror = () => setConnected(false);
      ws.onmessage = (message) => {
        try {
          const event = JSON.parse(String(message.data)) as WireEvent;
          inboundEvents.enqueue(event, isLiveTurnProgressEvent(event));
        } catch {
          /* Ignore malformed transport data. */
        }
      };
    };
    connect();
    return () => {
      stopped = true;
      inboundEvents.dispose();
      if (retryTimer !== undefined) window.clearTimeout(retryTimer);
      cancelAllPendingSessionApiKeyCommands();
      socket.current?.close();
      socket.current = null;
    };
  }, [cancelAllPendingSessionApiKeyCommands, pushActivity, receive]);

  const sendTextForSession = useCallback(
    (
      sessionId: string,
      text: string,
      commandId?: string,
      attachmentIds?: readonly string[],
      forceSupplement = false,
      roleIds: readonly string[] = [],
      forceNewTurn = false,
    ): boolean => {
      const targetSession = sessionsRef.current.find(
        (session) => session.session_id === sessionId,
      );
      if (server && server.model_endpoints.length === 0) {
        setShowAppearance(false);
        setShowMcp(false);
        setShowToolRepo(false);
        setShowRuntime(false);
        pushActivity({
          id: clientId(),
          sessionId,
          tone: "notice",
          ...NO_MODEL_ENDPOINTS_ISSUE,
          createdAt: Date.now(),
        });
        return false;
      }
      const decision = composerSendDecision(
        targetSession,
        text,
        false,
        pendingMemSwitch,
        attachmentIds,
        forceSupplement,
        forceNewTurn,
      );
      if (decision.kind === "skip") {
        if (decision.reason === "cancelling" && targetSession) {
          pushActivity({
            id: clientId(),
            sessionId: targetSession.session_id,
            tone: "notice",
            title: "Cancellation in progress",
            detail:
              "Wait for the current turn to stop before sending another message.",
            createdAt: Date.now(),
          });
        } else if (decision.reason === "mem_switching") {
          pushActivity({
            id: clientId(),
            sessionId: targetSession?.session_id ?? "system",
            tone: "notice",
            title: "Switching mem",
            detail:
              "Wait for the new mem space to load before sending another message.",
            createdAt: Date.now(),
          });
        }
        return false;
      }
      const command =
        roleIds.length > 0
          ? { ...decision.command, role_ids: [...new Set(roleIds)] }
          : decision.command;
      if (!sendCommand(command, commandId)) {
        pushActivity({
          id: clientId(),
          sessionId: decision.command.session_id,
          tone: "error",
          title: "Runtime unavailable",
          detail:
            "Timem Web runtime is not connected. Restart timem and reopen the authenticated URL before sending another message.",
          createdAt: Date.now(),
        });
        return false;
      }
      return decision.clearDraftOnSuccess;
    },
    [pendingMemSwitch, pushActivity, sendCommand, server],
  );
  const sendText = useCallback(
    (text: string, commandId?: string) =>
      activeSession
        ? sendTextForSession(activeSession.session_id, text, commandId)
        : false,
    [activeSession, sendTextForSession],
  );

  const uploadFile = useCallback(
    async (file: File) => {
      if (!activeSession || pendingMemSwitch) return;
      if (
        !addPendingKey(
          pendingUploadSessionIdsRef,
          setPendingUploadSessionIds,
          activeSession.session_id,
        )
      ) {
        const activity: Activity = {
          id: clientId(),
          sessionId: activeSession.session_id,
          tone: "notice",
          title: "Upload already in progress",
          detail:
            "Wait for the current file upload to finish before attaching another file.",
          createdAt: Date.now(),
        };
        pushActivity(activity);
        return;
      }
      setPendingUploadFiles((current) => ({
        ...current,
        [activeSession.session_id]: { name: file.name, bytes: file.size },
      }));
      const token = queryToken();
      const form = new FormData();
      form.append("file", file);
      try {
        const params = new URLSearchParams({
          session_id: activeSession.session_id,
        });
        if (token) params.set("token", token);
        const response = await fetch(`/api/upload?${params.toString()}`, {
          method: "POST",
          body: form,
        });
        if (!response.ok)
          throw new Error(
            ((await response.json()) as { error?: string }).error ??
              "upload_failed",
          );
      } catch (error) {
        const activity: Activity = {
          id: clientId(),
          sessionId: activeSession.session_id,
          tone: "error",
          title: "File upload failed",
          detail: error instanceof Error ? error.message : "upload_failed",
          createdAt: Date.now(),
        };
        pushActivity(activity);
      } finally {
        removePendingKey(
          pendingUploadSessionIdsRef,
          setPendingUploadSessionIds,
          activeSession.session_id,
        );
        setPendingUploadFiles((current) => {
          const next = { ...current };
          delete next[activeSession.session_id];
          return next;
        });
      }
    },
    [
      activeSession,
      addPendingKey,
      pendingMemSwitch,
      pushActivity,
      removePendingKey,
      reportUiError,
    ],
  );

  const loadMoreHistory = useCallback(
    (session: Session) => {
      if (pendingMemSwitch) return;
      if (!session.history_has_more || !session.history_before_cursor) return;
      if (
        !addPendingKey(
          pendingHistorySessionIdsRef,
          setPendingHistorySessionIds,
          session.session_id,
        )
      )
        return;
      if (
        !sendCommand({
          type: "history_page",
          session_id: session.session_id,
          before_cursor: session.history_before_cursor,
          limit: STORED_HISTORY_PAGE_SIZE,
        })
      ) {
        removePendingKey(
          pendingHistorySessionIdsRef,
          setPendingHistorySessionIds,
          session.session_id,
        );
        const activity: Activity = {
          id: clientId(),
          sessionId: session.session_id,
          tone: "error",
          title: "Load history failed",
          detail: "Reconnect to Timem Web before loading earlier history.",
          createdAt: Date.now(),
        };
        pushActivity(activity);
      }
    },
    [
      addPendingKey,
      pendingMemSwitch,
      pushActivity,
      removePendingKey,
      sendCommand,
    ],
  );

  const runtimeMessages = useMemo<readonly ThreadMessageLike[]>(
    () =>
      activeMessages
        .filter(
          (message): message is ChatMessage & { role: "user" | "assistant" } =>
            message.role !== "system",
        )
        .map((message) => ({
          id: message.id,
          role: message.role,
          content: [{ type: "text" as const, text: message.text }],
        })),
    [activeMessages],
  );
  const runtimeMessageSessionId = activeSession?.session_id ?? "";
  const [auiMessageState, setAuiMessageState] = useState<{
    sessionId: string;
    messages: readonly ThreadMessageLike[];
  }>(() => ({ sessionId: runtimeMessageSessionId, messages: runtimeMessages }));
  const auiMessages =
    auiMessageState.sessionId === runtimeMessageSessionId
      ? auiMessageState.messages
      : runtimeMessages;
  const setAuiMessages = useCallback(
    (messages: readonly ThreadMessageLike[]) => {
      setAuiMessageState({ sessionId: runtimeMessageSessionId, messages });
    },
    [runtimeMessageSessionId],
  );
  useEffect(() => {
    setAuiMessageState((current) =>
      current.sessionId === runtimeMessageSessionId &&
      current.messages === runtimeMessages
        ? current
        : { sessionId: runtimeMessageSessionId, messages: runtimeMessages },
    );
  }, [runtimeMessageSessionId, runtimeMessages]);
  const cancelActiveTurn = useCallback(
    async (targetCommandId?: string) => {
      if (!activeSession || pendingMemSwitch) return;
      const authoritativeTurnId =
        activeSession.pending_turn_id ?? activeSession.active_turn_id;
      const authoritativeCommandId = turnCommandId(
        activeSession,
        authoritativeTurnId,
      );
      const cancelTargetCommandId = targetCommandId ?? authoritativeCommandId;
      const hasCancellableTurn =
        !!activeSession.pending_turn_id ||
        !!activeSession.active_turn_id ||
        activeSession.state === "working" ||
        !!cancelTargetCommandId;
      if (!hasCancellableTurn) return;
      if (cancellingSessionIds.current.has(activeSession.session_id)) return;
      const sessionId = activeSession.session_id;
      const commandId = clientId("turn-cancel");
      cancellingSessionIds.current.add(sessionId);
      cancellingSessionCommandIds.current.set(sessionId, commandId);
      const previousTimeoutId =
        cancellingSessionTimeouts.current.get(sessionId);
      if (previousTimeoutId !== undefined)
        window.clearTimeout(previousTimeoutId);
      const timeoutId = window.setTimeout(() => {
        if (cancellingSessionCommandIds.current.get(sessionId) !== commandId)
          return;
        cancellingSessionTimeouts.current.delete(sessionId);
        // This timer is transport bookkeeping only. It must not change the
        // Session or Turn presentation; only a Host projection may do that.
      }, TURN_CANCEL_UI_TIMEOUT_MS);
      cancellingSessionTimeouts.current.set(sessionId, timeoutId);
      if (
        !sendCommand(
          {
            type: "turn_cancel",
            session_id: sessionId,
            ...(cancelTargetCommandId
              ? { target_command_id: cancelTargetCommandId }
              : {}),
          },
          commandId,
        )
      ) {
        window.clearTimeout(timeoutId);
        cancellingSessionTimeouts.current.delete(sessionId);
        cancellingSessionCommandIds.current.delete(sessionId);
        cancellingSessionIds.current.delete(sessionId);
        const activity: Activity = {
          id: clientId(),
          sessionId,
          tone: "error",
          title: "Cancel failed",
          detail: "Reconnect to Timem Web before cancelling this turn.",
          createdAt: Date.now(),
        };
        pushActivity(activity);
      }
    },
    [activeSession, pendingMemSwitch, pushActivity, sendCommand],
  );
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

  const activeSessionKey = activeSession?.session_id ?? "";
  const sessionDecisions = useMemo(
    () =>
      decisions.filter(
        (decision) => decision.event.session_id === activeSessionKey,
      ),
    [activeSessionKey, decisions],
  );
  const runtimeDisconnected = runtimeEverConnected && !connected;
  const runtimeUnavailable = runtimeDisconnected && reconnectAttempt >= 3;
  const runtimeDisconnectedTitle = runtimeUnavailable
    ? "Runtime unavailable"
    : "Connection lost";
  const runtimeDisconnectedDetail = runtimeUnavailable
    ? "Restart timem and reopen the authenticated URL to continue."
    : "Reconnecting to Timem runtime… sending and session changes are paused until it reconnects.";
  const showRuntimeUnavailableDialog =
    runtimeUnavailable && !runtimeUnavailableDialogDismissed;
  useEffect(() => {
    if (!runtimeUnavailable) setRuntimeUnavailableDialogDismissed(false);
  }, [runtimeUnavailable]);
  const sessionInteractionLockReason = sessionInteractionLockReasonForState(
    pendingMemSwitch,
    connected,
    runtimeEverConnected,
    reconnectAttempt,
  );
  const runtimeReady = connected && snapshotReady;
  const runtimeLocked = pendingMemSwitch || !runtimeReady;
  const connectionLabel = runtimeConnectionLabel(
    connected,
    snapshotReady,
    runtimeEverConnected,
    reconnectAttempt,
  );
  const settingsTitle = !runtimeReady
    ? "Wait for the runtime snapshot before opening settings"
    : pendingMemSwitch
      ? "Memory switch is in progress"
      : "Open settings";
  const newSessionLabel = runtimeLocked
    ? "Session controls are temporarily locked"
    : "New session";
  const modelEndpointsUnavailable =
    !!server && server.model_endpoints.length === 0;
  const headerModelLabel =
    endpointNameForProfile(
      server?.model_endpoints ?? [],
      activeSession?.runtime_profile,
    ) ?? UNCONFIGURED_MODEL_LABEL;
  const openEndpointSettings = () => {
    setShowRuntime(false);
    setShowMcp(false);
    setShowToolRepo(false);
    setSettingsSection("endpoints");
    setEndpointEditor(null);
    setShowAppearance(true);
  };
  const openEndpointCreator = () => {
    openEndpointSettings();
    setEndpointEditor("new");
  };
  const runtimeLabel = showRuntime
    ? "Close runtime information"
    : "Open runtime information";
  const activeToolCount = activeSession?.tools.length ?? 0;
  const activeMcpServerIds = new Set(activeSession?.mcp_server_ids ?? []);
  const activeMcpServers = (server?.mcp_servers ?? []).filter((item) =>
    activeMcpServerIds.has(item.config.id),
  );
  const connectedMcpCount = activeMcpServers.filter(
    (item) => item.state === "connected",
  ).length;
  const failedMcpCount = activeMcpServers.filter(
    (item) =>
      item.state !== "connected" && (item.state === "error" || !!item.error),
  ).length;
  const mcpLabel = `Manage MCP servers · ${connectedMcpCount} connected${failedMcpCount ? ` · ${failedMcpCount} failed` : ""}`;
  const selectedRoleIdsForSession = activeSession
    ? (selectedRoleIds[activeSession.session_id] ?? [])
    : [];
  const activePendingToolGenTurnIds = useMemo(
    () =>
      activeSessionKey
        ? pendingToolgenTurnIds(pendingToolgenRequests, activeSessionKey)
        : new Set<string>(),
    [activeSessionKey, pendingToolgenRequests],
  );
  const activeToolGenBusy = useMemo(
    () =>
      !!activeSessionKey &&
      hasPendingToolgenForSession(pendingToolgenRequests, activeSessionKey),
    [activeSessionKey, pendingToolgenRequests],
  );
  const replyToDecision = useCallback(
    (
      decision: Decision,
      decisionValue: "accept" | "decline" | "always_allow",
    ) => {
      if (runtimeLocked) return;
      const key = decisionKey(decision);
      if (!addPendingKey(pendingDecisionKeysRef, setPendingDecisionKeys, key))
        return;
      const event = decision.event;
      if (
        sendCommand({
          type: "topic_reply",
          session_id: event.session_id,
          worker_id: event.worker_id ?? undefined,
          topic_name: event.topic.name,
          request_id:
            typeof event.payload.request_id === "string"
              ? event.payload.request_id
              : undefined,
          decision: decisionValue,
          payload: { summary: decision.detail },
        })
      ) {
        setDecisions((current) =>
          current.filter((candidate) => candidate !== decision),
        );
      } else {
        removePendingKey(pendingDecisionKeysRef, setPendingDecisionKeys, key);
        reportUiError(
          "Decision reply failed",
          "Reconnect to Timem Web before replying to this runtime request.",
          event.session_id,
        );
      }
    },
    [
      addPendingKey,
      removePendingKey,
      reportUiError,
      runtimeLocked,
      sendCommand,
    ],
  );
  const requestActiveToolGen = useCallback(
    (turnId: string) => {
      if (
        !toolGenEnabled ||
        !activeSessionKey ||
        activeSession?.state === "working" ||
        runtimeLocked ||
        activeToolGenBusy
      )
        return;
      setToolgenDialog({ sessionId: activeSessionKey, turnId });
    },
    [
      activeSession?.state,
      activeSessionKey,
      activeToolGenBusy,
      runtimeLocked,
      toolGenEnabled,
    ],
  );
  const toolRepoLabel = showToolRepo
    ? "Close ToolRepo"
    : `Open ToolRepo · ${activeToolCount} reusable tools`;
  const mobileSessionsLabel = showMobileSessions
    ? "Close session navigation"
    : "Open session navigation";
  const knownSessionGroupIds = new Set(sessionGroups.map((group) => group.id));
  const sessionBucketId = (session: Session) =>
    session.group_id && knownSessionGroupIds.has(session.group_id)
      ? session.group_id
      : "__ungrouped";
  const ungroupedSessions = sessions.filter(
    (session) => sessionBucketId(session) === "__ungrouped",
  );
  const sessionBuckets = [
    ...sessionGroups.map((group) => ({
      id: group.id,
      group,
      sessions: sessions.filter((session) => session.group_id === group.id),
    })),
    { id: "__ungrouped", group: undefined, sessions: ungroupedSessions },
  ];
  const sessionDragSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  useEffect(() => {
    if (
      selectedDeleteSessionId &&
      !sessions.some(
        (session) => session.session_id === selectedDeleteSessionId,
      )
    ) {
      setSelectedDeleteSessionId("");
    }
    if (sessionDeleteMode && sessions.length === 0) setSessionDeleteMode(false);
  }, [selectedDeleteSessionId, sessionDeleteMode, sessions]);
  const draggedSession = sessions.find(
    (session) => session.session_id === draggedSessionId,
  );
  const finishSessionDrag = (event: DragEndEvent) => {
    setDraggedSessionId("");
    const sessionId = String(event.active.id).replace(/^session:/, "");
    const overId = event.over ? String(event.over.id) : "";
    if (!overId) return;
    let targetGroupId: string | null | undefined;
    if (overId.startsWith("session-group:")) {
      const bucketId = overId.slice("session-group:".length);
      targetGroupId = bucketId === "__ungrouped" ? null : bucketId;
    } else if (overId.startsWith("session:")) {
      const targetSession = sessions.find(
        (session) => session.session_id === overId.slice("session:".length),
      );
      if (targetSession)
        targetGroupId =
          sessionBucketId(targetSession) === "__ungrouped"
            ? null
            : sessionBucketId(targetSession);
    }
    const session = sessions.find(
      (candidate) => candidate.session_id === sessionId,
    );
    if (
      !session ||
      targetGroupId === undefined ||
      (session.group_id ?? null) === targetGroupId
    )
      return;
    sendCommand({
      type: "session_group_move",
      session_id: sessionId,
      group_id: targetGroupId,
    });
  };
  const saveSessionGroup = (editor = sessionGroupEditor) => {
    const name = editor?.name.trim();
    if (!editor || !name || runtimeLocked) return;
    const command: ClientCommand = editor.id
      ? { type: "session_group_update", group_id: editor.id, name }
      : { type: "session_group_create", name };
    if (sendCommand(command)) setSessionGroupEditor(null);
  };
  const moveSessionGroup = (groupId: string, offset: number) => {
    const index = sessionGroups.findIndex((group) => group.id === groupId);
    const target = index + offset;
    if (index < 0 || target < 0 || target >= sessionGroups.length) return;
    const groups = [...sessionGroups];
    [groups[index], groups[target]] = [groups[target], groups[index]];
    setSessionGroups(groups);
    if (!sendCommand({ type: "session_groups_reorder", groups }))
      setSessionGroups(sessionGroups);
  };
  const cancelSessionDeleteMode = () => {
    setSessionDeleteMode(false);
    setSelectedDeleteSessionId("");
  };
  const confirmSelectedSessionDelete = () => {
    const session = sessions.find(
      (candidate) => candidate.session_id === selectedDeleteSessionId,
    );
    if (!session) return;
    setDeleteSessionCandidate(session);
    cancelSessionDeleteMode();
    closeMobileSidebar(false);
  };
  useEffect(() => {
    if (!memTemporaryItemsLoading) return;
    const timeoutId = window.setTimeout(() => {
      memTemporaryItemsLoadedForRef.current = "";
      setMemTemporaryItemsLoading(false);
      setMemTemporaryItemsError(
        "Temporary files took too long to load. Select Refresh to try again.",
      );
    }, 15_000);
    return () => window.clearTimeout(timeoutId);
  }, [memTemporaryItemsLoading]);
  useEffect(() => {
    const memPath = server?.mem.space_dir ?? "";
    if (
      !showAppearance ||
      settingsSection !== "memory" ||
      !runtimeReady ||
      !memPath
    )
      return;
    if (memTemporaryItemsLoadedForRef.current === memPath) return;
    memTemporaryItemsLoadedForRef.current = memPath;
    setMemTemporaryItemsLoading(true);
    setMemTemporaryItemsError("");
    if (!sendCommand({ type: "mem_temporary_items_list" })) {
      memTemporaryItemsLoadedForRef.current = "";
      setMemTemporaryItemsLoading(false);
    }
  }, [
    runtimeReady,
    sendCommand,
    server?.mem.space_dir,
    settingsSection,
    showAppearance,
  ]);
  const leftSidebarCollapsed =
    sidebarLayout.leftCollapsed && !showMobileSessions;
  const rightSidebarCollapsed = sidebarLayout.rightCollapsed && !showRoles;
  const workspaceModalOpen = showAppearance || chatLibraryMode !== null;
  useEffect(() => {
    document.body.classList.toggle("workspace-modal-open", workspaceModalOpen);
    return () => document.body.classList.remove("workspace-modal-open");
  }, [workspaceModalOpen]);
  return (
    <AssistantRuntimeProvider runtime={runtime}>
      <div
        inert={workspaceModalOpen}
        aria-hidden={workspaceModalOpen || undefined}
        className={`app-shell ${leftSidebarCollapsed ? "left-sidebar-collapsed" : ""} ${!showToolRepo && rightSidebarCollapsed ? "right-sidebar-collapsed" : ""}`}
        style={
          {
            "--left-sidebar-width": `${sidebarLayout.leftWidth}px`,
            "--right-sidebar-width": `${sidebarLayout.rightWidth}px`,
          } as CSSProperties
        }
      >
        {showMobileSessions && (
          <button
            type="button"
            className="mobile-sidebar-backdrop"
            aria-label="Close session navigation"
            onClick={() => closeMobileSidebar()}
          />
        )}
        <aside
          id="session-navigation"
          ref={mobileSidebarRef}
          className={`sidebar ${leftSidebarCollapsed ? "collapsed" : ""} ${showMobileSessions ? "mobile-open" : ""}`}
          aria-label="Session navigation"
          tabIndex={-1}
        >
          {leftSidebarCollapsed && (
            <button
              type="button"
              className="collapsed-brand brand-logo-toggle brand-logo-restore"
              title="Show session navigation"
              aria-label="Show session navigation"
              onClick={() =>
                setSidebarLayout((current) => ({
                  ...current,
                  leftCollapsed: false,
                }))
              }
            >
              <img src="/timem_logo.png" alt="" className="brand-logo" />
              <span
                className="brand-scale-corner top-left"
                aria-hidden="true"
              />
              <span
                className="brand-scale-corner bottom-right"
                aria-hidden="true"
              />
            </button>
          )}
          {!leftSidebarCollapsed && (
            <button
              type="button"
              className="sidebar-resize-handle left"
              title="Resize session navigation"
              aria-label="Resize session navigation"
              onPointerDown={(event) => startSidebarResize("left", event)}
            />
          )}
          <div className="brand">
            <button
              type="button"
              className="brand-logo-toggle"
              title="Hide session navigation"
              aria-label="Hide session navigation"
              onClick={() =>
                setSidebarLayout((current) => ({
                  ...current,
                  leftCollapsed: true,
                }))
              }
            >
              <img
                src="/timem_logo.png"
                alt="Timem logo"
                className="brand-logo"
              />
              <span
                className="brand-scale-corner top-right"
                aria-hidden="true"
              />
              <span
                className="brand-scale-corner bottom-left"
                aria-hidden="true"
              />
            </button>
            <span>TIMEM</span>
            <button
              type="button"
              className="mobile-sidebar-close"
              title="Close sessions"
              aria-label="Close sessions"
              onClick={() => closeMobileSidebar()}
            >
              <X size={17} />
            </button>
          </div>
          <div
            className={`session-management-actions ${sessionDeleteMode ? "deleting" : ""}`}
          >
            <div className="session-create-actions">
              <button
                type="button"
                className="new-session-group"
                title="New session group"
                aria-label="New session group"
                disabled={runtimeLocked || sessionDeleteMode}
                onClick={() => setSessionGroupEditor({ name: "" })}
              >
                <FolderPlus size={16} />
              </button>
              <button
                type="button"
                ref={newSessionButtonRef}
                className="new-session"
                title={newSessionLabel}
                aria-label={newSessionLabel}
                disabled={runtimeLocked || sessionDeleteMode}
                onClick={() => {
                  setShowNewSession(true);
                  closeMobileSidebar(false);
                }}
              >
                <Plus size={16} />
              </button>
            </div>
            <div className="session-delete-actions">
              {sessionDeleteMode && (
                <button
                  type="button"
                  className="session-delete-cancel"
                  title="取消删除 Session"
                  aria-label="取消删除 Session"
                  onClick={cancelSessionDeleteMode}
                >
                  <X size={14} strokeWidth={3} />
                </button>
              )}
              <button
                type="button"
                className={`session-delete-manage ${sessionDeleteMode ? "confirm" : ""}`}
                title={
                  sessionDeleteMode
                    ? selectedDeleteSessionId
                      ? "确认删除选中的 Session"
                      : "请选择要删除的 Session"
                    : "选择要删除的 Session"
                }
                aria-label={
                  sessionDeleteMode
                    ? selectedDeleteSessionId
                      ? "确认删除选中的 Session"
                      : "请选择要删除的 Session"
                    : "选择要删除的 Session"
                }
                disabled={
                  runtimeLocked ||
                  sessions.length === 0 ||
                  (sessionDeleteMode && !selectedDeleteSessionId)
                }
                onClick={() => {
                  if (!sessionDeleteMode) {
                    setSessionGroupEditor(null);
                    setRenamingSessionId("");
                    setRenameDraft("");
                    setSessionDeleteMode(true);
                    setSelectedDeleteSessionId("");
                  } else confirmSelectedSessionDelete();
                }}
              >
                {sessionDeleteMode ? (
                  <Check size={15} strokeWidth={3} />
                ) : (
                  <Trash2 size={15} />
                )}
              </button>
            </div>
          </div>
          {sessionGroupEditor && !sessionGroupEditor.id && (
            <form
              className="session-group-editor"
              onSubmit={(event) => {
                event.preventDefault();
                saveSessionGroup();
              }}
            >
              <input
                autoFocus
                value={sessionGroupEditor.name}
                placeholder="Group name"
                aria-label="New session group name"
                onChange={(event) =>
                  setSessionGroupEditor({ name: event.target.value })
                }
                onKeyDown={(event) => {
                  if (event.key === "Escape") {
                    event.preventDefault();
                    setSessionGroupEditor(null);
                  }
                }}
              />
              <button type="submit" disabled={!sessionGroupEditor.name.trim()}>
                <Check size={13} />
              </button>
              <button type="button" onClick={() => setSessionGroupEditor(null)}>
                <X size={13} />
              </button>
            </form>
          )}
          <DndContext
            sensors={sessionDragSensors}
            collisionDetection={closestCenter}
            onDragStart={(event) =>
              setDraggedSessionId(
                String(event.active.id).replace(/^session:/, ""),
              )
            }
            onDragCancel={() => setDraggedSessionId("")}
            onDragEnd={finishSessionDrag}
          >
            <nav
              className="session-list"
              aria-label="Sessions"
              aria-busy={!snapshotReady}
            >
              {!snapshotReady ? (
                <div
                  className="session-list-loading"
                  role="status"
                  aria-live="polite"
                >
                  <LoaderCircle size={18} aria-hidden="true" />
                  <span>Loading sessions…</span>
                </div>
              ) : (
                sessionBuckets.map(
                  ({
                    id: bucketId,
                    group: bucket,
                    sessions: bucketSessions,
                  }) => {
                    const collapsed = collapsedSessionGroupIds.has(bucketId);
                    return (
                      <SessionDropGroup
                        id={bucketId}
                        sessionIds={bucketSessions.map(
                          (session) => session.session_id,
                        )}
                        className="session-group"
                        key={bucketId}
                      >
                        <div className="session-group-heading">
                          <button
                            type="button"
                            className="session-group-toggle"
                            title={bucket?.name ?? "Unsorted"}
                            aria-expanded={!collapsed}
                            onClick={() =>
                              setCollapsedSessionGroupIds((current) => {
                                const next = new Set(current);
                                if (next.has(bucketId)) next.delete(bucketId);
                                else next.add(bucketId);
                                return next;
                              })
                            }
                          >
                            {collapsed ? (
                              <Folder
                                className="session-group-folder"
                                size={14}
                              />
                            ) : (
                              <FolderOpen
                                className="session-group-folder"
                                size={14}
                              />
                            )}
                            <span>{bucket?.name ?? "Unsorted"}</span>
                            <small>{bucketSessions.length}</small>
                          </button>
                          {bucket && (
                            <div className="session-group-actions">
                              {sessionGroupEditor?.id === bucket.id ? (
                                <form
                                  className="session-group-editor inline"
                                  onSubmit={(event) => {
                                    event.preventDefault();
                                    saveSessionGroup();
                                  }}
                                >
                                  <input
                                    autoFocus
                                    value={sessionGroupEditor.name}
                                    aria-label={`Rename ${bucket.name}`}
                                    onChange={(event) =>
                                      setSessionGroupEditor({
                                        id: bucket.id,
                                        name: event.target.value,
                                      })
                                    }
                                    onKeyDown={(event) => {
                                      if (event.key === "Escape") {
                                        event.preventDefault();
                                        setSessionGroupEditor(null);
                                      }
                                    }}
                                  />
                                  <button
                                    type="submit"
                                    disabled={!sessionGroupEditor.name.trim()}
                                  >
                                    <Check size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    onClick={() => setSessionGroupEditor(null)}
                                  >
                                    <X size={12} />
                                  </button>
                                </form>
                              ) : (
                                <>
                                  <button
                                    type="button"
                                    title="Move group up"
                                    aria-label={`Move ${bucket.name} up`}
                                    disabled={
                                      sessionDeleteMode ||
                                      sessionGroups[0]?.id === bucket.id
                                    }
                                    onClick={() =>
                                      moveSessionGroup(bucket.id, -1)
                                    }
                                  >
                                    <ChevronUp size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    title="Move group down"
                                    aria-label={`Move ${bucket.name} down`}
                                    disabled={
                                      sessionDeleteMode ||
                                      sessionGroups.at(-1)?.id === bucket.id
                                    }
                                    onClick={() =>
                                      moveSessionGroup(bucket.id, 1)
                                    }
                                  >
                                    <ChevronDown size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    title={`Rename ${bucket.name}`}
                                    aria-label={`Rename ${bucket.name}`}
                                    disabled={sessionDeleteMode}
                                    onClick={() =>
                                      setSessionGroupEditor({
                                        id: bucket.id,
                                        name: bucket.name,
                                      })
                                    }
                                  >
                                    <Pencil size={12} />
                                  </button>
                                  <button
                                    type="button"
                                    className={
                                      sessionGroupDeleteConfirmId === bucket.id
                                        ? "confirming"
                                        : ""
                                    }
                                    disabled={sessionDeleteMode}
                                    title={
                                      sessionGroupDeleteConfirmId === bucket.id
                                        ? "Click again to delete; sessions become unsorted"
                                        : `Delete ${bucket.name}`
                                    }
                                    aria-label={`Delete ${bucket.name}`}
                                    onClick={() => {
                                      if (
                                        sessionGroupDeleteConfirmId ===
                                        bucket.id
                                      ) {
                                        sendCommand({
                                          type: "session_group_delete",
                                          group_id: bucket.id,
                                        });
                                        setSessionGroupDeleteConfirmId("");
                                      } else
                                        setSessionGroupDeleteConfirmId(
                                          bucket.id,
                                        );
                                    }}
                                  >
                                    <Trash2 size={12} />
                                  </button>
                                </>
                              )}
                            </div>
                          )}
                        </div>
                        {!collapsed && (
                          <div className="session-group-list">
                            {bucketSessions.map((session) => {
                              const renamingSession =
                                pendingRenameSessionIds.has(session.session_id);
                              const deletingSession =
                                pendingDeleteSessionIds.has(session.session_id);
                              const visuallyWorking =
                                sessionVisuallyWorking(session);
                              const sessionEndpointName =
                                endpointNameForProfile(
                                  server?.model_endpoints ?? [],
                                  session.runtime_profile,
                                ) ?? UNCONFIGURED_MODEL_LABEL;
                              return (
                                <SortableSession
                                  id={session.session_id}
                                  disabled={runtimeLocked || sessionDeleteMode}
                                  key={session.session_id}
                                >
                                  {({
                                    setNodeRef,
                                    style,
                                    attributes,
                                    listeners,
                                    isDragging,
                                  }) => (
                                    <>
                                      <div
                                        ref={setNodeRef}
                                        style={style}
                                        className={`session-row ${server?.debug_mode && session.workers.length > 0 ? "has-workers" : ""} ${session.session_id === activeSession?.session_id ? "active" : ""} ${visuallyWorking ? "working" : ""} ${unreadCompletedSessionIds.has(session.session_id) ? "has-unread-completion" : ""} ${renamingSession ? "renaming-session" : ""} ${renamingSessionId === session.session_id || runtimeLocked || isDragging ? "controls-suppressed" : ""} ${sessionDeleteMode ? "delete-selecting" : ""} ${selectedDeleteSessionId === session.session_id ? "delete-selected" : ""} ${isDragging ? "dragging" : ""}`}
                                        aria-busy={
                                          renamingSession ||
                                          deletingSession ||
                                          undefined
                                        }
                                      >
                                        <button
                                          type="button"
                                          className="session-drag"
                                          disabled={
                                            runtimeLocked ||
                                            sessionDeleteMode ||
                                            renamingSessionId ===
                                              session.session_id
                                          }
                                          title={`拖动 ${session.display_name} 到其他分组`}
                                          aria-label={`拖动 ${session.display_name} 到其他分组`}
                                          {...attributes}
                                          {...listeners}
                                        >
                                          <GripVertical size={13} />
                                        </button>
                                        {server?.debug_mode && (
                                          <button
                                            type="button"
                                            className={`session-expand ${session.workers.length > 0 ? "available" : ""} ${expandedSessionIds.has(session.session_id) ? "expanded" : ""}`}
                                            title={
                                              runtimeLocked
                                                ? "Session controls are temporarily locked"
                                                : session.workers.length === 0
                                                  ? "No workers in this session"
                                                  : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers`
                                            }
                                            aria-label={
                                              runtimeLocked
                                                ? `Workers locked while the runtime synchronizes for ${session.display_name}`
                                                : session.workers.length === 0
                                                  ? `No workers for ${session.display_name}`
                                                  : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers for ${session.display_name}`
                                            }
                                            aria-expanded={
                                              session.workers.length > 0 &&
                                              expandedSessionIds.has(
                                                session.session_id,
                                              )
                                            }
                                            disabled={
                                              runtimeLocked ||
                                              sessionDeleteMode ||
                                              renamingSessionId ===
                                                session.session_id ||
                                              session.workers.length === 0
                                            }
                                            onClick={() =>
                                              setExpandedSessionIds(
                                                (current) => {
                                                  const next = new Set(current);
                                                  if (
                                                    next.has(session.session_id)
                                                  )
                                                    next.delete(
                                                      session.session_id,
                                                    );
                                                  else
                                                    next.add(
                                                      session.session_id,
                                                    );
                                                  return next;
                                                },
                                              )
                                            }
                                          >
                                            <ChevronRight size={13} />
                                          </button>
                                        )}
                                        {renamingSessionId ===
                                        session.session_id ? (
                                          <input
                                            className="session-rename-input"
                                            autoFocus
                                            value={renameDraft}
                                            aria-label={`Rename ${session.display_name}`}
                                            disabled={runtimeLocked}
                                            onChange={(event) =>
                                              setRenameDraft(event.target.value)
                                            }
                                            onBlur={() =>
                                              finishRename(session.session_id)
                                            }
                                            onKeyDown={(event) => {
                                              if (
                                                event.key === "Enter" &&
                                                !event.nativeEvent.isComposing
                                              ) {
                                                event.preventDefault();
                                                finishRename(
                                                  session.session_id,
                                                );
                                              }
                                              if (event.key === "Escape") {
                                                event.preventDefault();
                                                setRenamingSessionId("");
                                                setRenameDraft("");
                                              }
                                            }}
                                          />
                                        ) : (
                                          <button
                                            type="button"
                                            className={`session ${session.session_id === activeSession?.session_id ? "active" : ""}`}
                                            title={
                                              runtimeLocked
                                                ? "Session controls are temporarily locked"
                                                : session.display_name
                                            }
                                            aria-label={
                                              runtimeLocked
                                                ? `${session.display_name} locked while the runtime synchronizes`
                                                : renamingSession
                                                  ? `${session.display_name} rename is being saved`
                                                  : undefined
                                            }
                                            aria-current={
                                              session.session_id ===
                                              activeSession?.session_id
                                                ? "page"
                                                : undefined
                                            }
                                            disabled={runtimeLocked}
                                            onClick={() => {
                                              if (sessionDeleteMode) {
                                                setSelectedDeleteSessionId(
                                                  (current) =>
                                                    current ===
                                                    session.session_id
                                                      ? ""
                                                      : session.session_id,
                                                );
                                                return;
                                              }
                                              performanceTraceRef.current.beginSessionSelection(
                                                session.session_id,
                                              );
                                              if (
                                                session.session_id ===
                                                activeSessionIdRef.current
                                              )
                                                performanceTraceRef.current.observeSessionPainted(
                                                  session.session_id,
                                                );
                                              setActiveSessionId(
                                                session.session_id,
                                              );
                                              closeMobileSidebar();
                                            }}
                                          >
                                            {visuallyWorking ? (
                                              <LoaderCircle
                                                className="session-working-icon"
                                                size={15}
                                                aria-label="Session working"
                                              />
                                            ) : session.state ===
                                              "interrupted" ? (
                                              <CircleStop
                                                className="session-interrupted-icon"
                                                size={15}
                                                aria-label="Session interrupted by runtime restart"
                                              />
                                            ) : unreadCompletedSessionIds.has(
                                                session.session_id,
                                              ) ? (
                                              <span
                                                className="session-unread-dot"
                                                aria-label="Session has new completed work"
                                              />
                                            ) : null}
                                            <span className="session-identity">
                                              <span
                                                className="session-name"
                                                title={session.display_name}
                                                onDoubleClick={() => {
                                                  if (
                                                    !runtimeLocked &&
                                                    !sessionDeleteMode &&
                                                    renamingSessionId !==
                                                      session.session_id
                                                  )
                                                    beginRename(session);
                                                }}
                                              >
                                                {session.display_name}
                                              </span>
                                            </span>
                                            <span
                                              className={`session-endpoint-reveal ${renamingSession ? "pending" : ""}`}
                                              title={
                                                renamingSession
                                                  ? "Saving name"
                                                  : sessionEndpointName
                                              }
                                            >
                                              {renamingSession ? (
                                                <span className="session-pending">
                                                  Saving name...
                                                </span>
                                              ) : (
                                                <span>
                                                  {sessionEndpointName}
                                                </span>
                                              )}
                                            </span>
                                            <span className="sr-only">
                                              Session state: {session.state}
                                            </span>
                                          </button>
                                        )}
                                        {sessionDeleteMode && (
                                          <button
                                            type="button"
                                            className={`session-delete-select ${selectedDeleteSessionId === session.session_id ? "selected" : ""}`}
                                            title={`选择删除 ${session.display_name}`}
                                            aria-label={`选择删除 ${session.display_name}`}
                                            aria-pressed={
                                              selectedDeleteSessionId ===
                                              session.session_id
                                            }
                                            disabled={
                                              runtimeLocked || deletingSession
                                            }
                                            onClick={() =>
                                              setSelectedDeleteSessionId(
                                                (current) =>
                                                  current === session.session_id
                                                    ? ""
                                                    : session.session_id,
                                              )
                                            }
                                          >
                                            {deletingSession ? (
                                              <LoaderCircle size={14} />
                                            ) : selectedDeleteSessionId ===
                                              session.session_id ? (
                                              <Check size={14} />
                                            ) : null}
                                          </button>
                                        )}
                                      </div>
                                      {server?.debug_mode &&
                                        session.workers.length > 0 &&
                                        expandedSessionIds.has(
                                          session.session_id,
                                        ) && (
                                          <div
                                            className="worker-list"
                                            role="tree"
                                            aria-label={`Workers for ${session.display_name}: ${session.workers.length} worker${session.workers.length === 1 ? "" : "s"}`}
                                          >
                                            {sessionWorkerTreeRows(
                                              session.workers,
                                            ).map(
                                              ({ worker, depth, isLast }) => {
                                                const workerName =
                                                  worker.display_name ||
                                                  `ID${worker.ordinal}`;
                                                return (
                                                  <div
                                                    className={`worker-row ${depth > 0 ? "child-worker" : "root-worker"} ${isLast ? "last-child" : ""}`}
                                                    role="treeitem"
                                                    aria-level={depth + 1}
                                                    key={worker.worker_id}
                                                    title={`${workerName} · level ${depth + 1} · ${worker.worker_id} · ${worker.context_id}`}
                                                    style={
                                                      {
                                                        "--worker-depth": depth,
                                                      } as CSSProperties
                                                    }
                                                  >
                                                    <span
                                                      className="worker-relation"
                                                      aria-hidden="true"
                                                    >
                                                      <span />
                                                    </span>
                                                    {worker.state ===
                                                    "working" ? (
                                                      <LoaderCircle
                                                        className="worker-working-icon"
                                                        size={12}
                                                        aria-label={`${workerName} working`}
                                                      />
                                                    ) : (
                                                      <span
                                                        className="worker-idle-spacer"
                                                        aria-hidden="true"
                                                      />
                                                    )}
                                                    <span
                                                      className="worker-name"
                                                      title={workerName}
                                                    >
                                                      {workerName}
                                                    </span>
                                                  </div>
                                                );
                                              },
                                            )}
                                          </div>
                                        )}
                                    </>
                                  )}
                                </SortableSession>
                              );
                            })}
                            {bucketSessions.length === 0 && (
                              <div className="session-group-drop-hint">
                                拖动Session以归组
                              </div>
                            )}
                          </div>
                        )}
                      </SessionDropGroup>
                    );
                  },
                )
              )}
            </nav>
            <DragOverlay
              dropAnimation={
                prefersReducedMotion()
                  ? null
                  : { duration: 180, easing: "cubic-bezier(.2, .8, .2, 1)" }
              }
            >
              {draggedSession && (
                <div className="session-row session-overlay" aria-hidden="true">
                  <span className="session-drag">
                    <GripVertical size={13} />
                  </span>
                  <span className="session-name">
                    {draggedSession.display_name}
                  </span>
                </div>
              )}
            </DragOverlay>
          </DndContext>
          <div className="sidebar-footer">
            <button
              type="button"
              className={`sidebar-library-button ${chatLibraryMode === "search" ? "active" : ""}`}
              title="Search chats"
              aria-label="Search chats"
              aria-expanded={chatLibraryMode === "search"}
              aria-controls="chat-library-center"
              disabled={!runtimeReady || pendingMemSwitch}
              onClick={(event) => {
                chatLibraryTriggerRef.current = event.currentTarget;
                setShowAppearance(false);
                setShowToolRepo(false);
                setShowRoles(false);
                const opening = chatLibraryMode !== "search";
                setChatLibraryMode(opening ? "search" : null);
                if (opening) setChatSearchScope("all");
              }}
            >
              <Search size={17} aria-hidden="true" />
              {!leftSidebarCollapsed && <span>Search</span>}
            </button>
            <button
              type="button"
              className={`sidebar-library-button ${chatLibraryMode === "favorites" ? "active" : ""}`}
              title="Favorite answers"
              aria-label="Favorite answers"
              aria-expanded={chatLibraryMode === "favorites"}
              aria-controls="chat-library-center"
              disabled={!runtimeReady || pendingMemSwitch}
              onClick={(event) => {
                chatLibraryTriggerRef.current = event.currentTarget;
                setShowAppearance(false);
                setShowToolRepo(false);
                setShowRoles(false);
                const opening = chatLibraryMode !== "favorites";
                setChatLibraryMode(opening ? "favorites" : null);
                if (opening) {
                  setChatSearchScope("favorites");
                  setFavoritesLoading(true);
                  if (!sendCommand({ type: "favorites_list" }))
                    setFavoritesLoading(false);
                }
              }}
            >
              <Star size={17} aria-hidden="true" />
              {!leftSidebarCollapsed && <span>Favorite</span>}
            </button>
            <button
              type="button"
              ref={settingsButtonRef}
              className={`sidebar-settings-button ${showAppearance ? "active" : ""}`}
              title={settingsTitle}
              aria-label={settingsTitle}
              aria-expanded={showAppearance}
              aria-controls="settings-center"
              disabled={!runtimeReady || pendingMemSwitch}
              onClick={() => {
                setChatLibraryMode(null);
                setSettingsSection("appearance");
                setShowAppearance(true);
              }}
            >
              <Settings size={17} aria-hidden="true" />
              {!leftSidebarCollapsed && <span>Settings</span>}
            </button>
          </div>
        </aside>
        <main className="chat-shell">
          <header className="chat-header">
            <div className="header-context-actions">
              <HeaderContextUsage session={activeSession} />
              <button
                type="button"
                ref={mcpButtonRef}
                title={mcpLabel}
                aria-label={mcpLabel}
                className={`icon-button mcp-button ${connectedMcpCount > 0 ? "enabled" : ""} ${failedMcpCount > 0 ? "has-failures" : ""} ${showMcp ? "selected" : ""}`}
                aria-expanded={showMcp}
                aria-controls="mcp-panel"
                onClick={() => {
                  setShowAppearance(false);
                  setShowRuntime(false);
                  setShowToolRepo(false);
                  if (showMcp) closeMcpPanel();
                  else setShowMcp(true);
                }}
              >
                <Plug size={16} />
                {connectedMcpCount > 0 && (
                  <span
                    className="mcp-count mcp-count-connected"
                    aria-hidden="true"
                  >
                    {connectedMcpCount}
                  </span>
                )}
                {failedMcpCount > 0 && (
                  <span className="mcp-failure-indicator" aria-hidden="true">
                    <TriangleAlert size={9} />
                  </span>
                )}
              </button>
            </div>
            <div className="header-session-cluster">
              <strong title={activeSession?.display_name ?? "No session"}>
                {activeSession?.display_name ?? "No session"}
              </strong>
              <div className="header-model-guide-anchor">
                <button
                  type="button"
                  ref={runtimeButtonRef}
                  className={`header-model ${showRuntime ? "selected" : ""}`}
                  title={runtimeLabel}
                  aria-label={`${runtimeLabel}: ${headerModelLabel}`}
                  aria-expanded={showRuntime}
                  aria-controls="runtime-panel"
                  onClick={() => {
                    setShowAppearance(false);
                    setShowMcp(false);
                    setShowToolRepo(false);
                    if (showRuntime) closeRuntimePanel();
                    else setShowRuntime(true);
                  }}
                >
                  <Sparkles size={10} aria-hidden="true" />
                  <span title={headerModelLabel}>{headerModelLabel}</span>
                  <ChevronDown size={11} aria-hidden="true" />
                </button>
                {modelEndpointsUnavailable && !showRuntime && (
                  <div className="endpoint-guide-bubble" role="status">
                    <span className="endpoint-guide-icon" aria-hidden="true">
                      <Sparkles size={14} />
                    </span>
                    <div className="endpoint-guide-copy">
                      <strong>尚未配置模型接入点</strong>
                      <span>添加一个接入点，即可开始使用当前 Session。</span>
                    </div>
                    <button type="button" onClick={openEndpointCreator}>
                      <Plus size={13} />
                      <span>立即配置</span>
                    </button>
                  </div>
                )}
              </div>
            </div>
            <div className="header-actions">
              <button
                type="button"
                ref={mobileSessionButtonRef}
                title={mobileSessionsLabel}
                aria-label={mobileSessionsLabel}
                className="icon-button mobile-session-button"
                aria-expanded={showMobileSessions}
                aria-controls="session-navigation"
                onClick={() => setShowMobileSessions(true)}
              >
                <Menu size={18} />
              </button>
              <button
                type="button"
                title="Open worker roles"
                aria-label="Open worker roles"
                className="icon-button mobile-role-button"
                aria-expanded={showRoles}
                aria-controls="worker-role-panel"
                onClick={() => setShowRoles(true)}
              >
                <BriefcaseBusiness size={17} />
              </button>
              {toolGenEnabled && (
                <button
                  type="button"
                  ref={toolRepoButtonRef}
                  title={toolRepoLabel}
                  aria-label={toolRepoLabel}
                  className={`icon-button toolrepo-header-button ${showToolRepo ? "selected" : ""} ${toolCountPulseSessionId === activeSession?.session_id ? "count-pulse" : ""}`}
                  aria-expanded={showToolRepo}
                  aria-controls="toolrepo-panel"
                  onClick={() => {
                    setShowAppearance(false);
                    setShowRuntime(false);
                    setShowMcp(false);
                    if (showToolRepo) closeToolRepoPanel();
                    else setShowToolRepo(true);
                  }}
                >
                  <Wrench size={17} />
                  <span className="toolrepo-header-count" aria-hidden="true">
                    {activeToolCount}
                  </span>
                </button>
              )}
            </div>
          </header>
          {showAppearance && (
            <SettingsCenter
              panelRef={appearancePanelRef}
              section={settingsSection}
              onSectionChange={setSettingsSection}
              appearance={appearance}
              onAppearanceChange={setAppearance}
              toolGenEnabled={toolGenEnabled}
              toolGenToggleDisabled={pendingToolgenRequests.size > 0}
              onToolGenEnabledChange={setToolGenEnabled}
              memPath={server?.mem?.space_dir ?? ""}
              connected={connected}
              connectionLabel={connectionLabel}
              retentionDays={server ? server.mem.temporary_retention_days : 5}
              temporaryCapacityBytes={
                server?.mem.temporary_capacity_bytes ?? null
              }
              conversationCapacityBytes={
                server?.mem.conversation_capacity_bytes ?? null
              }
              favoriteCapacity={favoriteCapacity}
              retentionPending={pendingMemRetention}
              conversationCapacityPending={pendingMemConversationCapacity}
              favoriteCapacityPending={favoriteCapacityUpdating}
              switchPending={pendingMemSwitch}
              temporaryItems={memTemporaryItems}
              temporaryItemsLoading={memTemporaryItemsLoading}
              temporaryItemsDeleting={memTemporaryItemsDeleting}
              temporaryItemsError={memTemporaryItemsError}
              endpoints={server?.model_endpoints ?? []}
              endpointEditor={endpointEditor}
              revealedEndpointApiKeys={revealedEndpointApiKeys}
              revealedEndpointHeaders={revealedEndpointHeaders}
              revealedEndpointRequestFields={revealedEndpointRequestFields}
              onClose={closeSettingsCenter}
              onRefreshTemporaryItems={refreshMemTemporaryItems}
              onDeleteTemporaryItems={deleteMemTemporaryItems}
              onEditEndpoint={setEndpointEditor}
              onDeleteEndpoint={setDeleteEndpointCandidate}
              onRevealEndpoint={revealModelEndpoint}
              onSaveEndpoint={saveModelEndpoint}
              onSaveTemporaryPolicy={saveMemTemporaryPolicy}
              onSaveConversationCapacity={saveMemConversationCapacity}
              onSaveFavoriteCapacity={saveMemFavoriteCapacity}
              onSwitchMemory={switchMemWorkspace}
            />
          )}
          {showMcp && (
            <McpPanel
              key={activeSession?.session_id ?? "no-session"}
              panelRef={mcpPanelRef}
              servers={server?.mcp_servers ?? []}
              session={activeSession}
              pendingKeys={pendingMcpKeys}
              revealedSecrets={revealedMcpSecrets}
              onClose={closeMcpPanel}
              onCommand={(key, command) => {
                if (
                  !connected ||
                  !addPendingKey(pendingMcpKeysRef, setPendingMcpKeys, key)
                )
                  return;
                if (!sendCommand(command))
                  removePendingKey(pendingMcpKeysRef, setPendingMcpKeys, key);
              }}
            />
          )}
          {runtimeDisconnected && (
            <div className="runtime-disconnect-banner" role="alert">
              <strong>{runtimeDisconnectedTitle}</strong>
              <span>{runtimeDisconnectedDetail}</span>
            </div>
          )}
          {showRuntime && (
            <ModelEndpointPanel
              panelRef={runtimePanelRef}
              server={server}
              session={activeSession}
              onEdit={openEndpointSettings}
              onApply={(endpointId) => {
                if (!activeSession || activeSession.state === "working") return;
                sendCommand({
                  type: "model_endpoint_apply",
                  session_id: activeSession.session_id,
                  endpoint_id: endpointId,
                });
              }}
            />
          )}
          <TimemThread
            activeSession={activeSession}
            sessions={sessions}
            completedTurnsBySession={completedTurnsBySession}
            commandAcks={commandAcks}
            onConsumeCommandAcks={consumeCommandAcks}
            persistedSubmitCommandId={
              activeSession
                ? persistedSubmitCommandIds[activeSession.session_id]
                : undefined
            }
            reliableStorageScope={
              server
                ? reliableStorageScope(
                    window.location.origin,
                    server.mem.space_dir,
                  )
                : ""
            }
            sessionIds={sessions.map((session) => session.session_id)}
            sessionInteractionLocked={runtimeLocked}
            sessionInteractionLockReason={sessionInteractionLockReason}
            decisions={sessionDecisions}
            fileInput={fileInput}
            isCancelling={sessionCancellationApplies(activeSession)}
            pendingAttachmentRemoveIds={pendingAttachmentRemoveIds}
            pendingDecisionKeys={pendingDecisionKeys}
            uploadingAttachment={
              !!activeSession &&
              pendingUploadSessionIds.has(activeSession.session_id)
            }
            uploadingAttachmentFile={
              activeSession
                ? pendingUploadFiles[activeSession.session_id]
                : undefined
            }
            loadingHistory={
              activeSession
                ? pendingHistorySessionIds.has(activeSession.session_id)
                : false
            }
            onLoadMoreHistory={loadMoreHistory}
            onSend={sendText}
            onSendForSession={sendTextForSession}
            selectedRoleIds={selectedRoleIdsForSession}
            onRolesConsumed={(sessionId, expectedRoleIds) =>
              setSelectedRoleIds((current) => {
                if (
                  expectedRoleIds &&
                  JSON.stringify(current[sessionId] ?? []) !==
                    JSON.stringify(expectedRoleIds)
                )
                  return current;
                return Object.fromEntries(
                  Object.entries(current).filter(([key]) => key !== sessionId),
                );
              })
            }
            pendingToolGenTurnIds={activePendingToolGenTurnIds}
            toolGenSessionBusy={activeToolGenBusy}
            onRequestToolGen={toolGenEnabled ? requestActiveToolGen : undefined}
            favoriteBySource={
              new Map(
                favorites.map((favorite) => [favorite.source_key, favorite]),
              )
            }
            pendingFavoriteSourceKeys={pendingFavoriteSourceKeys}
            onToggleFavorite={toggleFavorite}
            onRequestMessageDelete={setDeleteMessageCandidate}
            onCancel={cancelActiveTurn}
            onUpload={uploadFile}
            onRemoveAttachment={(attachmentId) => {
              if (!activeSession || runtimeLocked) return;
              const key = `${activeSession.session_id}:${attachmentId}`;
              if (
                !addPendingKey(
                  pendingAttachmentRemoveIdsRef,
                  setPendingAttachmentRemoveIds,
                  key,
                )
              )
                return;
              if (
                !sendCommand({
                  type: "attachment_remove",
                  session_id: activeSession.session_id,
                  attachment_id: attachmentId,
                })
              ) {
                removePendingKey(
                  pendingAttachmentRemoveIdsRef,
                  setPendingAttachmentRemoveIds,
                  key,
                );
                const activity: Activity = {
                  id: clientId(),
                  sessionId: activeSession.session_id,
                  tone: "error",
                  title: "Remove attachment failed",
                  detail:
                    "Reconnect to Timem Web before removing this attachment.",
                  createdAt: Date.now(),
                };
                pushActivity(activity);
              }
            }}
            onDecisionReply={replyToDecision}
          />
        </main>
        {!showToolRepo && showRoles && (
          <button
            type="button"
            className="role-panel-backdrop"
            aria-label="Close worker roles"
            onClick={() => setShowRoles(false)}
          />
        )}
        {!showToolRepo && (
          <WorkerRolePanel
            key={activeSession?.session_id ?? "no-session"}
            session={activeSession}
            library={roleLibrary}
            mobileOpen={showRoles}
            collapsed={rightSidebarCollapsed}
            onCollapse={() =>
              setSidebarLayout((current) => ({
                ...current,
                rightCollapsed: true,
              }))
            }
            onRestore={() =>
              setSidebarLayout((current) => ({
                ...current,
                rightCollapsed: false,
              }))
            }
            onResizeStart={(event) => startSidebarResize("right", event)}
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
            onCommand={(command) => {
              if (!isOptimisticWorkerRoleMutation(command))
                return sendCommand(command);
              const commandId = clientId("worker-role-command");
              const optimisticCommand: WorkerRoleMutation =
                command.type === "worker_role_create"
                  ? { ...command, role_id: command.role_id ?? clientId("role") }
                  : command;
              pendingWorkerRoleMutationsRef.current.set(
                commandId,
                optimisticCommand,
              );
              setRoleLibrary((current) =>
                applyWorkerRoleMutation(current, optimisticCommand),
              );
              setSessions((current) =>
                current.map((session) => ({
                  ...session,
                  roles: applyWorkerRoleMutation(
                    { roles: session.roles, groups: [] },
                    optimisticCommand,
                  ).roles,
                })),
              );
              if (sendCommand(optimisticCommand, commandId)) return true;
              pendingWorkerRoleMutationsRef.current.delete(commandId);
              const visibleLibrary = replayWorkerRoleMutations(
                authoritativeRoleLibraryRef.current,
                pendingWorkerRoleMutationsRef.current.values(),
              );
              setRoleLibrary(visibleLibrary);
              setSessions((current) =>
                current.map((session) => ({
                  ...session,
                  roles: visibleLibrary.roles,
                })),
              );
              return false;
            }}
          />
        )}
        {chatLibraryMode && (
          <ChatLibraryPanel
            panelRef={chatLibraryPanelRef}
            query={chatSearchQuery}
            scope={chatSearchScope}
            activeSession={activeSession}
            results={chatSearchResults}
            favorites={favorites}
            searchPending={chatSearchPending}
            favoritesLoading={favoritesLoading}
            onQueryChange={setChatSearchQuery}
            onScopeChange={(scope) => {
              setChatSearchScope(scope);
              setChatLibraryMode(
                scope === "favorites" ? "favorites" : "search",
              );
              if (scope === "favorites") {
                setFavoritesLoading(true);
                if (!sendCommand({ type: "favorites_list" }))
                  setFavoritesLoading(false);
              }
            }}
            onSearch={() => {
              if (chatSearchScope === "favorites") return;
              const query = chatSearchQuery.trim();
              if (!query) {
                setChatSearchResults([]);
                return;
              }
              setChatSearchPending(true);
              if (
                !sendCommand({
                  type: "chat_search",
                  query,
                  session_id:
                    chatSearchScope === "session"
                      ? activeSession?.session_id
                      : undefined,
                  limit: 100,
                })
              )
                setChatSearchPending(false);
            }}
            pendingFavoriteSourceKeys={pendingFavoriteSourceKeys}
            onToggleFavorite={toggleFavorite}
            onOpen={(sessionId, turnId) => {
              setActiveSessionId(sessionId);
              closeMobileSidebar(false);
              closeChatLibrary();
              window.setTimeout(
                () =>
                  document
                    .querySelector<HTMLElement>(
                      `[data-turn-id="${globalThis.CSS.escape(turnId)}"]`,
                    )
                    ?.scrollIntoView({
                      behavior: prefersReducedMotion() ? "auto" : "smooth",
                      block: "center",
                    }),
                80,
              );
            }}
            onClose={closeChatLibrary}
          />
        )}
        {toolGenEnabled && showToolRepo && (
          <button
            type="button"
            className="side-panel-backdrop"
            aria-label="Close ToolRepo"
            onClick={closeToolRepoPanel}
          />
        )}
        {toolGenEnabled && showToolRepo && (
          <ToolRepoPanel
            key={activeSession?.session_id ?? "no-session"}
            panelRef={toolRepoPanelRef}
            onResizeStart={(event) => startSidebarResize("right", event)}
            onClose={closeToolRepoPanel}
            session={activeSession}
            searchQuery={toolSearchQuery}
            searchPending={
              !!activeSession &&
              pendingToolSearchKey ===
                `${activeSession.session_id}:${toolSearchQuery}`
            }
            onSearchQueryChange={setToolSearchQuery}
            tools={
              activeSession
                ? (toolSearchResults[activeSession.session_id] ??
                  activeSession.tools)
                : []
            }
            selectedTool={selectedTool}
            pendingToolDetailId={
              activeSession &&
              pendingToolDetailKey.startsWith(`${activeSession.session_id}:`)
                ? pendingToolDetailKey.slice(
                    activeSession.session_id.length + 1,
                  )
                : ""
            }
            pendingToolRenameIds={
              activeSession
                ? pendingToolIdsForSession(
                    pendingToolRenameKeys,
                    activeSession.session_id,
                  )
                : new Set()
            }
            onSelectTool={(toolId) => {
              if (selectedTool?.summary.tool_id === toolId) {
                setSelectedTool(null);
                setPendingToolDetailKey("");
                return true;
              }
              if (!activeSession) return false;
              setPendingToolDetailKey(`${activeSession.session_id}:${toolId}`);
              if (
                sendCommand({
                  type: "tool_repo_detail",
                  session_id: activeSession.session_id,
                  tool_id: toolId,
                })
              )
                return true;
              setPendingToolDetailKey("");
              reportUiError(
                "Tool detail failed",
                "Reconnect to Timem Web before opening tool details.",
                activeSession.session_id,
              );
              return false;
            }}
            onCollapseTool={() => {
              setSelectedTool(null);
              setPendingToolDetailKey("");
            }}
            onRenameTool={(toolId, newName) => {
              if (activeSession) {
                const renameKey = toolKey(activeSession.session_id, toolId);
                setPendingToolRenameKeys((current) =>
                  new Set(current).add(renameKey),
                );
                if (
                  sendCommand({
                    type: "tool_repo_rename",
                    session_id: activeSession.session_id,
                    tool_id: toolId,
                    new_name: newName,
                  })
                )
                  return true;
                setPendingToolRenameKeys((current) => {
                  const next = new Set(current);
                  next.delete(renameKey);
                  return next;
                });
              }
              const activity: Activity = {
                id: clientId(),
                sessionId: activeSession?.session_id ?? "system",
                tone: "error",
                title: "Tool rename failed",
                detail: "Reconnect to Timem Web before renaming this tool.",
                createdAt: Date.now(),
              };
              pushActivity(activity);
              return false;
            }}
            onOpenTerminal={(toolId) => {
              if (
                activeSession &&
                sendCommand({
                  type: "tool_repo_open_terminal",
                  session_id: activeSession.session_id,
                  tool_id: toolId,
                })
              )
                return true;
              const activity: Activity = {
                id: clientId(),
                sessionId: activeSession?.session_id ?? "system",
                tone: "error",
                title: "Open terminal failed",
                detail:
                  "Reconnect to Timem Web before opening a tool directory.",
                createdAt: Date.now(),
              };
              pushActivity(activity);
              return false;
            }}
          />
        )}
        {showNewSession && (
          <NewSessionDialog
            workspaces={server?.workspace_dirs ?? []}
            runtimeDefaults={server?.session_env_defaults ?? {}}
            creating={creatingSession}
            memSwitching={runtimeLocked}
            onClose={() => {
              if (!creatingSessionRef.current) closeNewSessionDialog();
            }}
            onCreate={(command) => {
              if (runtimeLocked) return;
              if (creatingSessionRef.current) return;
              creatingSessionRef.current = true;
              setCreatingSession(true);
              if (sendCommand(command)) {
                closeNewSessionDialog();
              } else {
                creatingSessionRef.current = false;
                setCreatingSession(false);
                reportUiError(
                  "Create session failed",
                  "Reconnect to Timem Web before creating a new session.",
                  "system",
                );
              }
            }}
          />
        )}
        {memSwitchCandidate && (
          <MemSwitchConfirmDialog
            candidate={memSwitchCandidate}
            pending={pendingMemSwitch}
            onClose={() => {
              if (!pendingMemSwitch) setMemSwitchCandidate(null);
            }}
            onConfirm={() => {
              setRenamingSessionId("");
              setRenameDraft("");
              setPendingMemSwitch(true);
              if (
                !sendCommand({
                  type: "mem_switch",
                  path: memSwitchCandidate.path,
                  stop_running: true,
                })
              ) {
                setPendingMemSwitch(false);
                reportUiError(
                  "Mem switch failed",
                  "Reconnect to Timem Web before switching the mem directory.",
                  "system",
                );
              }
            }}
          />
        )}
        {deleteEndpointCandidate && (
          <ModelEndpointDeleteDialog
            endpoint={deleteEndpointCandidate}
            onClose={() => setDeleteEndpointCandidate(null)}
            onConfirm={() =>
              sendCommand({
                type: "model_endpoint_delete",
                endpoint_id: deleteEndpointCandidate.id,
              })
            }
          />
        )}
        {deleteSessionCandidate && (
          <SessionDeleteDialog
            session={deleteSessionCandidate}
            pending={pendingDeleteSessionIds.has(
              deleteSessionCandidate.session_id,
            )}
            onClose={() => {
              if (
                !pendingDeleteSessionIdsRef.current.has(
                  deleteSessionCandidate.session_id,
                )
              )
                setDeleteSessionCandidate(null);
            }}
            onConfirm={() => {
              const sessionId = deleteSessionCandidate.session_id;
              if (
                !addPendingKey(
                  pendingDeleteSessionIdsRef,
                  setPendingDeleteSessionIds,
                  sessionId,
                )
              )
                return;
              if (
                !sendCommand({ type: "session_delete", session_id: sessionId })
              ) {
                removePendingKey(
                  pendingDeleteSessionIdsRef,
                  setPendingDeleteSessionIds,
                  sessionId,
                );
                reportUiError(
                  "Delete session failed",
                  "Reconnect to Timem Web before deleting this session.",
                  sessionId,
                );
              }
            }}
          />
        )}
        {favoriteCapacityNotice && (
          <FavoriteCapacityDialog
            notice={favoriteCapacityNotice}
            currentCapacity={favoriteCapacity}
            updating={favoriteCapacityUpdating}
            onClose={() => {
              if (!favoriteCapacityUpdating) setFavoriteCapacityNotice(null);
            }}
            onSelectLimit={(maxBytes) => {
              if (favoriteCapacityUpdating) return;
              setFavoriteCapacityUpdating(true);
              if (
                !sendCommand({
                  type: "favorite_capacity_update",
                  max_bytes: maxBytes,
                })
              ) {
                setFavoriteCapacityUpdating(false);
                reportUiError(
                  "无法调整收藏夹空间",
                  "请检查连接后重试。",
                  "system",
                );
              }
            }}
          />
        )}
        {deleteMessageCandidate &&
          deleteMessageCandidate.sessionId === activeSessionKey && (
            <ChatMessageDeleteDialog
              candidate={deleteMessageCandidate}
              pending={pendingDeleteMessageKeys.has(
                chatMessageDeleteKey(deleteMessageCandidate),
              )}
              onClose={() => {
                if (
                  !pendingDeleteMessageKeysRef.current.has(
                    chatMessageDeleteKey(deleteMessageCandidate),
                  )
                )
                  setDeleteMessageCandidate(null);
              }}
              onConfirm={() => {
                const key = chatMessageDeleteKey(deleteMessageCandidate);
                if (
                  !addPendingKey(
                    pendingDeleteMessageKeysRef,
                    setPendingDeleteMessageKeys,
                    key,
                  )
                )
                  return;
                if (
                  !sendCommand({
                    type: "chat_message_delete",
                    session_id: deleteMessageCandidate.sessionId,
                    turn_id: deleteMessageCandidate.turnId,
                    role: deleteMessageCandidate.role,
                    role_index: deleteMessageCandidate.roleIndex,
                  })
                ) {
                  removePendingKey(
                    pendingDeleteMessageKeysRef,
                    setPendingDeleteMessageKeys,
                    key,
                  );
                  reportUiError(
                    "Delete message failed",
                    "Reconnect to Timem Web before deleting this message.",
                    deleteMessageCandidate.sessionId,
                  );
                }
              }}
            />
          )}
        {showRuntimeUnavailableDialog && (
          <RuntimeUnavailableDialog
            detail={runtimeDisconnectedDetail}
            onClose={() => setRuntimeUnavailableDialogDismissed(true)}
          />
        )}
        {toolGenEnabled &&
          toolgenDialog &&
          toolgenDialog.sessionId === activeSessionKey && (
            <ToolGenDialog
              key={`${toolgenDialog.sessionId}:${toolgenDialog.turnId}`}
              pending={pendingToolgenRequests.has(
                toolgenRequestKey(
                  toolgenDialog.sessionId,
                  toolgenDialog.turnId,
                ),
              )}
              onClose={() => {
                if (
                  !pendingToolgenRequests.has(
                    toolgenRequestKey(
                      toolgenDialog.sessionId,
                      toolgenDialog.turnId,
                    ),
                  )
                )
                  setToolgenDialog(null);
              }}
              onSubmit={(text) => {
                if (!toolGenEnabled) return;
                const request = toolgenDialog;
                const requestKey = toolgenRequestKey(
                  request.sessionId,
                  request.turnId,
                );
                if (pendingToolgenRequestsRef.current.has(requestKey)) return;
                pendingToolgenRequestsRef.current.add(requestKey);
                setPendingToolgenRequests(
                  new Set(pendingToolgenRequestsRef.current),
                );
                if (
                  sendCommand(
                    manualToolGenCommand(
                      request.sessionId,
                      request.turnId,
                      text,
                    ),
                  )
                ) {
                  setToolgenDialog(null);
                } else {
                  pendingToolgenRequestsRef.current.delete(requestKey);
                  setPendingToolgenRequests(
                    new Set(pendingToolgenRequestsRef.current),
                  );
                  reportUiError(
                    "ToolGen start failed",
                    "Reconnect to Timem Web before generating a reusable tool.",
                    request.sessionId,
                  );
                }
              }}
            />
          )}
      </div>
    </AssistantRuntimeProvider>
  );
}

type SortableWorkerRoleRenderState = {
  setNodeRef: (node: HTMLElement | null) => void;
  style: CSSProperties;
  attributes: ReturnType<typeof useSortable>["attributes"];
  listeners: ReturnType<typeof useSortable>["listeners"];
  isDragging: boolean;
};

function SortableWorkerRole({
  id,
  disabled,
  children,
}: {
  id: string;
  disabled: boolean;
  children: (state: SortableWorkerRoleRenderState) => ReactNode;
}) {
  const sortable = useSortable({
    id,
    disabled,
    transition: {
      duration: 180,
      easing: "cubic-bezier(.2, .8, .2, 1)",
    },
  });
  return children({
    setNodeRef: sortable.setNodeRef,
    style: {
      transform: CSS.Transform.toString(sortable.transform),
      transition: sortable.transition,
      zIndex: sortable.isDragging ? 4 : undefined,
    },
    attributes: sortable.attributes,
    listeners: sortable.listeners,
    isDragging: sortable.isDragging,
  });
}

function WorkerRoleDropGroup({
  id,
  roleIds,
  className,
  children,
}: {
  id: string;
  roleIds: string[];
  className: string;
  children: ReactNode;
}) {
  const droppable = useDroppable({ id });
  return (
    <section
      ref={droppable.setNodeRef}
      className={`${className} ${droppable.isOver ? "drop-target" : ""}`}
    >
      <SortableContext items={roleIds} strategy={verticalListSortingStrategy}>
        {children}
      </SortableContext>
    </section>
  );
}

function ExpandedTextEditor({
  title,
  eyebrow,
  value,
  placeholder,
  maxLength,
  disabled,
  onCommit,
  onClose,
}: {
  title: string;
  eyebrow: string;
  value: string;
  placeholder: string;
  maxLength?: number;
  disabled?: boolean;
  onCommit: (value: string) => void;
  onClose: () => void;
}) {
  // Keep high-frequency typing local to the fullscreen editor. Updating the
  // thread-level session draft for every key would rerender the entire chat UI.
  const [draft, setDraft] = useState(value);
  const finish = () => {
    onCommit(draft);
    onClose();
  };
  return createPortal(
    <div className="expanded-text-backdrop" role="presentation">
      <section
        className="expanded-text-editor"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            finish();
          }
        }}
      >
        <header>
          <div>
            <span className="eyebrow">{eyebrow}</span>
            <h2>{title}</h2>
            <p>输入内容会在完成编辑时一次性同步；不会自动保存或发送。</p>
          </div>
          <button
            type="button"
            className="expanded-text-collapse"
            title="收起编辑器"
            aria-label="收起编辑器"
            onClick={finish}
          >
            <Minimize2 size={16} />
          </button>
        </header>
        <textarea
          autoFocus
          value={draft}
          maxLength={maxLength}
          disabled={disabled}
          spellCheck={false}
          placeholder={placeholder}
          onChange={(event) => setDraft(event.target.value)}
        />
        <footer>
          <span>
            {maxLength
              ? `${draft.length.toLocaleString()} / ${maxLength.toLocaleString()}`
              : `${draft.length.toLocaleString()} 字符`}
          </span>
          <div>
            <button type="button" className="secondary" onClick={onClose}>
              取消修改
            </button>
            <button type="button" className="primary" onClick={finish}>
              完成编辑
            </button>
          </div>
        </footer>
      </section>
    </div>,
    document.body,
  );
}

function WorkerRolePanel({
  session,
  library,
  selectedRoleIds,
  disabled,
  mobileOpen,
  collapsed,
  onCollapse,
  onRestore,
  onResizeStart,
  onClose,
  onSelect,
  onCommand,
}: {
  session?: Session;
  library: WorkerRoleLibrary;
  selectedRoleIds: readonly string[];
  disabled: boolean;
  mobileOpen: boolean;
  collapsed: boolean;
  onCollapse: () => void;
  onRestore: () => void;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
  onClose: () => void;
  onSelect: (roleId: string) => void;
  onCommand: (command: ClientCommand) => boolean;
}) {
  const [editingId, setEditingId] = useState<string | null>(null);
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [roleDeleteMode, setRoleDeleteMode] = useState(false);
  const [selectedDeleteRoleId, setSelectedDeleteRoleId] = useState("");
  const [newGroupName, setNewGroupName] = useState("");
  const [editingGroupId, setEditingGroupId] = useState("");
  const [editingGroupName, setEditingGroupName] = useState("");
  const [draggedRoleId, setDraggedRoleId] = useState("");
  const [collapsedRoleGroupIds, setCollapsedRoleGroupIds] = useState<
    Set<string>
  >(() => new Set());
  const [descriptionExpanded, setDescriptionExpanded] = useState(false);
  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const resetEditor = () => {
    setEditingId(null);
    setName("");
    setDescription("");
    setDescriptionExpanded(false);
  };
  useEffect(() => {
    resetEditor();
    setRoleDeleteMode(false);
    setSelectedDeleteRoleId("");
    setEditingGroupId("");
  }, [session?.session_id]);
  useEffect(() => {
    const validGroupIds = new Set([
      "ungrouped",
      ...library.groups.map((group) => group.id),
    ]);
    setCollapsedRoleGroupIds(
      (current) =>
        new Set(Array.from(current).filter((id) => validGroupIds.has(id))),
    );
  }, [library.groups]);
  useEffect(() => {
    if (
      selectedDeleteRoleId &&
      !library.roles.some((role) => role.id === selectedDeleteRoleId)
    ) {
      setSelectedDeleteRoleId("");
    }
    if (roleDeleteMode && library.roles.length === 0) setRoleDeleteMode(false);
  }, [library.roles, roleDeleteMode, selectedDeleteRoleId]);
  const toggleRoleGroup = (groupId: string) =>
    setCollapsedRoleGroupIds((current) => {
      const next = new Set(current);
      if (next.has(groupId)) next.delete(groupId);
      else next.add(groupId);
      return next;
    });

  const submit = () => {
    if (!session || !name.trim() || !description.trim()) return;
    const command: ClientCommand = editingId
      ? {
          type: "worker_role_update",
          session_id: session.session_id,
          role_id: editingId,
          name,
          description,
        }
      : {
          type: "worker_role_create",
          session_id: session.session_id,
          name,
          description,
        };
    if (onCommand(command)) resetEditor();
  };

  const groupedRoleIds = new Set(
    library.groups.flatMap((group) => group.role_ids),
  );
  const ungroupedRoles = library.roles.filter(
    (role) => !groupedRoleIds.has(role.id),
  );
  const roleById = new Map(library.roles.map((role) => [role.id, role]));
  const groupForRole = (roleId: string) =>
    library.groups.find((group) => group.role_ids.includes(roleId))?.id ?? null;

  const reorderRole = (
    roleId: string,
    targetGroupId: string | null,
    beforeRoleId?: string,
  ) => {
    if (!session || disabled) return;
    const groups = library.groups.map((group) => ({
      ...group,
      role_ids: group.role_ids.filter((id) => id !== roleId),
    }));
    const ungrouped = ungroupedRoles
      .map((role) => role.id)
      .filter((id) => id !== roleId);
    const target = targetGroupId
      ? groups.find((group) => group.id === targetGroupId)?.role_ids
      : ungrouped;
    if (!target) return;
    const beforeIndex = beforeRoleId ? target.indexOf(beforeRoleId) : -1;
    if (beforeIndex >= 0) target.splice(beforeIndex, 0, roleId);
    else target.push(roleId);
    onCommand({
      type: "worker_role_library_reorder",
      session_id: session.session_id,
      groups,
      ungrouped_role_ids: ungrouped,
    });
  };

  const finishRoleDrag = (event: DragEndEvent) => {
    setDraggedRoleId("");
    const roleId = String(event.active.id);
    const overId = event.over ? String(event.over.id) : "";
    if (!overId || overId === roleId) return;
    if (overId === "role-group:ungrouped") {
      reorderRole(roleId, null);
      return;
    }
    if (overId.startsWith("role-group:")) {
      reorderRole(roleId, overId.slice("role-group:".length));
      return;
    }
    if (roleById.has(overId)) reorderRole(roleId, groupForRole(overId), overId);
  };

  const roleItem = (role: WorkerRole) => (
    <SortableWorkerRole
      id={role.id}
      disabled={disabled || roleDeleteMode}
      key={role.id}
    >
      {({ setNodeRef, style, attributes, listeners, isDragging }) => (
        <article
          ref={setNodeRef}
          style={style}
          className={`worker-role-item ${!roleDeleteMode && selectedRoleIds.includes(role.id) ? "selected" : ""} ${roleDeleteMode ? "delete-selecting" : ""} ${selectedDeleteRoleId === role.id ? "delete-selected" : ""} ${isDragging ? "dragging" : ""}`}
        >
          <button
            type="button"
            className="worker-role-drag"
            disabled={disabled || roleDeleteMode}
            title={`拖动 ${role.name}`}
            aria-label={`拖动 ${role.name}`}
            {...attributes}
            {...listeners}
          >
            <GripVertical size={13} />
          </button>
          <label
            title={
              roleDeleteMode
                ? `选择删除 ${role.name}`
                : `Use ${role.name} for the next message`
            }
          >
            <input
              type="checkbox"
              checked={
                roleDeleteMode
                  ? selectedDeleteRoleId === role.id
                  : selectedRoleIds.includes(role.id)
              }
              disabled={disabled}
              onChange={() =>
                roleDeleteMode
                  ? setSelectedDeleteRoleId((current) =>
                      current === role.id ? "" : role.id,
                    )
                  : onSelect(role.id)
              }
            />
            <span>
              <strong>{role.name}</strong>
              <small>{role.description}</small>
            </span>
          </label>
          {!roleDeleteMode && (
            <div className="worker-role-actions">
              <button
                type="button"
                className="worker-role-action worker-role-edit"
                disabled={disabled}
                title={`Edit ${role.name}`}
                aria-label={`Edit ${role.name}`}
                onClick={() => {
                  setEditingId(role.id);
                  setName(role.name);
                  setDescription(role.description);
                }}
              >
                <Pencil size={12} />
              </button>
            </div>
          )}
        </article>
      )}
    </SortableWorkerRole>
  );

  const draggedRole = roleById.get(draggedRoleId);
  return (
    <aside
      id="worker-role-panel"
      className={`worker-role-panel ${collapsed ? "collapsed" : ""} ${mobileOpen ? "mobile-open" : ""}`}
      aria-label="Worker roles"
    >
      {collapsed ? (
        <button
          type="button"
          className="worker-role-restore"
          title="Show worker roles"
          aria-label="Show worker roles"
          onClick={onRestore}
        >
          <ChevronLeft size={15} />
        </button>
      ) : (
        <>
          <button
            type="button"
            className="sidebar-resize-handle right"
            title="Resize worker roles"
            aria-label="Resize worker roles"
            onPointerDown={onResizeStart}
          />
          <header>
            <span>
              <BriefcaseBusiness size={16} /> Roles
            </span>
            <div>
              <button
                type="button"
                className="desktop-sidebar-toggle worker-role-collapse"
                title="Hide worker roles"
                aria-label="Hide worker roles"
                onClick={onCollapse}
              >
                <ChevronRight size={15} />
              </button>
              {roleDeleteMode && (
                <button
                  type="button"
                  className="worker-role-delete-cancel"
                  title="取消删除 Role"
                  aria-label="取消删除 Role"
                  onClick={() => {
                    setRoleDeleteMode(false);
                    setSelectedDeleteRoleId("");
                  }}
                >
                  <X size={14} strokeWidth={3} />
                </button>
              )}
              <button
                type="button"
                className={`worker-role-delete-manage ${roleDeleteMode ? "confirm" : ""}`}
                title={
                  roleDeleteMode
                    ? selectedDeleteRoleId
                      ? "确认删除选中的 Role"
                      : "请选择要删除的 Role"
                    : "选择要删除的 Role"
                }
                aria-label={
                  roleDeleteMode
                    ? selectedDeleteRoleId
                      ? "确认删除选中的 Role"
                      : "请选择要删除的 Role"
                    : "选择要删除的 Role"
                }
                disabled={
                  disabled ||
                  library.roles.length === 0 ||
                  (roleDeleteMode && !selectedDeleteRoleId)
                }
                onClick={() => {
                  if (!roleDeleteMode) {
                    resetEditor();
                    setRoleDeleteMode(true);
                    setSelectedDeleteRoleId("");
                    return;
                  }
                  if (
                    session &&
                    selectedDeleteRoleId &&
                    onCommand({
                      type: "worker_role_delete",
                      session_id: session.session_id,
                      role_id: selectedDeleteRoleId,
                    })
                  ) {
                    setRoleDeleteMode(false);
                    setSelectedDeleteRoleId("");
                  }
                }}
              >
                {roleDeleteMode ? (
                  <Check size={14} strokeWidth={3} />
                ) : (
                  <Trash2 size={14} />
                )}
              </button>
              <button
                type="button"
                className="worker-role-close"
                title="Close roles"
                aria-label="Close roles"
                onClick={onClose}
              >
                <X size={15} />
              </button>
            </div>
          </header>
          <p
            className={`worker-role-help ${roleDeleteMode ? "delete-mode" : ""}`}
          >
            {roleDeleteMode
              ? "勾选一个 Role，然后点击顶部对勾确认删除。"
              : "拖动 Role 可排序或归入分组。"}
          </p>
          <DndContext
            sensors={sensors}
            collisionDetection={closestCenter}
            onDragStart={(event) => setDraggedRoleId(String(event.active.id))}
            onDragCancel={() => setDraggedRoleId("")}
            onDragEnd={finishRoleDrag}
          >
            <div className="worker-role-list">
              {library.groups.map((group) => {
                const collapsed = collapsedRoleGroupIds.has(group.id);
                return (
                  <WorkerRoleDropGroup
                    id={`role-group:${group.id}`}
                    roleIds={group.role_ids}
                    className={`worker-role-group ${collapsed ? "collapsed" : ""}`}
                    key={group.id}
                  >
                    <header>
                      {editingGroupId === group.id ? (
                        <input
                          autoFocus
                          value={editingGroupName}
                          maxLength={80}
                          aria-label={`Rename group ${group.name}`}
                          onChange={(event) =>
                            setEditingGroupName(event.target.value)
                          }
                          onKeyDown={(event) => {
                            if (event.key === "Escape") setEditingGroupId("");
                            if (
                              event.key === "Enter" &&
                              session &&
                              editingGroupName.trim() &&
                              onCommand({
                                type: "worker_role_group_update",
                                session_id: session.session_id,
                                group_id: group.id,
                                name: editingGroupName,
                              })
                            )
                              setEditingGroupId("");
                          }}
                        />
                      ) : (
                        <button
                          type="button"
                          className="worker-role-group-toggle"
                          aria-expanded={!collapsed}
                          aria-controls={`worker-role-group-list-${group.id}`}
                          title={
                            collapsed
                              ? `展开 ${group.name}`
                              : `折叠 ${group.name}`
                          }
                          onClick={() => toggleRoleGroup(group.id)}
                        >
                          <ChevronRight size={12} />
                          <strong>{group.name}</strong>
                          <small>{group.role_ids.length}</small>
                        </button>
                      )}
                      <div>
                        <button
                          type="button"
                          className="worker-role-action worker-role-edit"
                          disabled={disabled || roleDeleteMode}
                          title={`Rename ${group.name}`}
                          aria-label={`Rename ${group.name}`}
                          onClick={() => {
                            setEditingGroupId(group.id);
                            setEditingGroupName(group.name);
                          }}
                        >
                          <Pencil size={11} />
                        </button>
                        <button
                          type="button"
                          className="worker-role-action worker-role-delete"
                          disabled={disabled || roleDeleteMode}
                          title={`Delete ${group.name}`}
                          aria-label={`Delete ${group.name}`}
                          onClick={() =>
                            session &&
                            onCommand({
                              type: "worker_role_group_delete",
                              session_id: session.session_id,
                              group_id: group.id,
                            })
                          }
                        >
                          <Trash2 size={11} />
                        </button>
                      </div>
                    </header>
                    {!collapsed && (
                      <div
                        id={`worker-role-group-list-${group.id}`}
                        className="worker-role-group-list"
                      >
                        {group.role_ids
                          .map((id) => roleById.get(id))
                          .filter((role): role is WorkerRole => !!role)
                          .map(roleItem)}
                        {group.role_ids.length === 0 && (
                          <span className="worker-role-drop-hint">
                            拖动 Role 到这里
                          </span>
                        )}
                      </div>
                    )}
                  </WorkerRoleDropGroup>
                );
              })}
              <WorkerRoleDropGroup
                id="role-group:ungrouped"
                roleIds={ungroupedRoles.map((role) => role.id)}
                className={`worker-role-group worker-role-ungrouped ${collapsedRoleGroupIds.has("ungrouped") ? "collapsed" : ""}`}
              >
                <header>
                  <button
                    type="button"
                    className="worker-role-group-toggle"
                    aria-expanded={!collapsedRoleGroupIds.has("ungrouped")}
                    aria-controls="worker-role-group-list-ungrouped"
                    title={
                      collapsedRoleGroupIds.has("ungrouped")
                        ? "展开未分组"
                        : "折叠未分组"
                    }
                    onClick={() => toggleRoleGroup("ungrouped")}
                  >
                    <ChevronRight size={12} />
                    <strong>未分组</strong>
                    <small>{ungroupedRoles.length}</small>
                  </button>
                </header>
                {!collapsedRoleGroupIds.has("ungrouped") && (
                  <div
                    id="worker-role-group-list-ungrouped"
                    className="worker-role-group-list"
                  >
                    {ungroupedRoles.map(roleItem)}
                    {ungroupedRoles.length === 0 &&
                      library.roles.length > 0 && (
                        <span className="worker-role-drop-hint">
                          所有 Role 已归组
                        </span>
                      )}
                  </div>
                )}
              </WorkerRoleDropGroup>
              {library.roles.length === 0 && (
                <div className="worker-role-empty">
                  还没有 Role。创建一个，供所有 Session 使用。
                </div>
              )}
              {!session && (
                <div className="worker-role-empty">
                  Select a session to manage roles.
                </div>
              )}
            </div>
            <DragOverlay
              dropAnimation={
                prefersReducedMotion()
                  ? null
                  : { duration: 180, easing: "cubic-bezier(.2, .8, .2, 1)" }
              }
            >
              {draggedRole && (
                <article
                  className="worker-role-item worker-role-overlay"
                  aria-hidden="true"
                >
                  <span className="worker-role-drag">
                    <GripVertical size={13} />
                  </span>
                  <span>
                    <strong>{draggedRole.name}</strong>
                    <small>{draggedRole.description}</small>
                  </span>
                </article>
              )}
            </DragOverlay>
          </DndContext>
          {session && !roleDeleteMode && (
            <form
              className="worker-role-group-editor"
              onSubmit={(event) => {
                event.preventDefault();
                if (
                  newGroupName.trim() &&
                  onCommand({
                    type: "worker_role_group_create",
                    session_id: session.session_id,
                    name: newGroupName,
                  })
                )
                  setNewGroupName("");
              }}
            >
              <input
                value={newGroupName}
                maxLength={80}
                disabled={disabled}
                placeholder="新分组名称"
                aria-label="Role group name"
                onChange={(event) => setNewGroupName(event.target.value)}
              />
              <button
                type="submit"
                className="worker-role-group-create"
                disabled={disabled || !newGroupName.trim()}
              >
                <Plus size={12} /> 分组
              </button>
            </form>
          )}
          {session && !roleDeleteMode && (
            <form
              className={`worker-role-editor ${editingId ? "editing" : "creating"}`}
              onSubmit={(event) => {
                event.preventDefault();
                submit();
              }}
            >
              <strong>{editingId ? "编辑 Role" : "新建 Role"}</strong>
              <input
                value={name}
                maxLength={80}
                disabled={disabled}
                placeholder="称呼，例如：严谨审查员"
                aria-label="Role name"
                onChange={(event) => setName(event.target.value)}
              />
              <div className="expandable-text-field worker-role-description-field">
                <textarea
                  value={description}
                  maxLength={16384}
                  disabled={disabled}
                  placeholder="描述工作要求、步骤和约束…"
                  aria-label="Role description"
                  onChange={(event) => setDescription(event.target.value)}
                />
                <button
                  type="button"
                  className="text-field-expand"
                  title="展开编辑 Role 描述"
                  aria-label="展开编辑 Role 描述"
                  disabled={disabled}
                  onClick={() => setDescriptionExpanded(true)}
                >
                  <Maximize2 size={13} />
                </button>
              </div>
              <div>
                <button
                  type="submit"
                  className="worker-role-primary-action"
                  disabled={disabled || !name.trim() || !description.trim()}
                >
                  {editingId ? "保存" : "创建"}
                </button>
                {editingId && (
                  <button type="button" onClick={resetEditor}>
                    取消
                  </button>
                )}
              </div>
              {descriptionExpanded && (
                <ExpandedTextEditor
                  eyebrow="ROLE DESCRIPTION"
                  title={
                    editingId
                      ? `编辑 ${name.trim() || "Role"} 的描述`
                      : "编写 Role 描述"
                  }
                  value={description}
                  maxLength={16384}
                  disabled={disabled}
                  placeholder="描述工作要求、步骤和约束…"
                  onCommit={setDescription}
                  onClose={() => setDescriptionExpanded(false)}
                />
              )}
            </form>
          )}
        </>
      )}
    </aside>
  );
}

type ChatLibrarySearchItem = {
  id: string;
  sessionId: string;
  sessionName: string;
  turnId: string;
  title: string;
  content: string;
  createdAt: number;
  formattedDate: string;
  role: "user" | "assistant";
};

type ChatLibraryFavoriteItem = {
  id: string;
  sourceKey: string;
  sessionId: string;
  sessionName: string;
  turnId: string;
  title: string;
  content: string;
  createdAt: number;
  formattedDate: string;
  favoriteId: string;
  bytes: number;
};

const ChatLibrarySearchRow = memo(function ChatLibrarySearchRow({
  item,
  onOpen,
}: {
  item: ChatLibrarySearchItem;
  onOpen: (sessionId: string, turnId: string) => void;
}) {
  return (
    <article className="chat-library-search-row">
      <button
        type="button"
        className="chat-library-search-main"
        title={`Open ${item.sessionName}`}
        onClick={() => onOpen(item.sessionId, item.turnId)}
      >
        <span className={`chat-library-role ${item.role}`}>
          {item.role === "assistant" ? (
            <Sparkles size={11} />
          ) : (
            <CornerUpLeft size={11} />
          )}
          <b>{item.title}</b>
        </span>
        <p>{item.content}</p>
        <small>
          <span title={item.sessionName}>{item.sessionName}</span>
          <time dateTime={new Date(item.createdAt).toISOString()}>
            {item.formattedDate}
          </time>
        </small>
      </button>
    </article>
  );
});

const ChatLibraryFavoriteRow = memo(function ChatLibraryFavoriteRow({
  item,
  deleteMode,
  selected,
  pending,
  onOpen,
  onToggleSelection,
}: {
  item: ChatLibraryFavoriteItem;
  deleteMode: boolean;
  selected: boolean;
  pending: boolean;
  onOpen: (sessionId: string, turnId: string) => void;
  onToggleSelection: (favoriteId: string) => void;
}) {
  return (
    <article
      className={`chat-library-favorite-row ${deleteMode ? "selecting" : ""} ${selected ? "selected" : ""} ${pending ? "pending" : ""}`}
    >
      <button
        type="button"
        className="chat-library-favorite-main"
        disabled={pending}
        aria-pressed={deleteMode ? selected : undefined}
        title={
          deleteMode
            ? selected
              ? "Deselect favorite"
              : "Select favorite"
            : `Open ${item.sessionName}`
        }
        onClick={() =>
          deleteMode
            ? onToggleSelection(item.id)
            : onOpen(item.sessionId, item.turnId)
        }
      >
        {deleteMode && (
          <span className="chat-library-favorite-check" aria-hidden="true">
            {selected && <Check size={13} strokeWidth={3} />}
          </span>
        )}
        <span className="chat-library-favorite-copy">
          <strong>{item.title}</strong>
          <p>{item.content}</p>
          <small>
            <span title={item.sessionName}>{item.sessionName}</span>
            <time dateTime={new Date(item.createdAt).toISOString()}>
              {item.formattedDate}
            </time>
          </small>
        </span>
        <span className="chat-library-favorite-size">
          {formatBytes(item.bytes)}
        </span>
      </button>
    </article>
  );
});

const CHAT_LIBRARY_INITIAL_ROWS = 40;
const CHAT_LIBRARY_MORE_ROWS = 40;

function utf8ByteLength(value: string) {
  let bytes = 0;
  for (const character of value) {
    const codePoint = character.codePointAt(0) ?? 0;
    bytes +=
      codePoint <= 0x7f
        ? 1
        : codePoint <= 0x7ff
          ? 2
          : codePoint <= 0xffff
            ? 3
            : 4;
  }
  return bytes;
}

function ChatLibraryPanel({
  panelRef,
  query,
  scope,
  activeSession,
  results,
  favorites,
  searchPending,
  favoritesLoading,
  pendingFavoriteSourceKeys,
  onQueryChange,
  onScopeChange,
  onSearch,
  onToggleFavorite,
  onOpen,
  onClose,
}: {
  panelRef: React.RefObject<HTMLElement | null>;
  query: string;
  scope: "all" | "session" | "favorites";
  activeSession: Session | undefined;
  results: ChatSearchHit[];
  favorites: ChatFavorite[];
  searchPending: boolean;
  favoritesLoading: boolean;
  pendingFavoriteSourceKeys: ReadonlySet<string>;
  onQueryChange: (query: string) => void;
  onScopeChange: (scope: "all" | "session" | "favorites") => void;
  onSearch: () => void;
  onToggleFavorite: (
    sessionId: string,
    turnId: string,
    favoriteId?: string,
    sourceKey?: string,
  ) => boolean;
  onOpen: (sessionId: string, turnId: string) => void;
  onClose: () => void;
}) {
  const [favoriteSort, setFavoriteSort] = useState<
    "time-desc" | "time-asc" | "size-desc" | "size-asc"
  >("time-desc");
  const [favoriteDeleteMode, setFavoriteDeleteMode] = useState(false);
  const [selectedFavoriteIds, setSelectedFavoriteIds] = useState<Set<string>>(
    new Set(),
  );
  const [visibleSearchCount, setVisibleSearchCount] = useState(
    CHAT_LIBRARY_INITIAL_ROWS,
  );
  const [visibleFavoriteCount, setVisibleFavoriteCount] = useState(
    CHAT_LIBRARY_INITIAL_ROWS,
  );
  const onOpenRef = useRef(onOpen);
  onOpenRef.current = onOpen;
  const openLibraryItem = useCallback(
    (sessionId: string, turnId: string) => onOpenRef.current(sessionId, turnId),
    [],
  );
  useEffect(() => {
    if (scope !== "favorites") {
      setFavoriteDeleteMode(false);
      setSelectedFavoriteIds(new Set());
    }
  }, [scope]);
  useEffect(() => {
    setVisibleSearchCount(CHAT_LIBRARY_INITIAL_ROWS);
  }, [results, scope]);
  useEffect(() => {
    setVisibleFavoriteCount(CHAT_LIBRARY_INITIAL_ROWS);
  }, [favorites, favoriteSort, query, scope]);
  useEffect(() => {
    const available = new Set(favorites.map((favorite) => favorite.id));
    setSelectedFavoriteIds((current) => {
      const next = new Set([...current].filter((id) => available.has(id)));
      return next.size === current.size ? current : next;
    });
  }, [favorites]);

  const searchItems = useMemo<ChatLibrarySearchItem[]>(
    () =>
      results.map((hit) => ({
        id: hit.source_key,
        sessionId: hit.session_id,
        sessionName: hit.session_display_name,
        turnId: hit.turn_id,
        title: hit.role === "assistant" ? "Assistant answer" : "User message",
        content: hit.content,
        createdAt: hit.created_at_ms,
        formattedDate: formatChatLibraryDate(hit.created_at_ms),
        role: hit.role,
      })),
    [results],
  );
  const visibleSearchItems = searchItems.slice(0, visibleSearchCount);
  const favoriteItems = useMemo<ChatLibraryFavoriteItem[]>(
    () =>
      favorites
        .map((favorite) => ({
          id: favorite.id,
          sourceKey: favorite.source_key,
          sessionId: favorite.session_id,
          sessionName: favorite.session_display_name,
          turnId: favorite.turn_id,
          title: favorite.title,
          content: favorite.content_snapshot,
          createdAt: favorite.source_created_at_ms,
          formattedDate: formatChatLibraryDate(favorite.source_created_at_ms),
          favoriteId: favorite.id,
          bytes: utf8ByteLength(favorite.content_snapshot),
        }))
        .sort((left, right) => {
          if (favoriteSort === "time-desc")
            return right.createdAt - left.createdAt;
          if (favoriteSort === "time-asc")
            return left.createdAt - right.createdAt;
          if (favoriteSort === "size-desc")
            return right.bytes - left.bytes || right.createdAt - left.createdAt;
          return left.bytes - right.bytes || right.createdAt - left.createdAt;
        }),
    [favorites, favoriteSort],
  );
  const normalizedFavoriteQuery = query.trim().toLocaleLowerCase();
  const filteredFavoriteItems = normalizedFavoriteQuery
    ? favoriteItems.filter((item) =>
        `${item.title}\n${item.content}\n${item.sessionName}`
          .toLocaleLowerCase()
          .includes(normalizedFavoriteQuery),
      )
    : favoriteItems;
  const visibleFavoriteItems = filteredFavoriteItems.slice(
    0,
    visibleFavoriteCount,
  );
  const showingFavorites = scope === "favorites";
  const itemCount = showingFavorites
    ? filteredFavoriteItems.length
    : searchItems.length;
  const loading = showingFavorites ? favoritesLoading : searchPending;
  const emptyLabel = showingFavorites
    ? normalizedFavoriteQuery
      ? "No matching favorites."
      : "Favorite final answers to keep them close."
    : query.trim()
      ? "No matching messages."
      : "Search across user messages and final answers.";
  const toggleFavoriteSelection = useCallback(
    (favoriteId: string) =>
      setSelectedFavoriteIds((current) => {
        const next = new Set(current);
        if (next.has(favoriteId)) next.delete(favoriteId);
        else next.add(favoriteId);
        return next;
      }),
    [],
  );
  const cancelFavoriteDelete = () => {
    setFavoriteDeleteMode(false);
    setSelectedFavoriteIds(new Set());
  };
  const deleteSelectedFavorites = () => {
    const selected = favoriteItems.filter((item) =>
      selectedFavoriteIds.has(item.id),
    );
    if (selected.length === 0) return;
    const confirmed = window.confirm(
      `Remove ${selected.length} selected favorite${selected.length === 1 ? "" : "s"}?`,
    );
    if (!confirmed) return;
    let allQueued = true;
    for (const item of selected) {
      if (
        !onToggleFavorite(
          item.sessionId,
          item.turnId,
          item.favoriteId,
          item.sourceKey,
        )
      )
        allQueued = false;
    }
    if (allQueued) cancelFavoriteDelete();
  };
  return createPortal(
    <div
      className="chat-library-center-backdrop"
      role="presentation"
      onClick={onClose}
    >
      <section
        id="chat-library-center"
        ref={panelRef}
        className="chat-library-center"
        role="dialog"
        aria-modal="true"
        aria-label="Chat library"
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
      >
        <header className="chat-library-header">
          <div>
            <span className="eyebrow">CHAT LIBRARY</span>
            <strong>Search</strong>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close chat library"
            aria-label="Close chat library"
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </header>
        <form
          className="chat-library-search"
          onSubmit={(event) => {
            event.preventDefault();
            onSearch();
          }}
        >
          <label>
            <Search size={14} />
            <input
              autoFocus
              value={query}
              maxLength={256}
              placeholder={showingFavorites ? "Filter favorites" : "Keywords"}
              aria-label={
                showingFavorites
                  ? "Filter favorite answers"
                  : "Search chat history"
              }
              onChange={(event) => onQueryChange(event.target.value)}
            />
            {query && (
              <button
                type="button"
                title="Clear search"
                aria-label="Clear search"
                onClick={() => onQueryChange("")}
              >
                <X size={12} />
              </button>
            )}
          </label>
          <div className="chat-library-scope">
            <label>
              <span>Search scope</span>
              <select
                aria-label="Search scope"
                value={scope}
                onChange={(event) =>
                  onScopeChange(
                    event.target.value as "all" | "session" | "favorites",
                  )
                }
              >
                <option value="all">All Sessions</option>
                <option value="session" disabled={!activeSession}>
                  Current Session
                </option>
                <option value="favorites">Favorites</option>
              </select>
            </label>
            {!showingFavorites && (
              <button
                type="submit"
                className="chat-library-submit"
                disabled={!query.trim() || searchPending}
              >
                {searchPending ? (
                  <LoaderCircle size={13} />
                ) : (
                  <ArrowDown className="chat-library-submit-arrow" size={13} />
                )}
                <span>Search</span>
              </button>
            )}
          </div>
        </form>
        <div
          className={`chat-library-summary ${showingFavorites ? "favorites" : ""}`}
        >
          <span>
            {showingFavorites
              ? `${itemCount} saved`
              : `${itemCount} result${itemCount === 1 ? "" : "s"}`}
          </span>
          {scope === "session" && activeSession && (
            <small title={activeSession.display_name}>
              {activeSession.display_name}
            </small>
          )}
          {showingFavorites && (
            <div className="chat-library-favorite-tools">
              {!favoriteDeleteMode && (
                <label>
                  Sort{" "}
                  <select
                    aria-label="Sort favorites"
                    value={favoriteSort}
                    onChange={(event) =>
                      setFavoriteSort(event.target.value as typeof favoriteSort)
                    }
                  >
                    <option value="time-desc">Newest</option>
                    <option value="time-asc">Oldest</option>
                    <option value="size-desc">Largest</option>
                    <option value="size-asc">Smallest</option>
                  </select>
                </label>
              )}
              {favoriteDeleteMode && (
                <button
                  type="button"
                  className="chat-library-delete-cancel"
                  onClick={cancelFavoriteDelete}
                >
                  Cancel
                </button>
              )}
              <button
                type="button"
                className={`chat-library-delete ${favoriteDeleteMode ? "confirm" : ""}`}
                disabled={
                  favoriteItems.length === 0 ||
                  (favoriteDeleteMode && selectedFavoriteIds.size === 0)
                }
                onClick={() =>
                  favoriteDeleteMode
                    ? deleteSelectedFavorites()
                    : setFavoriteDeleteMode(true)
                }
              >
                {favoriteDeleteMode ? (
                  <>
                    <Trash2 size={12} /> Delete {selectedFavoriteIds.size}
                  </>
                ) : (
                  <>
                    <Trash2 size={12} /> Delete
                  </>
                )}
              </button>
            </div>
          )}
        </div>
        {!showingFavorites ? (
          <div
            className="chat-library-list search-results"
            aria-busy={loading || undefined}
          >
            {loading && searchItems.length === 0 ? (
              <div className="chat-library-empty">
                <LoaderCircle size={16} />
                <span>Loading…</span>
              </div>
            ) : searchItems.length === 0 ? (
              <div className="chat-library-empty">
                <span>{emptyLabel}</span>
              </div>
            ) : (
              visibleSearchItems.map((item) => (
                <ChatLibrarySearchRow
                  item={item}
                  onOpen={openLibraryItem}
                  key={item.id}
                />
              ))
            )}
            {visibleSearchCount < searchItems.length && (
              <button
                type="button"
                className="chat-library-load-more"
                onClick={() =>
                  setVisibleSearchCount((current) =>
                    Math.min(
                      current + CHAT_LIBRARY_MORE_ROWS,
                      searchItems.length,
                    ),
                  )
                }
              >
                Show{" "}
                {Math.min(
                  CHAT_LIBRARY_MORE_ROWS,
                  searchItems.length - visibleSearchCount,
                )}{" "}
                more
              </button>
            )}
          </div>
        ) : (
          <div
            className={`chat-library-list favorites ${favoriteDeleteMode ? "selecting" : ""}`}
            aria-busy={loading || undefined}
          >
            {loading && favoriteItems.length === 0 ? (
              <div className="chat-library-empty">
                <LoaderCircle size={16} />
                <span>Loading…</span>
              </div>
            ) : favoriteItems.length === 0 ? (
              <div className="chat-library-empty">
                <span>{emptyLabel}</span>
              </div>
            ) : (
              visibleFavoriteItems.map((item) => (
                <ChatLibraryFavoriteRow
                  item={item}
                  deleteMode={favoriteDeleteMode}
                  selected={selectedFavoriteIds.has(item.id)}
                  pending={pendingFavoriteSourceKeys.has(item.sourceKey)}
                  onOpen={openLibraryItem}
                  onToggleSelection={toggleFavoriteSelection}
                  key={item.id}
                />
              ))
            )}
            {visibleFavoriteCount < filteredFavoriteItems.length && (
              <button
                type="button"
                className="chat-library-load-more"
                onClick={() =>
                  setVisibleFavoriteCount((current) =>
                    Math.min(
                      current + CHAT_LIBRARY_MORE_ROWS,
                      filteredFavoriteItems.length,
                    ),
                  )
                }
              >
                Show{" "}
                {Math.min(
                  CHAT_LIBRARY_MORE_ROWS,
                  filteredFavoriteItems.length - visibleFavoriteCount,
                )}{" "}
                more
              </button>
            )}
          </div>
        )}
      </section>
    </div>,
    document.body,
  );
}

function formatChatLibraryDate(value: number) {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleDateString([], {
    month: "short",
    day: "numeric",
    year:
      date.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
  });
}

function ToolRepoPanel({
  panelRef,
  onResizeStart,
  onClose,
  session,
  searchQuery,
  searchPending,
  onSearchQueryChange,
  tools,
  selectedTool,
  pendingToolDetailId,
  pendingToolRenameIds,
  onSelectTool,
  onCollapseTool,
  onRenameTool,
  onOpenTerminal,
}: {
  panelRef: MutableRefObject<HTMLElement | null>;
  onResizeStart: (event: React.PointerEvent<HTMLButtonElement>) => void;
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
  const [contextMenu, setContextMenu] = useState<{
    toolId: string;
    x: number;
    y: number;
  } | null>(null);
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
  const sortedTools = useMemo(
    () =>
      [...tools].sort((left, right) => {
        if (sort === "type")
          return (
            left.tool_type.localeCompare(right.tool_type) ||
            left.name.localeCompare(right.name)
          );
        if (sort === "language")
          return (
            left.language.localeCompare(right.language) ||
            left.name.localeCompare(right.name)
          );
        return (
          right.updated_at_ms - left.updated_at_ms ||
          left.name.localeCompare(right.name)
        );
      }),
    [sort, tools],
  );
  const pendingTool = pendingToolDetailId
    ? sortedTools.find((tool) => tool.tool_id === pendingToolDetailId)
    : undefined;
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
  const toolRepoEmptyTitle = !session
    ? "No active session"
    : searchPending
      ? "Searching ToolRepo…"
      : hasToolSearch
        ? "No matching tools"
        : "No reusable tools yet";
  const toolRepoEmptyText = !session
    ? "Select or create a session to browse its ToolRepo."
    : searchPending
      ? "Searching tool names and file contents. Results will update automatically."
      : hasToolSearch
        ? "Try a different keyword, or clear search to show all saved tools."
        : "Use ToolGen on a completed task to preserve a reusable script here.";
  const pendingToolDetailLabel = pendingTool
    ? `Loading ${pendingTool.name} tool directory`
    : "";
  const sortLabel = sort === "time" ? "recent update" : sort;
  const sortControlLabel = `Sort ToolRepo by ${sortLabel}`;
  return (
    <aside
      id="toolrepo-panel"
      ref={panelRef}
      className="toolrepo-side-panel session-side-panel"
      aria-label="ToolRepo"
      tabIndex={-1}
    >
      <button
        type="button"
        className="sidebar-resize-handle right"
        title="Resize ToolRepo"
        aria-label="Resize ToolRepo"
        onPointerDown={onResizeStart}
      />
      <header className="side-panel-header">
        <div className="side-panel-title">
          <Wrench size={15} />
          <strong>ToolRepo</strong>
        </div>
        <button
          type="button"
          className="icon-button"
          title="Close ToolRepo"
          aria-label="Close ToolRepo"
          onClick={onClose}
        >
          <X size={16} />
        </button>
      </header>
      <div className="toolrepo-panel">
        <div className="toolrepo-controls">
          <label
            className={searchPending ? "searching" : ""}
            aria-busy={searchPending}
          >
            <Search size={14} />
            <input
              value={searchQuery}
              disabled={!session}
              onChange={(event) => onSearchQueryChange(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Escape" && searchQuery) {
                  event.preventDefault();
                  event.stopPropagation();
                  onSearchQueryChange("");
                }
              }}
              placeholder={
                session ? "Search names and code" : "Select a session first"
              }
              aria-label="Search ToolRepo"
            />
            {searchPending && (
              <span className="toolrepo-search-pending" aria-hidden="true" />
            )}
            {hasToolSearch && (
              <button
                type="button"
                title="Clear ToolRepo search"
                aria-label="Clear ToolRepo search"
                onClick={() => onSearchQueryChange("")}
              >
                <X size={13} />
              </button>
            )}
          </label>
          <select
            value={sort}
            disabled={!session}
            onChange={(event) => setSort(event.target.value as typeof sort)}
            title={sortControlLabel}
            aria-label={sortControlLabel}
          >
            <option value="time">Recent</option>
            <option value="type">Type</option>
            <option value="language">Language</option>
          </select>
        </div>
        {session && (
          <div className="toolrepo-result-count" aria-live="polite">
            {toolRepoResultText}
          </div>
        )}
        {!sortedTools.length ? (
          <div
            className={`toolrepo-empty ${searchPending ? "searching" : ""}`}
            aria-label={`${toolRepoEmptyTitle}. ${toolRepoEmptyText}`}
            aria-busy={searchPending || undefined}
          >
            <Wrench size={20} />
            <strong>{toolRepoEmptyTitle}</strong>
            <span>{toolRepoEmptyText}</span>
          </div>
        ) : (
          <div className="toolrepo-browser">
            <div className="toolrepo-list" role="tree">
              {sortedTools.map((tool) => {
                const loadingDetail = pendingToolDetailId === tool.tool_id;
                const renamingTool = pendingToolRenameIds.has(tool.tool_id);
                const expanded = selectedTool?.summary.tool_id === tool.tool_id;
                const toolToggleLabel = expanded
                  ? `收起 ${tool.name} 详情`
                  : `展开 ${tool.name} 详情`;
                return (
                  <div
                    className={`toolrepo-item ${selectedTool?.summary.tool_id === tool.tool_id ? "selected" : ""} ${loadingDetail ? "loading-detail" : ""} ${renamingTool ? "renaming-tool" : ""}`}
                    role="treeitem"
                    tabIndex={0}
                    aria-selected={
                      selectedTool?.summary.tool_id === tool.tool_id
                    }
                    aria-expanded={expanded}
                    aria-busy={loadingDetail || renamingTool || undefined}
                    key={tool.tool_id}
                    onKeyDown={(event) => {
                      if (
                        event.target instanceof HTMLElement &&
                        (event.target.closest(
                          "button, input, select, textarea",
                        ) ||
                          event.target !== event.currentTarget)
                      )
                        return;
                      if (event.key === "Enter" || event.key === " ") {
                        event.preventDefault();
                        if (expanded) onCollapseTool();
                        else onSelectTool(tool.tool_id);
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
                    }}
                    onContextMenu={(event) => {
                      event.preventDefault();
                      setContextMenu({
                        toolId: tool.tool_id,
                        x: Math.max(
                          8,
                          Math.min(event.clientX, window.innerWidth - 220),
                        ),
                        y: Math.max(
                          8,
                          Math.min(event.clientY, window.innerHeight - 76),
                        ),
                      });
                    }}
                  >
                    <button
                      type="button"
                      className="toolrepo-item-main"
                      title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`}
                      aria-label={toolToggleLabel}
                      aria-expanded={expanded}
                      onClick={() => {
                        if (expanded) onCollapseTool();
                        else onSelectTool(tool.tool_id);
                      }}
                    >
                      <ChevronRight size={13} />
                      <span>
                        <strong>{tool.name}</strong>
                        <small>
                          {renamingTool
                            ? "Renaming..."
                            : loadingDetail
                              ? "Loading details..."
                              : `${tool.language} · ${tool.tool_type}`}
                        </small>
                        <em className="toolrepo-toggle-state">
                          {expanded ? "收起" : "展开"}
                        </em>
                      </span>
                    </button>
                    <button
                      type="button"
                      className="toolrepo-open"
                      title={`Open ${tool.name} directory in terminal`}
                      aria-label={`Open ${tool.name} directory in terminal`}
                      onClick={() => onOpenTerminal(tool.tool_id)}
                    >
                      <Terminal size={12} />
                    </button>
                    {renameToolId === tool.tool_id ? (
                      <input
                        className="toolrepo-rename"
                        autoFocus
                        value={renameValue}
                        aria-label={`Rename ${tool.name}`}
                        disabled={renamingTool}
                        onChange={(event) => setRenameValue(event.target.value)}
                        onBlur={() => finishToolRename(tool)}
                        onKeyDown={(event) => {
                          if (
                            event.key === "Enter" &&
                            !event.nativeEvent.isComposing
                          ) {
                            event.preventDefault();
                            finishToolRename(tool);
                          }
                          if (event.key === "Escape") {
                            event.preventDefault();
                            setRenameToolId("");
                            setRenameValue("");
                          }
                        }}
                      />
                    ) : (
                      <button
                        type="button"
                        className="toolrepo-edit"
                        title={
                          renamingTool
                            ? `Renaming ${tool.name}`
                            : `Rename ${tool.name}`
                        }
                        aria-label={
                          renamingTool
                            ? `Renaming ${tool.name}`
                            : `Rename ${tool.name}`
                        }
                        disabled={renamingTool}
                        onClick={() => {
                          setRenameToolId(tool.tool_id);
                          setRenameValue(tool.name);
                        }}
                      >
                        <Pencil size={12} />
                      </button>
                    )}
                  </div>
                );
              })}
            </div>
            {pendingTool ? (
              <section
                className="toolrepo-detail loading"
                aria-busy="true"
                aria-label={pendingToolDetailLabel}
              >
                <header>
                  <div>
                    <strong title={pendingTool.name}>{pendingTool.name}</strong>
                    <code>Reading tool directory…</code>
                  </div>
                  <div className="toolrepo-detail-actions">
                    <button
                      type="button"
                      className="toolrepo-detail-collapse"
                      title={`Stop viewing ${pendingTool.name} details`}
                      aria-label={`Stop viewing ${pendingTool.name} details`}
                      onClick={onCollapseTool}
                    >
                      收起详情
                    </button>
                  </div>
                </header>
                <div
                  className="toolrepo-detail-loading"
                  role="status"
                  aria-live="polite"
                  aria-label={pendingToolDetailLabel}
                >
                  <span
                    className="toolrepo-search-pending"
                    aria-hidden="true"
                  />
                  Reading directory tree...
                </div>
              </section>
            ) : (
              selectedTool && (
                <section className="toolrepo-detail">
                  <header>
                    <div>
                      <strong title={selectedTool.summary.name}>
                        {selectedTool.summary.name}
                      </strong>
                      <code title={selectedTool.summary.synopsis}>
                        {selectedTool.summary.synopsis}
                      </code>
                    </div>
                    <div className="toolrepo-detail-actions">
                      <button
                        type="button"
                        title="Open directory in terminal"
                        aria-label="Open directory in terminal"
                        onClick={() =>
                          onOpenTerminal(selectedTool.summary.tool_id)
                        }
                      >
                        <Terminal size={14} />
                      </button>
                      <button
                        type="button"
                        className="toolrepo-detail-collapse"
                        title="Collapse tool detail"
                        aria-label="Collapse tool detail"
                        onClick={onCollapseTool}
                      >
                        收起详情
                      </button>
                    </div>
                  </header>
                  <div
                    className="toolrepo-files"
                    aria-label="Tool directory tree"
                  >
                    {selectedTool.files.map((file) => (
                      <div
                        key={file.path}
                        title={`${file.path} · ${formatBytes(file.bytes)}`}
                        style={{
                          paddingLeft: `${8 + Math.max(0, file.path.split("/").length - 1) * 12}px`,
                        }}
                      >
                        <span>{file.path}</span>
                        <small>{formatBytes(file.bytes)}</small>
                      </div>
                    ))}
                  </div>
                </section>
              )
            )}
          </div>
        )}
      </div>
      {contextMenu && (
        <div
          className="toolrepo-context-menu"
          role="menu"
          aria-label="Tool actions"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onPointerDown={(event) => event.stopPropagation()}
          onKeyDownCapture={(event) => {
            if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              setContextMenu(null);
            }
          }}
        >
          <button
            ref={contextMenuActionRef}
            type="button"
            role="menuitem"
            onClick={() => {
              onOpenTerminal(contextMenu.toolId);
              setContextMenu(null);
            }}
          >
            <Terminal size={14} />
            在命令行中打开目录
          </button>
        </div>
      )}
    </aside>
  );
}

const EMPTY_DECISIONS: Decision[] = [];
const SessionTimelineActiveContext = createContext(false);

const VisibleTurnList = memo(function VisibleTurnList({
  sessionId,
  turns,
  restartMarkers,
  decisionsByTurn,
  sessionInteractionLocked,
  isCancelling,
  pendingDecisionKeys,
  pendingToolGenTurnIds,
  toolGenSessionBusy,
  favoriteBySource,
  pendingFavoriteSourceKeys,
  onToggleFavorite,
  onDecisionReply,
  onRequestToolGen,
  onRequestMessageDelete,
}: {
  sessionId: string;
  turns: WebTurn[];
  isCancelling: boolean;
  restartMarkers: ChatMessage[];
  decisionsByTurn: ReadonlyMap<string, Decision[]>;
  sessionInteractionLocked: boolean;
  pendingDecisionKeys: Set<string>;
  pendingToolGenTurnIds: Set<string>;
  toolGenSessionBusy: boolean;
  favoriteBySource: ReadonlyMap<string, ChatFavorite>;
  pendingFavoriteSourceKeys: ReadonlySet<string>;
  onToggleFavorite: (
    sessionId: string,
    turnId: string,
    favoriteId?: string,
  ) => boolean;
  onDecisionReply: (
    decision: Decision,
    reply: "accept" | "decline" | "always_allow",
  ) => void;
  onRequestToolGen?: (turnId: string) => void;
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
    if (item.type === "restart")
      return <RuntimeRestartDivider key={item.id} marker={item.marker} />;
    const turn = item.turn;
    return (
      <TurnInteraction
        key={turn.turn_id}
        sessionId={sessionId}
        turn={turn}
        isCancelling={isCancelling && turn.state === "working"}
        decisions={
          decisionsByTurn.get(sessionTurnKey(sessionId, turn.turn_id)) ??
          EMPTY_DECISIONS
        }
        sessionInteractionLocked={sessionInteractionLocked}
        pendingDecisionKeys={pendingDecisionKeys}
        toolGenPending={pendingToolGenTurnIds.has(turn.turn_id)}
        toolGenBlocked={
          toolGenSessionBusy && !pendingToolGenTurnIds.has(turn.turn_id)
        }
        favorite={favoriteBySource.get(
          `legacy:${sessionId}:${turn.turn_id}:assistant:0`,
        )}
        favoritePending={pendingFavoriteSourceKeys.has(
          `legacy:${sessionId}:${turn.turn_id}:assistant:0`,
        )}
        onToggleFavorite={onToggleFavorite}
        onDecisionReply={onDecisionReply}
        onRequestToolGen={onRequestToolGen}
        onRequestMessageDelete={onRequestMessageDelete}
      />
    );
  });
});

type SessionTimelinePaneProps = {
  session: Session;
  active: boolean;
  isCancelling: boolean;
  decisionsByTurn: ReadonlyMap<string, Decision[]>;
  sessionInteractionLocked: boolean;
  pendingDecisionKeys: Set<string>;
  pendingToolGenTurnIds: Set<string>;
  toolGenSessionBusy: boolean;
  favoriteBySource: ReadonlyMap<string, ChatFavorite>;
  pendingFavoriteSourceKeys: ReadonlySet<string>;
  onToggleFavorite: (
    sessionId: string,
    turnId: string,
    favoriteId?: string,
  ) => boolean;
  onDecisionReply: (
    decision: Decision,
    reply: "accept" | "decline" | "always_allow",
  ) => void;
  onRequestToolGen?: (turnId: string) => void;
  onRequestMessageDelete: (candidate: ChatMessageDeleteCandidate) => void;
};

const SessionTimelinePane = memo(function SessionTimelinePane({
  session,
  active,
  isCancelling,
  decisionsByTurn,
  sessionInteractionLocked,
  pendingDecisionKeys,
  pendingToolGenTurnIds,
  toolGenSessionBusy,
  favoriteBySource,
  pendingFavoriteSourceKeys,
  onToggleFavorite,
  onDecisionReply,
  onRequestToolGen,
  onRequestMessageDelete,
}: SessionTimelinePaneProps) {
  const renderedSessionRef = useRef(session);
  // Inactive cached timelines keep their last visible snapshot. Background
  // events must not repeatedly rebuild hidden Markdown on the active UI path;
  // the pane catches up synchronously when it becomes active again.
  if (active) renderedSessionRef.current = session;
  const renderedSession = renderedSessionRef.current;
  const visibleTurns = useMemo(
    () => renderedSession.turns.filter(turnShouldRenderInTimeline),
    [renderedSession.turns],
  );
  const restartMarkers = useMemo(
    () =>
      visibleRuntimeRestartMarkers(
        visibleTurns,
        renderedSession.messages.filter(
          (message) =>
            message.role === "system" && message.kind === "runtime_restart",
        ),
      ),
    [renderedSession.messages, visibleTurns],
  );
  return (
    <section
      className="session-timeline-pane"
      data-session-timeline-id={renderedSession.session_id}
      data-session-timeline-active={active ? "true" : "false"}
      hidden={!active}
      aria-hidden={!active || undefined}
    >
      <SessionTimelineActiveContext.Provider value={active}>
        <VisibleTurnList
          sessionId={renderedSession.session_id}
          turns={visibleTurns}
          restartMarkers={restartMarkers}
          isCancelling={active && isCancelling}
          decisionsByTurn={decisionsByTurn}
          sessionInteractionLocked={sessionInteractionLocked}
          pendingDecisionKeys={pendingDecisionKeys}
          pendingToolGenTurnIds={pendingToolGenTurnIds}
          toolGenSessionBusy={toolGenSessionBusy}
          favoriteBySource={favoriteBySource}
          pendingFavoriteSourceKeys={pendingFavoriteSourceKeys}
          onToggleFavorite={onToggleFavorite}
          onDecisionReply={onDecisionReply}
          onRequestToolGen={onRequestToolGen}
          onRequestMessageDelete={onRequestMessageDelete}
        />
      </SessionTimelineActiveContext.Provider>
    </section>
  );
});

function RuntimeRestartDivider({ marker }: { marker: ChatMessage }) {
  const restartedAt = new Date(marker.created_at_ms);
  const timeLabel = Number.isNaN(restartedAt.getTime())
    ? ""
    : restartedAt.toLocaleString([], {
        dateStyle: "medium",
        timeStyle: "medium",
      });
  return (
    <div
      className="runtime-restart-divider"
      role="separator"
      aria-label={`${marker.text}${timeLabel ? `，${timeLabel}` : ""}`}
    >
      <span aria-hidden="true" />
      <div>
        <strong>{marker.text}</strong>
        {timeLabel && (
          <time dateTime={restartedAt.toISOString()}>{timeLabel}</time>
        )}
      </div>
      <span aria-hidden="true" />
    </div>
  );
}

type SortableQueuedMessageRenderState = {
  setNodeRef: (node: HTMLElement | null) => void;
  style: CSSProperties;
  attributes: ReturnType<typeof useSortable>["attributes"];
  listeners: ReturnType<typeof useSortable>["listeners"];
  isDragging: boolean;
};

function SortableQueuedMessage({
  id,
  disabled,
  children,
}: {
  id: string;
  disabled: boolean;
  children: (state: SortableQueuedMessageRenderState) => ReactNode;
}) {
  const sortable = useSortable({
    id,
    disabled,
    transition: {
      duration: 180,
      easing: "cubic-bezier(.2, .8, .2, 1)",
    },
  });
  return children({
    setNodeRef: sortable.setNodeRef,
    style: {
      transform: CSS.Transform.toString(sortable.transform),
      transition: sortable.transition,
      zIndex: sortable.isDragging ? 4 : undefined,
    },
    attributes: sortable.attributes,
    listeners: sortable.listeners,
    isDragging: sortable.isDragging,
  });
}

function TimemThread({
  activeSession,
  sessions,
  completedTurnsBySession,
  commandAcks,
  onConsumeCommandAcks,
  persistedSubmitCommandId,
  reliableStorageScope,
  sessionIds,
  sessionInteractionLocked,
  sessionInteractionLockReason,
  decisions,
  fileInput,
  isCancelling,
  pendingAttachmentRemoveIds,
  pendingDecisionKeys,
  uploadingAttachment,
  uploadingAttachmentFile,
  loadingHistory,
  pendingToolGenTurnIds,
  toolGenSessionBusy,
  selectedRoleIds,
  onRolesConsumed,
  onLoadMoreHistory,
  onSend,
  onSendForSession,
  onCancel,
  onUpload,
  onRemoveAttachment,
  onDecisionReply,
  onRequestToolGen,
  favoriteBySource,
  pendingFavoriteSourceKeys,
  onToggleFavorite,
  onRequestMessageDelete,
}: {
  activeSession: Session | undefined;
  sessions: Session[];
  completedTurnsBySession: Record<
    string,
    { key: string; continuation: "normal" | "cancelled" | "blocked" }
  >;
  commandAcks: Record<string, Extract<WireEvent, { type: "command_ack" }>>;
  onConsumeCommandAcks: (commandIds: ReadonlySet<string>) => void;
  persistedSubmitCommandId?: string;
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
  favoriteBySource: ReadonlyMap<string, ChatFavorite>;
  pendingFavoriteSourceKeys: ReadonlySet<string>;
  onToggleFavorite: (
    sessionId: string,
    turnId: string,
    favoriteId?: string,
  ) => boolean;
  onLoadMoreHistory: (session: Session) => void;
  onSend: (text: string, commandId?: string) => boolean;
  onSendForSession: (
    sessionId: string,
    text: string,
    commandId?: string,
    attachmentIds?: readonly string[],
    forceSupplement?: boolean,
    roleIds?: readonly string[],
    forceNewTurn?: boolean,
  ) => boolean;
  selectedRoleIds: readonly string[];
  onRolesConsumed: (
    sessionId: string,
    expectedRoleIds?: readonly string[],
  ) => void;
  onCancel: (targetCommandId?: string) => Promise<void>;
  onUpload: (file: File) => Promise<void>;
  onRemoveAttachment: (attachmentId: string) => void;
  onDecisionReply: (
    decision: Decision,
    reply: "accept" | "decline" | "always_allow",
  ) => void;
  onRequestToolGen?: (turnId: string) => void;
  onRequestMessageDelete: (candidate: ChatMessageDeleteCandidate) => void;
}) {
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const composerTextareaRef = useRef<HTMLTextAreaElement | null>(null);
  const previousScrollMetrics = useRef<ScrollMetrics | null>(null);
  const sessionScrollPositionsRef = useRef<Map<string, SessionScrollPosition>>(
    new Map(),
  );
  const renderedSessionIdRef = useRef<string | undefined>(undefined);
  const restoredSessionIdRef = useRef<string | undefined>(undefined);
  const followThreadLatest = useRef(true);
  const [threadAwayFromBottom, setThreadAwayFromBottom] = useState(false);
  const [draftsBySession, setDraftsBySession] = useState<
    Record<string, string>
  >({});
  const [composerExpanded, setComposerExpanded] = useState(false);
  const [queuedMessagesBySession, setQueuedMessagesBySession] = useState<
    Record<string, QueuedMessage[]>
  >({});
  const queuedMessagesBySessionRef = useRef<Record<string, QueuedMessage[]>>(
    queuedMessagesBySession,
  );
  const [expandedQueueSessionIds, setExpandedQueueSessionIds] = useState<
    Set<string>
  >(() => new Set());
  const [collapsedQueuePanelSessionIds, setCollapsedQueuePanelSessionIds] =
    useState<Set<string>>(() => new Set());
  const [draggedQueueMessageId, setDraggedQueueMessageId] = useState<string>();
  const [queuedMessageOverId, setQueuedMessageOverId] = useState<string>();
  const queueDragSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const [editingQueuedMessage, setEditingQueuedMessage] = useState<{
    sessionId: string;
    id: string;
    text: string;
  }>();
  const queuedDispatchSessionIdsRef = useRef<Set<string>>(new Set());
  const queuedMessageClaimsRef = useRef<Set<string>>(new Set());
  const [queuedMessageClaims, setQueuedMessageClaims] = useState<Set<string>>(
    () => new Set(),
  );
  const [queuedMessagesPauseBySession, setQueuedMessagesPauseBySession] =
    useState<Record<string, QueuedMessagesPauseState>>({});
  const queuedMessagesPauseBySessionRef = useRef<
    Record<string, QueuedMessagesPauseState>
  >({});
  const queuedAutoContinueSessionIdsRef = useRef<Set<string>>(new Set());
  const processedCompletedTurnKeysRef = useRef<Map<string, string>>(new Map());
  const [queuedAutoContinueVersion, setQueuedAutoContinueVersion] = useState(0);
  const submittingDraftSessionIdsRef = useRef<Set<string>>(new Set());
  const submittingDraftStartedAtRef = useRef<Map<string, number>>(new Map());
  const directSubmissionsRef = useRef<
    Map<
      string,
      {
        commandId: string;
        text: string;
        roleIds: string[];
      }
    >
  >(new Map());
  const [submittingDraftSessionIds, setSubmittingDraftSessionIds] = useState<
    Set<string>
  >(() => new Set());
  const updateQueuedMessages = useCallback(
    (
      update: (
        current: Record<string, QueuedMessage[]>,
      ) => Record<string, QueuedMessage[]>,
    ) => {
      const previous = queuedMessagesBySessionRef.current;
      const next = update(previous);
      if (
        !reliableStorageScope ||
        !saveQueuedMessages(
          window.localStorage,
          reliableStorageScope,
          next,
          previous,
        )
      )
        return;
      queuedMessagesBySessionRef.current = next;
      setQueuedMessagesBySession(next);
    },
    [reliableStorageScope],
  );
  const releaseAllQueuedDispatches = useCallback(() => {
    queuedDispatchSessionIdsRef.current.clear();
    queuedMessageClaimsRef.current.clear();
    setQueuedMessageClaims(new Set());
    setDraggedQueueMessageId(undefined);
    setQueuedMessageOverId(undefined);
  }, []);
  const pauseQueuedMessages = useCallback(
    (sessionId: string, reason: string, source: QueuedMessagesPauseSource) => {
      const current =
        queuedMessagesPauseBySessionRef.current[sessionId] ?? null;
      const pause = stopQueuedAutoSend(current, reason, source, Date.now());
      if (pause === current) return false;
      if (
        reliableStorageScope &&
        !saveQueuedMessagesPause(
          window.localStorage,
          reliableStorageScope,
          sessionId,
          pause,
        )
      )
        return false;
      const next = {
        ...queuedMessagesPauseBySessionRef.current,
        [sessionId]: pause,
      };
      queuedMessagesPauseBySessionRef.current = next;
      queuedDispatchSessionIdsRef.current.delete(sessionId);
      if (
        releaseSessionQueuedMessageClaims(
          queuedMessageClaimsRef.current,
          sessionId,
        ) > 0
      ) {
        setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
      }
      setQueuedMessagesPauseBySession(next);
      return true;
    },
    [reliableStorageScope],
  );
  const resumeQueuedMessages = useCallback(
    (sessionId: string) => {
      if (
        reliableStorageScope &&
        !clearQueuedMessagesPause(
          window.localStorage,
          reliableStorageScope,
          sessionId,
        )
      )
        return false;
      const next = { ...queuedMessagesPauseBySessionRef.current };
      delete next[sessionId];
      queuedMessagesPauseBySessionRef.current = next;
      setQueuedMessagesPauseBySession(next);
      return true;
    },
    [reliableStorageScope],
  );
  const turns = activeSession?.turns ?? [];
  const activeSessionId = activeSession?.session_id;
  const queuedMessagesPause = activeSessionId
    ? (queuedMessagesPauseBySession[activeSessionId] ?? null)
    : null;
  const draft = draftForSession(draftsBySession, activeSessionId);
  const queuedMessages = activeSessionId
    ? (queuedMessagesBySession[activeSessionId] ?? [])
    : [];
  const displayQueuedMessages = activeSessionId
    ? unclaimedQueuedMessages(
        queuedMessages,
        queuedMessageClaims,
        activeSessionId,
      )
    : [];
  const queueExpanded =
    !!activeSessionId && expandedQueueSessionIds.has(activeSessionId);
  const queuePanelCollapsed =
    !!activeSessionId && collapsedQueuePanelSessionIds.has(activeSessionId);
  const firstQueuedMessage = displayQueuedMessages[0];
  const visibleQueuedMessages = queueExpanded
    ? displayQueuedMessages
    : displayQueuedMessages.slice(0, COLLAPSED_QUEUE_LIMIT);
  const hiddenQueuedMessageCount = Math.max(
    0,
    displayQueuedMessages.length - COLLAPSED_QUEUE_LIMIT,
  );
  const draggedQueuedMessage = draggedQueueMessageId
    ? visibleQueuedMessages.find(
        (message) => message.id === draggedQueueMessageId,
      )
    : undefined;
  const draggedQueuedMessagePosition = draggedQueuedMessage
    ? reorderQueuedMessages(
        queuedMessages,
        draggedQueuedMessage.id,
        queuedMessageOverId ?? draggedQueuedMessage.id,
      ).findIndex((message) => message.id === draggedQueuedMessage.id) + 1
    : 0;
  const reservedAttachmentIds = useMemo(
    () => reservedQueuedAttachmentIds(queuedMessages),
    [queuedMessages],
  );
  const availableAttachments = useMemo(
    () =>
      (activeSession?.attachments ?? []).filter(
        (attachment) => !reservedAttachmentIds.has(attachment.id),
      ),
    [activeSession?.attachments, reservedAttachmentIds],
  );
  const selectedRoles =
    activeSession?.roles.filter((role) => selectedRoleIds.includes(role.id)) ??
    [];
  const submittingDraft =
    !!activeSessionId && submittingDraftSessionIds.has(activeSessionId);
  const pendingDirectSubmission = activeSessionId
    ? directSubmissionsRef.current.get(activeSessionId)
    : undefined;
  const interactionPhase = turnInteractionPhase(
    activeSession,
    pendingDirectSubmission?.commandId ?? persistedSubmitCommandId,
    isCancelling,
  );
  const hasDraftText = !!draft.trim();
  const showStopAction =
    composerPrimaryAction(interactionPhase, draft) === "stop";
  const sendLabel =
    activeSession?.state === "working" ? "Queue message" : "Send message";
  const lockedControlHint = sessionInteractionLocked
    ? sessionInteractionLockReason
    : "";
  const missingSessionHint = activeSession
    ? ""
    : "Create a session before using Timem";
  const uploadingAttachmentText = uploadingAttachmentFile
    ? `Uploading ${uploadingAttachmentFile.name}`
    : "Uploading file…";
  const composerHint =
    missingSessionHint ||
    lockedControlHint ||
    (uploadingAttachment
      ? `${uploadingAttachmentText} · send is paused until it finishes`
      : activeSession?.state === "working"
        ? "Enter to queue · use 立即 to send during this turn"
        : "Enter to send · Shift+Enter for newline");
  const attachTitle =
    missingSessionHint ||
    lockedControlHint ||
    (uploadingAttachment ? uploadingAttachmentText : "Attach a file");
  const attachLabel =
    missingSessionHint ||
    lockedControlHint ||
    (uploadingAttachment ? uploadingAttachmentText : "Attach a file");
  const effectiveSendLabel =
    missingSessionHint ||
    lockedControlHint ||
    (submittingDraft
      ? "Sending…"
      : uploadingAttachment
        ? "Wait for file upload"
        : sendLabel);
  const attachedFileCount = activeSession?.attachments.length ?? 0;
  const attachmentSummary =
    attachedFileCount === 1
      ? "1 file attached"
      : `${attachedFileCount} files attached`;
  const attachmentStripLabel = uploadingAttachment
    ? `${attachmentSummary}; ${uploadingAttachmentText}`
    : `Files attached to the next message; ${attachmentSummary}`;
  const composerHintId = `composer-hint-${activeSessionId || "empty"}`;
  const canLoadStoredHistory =
    !!activeSession?.history_has_more && !!activeSession.history_before_cursor;
  const decisionsByTurn = useMemo(
    () => groupDecisionsBySessionTurn(decisions),
    [decisions],
  );
  const historyButtonLabel = sessionInteractionLocked
    ? `${sessionInteractionLockReason} · earlier history is locked`
    : loadingHistory
      ? "Loading earlier history…"
      : `Load ${STORED_HISTORY_PAGE_SIZE} older stored tasks`;
  const latestTurn = turns.at(-1);
  const latestTurnVersion = `${latestTurn?.turn_id ?? ""}:${latestTurn?.events.length ?? 0}:${latestTurn?.user_entries.length ?? 0}:${latestTurn?.final_answer?.length ?? 0}:${latestTurn?.completion ? 1 : 0}`;
  const liveSessionKey = sessionIds.join("\u0000");
  const liveSessionIds = useMemo(() => new Set(sessionIds), [liveSessionKey]);
  const [recentTimelineSessionIds, setRecentTimelineSessionIds] = useState<
    string[]
  >([]);
  const mountedTimelineSessionIds = useMemo(
    () =>
      reconcileSessionTimelineCache(
        recentTimelineSessionIds,
        activeSessionId,
        sessionIds,
        MAX_MOUNTED_SESSION_TIMELINES,
      ),
    [activeSessionId, liveSessionKey, recentTimelineSessionIds],
  );
  const mountedTimelineSessionIdSet = useMemo(
    () => new Set(mountedTimelineSessionIds),
    [mountedTimelineSessionIds],
  );
  // LRU order controls eviction only. Keep the host Session order stable so a
  // warm A/B switch toggles visibility instead of moving two large DOM trees.
  const mountedTimelineSessions = sessions.filter((session) =>
    mountedTimelineSessionIdSet.has(session.session_id),
  );
  useEffect(() => {
    setRecentTimelineSessionIds((current) => {
      const next = reconcileSessionTimelineCache(
        current,
        activeSessionId,
        sessionIds,
        MAX_MOUNTED_SESSION_TIMELINES,
      );
      return current.length === next.length &&
        current.every((sessionId, index) => sessionId === next[index])
        ? current
        : next;
    });
  }, [activeSessionId, liveSessionKey]);
  const welcomeTitle = activeSession
    ? "Ready when you are."
    : "Create a session to start.";
  const welcomeText = activeSession
    ? "Ask Timem to investigate, write, or work with you."
    : "Use New session to choose a workspace and runtime profile.";
  const [userMessageNavigation, setUserMessageNavigation] = useState({
    previous: false,
    next: false,
    bottom: false,
  });
  const [userMessageNavigationLayout, setUserMessageNavigationLayout] =
    useState<{ left?: number; overlap: "none" | "partial" | "full" }>({
      overlap: "none",
    });

  const [
    userMessageNavigationHoverLocked,
    setUserMessageNavigationHoverLocked,
  ] = useState(false);
  const userMessageNavigationRef = useRef<HTMLElement | null>(null);
  const userMessageNavigationHoverLockedRef = useRef(false);
  const pendingUserMessageNavigationLayoutRef = useRef<{
    left?: number;
    overlap: "none" | "partial" | "full";
  } | null>(null);
  const userMessageNavigationOffset = 18;
  const userMessageNavigationBodyGap = 16;
  const userMessageNavigationEdgeInset = 10;
  const userMessageNavigationAnimationRef = useRef<number | null>(null);
  const userMessageAnchorOffsetsRef = useRef<number[]>([]);
  const userMessageNavigationTaskRef = useRef<FrameTask | null>(null);
  const userMessageGeometryTaskRef = useRef<FrameTask | null>(null);
  const userMessageNavigationLayoutTaskRef = useRef<FrameTask | null>(null);

  useLayoutEffect(() => {
    if (userMessageNavigationAnimationRef.current !== null) {
      cancelAnimationFrame(userMessageNavigationAnimationRef.current);
      userMessageNavigationAnimationRef.current = null;
    }
    userMessageAnchorOffsetsRef.current = [];
    userMessageNavigationHoverLockedRef.current = false;
    pendingUserMessageNavigationLayoutRef.current = null;
    setUserMessageNavigationHoverLocked(false);
    setUserMessageNavigation({ previous: false, next: false, bottom: false });
    setUserMessageNavigationLayout({ overlap: "none" });
    setComposerExpanded(false);
    setDraggedQueueMessageId(undefined);
    setQueuedMessageOverId(undefined);
    setEditingQueuedMessage(undefined);
  }, [activeSessionId]);

  const userMessageAnchors = useCallback(() => {
    const viewport = viewportRef.current;
    return viewport
      ? Array.from(
          viewport.querySelectorAll<HTMLElement>(
            '[data-session-timeline-active="true"] [data-user-message-anchor]',
          ),
        )
      : [];
  }, []);

  const applyUserMessageNavigationLayout = useCallback(
    (next: { left?: number; overlap: "none" | "partial" | "full" }) => {
      if (userMessageNavigationHoverLockedRef.current) {
        pendingUserMessageNavigationLayoutRef.current = next;
        return;
      }
      setUserMessageNavigationLayout((current) =>
        current.left === next.left && current.overlap === next.overlap
          ? current
          : next,
      );
    },
    [],
  );

  const updateUserMessageNavigationLayout = useCallback(() => {
    const viewport = viewportRef.current;
    const navigation = userMessageNavigationRef.current;
    if (!viewport || !navigation) return;
    const viewportRect = viewport.getBoundingClientRect();
    const navigationRect = navigation.getBoundingClientRect();
    const navigationCenter = navigationRect.top + navigationRect.height / 2;
    const expandedOutlines = Array.from(
      viewport.querySelectorAll<HTMLElement>(
        '[data-session-timeline-active="true"] .final-answer-outline.expanded',
      ),
    );
    const activeOutline = expandedOutlines.find((candidate) => {
      const rect = candidate.getBoundingClientRect();
      return rect.top <= navigationCenter && rect.bottom >= navigationCenter;
    });
    if (!activeOutline) {
      applyUserMessageNavigationLayout({ overlap: "none" });
      return;
    }
    const readingId = activeOutline.dataset.finalAnswerReadingId;
    const reading = readingId ? document.getElementById(readingId) : null;
    const card = activeOutline.querySelector<HTMLElement>(
      ".final-answer-outline-card",
    );
    if (!reading || !card) return;
    const readingRect = reading.getBoundingClientRect();
    const cardRect = card.getBoundingClientRect();
    const next = markdownFloatingNavigationLayout(
      readingRect.left - viewportRect.left,
      navigationRect.width,
      userMessageNavigationBodyGap,
      viewportRect.width,
      userMessageNavigationEdgeInset,
      cardRect.left - viewportRect.left,
      cardRect.right - viewportRect.left,
    );

    applyUserMessageNavigationLayout(next);
  }, [
    applyUserMessageNavigationLayout,
    userMessageNavigationBodyGap,
    userMessageNavigationEdgeInset,
  ]);

  const lockUserMessageNavigationLayout = useCallback(() => {
    const viewport = viewportRef.current;
    const navigation = userMessageNavigationRef.current;
    if (!viewport || !navigation || userMessageNavigationHoverLockedRef.current)
      return;
    userMessageNavigationHoverLockedRef.current = true;
    setUserMessageNavigationHoverLocked(true);
    const viewportRect = viewport.getBoundingClientRect();
    const navigationRect = navigation.getBoundingClientRect();
    setUserMessageNavigationLayout((current) => {
      pendingUserMessageNavigationLayoutRef.current = current;
      return { ...current, left: navigationRect.left - viewportRect.left };
    });
  }, []);

  const unlockUserMessageNavigationLayout = useCallback(() => {
    if (!userMessageNavigationHoverLockedRef.current) return;
    userMessageNavigationHoverLockedRef.current = false;
    setUserMessageNavigationHoverLocked(false);
    const pending = pendingUserMessageNavigationLayoutRef.current;
    pendingUserMessageNavigationLayoutRef.current = null;
    if (pending) setUserMessageNavigationLayout(pending);
    window.requestAnimationFrame(updateUserMessageNavigationLayout);
  }, [updateUserMessageNavigationLayout]);

  const updateUserMessageNavigation = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) {
      setUserMessageNavigation((current) =>
        current.previous || current.next || current.bottom
          ? { previous: false, next: false, bottom: false }
          : current,
      );
      return;
    }
    const navigationTop = viewport.scrollTop + userMessageNavigationOffset;
    const anchorOffsets = userMessageAnchorOffsetsRef.current;
    const nextUserMessageAvailable =
      adjacentUserMessageIndex(anchorOffsets, navigationTop, "next") >= 0;
    const next = {
      previous:
        adjacentUserMessageIndex(anchorOffsets, navigationTop, "previous") >= 0,
      next: nextUserMessageAvailable,
      bottom:
        !nextUserMessageAvailable &&
        !isNearScrollBottom({
          scrollTop: viewport.scrollTop,
          scrollHeight: viewport.scrollHeight,
          clientHeight: viewport.clientHeight,
        }),
    };
    setUserMessageNavigation((current) =>
      current.previous === next.previous &&
      current.next === next.next &&
      current.bottom === next.bottom
        ? current
        : next,
    );
  }, [userMessageNavigationOffset]);

  const refreshUserMessageGeometry = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) {
      userMessageAnchorOffsetsRef.current = [];
      updateUserMessageNavigation();
      return;
    }
    const viewportTop = viewport.getBoundingClientRect().top;
    const scrollTop = viewport.scrollTop;
    userMessageAnchorOffsetsRef.current = userMessageAnchors().map(
      (anchor) => scrollTop + anchor.getBoundingClientRect().top - viewportTop,
    );
    updateUserMessageNavigation();
  }, [updateUserMessageNavigation, userMessageAnchors]);

  const navigateUserMessage = useCallback(
    (direction: UserMessageNavigationDirection) => {
      const viewport = viewportRef.current;
      if (!viewport) return;
      const anchors = userMessageAnchors();
      if (userMessageAnchorOffsetsRef.current.length !== anchors.length)
        refreshUserMessageGeometry();
      const anchorOffsets = userMessageAnchorOffsetsRef.current;
      const navigationTop = viewport.scrollTop + userMessageNavigationOffset;
      const index = adjacentUserMessageIndex(
        anchorOffsets,
        navigationTop,
        direction,
      );
      if (!anchors[index]) return;
      followThreadLatest.current = false;
      const targetTop = anchorOffsets[index] - userMessageNavigationOffset;
      if (userMessageNavigationAnimationRef.current !== null)
        cancelAnimationFrame(userMessageNavigationAnimationRef.current);
      if (prefersReducedMotion()) {
        viewport.scrollTop = targetTop;
        updateUserMessageNavigation();
        return;
      }
      const startTop = viewport.scrollTop;
      const distance = targetTop - startTop;
      const startedAt = performance.now();
      const durationMs = 180;
      const animate = (now: number) => {
        const progress = Math.min(1, (now - startedAt) / durationMs);
        const eased = 1 - Math.pow(1 - progress, 3);
        viewport.scrollTop = startTop + distance * eased;
        if (progress < 1) {
          userMessageNavigationAnimationRef.current =
            requestAnimationFrame(animate);
        } else {
          userMessageNavigationAnimationRef.current = null;
          updateUserMessageNavigation();
        }
      };
      userMessageNavigationAnimationRef.current =
        requestAnimationFrame(animate);
    },
    [
      refreshUserMessageGeometry,
      updateUserMessageNavigation,
      userMessageAnchors,
      userMessageNavigationOffset,
    ],
  );

  const navigateToThreadBottom = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    if (userMessageNavigationAnimationRef.current !== null) {
      cancelAnimationFrame(userMessageNavigationAnimationRef.current);
      userMessageNavigationAnimationRef.current = null;
    }
    followThreadLatest.current = true;
    viewport.scrollTo({
      top: viewport.scrollHeight,
      behavior: prefersReducedMotion() ? "auto" : "smooth",
    });
  }, []);

  const navigateWorkingToThreadBottom = useCallback(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    if (userMessageNavigationAnimationRef.current !== null)
      cancelAnimationFrame(userMessageNavigationAnimationRef.current);
    followThreadLatest.current = true;
    const targetTop = viewport.scrollHeight;
    if (prefersReducedMotion()) {
      viewport.scrollTop = targetTop;
      userMessageNavigationAnimationRef.current = null;
      updateUserMessageNavigation();
      return;
    }
    const startTop = viewport.scrollTop;
    const distance = targetTop - startTop;
    const startedAt = performance.now();
    const durationMs = 90;
    const animate = (now: number) => {
      const progress = Math.min(1, (now - startedAt) / durationMs);
      const eased = 1 - Math.pow(1 - progress, 3);
      viewport.scrollTop = startTop + distance * eased;
      if (progress < 1) {
        userMessageNavigationAnimationRef.current =
          requestAnimationFrame(animate);
      } else {
        userMessageNavigationAnimationRef.current = null;
        updateUserMessageNavigation();
      }
    };
    userMessageNavigationAnimationRef.current = requestAnimationFrame(animate);
  }, [updateUserMessageNavigation]);

  useLayoutEffect(() => {
    const navigationTask = createFrameTask({
      run: updateUserMessageNavigation,
    });
    const geometryTask = createFrameTask({ run: refreshUserMessageGeometry });
    const layoutTask = createFrameTask({
      run: updateUserMessageNavigationLayout,
    });
    userMessageNavigationTaskRef.current = navigationTask;
    userMessageGeometryTaskRef.current = geometryTask;
    userMessageNavigationLayoutTaskRef.current = layoutTask;
    return () => {
      navigationTask.dispose();
      geometryTask.dispose();
      layoutTask.dispose();
      if (userMessageNavigationTaskRef.current === navigationTask)
        userMessageNavigationTaskRef.current = null;
      if (userMessageGeometryTaskRef.current === geometryTask)
        userMessageGeometryTaskRef.current = null;
      if (userMessageNavigationLayoutTaskRef.current === layoutTask)
        userMessageNavigationLayoutTaskRef.current = null;
    };
  }, [
    refreshUserMessageGeometry,
    updateUserMessageNavigation,
    updateUserMessageNavigationLayout,
  ]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    let outlineIntersectionObserver: IntersectionObserver | undefined;
    const work = () => ({
      navigation: userMessageNavigationTaskRef.current,
      geometry: userMessageGeometryTaskRef.current,
      layout: userMessageNavigationLayoutTaskRef.current,
    });
    const observeOutlineCrossings = () => {
      outlineIntersectionObserver?.disconnect();
      outlineIntersectionObserver = undefined;
      const navigation = userMessageNavigationRef.current;
      if (typeof IntersectionObserver === "undefined" || !navigation) return;
      const viewportRect = viewport.getBoundingClientRect();
      const navigationRect = navigation.getBoundingClientRect();
      const center = Math.max(
        1,
        Math.min(
          viewport.clientHeight - 1,
          navigationRect.top + navigationRect.height / 2 - viewportRect.top,
        ),
      );
      outlineIntersectionObserver = new IntersectionObserver(
        () => requestTimelineNavigationWork("layout", work()),
        {
          root: viewport,
          rootMargin: `${-Math.max(0, center - 1)}px 0px ${-Math.max(0, viewport.clientHeight - center - 1)}px 0px`,
          threshold: 0,
        },
      );
      viewport
        .querySelectorAll<HTMLElement>(
          '[data-session-timeline-active="true"] .final-answer-outline.expanded',
        )
        .forEach((outline) => outlineIntersectionObserver?.observe(outline));
    };
    const update = () => {
      requestTimelineNavigationWork("layout", work());
      observeOutlineCrossings();
    };
    update();
    window.addEventListener("resize", update);
    window.addEventListener("markdown-outline-layout-change", update);
    const observer =
      typeof ResizeObserver === "undefined"
        ? undefined
        : new ResizeObserver(update);
    observer?.observe(viewport);
    return () => {
      window.removeEventListener("resize", update);
      window.removeEventListener("markdown-outline-layout-change", update);
      observer?.disconnect();
      outlineIntersectionObserver?.disconnect();
    };
  }, [activeSessionId]);

  useEffect(
    () => () => {
      if (userMessageNavigationAnimationRef.current !== null)
        cancelAnimationFrame(userMessageNavigationAnimationRef.current);
    },
    [],
  );

  useEffect(() => {
    const viewport = viewportRef.current;
    if (!viewport) return;
    const update = () =>
      requestTimelineNavigationWork("content", {
        navigation: userMessageNavigationTaskRef.current,
        geometry: userMessageGeometryTaskRef.current,
        layout: userMessageNavigationLayoutTaskRef.current,
      });
    update();
    const mutationObserver = new MutationObserver(update);
    mutationObserver.observe(viewport, { childList: true, subtree: true });
    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? undefined
        : new ResizeObserver(update);
    resizeObserver?.observe(viewport);
    const activePane = viewport.querySelector<HTMLElement>(
      '[data-session-timeline-active="true"]',
    );
    if (activePane) resizeObserver?.observe(activePane);
    return () => {
      mutationObserver.disconnect();
      resizeObserver?.disconnect();
    };
  }, [activeSessionId]);

  useEffect(() => {
    const textarea = composerTextareaRef.current;
    if (!textarea) return;
    const prioritizeComposerScroll = (event: WheelEvent) => {
      if (document.activeElement !== textarea) return;
      const deltaY = wheelDeltaPixels(
        event.deltaY,
        event.deltaMode,
        textarea.clientHeight,
      );
      if (!canScrollInDirection(textarea, deltaY)) return;
      event.preventDefault();
      event.stopPropagation();
      textarea.scrollTop += deltaY;
    };
    textarea.addEventListener("wheel", prioritizeComposerScroll, {
      passive: false,
    });
    return () =>
      textarea.removeEventListener("wheel", prioritizeComposerScroll);
  }, []);

  useLayoutEffect(() => {
    window.dispatchEvent(new Event("session-timeline-activation-change"));
  }, [activeSessionId]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    renderedSessionIdRef.current = activeSessionId;
    if (!viewport || !activeSessionId) return;
    const position = sessionScrollPositionsRef.current.get(activeSessionId);
    followThreadLatest.current = position?.followLatest ?? true;
    restoredSessionIdRef.current = activeSessionId;
    const previousBehavior = viewport.style.scrollBehavior;
    viewport.style.scrollBehavior = "auto";
    viewport.scrollTop = restoreSessionScrollTop(
      position,
      viewport.scrollHeight,
    );
    viewport.style.scrollBehavior = previousBehavior;
  }, [activeSessionId]);

  useEffect(() => {
    if (!reliableStorageScope) return;
    const restored = loadQueuedMessages(
      window.localStorage,
      reliableStorageScope,
    );
    const restoredPauses = Object.fromEntries(
      sessionIds.flatMap((sessionId) => {
        const pause = loadQueuedMessagesPause(
          window.localStorage,
          reliableStorageScope,
          sessionId,
        );
        return pause ? [[sessionId, pause] as const] : [];
      }),
    );
    queuedMessagesBySessionRef.current = restored;
    releaseAllQueuedDispatches();
    setQueuedMessagesBySession(restored);
    queuedMessagesPauseBySessionRef.current = restoredPauses;
    setQueuedMessagesPauseBySession(restoredPauses);
  }, [liveSessionKey, releaseAllQueuedDispatches, reliableStorageScope]);

  useEffect(() => {
    const syncCrossTabQueues = (event: StorageEvent) => {
      if (!reliableStorageScope || !event.key) return;
      const pauseSessionId = queuedMessagesPauseSessionId(
        reliableStorageScope,
        event.key,
      );
      if (pauseSessionId && liveSessionIds.has(pauseSessionId)) {
        const restoredPause = loadQueuedMessagesPause(
          window.localStorage,
          reliableStorageScope,
          pauseSessionId,
        );
        const next = { ...queuedMessagesPauseBySessionRef.current };
        if (restoredPause) {
          next[pauseSessionId] = restoredPause;
          queuedDispatchSessionIdsRef.current.delete(pauseSessionId);
          if (
            releaseSessionQueuedMessageClaims(
              queuedMessageClaimsRef.current,
              pauseSessionId,
            ) > 0
          ) {
            setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
          }
        } else {
          delete next[pauseSessionId];
        }
        queuedMessagesPauseBySessionRef.current = next;
        setQueuedMessagesPauseBySession(next);
        return;
      }
      if (
        !event.key.startsWith(
          `${queuedMessagesStorageKey(reliableStorageScope)}:`,
        )
      )
        return;
      const restored = loadQueuedMessages(
        window.localStorage,
        reliableStorageScope,
      );
      queuedMessagesBySessionRef.current = restored;
      for (const [sessionId, messages] of Object.entries(restored)) {
        if (messages.some((message) => message.deliveryError))
          queuedDispatchSessionIdsRef.current.delete(sessionId);
      }
      for (const key of Array.from(queuedMessageClaimsRef.current)) {
        if (
          !Object.entries(restored).some(([sessionId, messages]) =>
            messages.some(
              (message) => queuedMessageKey(sessionId, message.id) === key,
            ),
          )
        ) {
          queuedMessageClaimsRef.current.delete(key);
        }
      }
      setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
      setQueuedMessagesBySession(restored);
    };
    window.addEventListener("storage", syncCrossTabQueues);
    return () => window.removeEventListener("storage", syncCrossTabQueues);
  }, [liveSessionIds, reliableStorageScope]);

  useEffect(() => {
    if (sessionIds.length === 0) return;
    setDraftsBySession((current) => pruneSessionDrafts(current, sessionIds));
    updateQueuedMessages((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([sessionId]) =>
          liveSessionIds.has(sessionId),
        ),
      ),
    );
    setExpandedQueueSessionIds(
      (current) =>
        new Set(
          Array.from(current).filter((sessionId) =>
            liveSessionIds.has(sessionId),
          ),
        ),
    );
    setCollapsedQueuePanelSessionIds(
      (current) =>
        new Set(
          Array.from(current).filter((sessionId) =>
            liveSessionIds.has(sessionId),
          ),
        ),
    );
    const retainedPauses = Object.fromEntries(
      Object.entries(queuedMessagesPauseBySessionRef.current).filter(
        ([sessionId]) => liveSessionIds.has(sessionId),
      ),
    );
    queuedMessagesPauseBySessionRef.current = retainedPauses;
    for (const sessionId of Array.from(
      processedCompletedTurnKeysRef.current.keys(),
    )) {
      if (!liveSessionIds.has(sessionId))
        processedCompletedTurnKeysRef.current.delete(sessionId);
    }
    for (const sessionId of Array.from(
      queuedAutoContinueSessionIdsRef.current,
    )) {
      if (!liveSessionIds.has(sessionId))
        queuedAutoContinueSessionIdsRef.current.delete(sessionId);
    }
    setQueuedMessagesPauseBySession(retainedPauses);
    setEditingQueuedMessage((current) =>
      current && liveSessionIds.has(current.sessionId) ? current : undefined,
    );
    for (const sessionId of Array.from(queuedDispatchSessionIdsRef.current)) {
      if (!liveSessionIds.has(sessionId))
        queuedDispatchSessionIdsRef.current.delete(sessionId);
    }
    for (const key of Array.from(queuedMessageClaimsRef.current)) {
      if (!liveSessionIds.has(key.slice(0, key.indexOf("\u0000"))))
        queuedMessageClaimsRef.current.delete(key);
    }
    setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
    for (const sessionId of Array.from(directSubmissionsRef.current.keys())) {
      if (!liveSessionIds.has(sessionId))
        directSubmissionsRef.current.delete(sessionId);
    }
    if (pruneSessionSubmissionLocks(submittingDraftSessionIdsRef, sessionIds)) {
      setSubmittingDraftSessionIds(
        new Set(submittingDraftSessionIdsRef.current),
      );
    }
  }, [liveSessionIds, updateQueuedMessages]);

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
          directSubmissionReleased =
            releaseSessionDraftSubmission(
              submittingDraftSessionIdsRef,
              sessionId,
            ) || directSubmissionReleased;
          break;
        }
        continue;
      }
      if (ack.status === "accepted") continue;
      const result = applyQueuedMessagesAck(
        nextQueues,
        ack.command_id,
        ack.status,
        ack.error,
        clientId("queued"),
      );
      if (!result.matchedSessionId) continue;
      appliedCommandIds.add(ack.command_id);
      matchedSessionByCommand.set(ack.command_id, result.matchedSessionId);
      if (ack.status === "rejected")
        rejectedSessionIds.add(result.matchedSessionId);
      nextQueues = result.queues;
    }
    if (appliedCommandIds.size === 0) return;
    const queuesChanged = matchedSessionByCommand.size > 0;
    if (
      queuesChanged &&
      (!reliableStorageScope ||
        !saveQueuedMessages(
          window.localStorage,
          reliableStorageScope,
          nextQueues,
          queuedMessagesBySessionRef.current,
        ))
    )
      return;
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
      setSubmittingDraftSessionIds(
        new Set(submittingDraftSessionIdsRef.current),
      );
    }
    for (const [commandId, sessionId] of matchedSessionByCommand) {
      releaseQueuedMessageClaim(
        queuedMessageClaimsRef.current,
        sessionId,
        commandId,
      );
    }
    setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
    for (const sessionId of rejectedSessionIds)
      queuedDispatchSessionIdsRef.current.delete(sessionId);
    for (const sessionId of new Set([
      ...rejectedSessionIds,
      ...rejectedDirectDrafts.keys(),
    ])) {
      queuedAutoContinueSessionIdsRef.current.delete(sessionId);
    }
    onConsumeCommandAcks(appliedCommandIds);
  }, [commandAcks, onConsumeCommandAcks, reliableStorageScope]);

  useEffect(() => {
    let completionChanged = false;
    let claimsChanged = false;
    let draftLocksChanged = false;
    for (const [sessionId, completion] of Object.entries(
      completedTurnsBySession,
    )) {
      if (
        processedCompletedTurnKeysRef.current.get(sessionId) === completion.key
      )
        continue;
      processedCompletedTurnKeysRef.current.set(sessionId, completion.key);
      if (completion.continuation !== "blocked")
        queuedAutoContinueSessionIdsRef.current.add(sessionId);
      else queuedAutoContinueSessionIdsRef.current.delete(sessionId);
      claimsChanged =
        queuedDispatchSessionIdsRef.current.delete(sessionId) || claimsChanged;
      if (
        releaseSessionDraftSubmission(submittingDraftSessionIdsRef, sessionId)
      ) {
        submittingDraftStartedAtRef.current.delete(sessionId);
        draftLocksChanged = true;
      }
      completionChanged = true;
    }
    if (claimsChanged)
      setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
    if (draftLocksChanged)
      setSubmittingDraftSessionIds(
        new Set(submittingDraftSessionIdsRef.current),
      );
    if (completionChanged)
      setQueuedAutoContinueVersion((version) => version + 1);
  }, [completedTurnsBySession]);

  useEffect(() => {
    if (sessionInteractionLocked) return;
    for (const session of sessions) {
      if (session.state === "working") {
        queuedDispatchSessionIdsRef.current.delete(session.session_id);
        queuedAutoContinueSessionIdsRef.current.delete(session.session_id);
      }
    }
    const dispatches = selectQueuedDispatches(
      sessions,
      queuedMessagesBySessionRef.current,
      queuedDispatchSessionIdsRef.current,
      editingQueuedMessage?.sessionId,
      new Set(Object.keys(queuedMessagesPauseBySessionRef.current)),
      queuedAutoContinueSessionIdsRef.current,
    );
    for (const { sessionId, message: next } of dispatches) {
      if (
        !claimQueuedMessage(
          queuedMessageClaimsRef.current,
          sessionId,
          queuedMessagesBySessionRef.current[sessionId] ?? [],
          next.id,
        )
      )
        continue;
      // One authoritative normal or user-cancelled completion permits exactly one
      // continuation. Runtime/system failures remain blocked. Consume it before delivery so failure cannot cascade.
      queuedAutoContinueSessionIdsRef.current.delete(sessionId);
      queuedDispatchSessionIdsRef.current.add(sessionId);
      if (
        !onSendForSession(
          sessionId,
          next.text,
          next.id,
          next.attachmentIds,
          false,
          next.roleIds ?? (next.roleId ? [next.roleId] : []),
        )
      ) {
        queuedDispatchSessionIdsRef.current.delete(sessionId);
        releaseQueuedMessageClaim(
          queuedMessageClaimsRef.current,
          sessionId,
          next.id,
        );
        updateQueuedMessages((current) => ({
          ...current,
          [sessionId]: (current[sessionId] ?? []).map((message) =>
            message.id === next.id
              ? {
                  ...message,
                  deliveryError: "消息尚未安全保存，请检查浏览器存储后重试",
                }
              : message,
          ),
        }));
      }
    }
    setQueuedMessageClaims(new Set(queuedMessageClaimsRef.current));
  }, [
    editingQueuedMessage?.sessionId,
    onSendForSession,
    queuedAutoContinueVersion,
    queuedMessagesBySession,
    queuedMessagesPauseBySession,
    sessionInteractionLocked,
    sessions,
    updateQueuedMessages,
  ]);

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
      changed =
        releaseSessionDraftSubmission(
          submittingDraftSessionIdsRef,
          session.session_id,
        ) || changed;
    }
    if (changed)
      setSubmittingDraftSessionIds(
        new Set(submittingDraftSessionIdsRef.current),
      );
  }, [onRolesConsumed, sessions]);

  const latestActiveTurn = activeSession?.turns.at(-1);
  useEffect(() => {
    if (
      !activeSessionId ||
      !latestActiveTurn ||
      latestActiveTurn.state === "working"
    )
      return;
    const startedAt = submittingDraftStartedAtRef.current.get(activeSessionId);
    if (startedAt === undefined || latestActiveTurn.created_at_ms < startedAt)
      return;
    if (
      releaseSessionDraftSubmission(
        submittingDraftSessionIdsRef,
        activeSessionId,
      )
    ) {
      submittingDraftStartedAtRef.current.delete(activeSessionId);
      setSubmittingDraftSessionIds(
        new Set(submittingDraftSessionIdsRef.current),
      );
    }
  }, [
    activeSessionId,
    latestActiveTurn?.created_at_ms,
    latestActiveTurn?.state,
  ]);

  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    const previous = previousScrollMetrics.current;
    if (!viewport || !previous) return;
    viewport.scrollTop = preservePrependScrollTop(
      previous,
      viewport.scrollHeight,
    );
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
    if (
      !viewport ||
      !followThreadLatest.current ||
      previousScrollMetrics.current
    )
      return;
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
    const reserved = reserveSessionDraftSubmission(
      submittingDraftSessionIdsRef,
      activeSessionId,
      draftsBySession,
    );
    if (reserved === null) return;
    setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
    const attachmentIds = availableAttachments.map(
      (attachment) => attachment.id,
    );
    const existingQueue =
      queuedMessagesBySessionRef.current[reserved.sessionId] ?? [];
    const direct =
      activeSession?.session_id === reserved.sessionId &&
      shouldDirectManualMessage(
        activeSession.state,
        existingQueue.length,
        !!queuedMessagesPause,
        isCancelling || !!activeSession.cancelling_turn_id,
      );
    let sent: boolean;
    let directCommandId: string | undefined;
    if (direct) {
      queuedAutoContinueSessionIdsRef.current.delete(reserved.sessionId);
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
        [reserved.sessionId]: [
          ...existingQueue,
          {
            id: clientId("queued"),
            text: reserved.text,
            createdAtMs: Date.now(),
            attachmentIds,
            roleIds: [...selectedRoleIds],
          },
        ],
      };
      sent =
        !!reliableStorageScope &&
        saveQueuedMessages(
          window.localStorage,
          reliableStorageScope,
          nextQueues,
          queuedMessagesBySessionRef.current,
        );
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
    const nextDrafts = finishSessionDraftSubmission(
      submittingDraftSessionIdsRef,
      draftsBySession,
      reserved.sessionId,
      reserved.text,
      sent,
    );
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
    if (!submittingDraftSessionIdsRef.current.has(reserved.sessionId))
      submittingDraftStartedAtRef.current.delete(reserved.sessionId);
    setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current));
  };

  const submitDraftAsSupplement = () => {
    if (uploadingAttachment || sessionInteractionLocked) return;
    const reserved = reserveSessionDraftSubmission(
      submittingDraftSessionIdsRef,
      activeSessionId,
      draftsBySession,
    );
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
    if (!submittingDraftSessionIdsRef.current.has(reserved.sessionId))
      submittingDraftStartedAtRef.current.delete(reserved.sessionId);
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

  const previewQueuedMessageDrag = ({ over }: DragOverEvent) => {
    setQueuedMessageOverId(over ? String(over.id) : undefined);
  };

  const finishQueuedMessageDrag = ({ active, over }: DragEndEvent) => {
    setDraggedQueueMessageId(undefined);
    setQueuedMessageOverId(undefined);
    if (!activeSessionId || !over || active.id === over.id) return;
    updateQueuedMessages((current) => ({
      ...current,
      [activeSessionId]: reorderQueuedMessages(
        current[activeSessionId] ?? [],
        String(active.id),
        String(over.id),
        queuedMessageClaimsRef.current,
        activeSessionId,
      ),
    }));
  };

  const saveQueuedMessageEdit = () => {
    const edit = editingQueuedMessage;
    const text = edit?.text.trim();
    if (!edit || !text) return;
    updateQueuedMessages((current) => ({
      ...current,
      [edit.sessionId]: queuedMessageClaimsRef.current.has(
        queuedMessageKey(edit.sessionId, edit.id),
      )
        ? (current[edit.sessionId] ?? [])
        : (current[edit.sessionId] ?? []).map((message) =>
            message.id === edit.id
              ? { ...message, text, deliveryError: undefined }
              : message,
          ),
    }));
    setEditingQueuedMessage(undefined);
  };

  const cancelActiveSessionTurn = async () => {
    if (activeSessionId)
      queuedAutoContinueSessionIdsRef.current.delete(activeSessionId);
    setEditingQueuedMessage(undefined);
    await onCancel(
      interactionPhase.kind === "idle" ? undefined : interactionPhase.commandId,
    );
  };

  return (
    <ThreadPrimitive.Root className="aui-thread">
      <ThreadPrimitive.Viewport
        ref={viewportRef}
        className="chat-scroll aui-thread-viewport"
        autoScroll={false}
        scrollToBottomOnInitialize={false}
        scrollToBottomOnRunStart={false}
        scrollToBottomOnThreadSwitch={false}
        onScroll={(event) => {
          const nearBottom = isNearScrollBottom({
            scrollTop: event.currentTarget.scrollTop,
            scrollHeight: event.currentTarget.scrollHeight,
            clientHeight: event.currentTarget.clientHeight,
          });
          followThreadLatest.current = nearBottom;
          setThreadAwayFromBottom(!nearBottom);
          if (activeSessionId)
            sessionScrollPositionsRef.current.set(activeSessionId, {
              scrollTop: event.currentTarget.scrollTop,
              followLatest: followThreadLatest.current,
            });
          // Ordinary vertical scrolling only updates state from cached anchor offsets.
          // Geometry and floating-navigation layout are refreshed by structural,
          // sizing, outline-layout, and Session-activation invalidations instead.
          requestTimelineNavigationWork("scroll", {
            navigation: userMessageNavigationTaskRef.current,
            geometry: userMessageGeometryTaskRef.current,
            layout: userMessageNavigationLayoutTaskRef.current,
          });
        }}
      >
        {(activeSession?.turns.length ?? 0) === 0 && (
          <div className="welcome">
            <Sparkles size={24} />
            <h2>{welcomeTitle}</h2>
            <p>{welcomeText}</p>
          </div>
        )}
        {canLoadStoredHistory && (
          <button
            type="button"
            className={`load-history ${loadingHistory ? "loading" : ""}`}
            title={historyButtonLabel}
            aria-label={historyButtonLabel}
            aria-live="polite"
            aria-busy={loadingHistory || undefined}
            disabled={loadingHistory || sessionInteractionLocked}
            onClick={loadEarlierTurns}
          >
            {loadingHistory && <LoaderCircle size={13} aria-hidden="true" />}
            <span>{historyButtonLabel}</span>
          </button>
        )}
        {mountedTimelineSessions.map((session) => (
          <SessionTimelinePane
            key={session.session_id}
            session={session}
            active={session.session_id === activeSessionId}
            isCancelling={
              session.session_id === activeSessionId && isCancelling
            }
            decisionsByTurn={decisionsByTurn}
            sessionInteractionLocked={sessionInteractionLocked}
            pendingDecisionKeys={pendingDecisionKeys}
            pendingToolGenTurnIds={pendingToolGenTurnIds}
            toolGenSessionBusy={toolGenSessionBusy}
            favoriteBySource={favoriteBySource}
            pendingFavoriteSourceKeys={pendingFavoriteSourceKeys}
            onToggleFavorite={onToggleFavorite}
            onDecisionReply={onDecisionReply}
            onRequestToolGen={onRequestToolGen}
            onRequestMessageDelete={onRequestMessageDelete}
          />
        ))}
        <ThreadPrimitive.ViewportFooter className="composer-wrap aui-thread-footer">
          {!!activeSession && displayQueuedMessages.length > 0 && (
            <section
              className={`queued-message-list ${queueExpanded ? "expanded" : "collapsed"} ${queuePanelCollapsed ? "summary-only" : ""} ${queuedMessagesPause ? "paused" : ""}`}
              aria-label={`${displayQueuedMessages.length} queued message${displayQueuedMessages.length === 1 ? "" : "s"}`}
              aria-live="polite"
            >
              <header>
                <span>待发送</span>
                {queuePanelCollapsed ? (
                  <div
                    className={`queued-message-summary ${firstQueuedMessage?.deliveryError ? "delivery-error" : ""}`}
                    title={
                      firstQueuedMessage?.deliveryError ||
                      firstQueuedMessage?.text
                    }
                  >
                    <p>{firstQueuedMessage?.text}</p>
                    {firstQueuedMessage &&
                      firstQueuedMessage.attachmentIds.length > 0 && (
                        <small className="queued-message-summary-attachments">
                          <Paperclip size={10} />
                          {firstQueuedMessage.attachmentIds.length}
                        </small>
                      )}
                    <small className="queued-message-summary-count">
                      {displayQueuedMessages.length} 条
                    </small>
                  </div>
                ) : (
                  <small title={queuedMessagesPause?.reason}>
                    {queuedMessagesPause
                      ? `自动发送已停止${queuedMessagesPause.reason ? `：${queuedMessagesPause.reason}` : ""}`
                      : "上一条正常完成后自动发送"}
                  </small>
                )}
                <div className="queued-message-header-actions">
                  <label className="queued-auto-send-control">
                    <span>自动发送</span>
                    <button
                      type="button"
                      role="switch"
                      className="queued-auto-send-switch"
                      aria-checked={!queuedMessagesPause}
                      aria-label={
                        queuedMessagesPause ? "开启自动发送" : "停止自动发送"
                      }
                      title={
                        queuedMessagesPause ? "开启自动发送" : "停止自动发送"
                      }
                      onClick={() => {
                        if (!activeSessionId) return;
                        if (queuedMessagesPause)
                          resumeQueuedMessages(activeSessionId);
                        else
                          pauseQueuedMessages(
                            activeSessionId,
                            "用户关闭了自动发送",
                            "user",
                          );
                      }}
                    >
                      <span className="queued-auto-send-thumb" />
                    </button>
                  </label>
                  {!queuePanelCollapsed && hiddenQueuedMessageCount > 0 && (
                    <button
                      type="button"
                      className="queued-message-toggle"
                      aria-expanded={queueExpanded}
                      title={
                        queueExpanded
                          ? "收起待发送消息"
                          : `向上展开全部 ${displayQueuedMessages.length} 条待发送消息`
                      }
                      onClick={toggleQueuedMessages}
                    >
                      {queueExpanded ? (
                        <ChevronDown size={13} />
                      ) : (
                        <ChevronUp size={13} />
                      )}
                      <span>
                        {queueExpanded
                          ? "收起"
                          : `展开 ${hiddenQueuedMessageCount} 条`}
                      </span>
                    </button>
                  )}
                  <button
                    type="button"
                    className="queued-message-panel-toggle"
                    aria-expanded={!queuePanelCollapsed}
                    aria-controls={`queued-message-items-${activeSession.session_id}`}
                    title={
                      queuePanelCollapsed
                        ? "展开待发送队列"
                        : "折叠待发送队列为一行"
                    }
                    onClick={toggleQueuedMessagePanel}
                  >
                    {queuePanelCollapsed ? (
                      <ChevronDown size={14} />
                    ) : (
                      <ChevronUp size={14} />
                    )}
                    <span>{queuePanelCollapsed ? "展开" : "折叠"}</span>
                  </button>
                </div>
              </header>
              {!queuePanelCollapsed && (
                <DndContext
                  sensors={queueDragSensors}
                  collisionDetection={closestCenter}
                  onDragStart={({ active }) => {
                    setDraggedQueueMessageId(String(active.id));
                    setQueuedMessageOverId(String(active.id));
                  }}
                  onDragOver={previewQueuedMessageDrag}
                  onDragCancel={() => {
                    setDraggedQueueMessageId(undefined);
                    setQueuedMessageOverId(undefined);
                  }}
                  onDragEnd={finishQueuedMessageDrag}
                >
                  <SortableContext
                    items={visibleQueuedMessages.map(({ id }) => id)}
                    strategy={verticalListSortingStrategy}
                  >
                    <div
                      id={`queued-message-items-${activeSession.session_id}`}
                      className="queued-message-items"
                    >
                      {visibleQueuedMessages.map((message) => {
                        const index = queuedMessages.findIndex(
                          (candidate) => candidate.id === message.id,
                        );
                        const editing =
                          editingQueuedMessage?.sessionId ===
                            activeSession.session_id &&
                          editingQueuedMessage.id === message.id;
                        const claimed = queuedMessageClaims.has(
                          queuedMessageKey(
                            activeSession.session_id,
                            message.id,
                          ),
                        );
                        const messageRoleIds =
                          message.roleIds ??
                          (message.roleId ? [message.roleId] : []);
                        const messageRoleNames = messageRoleIds.map(
                          (roleId) =>
                            activeSession.roles.find(
                              (role) => role.id === roleId,
                            )?.name ?? roleId,
                        );
                        const dragDisabled =
                          editing ||
                          claimed ||
                          displayQueuedMessages.length <= 1;
                        const sendAsNewTurn = activeSession.state !== "working";
                        return (
                          <SortableQueuedMessage
                            id={message.id}
                            disabled={dragDisabled}
                            key={message.id}
                          >
                            {({
                              setNodeRef,
                              style,
                              attributes,
                              listeners,
                              isDragging,
                            }) => (
                              <article
                                ref={setNodeRef}
                                style={style}
                                className={`queued-message ${editing ? "editing" : ""} ${message.deliveryError ? "delivery-error" : ""} ${isDragging ? "dragging" : ""} ${claimed ? "sending" : ""}`}
                                aria-busy={claimed || undefined}
                              >
                                <button
                                  type="button"
                                  className="queued-message-drag"
                                  disabled={dragDisabled}
                                  title={`拖动调整第 ${index + 1} 条消息的顺序`}
                                  aria-label={`拖动调整第 ${index + 1} 条消息的顺序`}
                                  {...attributes}
                                  {...listeners}
                                >
                                  <GripVertical size={13} />
                                </button>
                                <span
                                  className="queued-message-order"
                                  aria-label={`Queue position ${index + 1}`}
                                >
                                  {index + 1}
                                </span>
                                <div className="queued-message-preview">
                                  {messageRoleNames.length > 0 && (
                                    <small
                                      className="queued-message-roles"
                                      title={messageRoleNames.join(" | ")}
                                    >
                                      <BriefcaseBusiness size={11} />
                                      <span className="queued-message-role-names">
                                        {messageRoleNames.map(
                                          (roleName, roleIndex) => (
                                            <span
                                              className="queued-message-role"
                                              key={`${messageRoleIds[roleIndex]}-${roleIndex}`}
                                            >
                                              {roleIndex > 0 && (
                                                <i
                                                  className="queued-message-role-separator"
                                                  aria-hidden="true"
                                                >
                                                  |
                                                </i>
                                              )}
                                              <span>{roleName}</span>
                                            </span>
                                          ),
                                        )}
                                      </span>
                                    </small>
                                  )}
                                  {editing ? (
                                    <textarea
                                      className="queued-message-editor"
                                      autoFocus
                                      value={editingQueuedMessage.text}
                                      aria-label={`编辑第 ${index + 1} 条待发送消息`}
                                      onChange={(event) =>
                                        setEditingQueuedMessage({
                                          ...editingQueuedMessage,
                                          text: event.target.value,
                                        })
                                      }
                                      onKeyDown={(event) => {
                                        if (
                                          (event.metaKey || event.ctrlKey) &&
                                          event.key === "Enter"
                                        ) {
                                          event.preventDefault();
                                          saveQueuedMessageEdit();
                                        }
                                        if (event.key === "Escape") {
                                          event.preventDefault();
                                          setEditingQueuedMessage(undefined);
                                        }
                                      }}
                                    />
                                  ) : (
                                    <p
                                      title={
                                        message.deliveryError || message.text
                                      }
                                    >
                                      {message.text}
                                    </p>
                                  )}
                                  {message.attachmentIds.length > 0 && (
                                    <small className="queued-message-attachments">
                                      <Paperclip size={11} />
                                      {message.attachmentIds.length} 个附件
                                    </small>
                                  )}
                                  {message.deliveryError && (
                                    <small className="queued-message-error">
                                      {message.deliveryError}
                                    </small>
                                  )}
                                </div>
                                <div className="queued-message-actions">
                                  {editing ? (
                                    <>
                                      <button
                                        type="button"
                                        className="queued-message-edit-save"
                                        disabled={
                                          !editingQueuedMessage.text.trim() ||
                                          claimed
                                        }
                                        onClick={saveQueuedMessageEdit}
                                      >
                                        保存
                                      </button>
                                      <button
                                        type="button"
                                        className="queued-message-edit-cancel"
                                        disabled={claimed}
                                        onClick={() =>
                                          setEditingQueuedMessage(undefined)
                                        }
                                      >
                                        取消
                                      </button>
                                    </>
                                  ) : (
                                    <>
                                      <button
                                        type="button"
                                        className="queued-message-edit"
                                        title="重新编辑这条待发送消息"
                                        aria-label={`重新编辑第 ${index + 1} 条待发送消息`}
                                        disabled={claimed}
                                        onClick={() => {
                                          setEditingQueuedMessage({
                                            sessionId: activeSession.session_id,
                                            id: message.id,
                                            text: message.text,
                                          });
                                          setExpandedQueueSessionIds(
                                            (current) =>
                                              new Set(current).add(
                                                activeSession.session_id,
                                              ),
                                          );
                                        }}
                                      >
                                        <Pencil size={12} />
                                      </button>
                                      <button
                                        type="button"
                                        className="queued-message-supplement"
                                        title={
                                          message.deliveryError
                                            ? "重试发送这条消息"
                                            : sendAsNewTurn
                                              ? "作为新消息开始任务"
                                              : "立即发送为当前任务的补充"
                                        }
                                        disabled={
                                          claimed ||
                                          sessionInteractionLocked ||
                                          isCancelling
                                        }
                                        onClick={() => {
                                          if (
                                            !claimQueuedMessage(
                                              queuedMessageClaimsRef.current,
                                              activeSession.session_id,
                                              queuedMessagesBySession[
                                                activeSession.session_id
                                              ] ?? [],
                                              message.id,
                                            )
                                          )
                                            return;
                                          queuedAutoContinueSessionIdsRef.current.delete(
                                            activeSession.session_id,
                                          );
                                          setQueuedMessageClaims(
                                            new Set(
                                              queuedMessageClaimsRef.current,
                                            ),
                                          );
                                          if (
                                            !onSendForSession(
                                              activeSession.session_id,
                                              message.text,
                                              message.id,
                                              message.attachmentIds,
                                              !sendAsNewTurn,
                                              messageRoleIds,
                                              sendAsNewTurn,
                                            )
                                          ) {
                                            releaseQueuedMessageClaim(
                                              queuedMessageClaimsRef.current,
                                              activeSession.session_id,
                                              message.id,
                                            );
                                            setQueuedMessageClaims(
                                              new Set(
                                                queuedMessageClaimsRef.current,
                                              ),
                                            );
                                            updateQueuedMessages((current) => ({
                                              ...current,
                                              [activeSession.session_id]: (
                                                current[
                                                  activeSession.session_id
                                                ] ?? []
                                              ).map((candidate) =>
                                                candidate.id === message.id
                                                  ? {
                                                      ...candidate,
                                                      deliveryError:
                                                        "消息尚未安全保存，请检查浏览器存储后重试",
                                                    }
                                                  : candidate,
                                              ),
                                            }));
                                            return;
                                          }
                                          if (message.deliveryError)
                                            updateQueuedMessages((current) => ({
                                              ...current,
                                              [activeSession.session_id]: (
                                                current[
                                                  activeSession.session_id
                                                ] ?? []
                                              ).map((candidate) =>
                                                candidate.id === message.id
                                                  ? {
                                                      ...candidate,
                                                      deliveryError: undefined,
                                                    }
                                                  : candidate,
                                              ),
                                            }));
                                        }}
                                      >
                                        {claimed
                                          ? "发送中…"
                                          : message.deliveryError
                                            ? "重试"
                                            : "立即"}
                                      </button>
                                      <button
                                        type="button"
                                        className="queued-message-remove"
                                        title="Remove queued message"
                                        aria-label={`Remove queued message ${index + 1}`}
                                        disabled={claimed}
                                        onClick={() =>
                                          updateQueuedMessages((current) => ({
                                            ...current,
                                            [activeSession.session_id]:
                                              removeQueuedMessage(
                                                current[
                                                  activeSession.session_id
                                                ] ?? [],
                                                message.id,
                                                queuedMessageClaimsRef.current,
                                                activeSession.session_id,
                                              ),
                                          }))
                                        }
                                      >
                                        <X size={13} />
                                      </button>
                                    </>
                                  )}
                                </div>
                              </article>
                            )}
                          </SortableQueuedMessage>
                        );
                      })}
                    </div>
                  </SortableContext>
                  <DragOverlay
                    dropAnimation={
                      prefersReducedMotion()
                        ? null
                        : {
                            duration: 180,
                            easing: "cubic-bezier(.2, .8, .2, 1)",
                          }
                    }
                  >
                    {draggedQueuedMessage && (
                      <article
                        className="queued-message queued-message-overlay"
                        aria-hidden="true"
                      >
                        <span className="queued-message-drag">
                          <GripVertical size={13} />
                        </span>
                        <span className="queued-message-order">
                          {draggedQueuedMessagePosition}
                        </span>
                        <div className="queued-message-preview">
                          <p>{draggedQueuedMessage.text}</p>
                          {draggedQueuedMessage.attachmentIds.length > 0 && (
                            <small className="queued-message-attachments">
                              <Paperclip size={11} />
                              {draggedQueuedMessage.attachmentIds.length} 个附件
                            </small>
                          )}
                        </div>
                      </article>
                    )}
                  </DragOverlay>
                </DndContext>
              )}
            </section>
          )}
          {!!activeSession &&
            (!!availableAttachments.length || uploadingAttachment) && (
              <div
                className="attachment-strip"
                aria-label={attachmentStripLabel}
                aria-live="polite"
                aria-busy={uploadingAttachment || undefined}
              >
                {attachedFileCount > 0 && (
                  <div className="attachment-summary" title={attachmentSummary}>
                    <Paperclip size={13} />
                    <span>{attachmentSummary}</span>
                  </div>
                )}
                {uploadingAttachment && (
                  <div
                    className="pending-attachment uploading"
                    role="status"
                    aria-label={
                      uploadingAttachmentFile
                        ? `${uploadingAttachmentText}, ${formatBytes(uploadingAttachmentFile.bytes)}`
                        : uploadingAttachmentText
                    }
                    title={
                      uploadingAttachmentFile?.name ?? uploadingAttachmentText
                    }
                  >
                    <span className="upload-dot" aria-hidden="true" />
                    <span className="pending-attachment-name">
                      {uploadingAttachmentFile?.name ?? "Uploading file…"}
                    </span>
                    {uploadingAttachmentFile && (
                      <small>
                        {formatBytes(uploadingAttachmentFile.bytes)}
                      </small>
                    )}
                  </div>
                )}
                {availableAttachments.map((attachment) => {
                  const removing = pendingAttachmentRemoveIds.has(
                    `${activeSession.session_id}:${attachment.id}`,
                  );
                  const removeLabel = removing
                    ? `Removing ${attachment.name}`
                    : sessionInteractionLocked
                      ? `${sessionInteractionLockReason} · cannot remove ${attachment.name}`
                      : `Remove ${attachment.name}`;
                  return (
                    <div
                      className="pending-attachment"
                      key={attachment.id}
                      title={attachment.name}
                    >
                      <Paperclip size={13} />
                      <span className="pending-attachment-name">
                        {attachment.name}
                      </span>
                      <small>{formatBytes(attachment.bytes)}</small>
                      <button
                        type="button"
                        title={removeLabel}
                        aria-label={removeLabel}
                        aria-busy={removing || undefined}
                        disabled={removing || sessionInteractionLocked}
                        onClick={() => onRemoveAttachment(attachment.id)}
                      >
                        {removing ? "…" : <X size={13} />}
                      </button>
                    </div>
                  );
                })}
              </div>
            )}
          <form
            className="composer"
            onSubmit={(event) => {
              event.preventDefault();
              submitDraft();
            }}
          >
            <div className="expandable-text-field composer-text-field">
              <textarea
                ref={composerTextareaRef}
                value={draft}
                placeholder={
                  !activeSession
                    ? "Create a session to start…"
                    : sessionInteractionLocked
                      ? sessionInteractionLockReason
                      : activeSession.state === "working"
                        ? "继续输入…"
                        : "Ask Timem to investigate, write, or work with you."
                }
                aria-label="Message Timem"
                aria-describedby={composerHintId}
                title={composerHint}
                disabled={!activeSession || sessionInteractionLocked}
                onChange={(event) =>
                  setDraftsBySession((current) =>
                    setSessionDraft(
                      current,
                      activeSessionId,
                      event.target.value,
                    ),
                  )
                }
                onKeyDown={(event) => {
                  if (event.key !== "Enter" || event.nativeEvent.isComposing)
                    return;
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
              <button
                type="button"
                className="text-field-expand"
                title="展开编辑用户信息"
                aria-label="展开编辑用户信息"
                disabled={!activeSession || sessionInteractionLocked}
                onClick={() => setComposerExpanded(true)}
              >
                <Maximize2 size={14} />
              </button>
            </div>
            {composerExpanded && activeSession && (
              <ExpandedTextEditor
                eyebrow="MESSAGE"
                title="编辑用户信息"
                value={draft}
                disabled={sessionInteractionLocked}
                placeholder={
                  activeSession.state === "working"
                    ? "继续输入…"
                    : "Ask Timem to investigate, write, or work with you."
                }
                onCommit={(value) =>
                  setDraftsBySession((current) =>
                    setSessionDraft(current, activeSessionId, value),
                  )
                }
                onClose={() => setComposerExpanded(false)}
              />
            )}
            {selectedRoles.length > 0 && activeSession && (
              <div
                className="composer-role"
                title={selectedRoles
                  .map((role) => `${role.name}: ${role.description}`)
                  .join("\n")}
              >
                <BriefcaseBusiness size={14} />
                <span>
                  本条将使用{" "}
                  <strong>
                    {selectedRoles.map((role) => role.name).join("、")}
                  </strong>
                </span>
                <button
                  type="button"
                  title="Clear roles for this message"
                  aria-label="Clear selected roles"
                  onClick={() => onRolesConsumed(activeSession.session_id)}
                >
                  <X size={13} />
                </button>
              </div>
            )}
            <div className="composer-actions">
              <div className="composer-paths">
                {activeSession && (
                  <span
                    className="composer-cwd-inline"
                    title={activeSession.current_dir}
                  >
                    <b>CWD:</b>
                    <span className="path-tail">
                      {tailPath(activeSession.current_dir, 64)}
                    </span>
                  </span>
                )}
                {activeSession?.debug_dir && (
                  <span
                    className="composer-cwd-inline composer-debug-inline"
                    title={activeSession.debug_dir}
                  >
                    <b>DEBUG:</b>
                    <span>{activeSession.debug_dir}</span>
                  </span>
                )}
              </div>
              <span
                id={composerHintId}
                className="sr-only"
                role="status"
                aria-live="polite"
              >
                {composerHint}
              </span>
              <div className="composer-buttons">
                <button
                  className={`attach-button ${uploadingAttachment ? "uploading" : ""}`}
                  type="button"
                  title={attachTitle}
                  aria-label={attachLabel}
                  disabled={
                    !activeSession ||
                    uploadingAttachment ||
                    sessionInteractionLocked
                  }
                  onClick={() => fileInput.current?.click()}
                >
                  {uploadingAttachment ? (
                    <LoaderCircle size={17} />
                  ) : (
                    <Paperclip size={17} />
                  )}
                </button>
                <input
                  ref={fileInput}
                  className="file-input"
                  type="file"
                  disabled={
                    !activeSession ||
                    uploadingAttachment ||
                    sessionInteractionLocked
                  }
                  onChange={(event) => {
                    const file = event.target.files?.[0];
                    event.currentTarget.value = "";
                    if (file) void onUpload(file);
                  }}
                />
                {showStopAction ? (
                  <button
                    className={`stop-button ${isCancelling ? "sending" : ""}`}
                    type="button"
                    title={
                      isCancelling
                        ? "Cancellation requested"
                        : lockedControlHint || "Cancel current turn"
                    }
                    aria-label={
                      isCancelling
                        ? "Cancellation requested"
                        : lockedControlHint || "Cancel current turn"
                    }
                    disabled={isCancelling || sessionInteractionLocked}
                    onClick={() => void cancelActiveSessionTurn()}
                  >
                    <CircleStop size={17} /> Stop
                  </button>
                ) : (
                  <button
                    className={`send-button ${submittingDraft ? "sending" : ""}`}
                    type="submit"
                    title={effectiveSendLabel}
                    aria-label={effectiveSendLabel}
                    disabled={
                      !activeSession ||
                      !hasDraftText ||
                      submittingDraft ||
                      uploadingAttachment ||
                      sessionInteractionLocked
                    }
                  >
                    {submittingDraft ? (
                      <LoaderCircle size={17} />
                    ) : (
                      <Send size={17} />
                    )}
                  </button>
                )}
              </div>
            </div>
          </form>
        </ThreadPrimitive.ViewportFooter>
      </ThreadPrimitive.Viewport>

      <nav
        ref={userMessageNavigationRef}
        className={`user-message-navigation outline-overlap-${userMessageNavigationLayout.overlap}${userMessageNavigationHoverLocked ? " hover-locked" : ""}`}
        style={
          userMessageNavigationLayout.left === undefined
            ? undefined
            : { left: `${userMessageNavigationLayout.left}px` }
        }
        aria-label="用户消息导航"
        onPointerEnter={lockUserMessageNavigationLayout}
        onPointerLeave={unlockUserMessageNavigationLayout}
      >
        <button
          type="button"
          title="上一条用户消息"
          aria-label="上一条用户消息"
          disabled={!userMessageNavigation.previous}
          onClick={() => navigateUserMessage("previous")}
        >
          <ChevronUp size={14} strokeWidth={2.2} aria-hidden="true" />
        </button>
        <button
          type="button"
          title={
            userMessageNavigation.next ? "下一条用户消息" : "导航至聊天最下方"
          }
          aria-label={
            userMessageNavigation.next ? "下一条用户消息" : "导航至聊天最下方"
          }
          disabled={
            !userMessageNavigation.next && !userMessageNavigation.bottom
          }
          onClick={() => {
            if (userMessageNavigation.next) navigateUserMessage("next");
            else navigateToThreadBottom();
          }}
        >
          <ChevronDown size={14} strokeWidth={2.2} aria-hidden="true" />
        </button>
        {activeSession && (
          <button
            type="button"
            className={`thread-working-away ${activeSession.state === "working" ? "is-working" : "is-idle"}${threadAwayFromBottom ? " away-from-bottom" : " at-live-edge"}`}
            title={
              activeSession.state === "working"
                ? threadAwayFromBottom
                  ? "工作仍在继续，跳转到最新内容"
                  : "工作仍在继续，当前已是最新内容"
                : threadAwayFromBottom
                  ? "跳转到聊天最下方"
                  : "当前已是聊天最下方"
            }
            aria-label={
              activeSession.state === "working"
                ? threadAwayFromBottom
                  ? "工作仍在继续，跳转到最新内容"
                  : "工作仍在继续，当前已是最新内容"
                : threadAwayFromBottom
                  ? "跳转到聊天最下方"
                  : "当前已是聊天最下方"
            }
            onClick={navigateWorkingToThreadBottom}
          >
            <span className="thread-edge-symbol" aria-hidden="true">
              {activeSession.state === "working" ? (
                <span className="thread-working-orbit">
                  <span className="thread-working-core" />
                </span>
              ) : (
                <ArrowDownToLine
                  className="thread-idle-bottom-icon"
                  size={17}
                  strokeWidth={2.35}
                />
              )}
            </span>
            <span className="sr-only" role="status" aria-live="polite">
              {activeSession.state === "working" ? "Working" : "Jump to bottom"}
            </span>
          </button>
        )}
      </nav>
    </ThreadPrimitive.Root>
  );
}

type TurnInteractionProps = {
  sessionId: string;
  turn: WebTurn;
  isCancelling: boolean;
  decisions: Decision[];
  sessionInteractionLocked: boolean;
  pendingDecisionKeys: Set<string>;
  toolGenPending: boolean;
  toolGenBlocked: boolean;
  favorite?: ChatFavorite;
  favoritePending: boolean;
  onToggleFavorite: (
    sessionId: string,
    turnId: string,
    favoriteId?: string,
  ) => boolean;
  onDecisionReply: (
    decision: Decision,
    reply: "accept" | "decline" | "always_allow",
  ) => void;
  onRequestToolGen?: (turnId: string) => void;
  onRequestMessageDelete: (candidate: ChatMessageDeleteCandidate) => void;
};

const WorkingElapsed = memo(function WorkingElapsed({
  createdAtMs,
  endedAtMs,
}: {
  createdAtMs: number;
  endedAtMs?: number | null;
}) {
  const elapsedAt = useCallback(
    () => turnElapsedMs(createdAtMs, Date.now(), endedAtMs),
    [createdAtMs, endedAtMs],
  );
  const [elapsedMs, setElapsedMs] = useState(elapsedAt);
  useEffect(() => {
    setElapsedMs(elapsedAt());
    if (endedAtMs != null) return;
    const timer = window.setInterval(() => setElapsedMs(elapsedAt()), 1_000);
    return () => window.clearInterval(timer);
  }, [elapsedAt, endedAtMs]);
  return (
    <span className="working-elapsed" aria-hidden="true">
      {formatDuration(elapsedMs)}
    </span>
  );
});

const TurnInteraction = memo(function TurnInteraction({
  sessionId,
  turn,
  isCancelling,
  decisions,
  sessionInteractionLocked,
  pendingDecisionKeys,
  toolGenPending,
  toolGenBlocked,
  favorite,
  favoritePending,
  onToggleFavorite,
  onDecisionReply,
  onRequestToolGen,
  onRequestMessageDelete,
}: TurnInteractionProps) {
  const workScrollRef = useRef<HTMLDivElement | null>(null);
  const workContentRef = useRef<HTMLDivElement | null>(null);
  const followLatest = useRef(true);
  const previousUpdateCount = useRef(
    turn.events.length +
      turn.user_entries.filter((entry) => entry.kind === "supplement").length +
      decisions.length,
  );
  const previousTurnState = useRef(turn.state);
  const previousFinalAnswer = useRef(!!turn.final_answer);
  const [pendingUpdates, setPendingUpdates] = useState(0);
  const [workEdgeFades, setWorkEdgeFades] = useState({
    top: false,
    bottom: false,
  });
  const updateWorkEdgeFades = useCallback((scroll: HTMLDivElement) => {
    const next = scrollEdgeFades({
      scrollTop: scroll.scrollTop,
      scrollHeight: scroll.scrollHeight,
      clientHeight: scroll.clientHeight,
    });
    setWorkEdgeFades((current) =>
      current.top === next.top && current.bottom === next.bottom
        ? current
        : next,
    );
  }, []);
  const lifecycleEvents = useMemo(
    () => coalesceActionLifecycle(turn.events),
    [turn.events],
  );
  const lifecycleItems = useMemo(
    () =>
      lifecycleEvents.map((event) => ({
        type: "event" as const,
        key: event.event_id,
        createdAt: event.created_at_ms,
        event,
        activity: activityFromTurnEvent(event, sessionId),
      })),
    [lifecycleEvents, sessionId],
  );
  const supplementItems = useMemo(
    () =>
      turn.user_entries
        .map((entry, roleIndex) => ({ entry, roleIndex }))
        .filter(({ entry }) => entry.kind === "supplement")
        .map(({ entry, roleIndex }) => ({
          type: "supplement" as const,
          key: `user-supplement-${entry.created_at_ms}-${roleIndex}`,
          createdAt: entry.created_at_ms,
          activity: {
            id: `user-supplement-${turn.turn_id}-${entry.created_at_ms}-${roleIndex}`,
            sessionId,
            tone: "thinking" as const,
            kind: "user_supplement" as const,
            title: "[用户补充]",
            detail: entry.text,
            createdAt: entry.created_at_ms,
          },
        })),
    [sessionId, turn.turn_id, turn.user_entries],
  );
  const timelineItems = useMemo(
    () =>
      [...lifecycleItems, ...supplementItems].sort(
        (left, right) => left.createdAt - right.createdAt,
      ),
    [lifecycleItems, supplementItems],
  );
  const visibleItems = timelineItems;
  const processActivities = useMemo(
    () =>
      timelineItems
        .map(({ activity }) => activity)
        .filter((activity): activity is Activity => activity !== null),
    [timelineItems],
  );
  const modelRetryStatus = useMemo(() => activeModelRetryStatus(turn), [turn]);
  const persistentToolGenItems = useMemo(
    () =>
      visibleItems.filter(
        (item) =>
          item.type === "event" &&
          item.activity?.kind === "toolgen" &&
          item.activity.toolgen_phase === "published",
      ),
    [visibleItems],
  );
  const persistentToolGenItemKeys = useMemo(
    () => new Set(persistentToolGenItems.map(({ key }) => key)),
    [persistentToolGenItems],
  );
  const scrollItems = useMemo(
    () =>
      visibleItems.filter((item) => !persistentToolGenItemKeys.has(item.key)),
    [persistentToolGenItemKeys, visibleItems],
  );
  const toolActivityRuns = useMemo(
    () =>
      summarizeConsecutiveToolActivities(
        scrollItems.map(({ activity }) => activity),
      ),
    [scrollItems],
  );
  const toolActivityRunByStartIndex = useMemo(
    () => new Map(toolActivityRuns.map((run) => [run.startIndex, run.summary])),
    [toolActivityRuns],
  );
  const hasVisibleProcess =
    scrollItems.some((item) => item.activity !== null) || decisions.length > 0;
  const isWorking = turn.state === "working" && !isCancelling;
  const showWorkFrame = shouldRenderTurnWorkFrame(
    turn.state,
    isCancelling,
    hasVisibleProcess,
  );
  const cancelled =
    isCancelling ||
    turn.completion?.stop_reason?.toLowerCase() === "cancelledbyuser";
  const interrupted = cancelled || turn.state === "interrupted";
  const [showWorkStream, setShowWorkStream] = useState(() => isWorking);
  const isToolGenTurn =
    turn.turn_id.startsWith("web_toolgen_turn_") ||
    turn.user_entries.some((entry) => entry.kind === "toolgen_instruction") ||
    turn.events.some(
      (event) =>
        (event.payload.topic as { name?: string } | undefined)?.name ===
        "core.toolgen",
    );
  const canCollapseCompletedWork =
    !isWorking && (!!turn.final_answer || interrupted);
  const canToggleWorkStream = isWorking || canCollapseCompletedWork;
  const workStreamVisible = !canToggleWorkStream || showWorkStream;
  const canDeleteConversationContent =
    turn.state !== "working" && !sessionInteractionLocked;

  useEffect(() => {
    const wasWorking = previousTurnState.current === "working";
    const finalArrived = !previousFinalAnswer.current && !!turn.final_answer;
    previousTurnState.current = isWorking ? "working" : turn.state;
    previousFinalAnswer.current = !!turn.final_answer;
    if (!wasWorking && isWorking) setShowWorkStream(true);
    if (finalArrived || (wasWorking && turn.state === "interrupted"))
      setShowWorkStream(false);
  }, [isWorking, turn.final_answer, turn.state]);

  useLayoutEffect(() => {
    const scroll = workScrollRef.current;
    const updateCount =
      turn.events.length + supplementItems.length + decisions.length;
    const added = Math.max(0, updateCount - previousUpdateCount.current);
    previousUpdateCount.current = updateCount;
    if (!scroll) return;
    if (followLatest.current) {
      scroll.scrollTop = scroll.scrollHeight;
      setPendingUpdates(0);
    } else if (added > 0) {
      setPendingUpdates((count) => count + added);
    }
    updateWorkEdgeFades(scroll);
  }, [
    turn.events.length,
    supplementItems.length,
    decisions.length,
    updateWorkEdgeFades,
  ]);
  useLayoutEffect(() => {
    const scroll = workScrollRef.current;
    const content = workContentRef.current;
    if (!scroll || !content || typeof ResizeObserver === "undefined") return;
    let scrollFrame: number | undefined;
    const observer = new ResizeObserver(() => {
      if (!followLatest.current || scrollFrame !== undefined) return;
      scrollFrame = window.requestAnimationFrame(() => {
        scrollFrame = undefined;
        if (!followLatest.current) return;
        scroll.scrollTop = scroll.scrollHeight;
        setPendingUpdates((count) => (count === 0 ? count : 0));
      });
    });
    observer.observe(content);
    return () => {
      observer.disconnect();
      if (scrollFrame !== undefined) window.cancelAnimationFrame(scrollFrame);
    };
  }, [workStreamVisible]);

  useLayoutEffect(() => {
    const scroll = workScrollRef.current;
    const content = workContentRef.current;
    if (!scroll || !content) return;
    const update = () => updateWorkEdgeFades(scroll);
    update();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(update);
    observer.observe(scroll);
    observer.observe(content);
    return () => observer.disconnect();
  }, [updateWorkEdgeFades, workStreamVisible]);

  const scrollWorkToLatest = () => {
    const scroll = workScrollRef.current;
    if (!scroll) return;
    scroll.scrollTo({
      top: scroll.scrollHeight,
      behavior: prefersReducedMotion() ? "auto" : "smooth",
    });
    followLatest.current = true;
    setPendingUpdates(0);
  };

  return (
    <article
      className={`turn-interaction ${isWorking ? "active" : "completed"}`}
      data-turn-id={turn.turn_id}
    >
      {!!turn.user_entries.filter((e) => e.kind !== "approval").length && (
        <section className="turn-user-frame" data-user-message-anchor>
          <div className="turn-user-content">
            {turn.user_entries
              .map((entry, roleIndex) => ({ entry, roleIndex }))
              .filter(({ entry }) => entry.kind !== "approval")
              .map(({ entry, roleIndex }) => (
                <div
                  className={`turn-user-entry ${entry.kind}`}
                  key={`${entry.created_at_ms}-${roleIndex}`}
                  onCopy={(event) => {
                    const selection = window.getSelection();
                    if (
                      !selection ||
                      selection.rangeCount === 0 ||
                      selection.isCollapsed ||
                      !selection.anchorNode ||
                      !selection.focusNode
                    )
                      return;
                    if (
                      !event.currentTarget.contains(selection.anchorNode) ||
                      !event.currentTarget.contains(selection.focusNode)
                    )
                      return;
                    const copiedText = normalizeCopiedUserMessageText(
                      selection.toString(),
                    );
                    event.clipboardData.setData("text/plain", copiedText);
                    event.preventDefault();
                  }}
                >
                  <button
                    type="button"
                    className="chat-message-delete user-message-delete"
                    title="Delete this message from the conversation and raw chat log"
                    aria-label="Delete user message"
                    disabled={!canDeleteConversationContent}
                    onClick={() =>
                      onRequestMessageDelete({
                        sessionId,
                        turnId: turn.turn_id,
                        role: "user",
                        roleIndex,
                        preview: entry.text,
                      })
                    }
                  >
                    <Trash2 size={13} />
                  </button>
                  {entry.kind === "supplement" && <span>[补充]</span>}
                  <MarkdownContent text={entry.text} />
                  {(
                    entry.worker_roles ??
                    (entry.worker_role ? [entry.worker_role] : [])
                  ).length > 0 && (
                    <div
                      className="turn-entry-roles"
                      aria-label={`使用 Role：${(entry.worker_roles ?? (entry.worker_role ? [entry.worker_role] : [])).map((role) => role.name).join("、")}`}
                    >
                      <BriefcaseBusiness size={12} />
                      <span>Role</span>
                      {(
                        entry.worker_roles ??
                        (entry.worker_role ? [entry.worker_role] : [])
                      ).map((role) => (
                        <b key={role.id} title={role.description}>
                          {role.name}
                        </b>
                      ))}
                    </div>
                  )}
                  {!!entry.attachments?.length && (
                    <div className="turn-entry-attachments">
                      {entry.attachments.map((attachment) => (
                        <span key={attachment.id} title={attachment.path}>
                          <Paperclip size={13} />
                          <i aria-hidden="true">:</i>
                          <b>{attachment.name}</b>
                          <small>{formatBytes(attachment.bytes)}</small>
                        </span>
                      ))}
                    </div>
                  )}
                </div>
              ))}
          </div>
        </section>
      )}
      {showWorkFrame && (
        <section
          className={`turn-assistant-frame ${isWorking ? "working" : interrupted ? "interrupted" : turn.state} ${workStreamVisible ? "" : "collapsed-work"}`}
        >
          {canToggleWorkStream && (
            <div className="turn-assistant-heading">
              <button
                type="button"
                className={`working-chip work-title-chip work-collapse-toggle${isWorking ? " active-work-title" : " completed-work-title"}${interrupted ? " interrupted-work-title" : ""}${isToolGenTurn ? ` toolgen-working${isWorking ? "" : " toolgen-completed-title"}` : ""}`}
                title={
                  showWorkStream ? "Hide work details" : "Show work details"
                }
                aria-label={
                  showWorkStream ? "Hide work details" : "Show work details"
                }
                aria-expanded={showWorkStream}
                onClick={() => setShowWorkStream((visible) => !visible)}
              >
                <ChevronRight
                  className="work-collapse-arrow"
                  size={13}
                  aria-hidden="true"
                />
                {isToolGenTurn && <Wrench size={11} />}{" "}
                {isWorking ? (
                  isToolGenTurn ? (
                    "Generating tools…"
                  ) : (
                    <span className="working-label">working</span>
                  )
                ) : isToolGenTurn ? (
                  "ToolGen"
                ) : (
                  "Thought/Action"
                )}
                {isWorking && (
                  <WorkingElapsed createdAtMs={turn.created_at_ms} />
                )}
                {!isWorking &&
                  turn.state === "interrupted" &&
                  turn.interrupted_at_ms != null && (
                    <WorkingElapsed
                      createdAtMs={turn.created_at_ms}
                      endedAtMs={turn.interrupted_at_ms}
                    />
                  )}
                {interrupted && (
                  <span className="work-title-status">
                    ({cancelled ? "Cancelled" : "Interrupted"})
                  </span>
                )}
              </button>
              {isWorking && modelRetryStatus && (
                <details
                  className={`model-retry-status ${modelRetryStatus.kind}`}
                >
                  <summary
                    title={`展开 ${modelRetryStatus.label} 详情`}
                    aria-label={`展开 ${modelRetryStatus.label} 详情`}
                  >
                    <ChevronRight size={12} aria-hidden="true" />
                    <span>{modelRetryStatus.label}</span>
                    {modelRetryStatus.progress && (
                      <small>{modelRetryStatus.progress}</small>
                    )}
                  </summary>
                  <div className="model-retry-detail">
                    <MarkdownContent text={modelRetryStatus.detail} />
                  </div>
                </details>
              )}
            </div>
          )}
          {workStreamVisible && (
            <div className="turn-work-panel">
              <div
                className={`turn-work-scroll has-content${workEdgeFades.top ? " fade-top" : ""}${workEdgeFades.bottom ? " fade-bottom" : ""}${pendingUpdates > 0 ? " has-pending-updates" : ""}`}
                role="region"
                aria-label={
                  isToolGenTurn ? "ToolGen work stream" : "Task work stream"
                }
                ref={workScrollRef}
                onScroll={(event) => {
                  updateWorkEdgeFades(event.currentTarget);
                  followLatest.current = isNearScrollBottom(
                    {
                      scrollTop: event.currentTarget.scrollTop,
                      scrollHeight: event.currentTarget.scrollHeight,
                      clientHeight: event.currentTarget.clientHeight,
                    },
                    36,
                  );
                  if (followLatest.current) setPendingUpdates(0);
                }}
              >
                <div className="turn-work-content" ref={workContentRef}>
                  {" "}
                  {scrollItems.map((item, index) => {
                    const { activity } = item;
                    if (activity?.tone === "action") {
                      const summary = toolActivityRunByStartIndex.get(index);
                      return summary ? (
                        <ToolActivityGroup
                          key={`tool-activity-group-${item.key}`}
                          summary={summary}
                        />
                      ) : null;
                    }
                    return activity ? (
                      <ActivityView key={item.key} activity={activity} />
                    ) : null;
                  })}{" "}
                  {decisions.map((decision, index) => (
                    <InlineDecision
                      key={decisionKey(decision)}
                      decision={decision}
                      pending={pendingDecisionKeys.has(decisionKey(decision))}
                      locked={sessionInteractionLocked}
                      position={index + 1}
                      total={decisions.length}
                      onReply={(reply) => onDecisionReply(decision, reply)}
                    />
                  ))}
                  {isWorking && <LiveTurnUsage turn={turn} />}
                </div>
              </div>
              {pendingUpdates > 0 && (
                <button
                  type="button"
                  className="turn-new-updates"
                  title="Scroll to latest work update"
                  aria-live="polite"
                  aria-label={`${pendingUpdates} new work update${pendingUpdates === 1 ? "" : "s"}; scroll to latest`}
                  onClick={scrollWorkToLatest}
                >
                  <ArrowDown size={13} aria-hidden="true" />
                  {pendingUpdates} new update{pendingUpdates === 1 ? "" : "s"}
                </button>
              )}
            </div>
          )}
        </section>
      )}
      {persistentToolGenItems.length > 0 && (
        <div className="turn-persistent-toolgen" aria-label="ToolGen result">
          {persistentToolGenItems.map(({ key, activity }) =>
            activity ? <ActivityView key={key} activity={activity} /> : null,
          )}
        </div>
      )}
      {(turn.sub_answers.length > 0 || turn.final_answer) && (
        <TurnAnswerDelivery
          turn={turn}
          toolGenPending={toolGenPending}
          toolGenBlocked={toolGenBlocked}
          onToolGen={
            isToolGenTurn || !onRequestToolGen
              ? undefined
              : () => onRequestToolGen(turn.turn_id)
          }
          favorite={favorite}
          favoritePending={favoritePending}
          onToggleFavorite={() =>
            onToggleFavorite(sessionId, turn.turn_id, favorite?.id)
          }
          onDelete={
            turn.final_answer && canDeleteConversationContent
              ? () =>
                  onRequestMessageDelete({
                    sessionId,
                    turnId: turn.turn_id,
                    role: "assistant",
                    roleIndex: 0,
                    preview: turn.final_answer ?? "",
                  })
              : undefined
          }
        />
      )}
      {!turn.final_answer &&
        turn.sub_answers.length === 0 &&
        (turn.completion || cancelled) && (
          <section className="turn-completion-only">
            <CompletionCard
              completion={turn.completion ?? { stop_reason: "CancelledByUser" }}
            />
          </section>
        )}
    </article>
  );
}, areTurnInteractionPropsEqual);

function areTurnInteractionPropsEqual(
  previous: TurnInteractionProps,
  next: TurnInteractionProps,
) {
  if (
    previous.sessionId !== next.sessionId ||
    previous.turn !== next.turn ||
    previous.isCancelling !== next.isCancelling ||
    previous.sessionInteractionLocked !== next.sessionInteractionLocked ||
    previous.toolGenPending !== next.toolGenPending ||
    previous.toolGenBlocked !== next.toolGenBlocked ||
    previous.favorite?.id !== next.favorite?.id ||
    previous.favoritePending !== next.favoritePending ||
    previous.onToggleFavorite !== next.onToggleFavorite ||
    previous.onDecisionReply !== next.onDecisionReply ||
    previous.onRequestToolGen !== next.onRequestToolGen ||
    previous.onRequestMessageDelete !== next.onRequestMessageDelete ||
    previous.decisions.length !== next.decisions.length
  )
    return false;
  return previous.decisions.every((decision, index) => {
    const nextDecision = next.decisions[index];
    return (
      decision === nextDecision &&
      previous.pendingDecisionKeys.has(decisionKey(decision)) ===
        next.pendingDecisionKeys.has(decisionKey(nextDecision))
    );
  });
}

function TurnAnswerDelivery({
  turn,
  toolGenPending,
  toolGenBlocked,
  favorite,
  favoritePending,
  onToggleFavorite,
  onToolGen,
  onDelete,
}: {
  turn: WebTurn;
  toolGenPending: boolean;
  toolGenBlocked: boolean;
  favorite?: ChatFavorite;
  favoritePending: boolean;
  onToggleFavorite: () => boolean;
  onToolGen?: () => void;
  onDelete?: () => void;
}) {
  const hasFinal = !!turn.final_answer;
  const hasChat = turn.sub_answers.length > 0;
  const [chatExpanded, setChatExpanded] = useState(() => !hasFinal);
  const previousFinal = useRef(hasFinal);
  const chatPanelId = `turn-chat-${turn.turn_id}`;
  const chatItems = newestInterimAnswersFirst(turn.sub_answers);
  useEffect(() => {
    const finalArrived = !previousFinal.current && !!turn.final_answer;
    previousFinal.current = !!turn.final_answer;
    if (finalArrived) setChatExpanded(false);
  }, [turn.final_answer]);
  if (!hasChat && !hasFinal) return null;
  return (
    <section className="turn-answer-delivery">
      {hasChat && (
        <section
          className={`turn-chat-delivery${chatExpanded ? " expanded" : " collapsed"}`}
        >
          <div className="turn-chat-heading">
            <button
              type="button"
              className="working-chip work-title-chip work-collapse-toggle chat-title-chip"
              title={chatExpanded ? "Hide chat answers" : "Show chat answers"}
              aria-label={
                chatExpanded ? "Hide chat answers" : "Show chat answers"
              }
              aria-expanded={chatExpanded}
              aria-controls={chatPanelId}
              onClick={() => setChatExpanded((expanded) => !expanded)}
            >
              <ChevronRight
                className="work-collapse-arrow"
                size={13}
                aria-hidden="true"
              />
              Chat
            </button>
          </div>
          {chatExpanded && (
            <div
              id={chatPanelId}
              className="turn-chat-panel"
              role="region"
              aria-label="Chat answers"
            >
              <div className="turn-interim-list">
                {chatItems.map(({ item, ordinal }) => (
                  <section
                    className="turn-interim-item"
                    key={item.sub_answer_id}
                  >
                    <h3>
                      <span>{ordinal}.</span> {item.task}
                    </h3>
                    <div className="message-content">
                      <MarkdownContent text={item.answer} />
                    </div>
                  </section>
                ))}
              </div>
            </div>
          )}
        </section>
      )}
      {hasFinal && turn.final_answer && (
        <FinalAnswerDelivery
          text={turn.final_answer}
          completion={turn.completion}
          toolGenPending={toolGenPending}
          toolGenBlocked={toolGenBlocked}
          favorite={favorite}
          favoritePending={favoritePending}
          onToggleFavorite={onToggleFavorite}
          onToolGen={onToolGen}
          onDelete={onDelete}
        />
      )}
    </section>
  );
}

function FinalAnswerDelivery({
  text,
  completion,
  toolGenPending,
  toolGenBlocked,
  favorite,
  favoritePending,
  onToggleFavorite,
  onToolGen,
  onDelete,
}: {
  text: string;
  completion: WebTurn["completion"];
  toolGenPending: boolean;
  toolGenBlocked: boolean;
  favorite?: ChatFavorite;
  favoritePending: boolean;
  onToggleFavorite: () => boolean;
  onToolGen?: () => void;
  onDelete?: () => void;
}) {
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(
    text,
    {
      idle: "Copy answer",
      copied: "Answer copied",
      failed: "Copy answer failed",
    },
  );
  const answerActions = (
    <div className="final-answer-actions">
      <button
        type="button"
        className={`final-favorite ${favorite || favoritePending ? "active" : ""} ${favoritePending ? "pending" : ""}`}
        title={
          favoritePending
            ? "Saving favorite"
            : favorite
              ? "Remove from favorites"
              : "Favorite this answer"
        }
        aria-label={
          favoritePending
            ? "Saving favorite"
            : favorite
              ? "Remove answer from favorites"
              : "Favorite answer"
        }
        aria-pressed={!!favorite || favoritePending}
        disabled={favoritePending}
        onClick={onToggleFavorite}
      >
        <Star
          size={13}
          fill={favorite || favoritePending ? "currentColor" : "none"}
        />
      </button>
      <button
        type="button"
        className={`final-copy ${copyClass}`}
        title={copyLabel}
        aria-label={copyLabel}
        onClick={() => void copy()}
      >
        {copyState === "copied" ? <CheckCheck size={13} /> : <Copy size={13} />}
      </button>
      {onDelete && (
        <button
          type="button"
          className="chat-message-delete assistant-message-delete"
          title="Delete this answer from the conversation and raw chat log"
          aria-label="Delete assistant answer"
          onClick={onDelete}
        >
          <Trash2 size={13} />
        </button>
      )}
    </div>
  );
  return (
    <section className="turn-final-delivery">
      <FinalAnswerContent text={text} />
      {completion ? (
        <CompletionCard
          completion={completion}
          toolGenPending={toolGenPending}
          toolGenBlocked={toolGenBlocked}
          onToolGen={onToolGen}
          answerActions={answerActions}
        />
      ) : (
        answerActions
      )}
    </section>
  );
}

const FINAL_ANSWER_OUTLINE_MIN_SECTIONS = 2;
const FINAL_ANSWER_OUTLINE_SCROLL_OFFSET = 24;
const FINAL_ANSWER_OUTLINE_SCROLL_DURATION_MS = 180;
const FINAL_ANSWER_OUTLINE_RAIL_EDGE_PADDING = 12;

const FINAL_ANSWER_OUTLINE_EDGE_GUARD = 12;
const FINAL_ANSWER_OUTLINE_VIEWPORT_RATIO = 0.15;
const FINAL_ANSWER_OUTLINE_TOGGLE_HEIGHT = 52;

function FinalAnswerContent({ text }: { text: string }) {
  const timelineActive = useContext(SessionTimelineActiveContext);
  const outline = useMemo(() => {
    try {
      return extractMarkdownOutline(text);
    } catch {
      return [];
    }
  }, [text]);
  const reactId = useId();
  const stableId = reactId.replaceAll(":", "");
  const headingPrefix = `final-heading-${stableId}`;
  const readingId = `final-answer-reading-${stableId}`;
  const rootRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const outlineNavRef = useRef<HTMLElement | null>(null);
  const outlineNavigationAnimationRef = useRef<number | null>(null);
  const outlineHeadingOffsetsRef = useRef(new Map<string, number>());
  const outlineActiveTaskRef = useRef<FrameTask | null>(null);
  const outlineGeometryTaskRef = useRef<FrameTask | null>(null);
  const [showOutline, setShowOutline] = useState(false);

  const [outlineHost, setOutlineHost] = useState<HTMLElement | null>(null);
  const [outlineGeometry, setOutlineGeometry] = useState({
    top: 0,
    height: 0,
    stickyTop: 0,
  });
  const [outlinePlacement, setOutlinePlacement] = useState<
    "docked" | "overlay"
  >("docked");
  const [outlineCollapsed, setOutlineCollapsed] = useState(false);
  const [activeId, setActiveId] = useState(MARKDOWN_OUTLINE_START_ID);

  useLayoutEffect(() => {
    if (timelineActive) return;
    if (outlineNavigationAnimationRef.current !== null) {
      cancelAnimationFrame(outlineNavigationAnimationRef.current);
      outlineNavigationAnimationRef.current = null;
    }
    outlineHeadingOffsetsRef.current.clear();
    setShowOutline(false);
  }, [timelineActive]);

  useEffect(() => setActiveId(MARKDOWN_OUTLINE_START_ID), [outline]);

  useLayoutEffect(() => {
    if (outlineCollapsed || !showOutline) return;
    const nav = outlineNavRef.current;
    const activeItem = nav?.querySelector<HTMLElement>(
      '[aria-current="location"]',
    );
    if (!nav || !activeItem) return;
    const navRect = nav.getBoundingClientRect();
    const itemRect = activeItem.getBoundingClientRect();
    const targetTop = nav.scrollTop + itemRect.top - navRect.top;
    const nextScrollTop = markdownOutlineRailScrollTop(
      nav.scrollTop,
      nav.clientHeight,
      nav.scrollHeight,
      targetTop,
      itemRect.height,
      FINAL_ANSWER_OUTLINE_RAIL_EDGE_PADDING,
    );
    if (Math.abs(nextScrollTop - nav.scrollTop) > 0.5)
      nav.scrollTop = nextScrollTop;
  }, [activeId, outlineCollapsed, showOutline]);

  useLayoutEffect(() => {
    const root = rootRef.current;
    const content = contentRef.current;
    const viewport = root?.closest<HTMLElement>(".chat-scroll");
    const chatShell = viewport?.closest<HTMLElement>(".chat-shell");
    if (
      !timelineActive ||
      !root ||
      !content ||
      !viewport ||
      !chatShell ||
      outline.length < FINAL_ANSWER_OUTLINE_MIN_SECTIONS
    ) {
      setShowOutline(false);
      return;
    }
    let updateFrame: number | null = null;
    const update = () => {
      updateFrame = null;
      const contentRect = content.getBoundingClientRect();
      const viewportRect = viewport.getBoundingClientRect();
      const bodyInset = Math.max(0, contentRect.left - viewportRect.left);
      const configuredWidth = Number.parseFloat(
        getComputedStyle(root).getPropertyValue("--final-outline-width"),
      );
      const outlineWidth =
        Number.isFinite(configuredWidth) && configuredWidth > 0
          ? configuredWidth
          : 0;
      const nextTop = contentRect.top - viewportRect.top + viewport.scrollTop;
      const nextHeight = contentRect.height;
      const targetWindowTop =
        window.innerHeight * FINAL_ANSWER_OUTLINE_VIEWPORT_RATIO;
      const targetViewportTop = targetWindowTop - viewportRect.top;
      const nextStickyTop = Math.max(
        0,
        targetViewportTop -
          (outlineCollapsed ? FINAL_ANSWER_OUTLINE_TOGGLE_HEIGHT / 2 : 0),
      );
      setOutlineHost((current) => (current === viewport ? current : viewport));
      setOutlineGeometry((current) =>
        current.top === nextTop &&
        current.height === nextHeight &&
        current.stickyTop === nextStickyTop
          ? current
          : { top: nextTop, height: nextHeight, stickyTop: nextStickyTop },
      );
      setOutlinePlacement(
        bodyInset >= outlineWidth + FINAL_ANSWER_OUTLINE_EDGE_GUARD
          ? "docked"
          : "overlay",
      );
      setShowOutline(
        finalAnswerNeedsOutline(
          content.offsetHeight,
          viewport.clientHeight,
          outline.length,
        ),
      );
    };
    const scheduleUpdate = () => {
      if (updateFrame !== null) return;
      updateFrame = window.requestAnimationFrame(update);
    };
    update();
    window.addEventListener("resize", scheduleUpdate);
    const observer =
      typeof ResizeObserver === "undefined"
        ? undefined
        : new ResizeObserver(scheduleUpdate);
    observer?.observe(root);
    observer?.observe(content);
    observer?.observe(viewport);
    observer?.observe(chatShell);
    return () => {
      window.removeEventListener("resize", scheduleUpdate);
      observer?.disconnect();
      if (updateFrame !== null) cancelAnimationFrame(updateFrame);
    };
  }, [outline, outlineCollapsed, text, timelineActive]);

  useEffect(() => {
    setOutlineCollapsed(outlinePlacement === "overlay");
  }, [outlinePlacement]);

  useLayoutEffect(() => {
    window.dispatchEvent(new Event("markdown-outline-layout-change"));
  }, [outlineCollapsed, outlinePlacement, showOutline]);

  const updateOutlineActive = useCallback(() => {
    const root = rootRef.current;
    const viewport = root?.closest<HTMLElement>(".chat-scroll");
    const pane = root?.closest<HTMLElement>("[data-session-timeline-active]");
    if (!root || !viewport || pane?.dataset.sessionTimelineActive !== "true")
      return;
    const threshold = viewport.scrollTop + FINAL_ANSWER_OUTLINE_SCROLL_OFFSET;
    const next = markdownOutlineActiveId(
      outline,
      outlineHeadingOffsetsRef.current,
      threshold,
    );
    setActiveId((current) => (current === next ? current : next));
  }, [outline]);

  const refreshOutlineGeometry = useCallback(() => {
    const root = rootRef.current;
    const viewport = root?.closest<HTMLElement>(".chat-scroll");
    const pane = root?.closest<HTMLElement>("[data-session-timeline-active]");
    if (!root || !viewport || pane?.dataset.sessionTimelineActive !== "true")
      return;
    const viewportTop = viewport.getBoundingClientRect().top;
    const scrollTop = viewport.scrollTop;
    const offsets = new Map<string, number>();
    for (const item of outline) {
      const heading = document.getElementById(`${headingPrefix}-${item.id}`);
      if (heading && root.contains(heading))
        offsets.set(
          item.id,
          scrollTop + heading.getBoundingClientRect().top - viewportTop,
        );
    }
    outlineHeadingOffsetsRef.current = offsets;
    updateOutlineActive();
  }, [headingPrefix, outline, updateOutlineActive]);

  useLayoutEffect(() => {
    const activeTask = createFrameTask({ run: updateOutlineActive });
    const geometryTask = createFrameTask({ run: refreshOutlineGeometry });
    outlineActiveTaskRef.current = activeTask;
    outlineGeometryTaskRef.current = geometryTask;
    return () => {
      activeTask.dispose();
      geometryTask.dispose();
      if (outlineActiveTaskRef.current === activeTask)
        outlineActiveTaskRef.current = null;
      if (outlineGeometryTaskRef.current === geometryTask)
        outlineGeometryTaskRef.current = null;
    };
  }, [refreshOutlineGeometry, updateOutlineActive]);

  useEffect(() => {
    const root = rootRef.current;
    const content = contentRef.current;
    const viewport = root?.closest<HTMLElement>(".chat-scroll");
    if (!root || !content || !viewport || !showOutline) return;
    const refresh = () => {
      outlineGeometryTaskRef.current?.request();
    };
    const updateScrollState = () => {
      outlineActiveTaskRef.current?.request();
    };
    let listeningForScroll = false;
    const setScrollListening = (enabled: boolean) => {
      const active =
        enabled &&
        root.closest<HTMLElement>("[data-session-timeline-active]")?.dataset
          .sessionTimelineActive === "true";
      if (active === listeningForScroll) return;
      listeningForScroll = active;
      if (active) {
        refresh();
        viewport.addEventListener("scroll", updateScrollState, {
          passive: true,
        });
      } else {
        viewport.removeEventListener("scroll", updateScrollState);
      }
    };
    const syncActivePane = () =>
      setScrollListening(
        root.closest<HTMLElement>("[data-session-timeline-active]")?.dataset
          .sessionTimelineActive === "true",
      );
    refresh();
    window.addEventListener(
      "session-timeline-activation-change",
      syncActivePane,
    );
    let visibilityObserver: IntersectionObserver | undefined;
    if (typeof IntersectionObserver === "undefined") {
      syncActivePane();
    } else {
      visibilityObserver = new IntersectionObserver(
        (entries) =>
          setScrollListening(entries.some((entry) => entry.isIntersecting)),
        { root: viewport, rootMargin: "100% 0px", threshold: 0 },
      );
      visibilityObserver.observe(root);
    }
    const resizeObserver =
      typeof ResizeObserver === "undefined"
        ? undefined
        : new ResizeObserver(refresh);
    resizeObserver?.observe(content);
    resizeObserver?.observe(viewport);
    const mutationObserver = new MutationObserver(refresh);
    mutationObserver.observe(content, { childList: true, subtree: true });
    return () => {
      setScrollListening(false);
      window.removeEventListener(
        "session-timeline-activation-change",
        syncActivePane,
      );
      visibilityObserver?.disconnect();
      resizeObserver?.disconnect();
      mutationObserver.disconnect();
    };
  }, [showOutline, text]);

  useEffect(
    () => () => {
      if (outlineNavigationAnimationRef.current !== null)
        cancelAnimationFrame(outlineNavigationAnimationRef.current);
    },
    [],
  );

  const animateOutlineNavigation = (
    viewport: HTMLElement,
    targetTop: number,
    nextActiveId: string,
  ) => {
    if (outlineNavigationAnimationRef.current !== null)
      cancelAnimationFrame(outlineNavigationAnimationRef.current);
    setActiveId(nextActiveId);
    if (prefersReducedMotion()) {
      viewport.scrollTop = targetTop;
      outlineNavigationAnimationRef.current = null;
      return;
    }
    const startTop = viewport.scrollTop;
    const distance = targetTop - startTop;
    const startedAt = performance.now();
    const animate = (now: number) => {
      const elapsedMs = now - startedAt;
      viewport.scrollTop = markdownOutlineAnimationPosition(
        startTop,
        targetTop,
        elapsedMs,
        FINAL_ANSWER_OUTLINE_SCROLL_DURATION_MS,
      );
      if (elapsedMs < FINAL_ANSWER_OUTLINE_SCROLL_DURATION_MS)
        outlineNavigationAnimationRef.current = requestAnimationFrame(animate);
      else outlineNavigationAnimationRef.current = null;
    };
    outlineNavigationAnimationRef.current = requestAnimationFrame(animate);
  };

  const navigateToStart = () => {
    const root = rootRef.current;
    const viewport = root?.closest<HTMLElement>(".chat-scroll");
    if (!root || !viewport) return;
    const viewportTop = viewport.getBoundingClientRect().top;
    const targetTop = markdownOutlineTargetScrollTop(
      viewport.scrollTop,
      root.getBoundingClientRect().top,
      viewportTop,
      FINAL_ANSWER_OUTLINE_SCROLL_OFFSET,
    );
    animateOutlineNavigation(viewport, targetTop, MARKDOWN_OUTLINE_START_ID);
  };

  const navigate = (item: MarkdownOutlineItem) => {
    const root = rootRef.current;
    const viewport = root?.closest<HTMLElement>(".chat-scroll");
    const heading = document.getElementById(`${headingPrefix}-${item.id}`);
    if (!root || !viewport || !heading || !root.contains(heading)) return;
    const viewportTop = viewport.getBoundingClientRect().top;
    const targetTop = markdownOutlineTargetScrollTop(
      viewport.scrollTop,
      heading.getBoundingClientRect().top,
      viewportTop,
      FINAL_ANSWER_OUTLINE_SCROLL_OFFSET,
    );
    animateOutlineNavigation(viewport, targetTop, item.id);
  };

  const outlineElement =
    timelineActive && showOutline && outlineHost
      ? createPortal(
          <aside
            className={`final-answer-outline ${outlinePlacement}${outlineCollapsed ? " collapsed" : " expanded"}`}
            data-final-answer-reading-id={readingId}
            style={
              {
                top: `${outlineGeometry.top}px`,
                height: `${outlineGeometry.height}px`,
                "--final-outline-sticky-top": `${outlineGeometry.stickyTop}px`,
              } as React.CSSProperties
            }
            aria-label="Final answer table of contents"
          >
            <div className="final-answer-outline-anchor">
              {outlineCollapsed && (
                <button
                  type="button"
                  className="final-answer-outline-toggle"
                  aria-expanded={false}
                  aria-label="Show table of contents"
                  title="Show table of contents"
                  onClick={() => setOutlineCollapsed(false)}
                >
                  <BookText
                    size={19}
                    strokeWidth={1.8}
                    aria-hidden="true"
                  />
                  <ChevronRight
                    className="final-answer-outline-toggle-arrow"
                    size={14}
                    strokeWidth={2.4}
                    aria-hidden="true"
                  />
                </button>
              )}
              {!outlineCollapsed && (
                <div className="final-answer-outline-card">
                  <header>
                    <button
                      type="button"
                      className="final-answer-outline-close"
                      aria-label="Hide table of contents"
                      title="Hide table of contents"
                      onClick={() => setOutlineCollapsed(true)}
                    >
                      <ChevronLeft
                        size={16}
                        strokeWidth={2.4}
                        aria-hidden="true"
                      />
                    </button>
                    <span>
                      <BookText size={13} strokeWidth={1.8} aria-hidden="true" />
                      Contents
                    </span>
                  </header>
                  <nav ref={outlineNavRef}>
                    <button
                      type="button"
                      className={`final-answer-outline-start${activeId === MARKDOWN_OUTLINE_START_ID ? " active" : ""}`}
                      aria-current={
                        activeId === MARKDOWN_OUTLINE_START_ID
                          ? "location"
                          : undefined
                      }
                      title="Go to the start of this answer"
                      onClick={navigateToStart}
                    >
                      <CornerUpLeft size={11} aria-hidden="true" />
                      <span>Start</span>
                    </button>
                    {outline.map((item) => (
                      <button
                        type="button"
                        key={item.id}
                        className={`${activeId === item.id ? "active " : ""}level-${item.level}`}
                        aria-current={
                          activeId === item.id ? "location" : undefined
                        }
                        title={item.title}
                        onClick={() => navigate(item)}
                      >
                        {item.title}
                      </button>
                    ))}
                  </nav>
                </div>
              )}
            </div>
          </aside>,
          outlineHost,
        )
      : null;

  return (
    <div
      id={readingId}
      ref={(node) => {
        rootRef.current = node;
        contentRef.current = node;
      }}
      className={`message-content final-answer-reading${showOutline ? " has-outline" : ""}`}
    >
      {outlineElement}
      <MarkdownContent
        text={text}
        headingIdPrefix={
          outline.length >= FINAL_ANSWER_OUTLINE_MIN_SECTIONS
            ? headingPrefix
            : undefined
        }
      />
    </div>
  );
}

function HeaderContextUsage({ session }: { session: Session | undefined }) {
  const usage = session ? sessionContextUsage(session) : undefined;
  const cacheHitPercent = session ? sessionCacheHitPercent(session) : undefined;
  const limit = session?.max_llm_input_tokens || undefined;
  const ratio = limit
    ? Math.min(100, Math.ceil(((usage?.prompt_tokens ?? 0) * 100) / limit))
    : 0;
  const level = ratio >= 90 ? "critical" : ratio >= 75 ? "warning" : "normal";
  const cacheLabel =
    cacheHitPercent === undefined
      ? "cache: —"
      : `cache: ${cacheHitPercent.toFixed(1)}%`;
  const contextUsageLabel = limit
    ? `Context usage ${ratio}% · ${formatTokens(usage?.prompt_tokens ?? 0)} / ${formatTokens(limit)} input tokens · ${cacheLabel}`
    : `Context usage waiting for runtime usage · ${cacheLabel}`;
  return (
    <span
      className={`header-context ${level}`}
      title={contextUsageLabel}
      aria-label={contextUsageLabel}
    >
      <span className="header-context-main">
        <span aria-hidden="true">· ctx</span>
        <span className="header-context-meter" aria-hidden="true">
          <span style={{ width: `${ratio}%` }} />
        </span>
        <span>{limit ? `${ratio}%/${formatTokens(limit)}` : "—"}</span>
      </span>
      <span className="header-cache-rate">
        <span aria-hidden="true">· </span>
        {cacheLabel}
      </span>
    </span>
  );
}

function LiveTurnUsage({ turn }: { turn: WebTurn }) {
  const usage = turnLiveUsage(turn);
  if (!usage) return null;
  return (
    <div className="live-turn-usage" aria-label="Current task token usage">
      <span>
        <b>Task</b> ▲{formatTokens(usage.total.prompt_tokens) ?? "0"} ▼
        {formatTokens(usage.total.completion_tokens) ?? "0"}
      </span>
      <span>
        <b>Latest</b> △{formatTokens(usage.latest.prompt_tokens) ?? "0"} ▽
        {formatTokens(usage.latest.completion_tokens) ?? "0"}
      </span>
      {!!usage.total.cached_tokens && (
        <span>
          <b>KVC</b> {formatTokens(usage.total.cached_tokens)}
        </span>
      )}
    </div>
  );
}

function ActivityView({ activity }: { activity: Activity }) {
  if (activity.kind === "context_compact")
    return <ContextCompactNotice activity={activity} />;
  if (activity.kind === "toolgen") return <ToolGenNotice activity={activity} />;
  if (activity.kind === "user_supplement")
    return (
      <div className="turn-work-item thinking user-supplement">
        <span className="activity-mark" aria-hidden="true">
          💡
        </span>
        <div className="user-supplement-line">
          <strong>{activity.title}</strong>
          {activity.detail && <span>{activity.detail}</span>}
        </div>
      </div>
    );
  if (activity.tone === "action") return <ToolActivity activity={activity} />;
  return (
    <div
      className={`turn-work-item ${activity.tone}${activity.kind === "free_talk" ? " free-talk" : ""}`}
    >
      <span className="activity-mark">
        {activity.tone === "thinking" ? (
          <span className="activity-thinking-dot" aria-hidden="true" />
        ) : activity.tone === "warning" ? (
          "⚠️"
        ) : activity.tone === "error" ? (
          "×"
        ) : (
          "i"
        )}
      </span>
      <div>
        {activity.title && <strong>{activity.title}</strong>}
        {activity.detail && (
          <div className="turn-work-detail">
            <MarkdownContent text={activity.detail} />
          </div>
        )}
        {activity.code && (
          <MarkdownContent
            text={fencedCode(activity.code_language ?? "text", activity.code)}
          />
        )}
      </div>
    </div>
  );
}

function ToolGenNotice({ activity }: { activity: Activity }) {
  const [open, setOpen] = useState(false);
  const hasDetail = !!activity.detail?.trim();
  if (!hasDetail)
    return (
      <blockquote className={`toolgen-notice ${activity.toolgen_phase ?? ""}`}>
        <span>{activity.title}</span>
      </blockquote>
    );
  const collapse = () => setOpen(false);
  const summaryLabel = `${open ? "收起" : "展开"} ToolGen 详情${activity.title ? `：${activity.title}` : ""}`;
  return (
    <details
      className={`toolgen-notice ${activity.toolgen_phase ?? ""}`}
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary
        title={open ? "收起 ToolGen 详情" : "展开 ToolGen 详情"}
        aria-label={summaryLabel}
      >
        <ChevronRight size={13} />
        <span>{activity.title}</span>
      </summary>
      <div>
        <button
          type="button"
          className="toolgen-collapse top"
          title="Collapse ToolGen details"
          aria-label="Collapse ToolGen details"
          onClick={collapse}
        >
          收起详情
        </button>
        <MarkdownContent text={activity.detail ?? ""} />
        <button
          type="button"
          className="toolgen-collapse"
          title="Collapse ToolGen details"
          aria-label="Collapse ToolGen details"
          onClick={collapse}
        >
          收起详情
        </button>
      </div>
    </details>
  );
}

function toolActivityGroupStatusLabel(summary: ToolActivitySummary) {
  if (summary.status === "completed") return "Succ";
  if (summary.status === "failed") return `Fail(${summary.failedCount})`;

  const activeParts: string[] = [];
  if (summary.foregroundRunningCount > 0)
    activeParts.push(`fg ${summary.foregroundRunningCount}`);
  if (summary.backgroundRunningCount > 0)
    activeParts.push(`bg ${summary.backgroundRunningCount}`);
  if (summary.failedCount > 0)
    activeParts.push(`failed ${summary.failedCount}`);
  return activeParts.length > 0
    ? `running (${activeParts.join(" · ")})`
    : "running";
}

function ToolActivityGroup({ summary }: { summary: ToolActivitySummary }) {
  const [open, setOpen] = useState(false);
  const singleActivity =
    summary.activities.length === 1 ? summary.activities[0] : undefined;
  if (
    singleActivity?.tool_name === "run_bash" &&
    singleActivity.tool_mode === "poll"
  )
    return <ToolActivity activity={singleActivity} />;
  const running = summary.status === "running";
  const groupStatusLabel = toolActivityGroupStatusLabel(summary);
  const summaryLabel = `${open ? "收起" : "展开"}工具活动：${summary.label}，${groupStatusLabel}`;
  return (
    <details
      className={`tool-activity-group ${summary.status}`}
      open={open}
      aria-busy={running || undefined}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary
        aria-label={summaryLabel}
        title={open ? "收起工具活动" : "展开工具活动"}
      >
        <span
          className="tool-activity-group-icon tool-command-symbol"
          aria-hidden="true"
        >
          &gt;_
        </span>
        <span className="tool-activity-group-status">{groupStatusLabel}</span>
        <span className="tool-activity-group-counts" aria-hidden="true">
          {summary.counts.map(({ name, count }, index) => (
            <span className="tool-activity-group-count" key={name}>
              {index > 0 && <i>|</i>}
              <span>{name}</span>
              <strong>{count}</strong>
            </span>
          ))}
        </span>
        <ChevronRight className="tool-activity-chevron" size={14} />
      </summary>
      <div className="tool-activity-group-body">
        {summary.activities.map((activity, index) => (
          <ToolActivity key={`${activity.id}-${index}`} activity={activity} />
        ))}
      </div>
    </details>
  );
}

function ToolActivity({ activity }: { activity: Activity }) {
  const status = activity.tool_status || TOOL_STATUS_RUNNING;
  const running = isToolActivityRunning(status);
  const bashActivity = activity.tool_name === "run_bash";
  const pollingActivity = bashActivity && activity.tool_mode === "poll";
  const [open, setOpen] = useState(false);
  const waitBudgetMs = pollingActivity
    ? activity.loop_timeout_ms
    : bashActivity && activity.tool_mode !== "background"
      ? activity.timeout_ms
      : undefined;
  const [liveElapsedMs, setLiveElapsedMs] = useState(() =>
    Math.max(0, Date.now() - activity.createdAt),
  );
  useEffect(() => {
    if (!running || (!pollingActivity && waitBudgetMs === undefined)) return;
    const updateElapsed = () =>
      setLiveElapsedMs(Math.max(0, Date.now() - activity.createdAt));
    updateElapsed();
    const timer = window.setInterval(updateElapsed, 1_000);
    return () => window.clearInterval(timer);
  }, [activity.createdAt, pollingActivity, running, waitBudgetMs]);
  const invocationPreview = toolInvocationPreview(activity);
  const detail = activity.detail?.trim();
  const code = activity.code?.trim();
  const hasExpandableDetail = !!detail || !!code;
  const toolName = toolActivityDisplayName(
    activity.tool_name || activity.title,
    activity.tool_mode,
  );
  const displayedElapsedMs =
    pollingActivity && running ? liveElapsedMs : activity.elapsed_ms;
  const remainingWaitMs =
    running && activity.execution_started && waitBudgetMs !== undefined
      ? Math.max(0, waitBudgetMs - liveElapsedMs)
      : undefined;
  const statusLabel =
    status === "timeout" && bashActivity
      ? activity.pid !== undefined
        ? `wait ended · process still running · pid ${activity.pid}`
        : "wait ended · process may still be running"
      : humanizeToolStatus(status);
  const summaryLabel = `${open ? "收起" : "展开"}工具详情：${toolName}`;
  const summaryContent = (
    <>
      <span
        className={`tool-activity-icon ${pollingActivity ? "poll-activity-icon" : "tool-command-symbol"}`}
        aria-hidden="true"
      >
        {pollingActivity ? <Clock3 size={13} /> : ">_"}
      </span>
      <b>{toolName}</b>
      <span className="tool-activity-meta">
        <span className="tool-activity-status">{statusLabel}</span>
        {remainingWaitMs !== undefined && (
          <span className="tool-activity-countdown">
            {formatRemainingDuration(remainingWaitMs)} remaining
          </span>
        )}
        {displayedElapsedMs !== undefined && (pollingActivity || !running) && (
          <span className="tool-activity-duration">
            {pollingActivity
              ? `${formatClockDuration(displayedElapsedMs)} elapsed`
              : formatDuration(displayedElapsedMs)}
          </span>
        )}
      </span>
      {invocationPreview && (
        <code className="tool-activity-command" title={invocationPreview}>
          {invocationPreview}
        </code>
      )}
    </>
  );
  if (!hasExpandableDetail)
    return (
      <div
        className={`tool-activity tool-activity-static ${bashActivity ? "bash-activity" : ""}${pollingActivity ? " poll-activity" : ""} ${running ? "running" : "settled"}`}
        aria-busy={running || undefined}
      >
        {summaryContent}
      </div>
    );
  return (
    <details
      className={`tool-activity ${bashActivity ? "bash-activity" : ""}${pollingActivity ? " poll-activity" : ""} ${running ? "running" : "settled"}`}
      aria-busy={running || undefined}
      open={open}
      onToggle={(event) => setOpen(event.currentTarget.open)}
    >
      <summary
        title={open ? "收起工具详情" : "展开工具详情"}
        aria-label={summaryLabel}
      >
        {summaryContent}
        <ChevronRight className="tool-activity-chevron" size={14} />
      </summary>
      <div className="tool-activity-body">
        {detail && (
          <div className="turn-work-detail">
            <MarkdownContent text={detail} />
          </div>
        )}
        {code && (
          <MarkdownContent
            text={fencedCode(activity.code_language ?? "text", code)}
          />
        )}
      </div>
    </details>
  );
}

function toolInvocationPreview(activity: Activity) {
  const code = activity.code?.split("\n", 1)[0]?.trim();
  if (code) return code;
  return activity.detail?.split("\n", 1)[0]?.trim();
}

function activityFromTurnEvent(
  event: WebTurnEvent,
  sessionId: string,
): Activity | null {
  if (event.source === "ui_activity") {
    const activity = event.payload as unknown as Activity;
    return {
      ...activity,
      id: event.event_id,
      sessionId,
      createdAt: event.created_at_ms,
    };
  }
  if (event.source === "core_topic") {
    const activity = activityFromTopic(
      event.payload as unknown as import("./protocol").CoreTopicEvent,
    );
    return activity
      ? {
          ...activity,
          id: event.event_id,
          sessionId,
          createdAt: event.created_at_ms,
        }
      : null;
  }
  if (event.source !== "worker_activity") return null;
  const kind = String(event.payload.kind ?? "worker_event");
  if (
    kind === "model_request" ||
    kind === "model_response" ||
    kind === "model_retry"
  )
    return null;
  if (kind === "model_error") {
    const issue = modelServiceIssue(event.payload.error);
    return {
      id: event.event_id,
      sessionId,
      tone: "error",
      title: issue.title,
      detail: issue.detail,
      createdAt: event.created_at_ms,
    };
  }
  const detail = Object.entries(event.payload)
    .filter(
      ([key]) =>
        !["kind", "session_id", "context_id", "worker_id"].includes(key),
    )
    .map(
      ([key, value]) =>
        `${key}: ${typeof value === "string" ? value : JSON.stringify(value)}`,
    )
    .join("\n");
  return {
    id: event.event_id,
    sessionId,
    tone: kind.includes("error")
      ? "error"
      : kind.includes("retry") || kind.includes("discarded")
        ? "warning"
        : "notice",
    title: kind.replaceAll("_", " "),
    detail,
    createdAt: event.created_at_ms,
  };
}

function ContextCompactNotice({ activity }: { activity: Activity }) {
  const before = activity.before_tokens;
  const after = activity.after_tokens;
  const ratio =
    before && after !== undefined
      ? Math.max(6, Math.min(100, (after / before) * 100))
      : 36;
  const hasBreakdown =
    activity.text_before_tokens !== undefined ||
    activity.native_before_tokens !== undefined;
  const breakdown = hasBreakdown
    ? `Text ${formatTokens(activity.text_before_tokens) ?? "?"} → ${formatTokens(activity.text_after_tokens) ?? "?"}; Tool ${formatTokens(activity.native_before_tokens) ?? "?"} → ${formatTokens(activity.native_after_tokens) ?? "?"}`
    : undefined;
  const label = `Dynamic context compacted: ${formatTokens(before) ?? "unknown"} to ${formatTokens(after) ?? "unknown"}${breakdown ? `. ${breakdown}` : ""}`;
  return (
    <section
      className="context-compact-notice"
      aria-label={label}
      title={breakdown}
    >
      <div className="compact-icon">
        <Gauge size={13} />
      </div>
      <div className="compact-copy">
        <span>Dynamic context</span>
        <strong>
          {formatTokens(before) ?? "?"} → {formatTokens(after) ?? "?"}
        </strong>
        {breakdown && <small>{breakdown}</small>}
      </div>
      <div className="compact-meter" aria-hidden="true">
        <span className="compact-before" />
        <span className="compact-after" style={{ width: `${ratio}%` }} />
      </div>
    </section>
  );
}

function prefersReducedMotion() {
  return (
    window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false
  );
}

function McpPanel({
  panelRef,
  servers,
  session,
  pendingKeys,
  revealedSecrets,
  onClose,
  onCommand,
}: {
  panelRef: MutableRefObject<HTMLElement | null>;
  servers: McpServerReport[];
  session: Session | undefined;
  pendingKeys: Set<string>;
  revealedSecrets: Record<string, Record<string, string>>;
  onClose: () => void;
  onCommand: (key: string, command: ClientCommand) => void;
}) {
  const [editing, setEditing] = useState<McpServerConfig | null>(null);
  const [deleteMode, setDeleteMode] = useState(false);
  const [selectedDeleteServerId, setSelectedDeleteServerId] = useState("");
  const enabled = new Set(session?.mcp_server_ids ?? []);
  useEffect(() => {
    if (
      selectedDeleteServerId &&
      !servers.some((server) => server.config.id === selectedDeleteServerId)
    )
      setSelectedDeleteServerId("");
    if (deleteMode && servers.length === 0) setDeleteMode(false);
  }, [deleteMode, selectedDeleteServerId, servers]);
  const cancelDeleteMode = () => {
    setDeleteMode(false);
    setSelectedDeleteServerId("");
  };
  const confirmSelectedDelete = () => {
    const server = servers.find(
      (candidate) => candidate.config.id === selectedDeleteServerId,
    );
    if (
      !server ||
      !window.confirm(
        `Delete MCP server “${server.config.name}”? This removes it from every session in the current mem.`,
      )
    )
      return;
    onCommand(`delete:${server.config.id}`, {
      type: "mcp_server_delete",
      server_id: server.config.id,
    });
    cancelDeleteMode();
  };
  const startNew = () =>
    setEditing({
      id: "",
      name: "",
      enabled: true,
      transport: { type: "stdio", command: "", args: [], env: {} },
      request_timeout_ms: 30000,
    });
  return (
    <section
      id="mcp-panel"
      ref={panelRef}
      className="mcp-panel"
      role="dialog"
      aria-modal="false"
      aria-label="MCP servers"
      tabIndex={-1}
    >
      <header>
        <div>
          <span className="eyebrow">MCP</span>
          <h2>
            <strong className="mcp-session-name">
              {session?.display_name ?? "Current session"}
            </strong>{" "}
            's Capabilities
          </h2>
        </div>
        <div className="mcp-panel-header-actions">
          {!editing && deleteMode && (
            <button
              type="button"
              className="mcp-delete-cancel"
              title="取消删除 MCP"
              aria-label="取消删除 MCP"
              onClick={cancelDeleteMode}
            >
              <X size={14} strokeWidth={3} />
            </button>
          )}
          {!editing && (
            <button
              type="button"
              className={`mcp-delete-manage ${deleteMode ? "confirm" : ""}`}
              title={
                deleteMode
                  ? selectedDeleteServerId
                    ? "确认删除选中的 MCP"
                    : "请选择要删除的 MCP"
                  : "选择要删除的 MCP"
              }
              aria-label={
                deleteMode
                  ? selectedDeleteServerId
                    ? "确认删除选中的 MCP"
                    : "请选择要删除的 MCP"
                  : "选择要删除的 MCP"
              }
              disabled={
                servers.length === 0 ||
                (deleteMode &&
                  (!selectedDeleteServerId ||
                    pendingKeys.has(`delete:${selectedDeleteServerId}`)))
              }
              onClick={() => {
                if (!deleteMode) {
                  setDeleteMode(true);
                  setSelectedDeleteServerId("");
                } else confirmSelectedDelete();
              }}
            >
              {deleteMode ? (
                <Check size={14} strokeWidth={3} />
              ) : (
                <Trash2 size={14} />
              )}
            </button>
          )}
          <button
            type="button"
            className="icon-button"
            title="Close MCP panel"
            aria-label="Close MCP panel"
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </div>
      </header>
      {editing ? (
        <McpEditor
          config={editing}
          pending={pendingKeys.has(`save:${editing.id || "new"}`)}
          revealPending={
            !!editing.id && pendingKeys.has(`reveal:${editing.id}`)
          }
          revealedSecrets={editing.id ? revealedSecrets[editing.id] : undefined}
          onReveal={() =>
            editing.id &&
            onCommand(`reveal:${editing.id}`, {
              type: "mcp_server_secrets_reveal",
              server_id: editing.id,
            })
          }
          onCancel={() => setEditing(null)}
          onSave={(config) => {
            if (!session) return;
            const key = `save:${config.id || "new"}`;
            onCommand(key, {
              type: "mcp_server_upsert",
              session_id: session.session_id,
              config,
            });
            setEditing(null);
          }}
        />
      ) : (
        <>
          <div className="mcp-list">
            {servers.length === 0 ? (
              <div className="mcp-empty">
                <Plug size={20} />
                <strong>No MCP servers</strong>
                <span>Add local stdio, Streamable HTTP, or legacy SSE.</span>
              </div>
            ) : (
              servers.map((server) => {
                const active = enabled.has(server.config.id);
                const pending = Array.from(pendingKeys).some((key) =>
                  key.endsWith(`:${server.config.id}`),
                );
                const connectionState = !active
                  ? "disabled"
                  : server.state === "connected"
                    ? "connected"
                    : server.state === "error" || !!server.error
                      ? "failed"
                      : "connecting";
                const connectionLabel =
                  connectionState === "connected"
                    ? `${server.tools.length} tools`
                    : connectionState === "failed"
                      ? "⚠️无法连接"
                      : connectionState === "connecting"
                        ? "连接中…"
                        : "";
                const connectionTitle =
                  connectionState === "failed" && server.error
                    ? `${connectionLabel}：${server.error}`
                    : connectionLabel || "未启用";
                return (
                  <article
                    className={`mcp-server ${connectionState} ${active && !deleteMode ? "selected" : ""} ${deleteMode ? "delete-selecting" : ""} ${selectedDeleteServerId === server.config.id ? "delete-selected" : ""}`}
                    key={server.config.id}
                  >
                    <div className="mcp-server-main">
                      <strong>{server.config.name}</strong>
                      <button
                        type="button"
                        role="switch"
                        aria-checked={active}
                        aria-label={`${active ? "Disable" : "Enable"} ${server.config.name} for this session`}
                        className={`mcp-session-toggle ${connectionState}`}
                        title={
                          deleteMode
                            ? "MCP controls are disabled while selecting a server to delete"
                            : `${connectionTitle} · click to ${active ? "disable" : "enable"}`
                        }
                        disabled={!session || pending || deleteMode}
                        onClick={() =>
                          session &&
                          onCommand(`toggle:${server.config.id}`, {
                            type: "mcp_session_toggle",
                            session_id: session.session_id,
                            server_id: server.config.id,
                            enabled: !active,
                          })
                        }
                      >
                        <span
                          className="mcp-session-toggle-thumb"
                          aria-hidden="true"
                        />
                      </button>
                    </div>
                    {active && (
                      <div
                        className={`mcp-server-meta ${connectionState}`}
                        title={
                          connectionState === "failed"
                            ? (server.error ?? undefined)
                            : undefined
                        }
                      >
                        <span>{connectionLabel}</span>
                      </div>
                    )}
                    {deleteMode ? (
                      <button
                        type="button"
                        className={`mcp-delete-select ${selectedDeleteServerId === server.config.id ? "selected" : ""}`}
                        title={`选择删除 ${server.config.name}`}
                        aria-label={`选择删除 ${server.config.name}`}
                        aria-pressed={
                          selectedDeleteServerId === server.config.id
                        }
                        disabled={pending}
                        onClick={() =>
                          setSelectedDeleteServerId((current) =>
                            current === server.config.id
                              ? ""
                              : server.config.id,
                          )
                        }
                      >
                        {selectedDeleteServerId === server.config.id ? (
                          <Check size={13} />
                        ) : null}
                      </button>
                    ) : (
                      <div className="mcp-server-actions">
                        <button
                          type="button"
                          title="Reconnect and refresh tools"
                          aria-label={`Reconnect ${server.config.name}`}
                          disabled={!session || pending}
                          onClick={() =>
                            session &&
                            onCommand(`reconnect:${server.config.id}`, {
                              type: "mcp_server_reconnect",
                              session_id: session.session_id,
                              server_id: server.config.id,
                            })
                          }
                        >
                          <RefreshCw size={13} />
                        </button>
                        <button
                          type="button"
                          title="Edit server"
                          aria-label={`Edit ${server.config.name}`}
                          disabled={pending}
                          onClick={() => setEditing(server.config)}
                        >
                          <Pencil size={13} />
                        </button>
                      </div>
                    )}
                  </article>
                );
              })
            )}
          </div>
          {!deleteMode && (
            <button
              type="button"
              className="mcp-add"
              disabled={!session}
              onClick={startNew}
            >
              <Plus size={15} /> Add MCP server
            </button>
          )}
        </>
      )}
    </section>
  );
}

type StructuredRow = { id: string; key: string; value: string };
type ListRow = { id: string; value: string };

function structuredRows(value: Record<string, string>): StructuredRow[] {
  return Object.entries(value).map(([key, item]) => ({
    id: clientId(),
    key,
    value: item,
  }));
}

function structuredRecord(rows: StructuredRow[]): Record<string, string> {
  return Object.fromEntries(
    rows.map((row) => [row.key.trim(), row.value]).filter(([key]) => key),
  );
}

const RESERVED_REQUEST_FIELDS = new Set([
  "model",
  "messages",
  "max_tokens",
  "max_output_tokens",
  "instructions",
  "input",
  "tools",
  "tool_choice",
  "parallel_tool_calls",
  "stream",
  "stream_options",
  "response_format",
  "enable_thinking",
  "reasoning_effort",
  "system",
]);

function requestFieldRows(value: Record<string, unknown>): StructuredRow[] {
  return Object.entries(value).map(([key, item]) => ({
    id: clientId(),
    key,
    value: item === "****" ? "****" : JSON.stringify(item),
  }));
}

function parseRequestFieldRows(rows: StructuredRow[]): {
  value: Record<string, unknown>;
  error: string;
} {
  const value: Record<string, unknown> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (!key) continue;
    if (RESERVED_REQUEST_FIELDS.has(key.toLowerCase()))
      return { value: {}, error: `${key} 由 Timem 管理，不能覆盖。` };
    if (!row.value.trim())
      return { value: {}, error: `${key} 的 Value 不能为空。` };
    try {
      value[key] = JSON.parse(row.value);
    } catch {
      return {
        value: {},
        error: `${key} 的 Value 必须是合法 JSON。字符串请使用双引号，例如 \"fast\"。`,
      };
    }
  }
  return { value, error: "" };
}

function hasDuplicateStructuredKeys(rows: StructuredRow[]) {
  const keys = rows.map((row) => row.key.trim().toLowerCase()).filter(Boolean);
  return new Set(keys).size !== keys.length;
}

function StructuredKeyValueEditor({
  label,
  description,
  rows,
  keyLabel = "Key",
  valueLabel = "Value",
  keyPlaceholder,
  valuePlaceholder,
  addLabel,
  showValues = true,
  revealAction,
  onChange,
}: {
  label: string;
  description?: string;
  rows: StructuredRow[];
  keyLabel?: string;
  valueLabel?: string;
  keyPlaceholder: string;
  valuePlaceholder: string;
  addLabel: string;
  showValues?: boolean;
  revealAction?: ReactNode;
  onChange: (rows: StructuredRow[]) => void;
}) {
  const duplicateKeys = hasDuplicateStructuredKeys(rows);
  const update = (id: string, patch: Partial<StructuredRow>) =>
    onChange(rows.map((row) => (row.id === id ? { ...row, ...patch } : row)));
  return (
    <section className="structured-field" aria-label={label}>
      <div className="structured-field-heading">
        <div>
          <strong>{label}</strong>
          {description && <small>{description}</small>}
        </div>
        {revealAction}
      </div>
      <div className="structured-field-list">
        {rows.map((row) => (
          <div className="structured-field-row" key={row.id}>
            <label>
              <span>{keyLabel}</span>
              <input
                value={row.key}
                placeholder={keyPlaceholder}
                spellCheck={false}
                onChange={(event) =>
                  update(row.id, { key: event.target.value })
                }
              />
            </label>
            <label>
              <span>{valueLabel}</span>
              <input
                type={showValues ? "text" : "password"}
                value={row.value}
                placeholder={valuePlaceholder}
                autoComplete="off"
                spellCheck={false}
                onChange={(event) =>
                  update(row.id, { value: event.target.value })
                }
              />
            </label>
            <button
              type="button"
              className="structured-field-delete"
              title={`删除这一项 ${label}`}
              aria-label={`删除 ${label}`}
              onClick={() =>
                onChange(rows.filter((item) => item.id !== row.id))
              }
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </div>
      {duplicateKeys && (
        <small className="structured-field-error" role="alert">
          {keyLabel} 不能重复。
        </small>
      )}
      <button
        type="button"
        className="structured-field-add"
        onClick={() =>
          onChange([...rows, { id: clientId(), key: "", value: "" }])
        }
      >
        <Plus size={14} /> {addLabel}
      </button>
    </section>
  );
}

function StructuredListEditor({
  label,
  description,
  rows,
  placeholder,
  addLabel,
  onChange,
}: {
  label: string;
  description?: string;
  rows: ListRow[];
  placeholder: string;
  addLabel: string;
  onChange: (rows: ListRow[]) => void;
}) {
  const update = (id: string, value: string) =>
    onChange(rows.map((row) => (row.id === id ? { ...row, value } : row)));
  return (
    <section
      className="structured-field structured-list-field"
      aria-label={label}
    >
      <div className="structured-field-heading">
        <div>
          <strong>{label}</strong>
          {description && <small>{description}</small>}
        </div>
      </div>
      <div className="structured-field-list">
        {rows.map((row, index) => (
          <div className="structured-list-row" key={row.id}>
            <span className="structured-list-index" aria-hidden="true">
              {index + 1}
            </span>
            <input
              aria-label={`${label} ${index + 1}`}
              value={row.value}
              placeholder={placeholder}
              spellCheck={false}
              onChange={(event) => update(row.id, event.target.value)}
            />
            <button
              type="button"
              className="structured-field-delete"
              title={`删除第 ${index + 1} 项`}
              aria-label={`删除 ${label} ${index + 1}`}
              onClick={() =>
                onChange(rows.filter((item) => item.id !== row.id))
              }
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
      </div>
      <button
        type="button"
        className="structured-field-add"
        onClick={() => onChange([...rows, { id: clientId(), value: "" }])}
      >
        <Plus size={14} /> {addLabel}
      </button>
    </section>
  );
}

function McpEditor({
  config,
  pending,
  revealPending,
  revealedSecrets,
  onReveal,
  onCancel,
  onSave,
}: {
  config: McpServerConfig;
  pending: boolean;
  revealPending: boolean;
  revealedSecrets?: Record<string, string>;
  onReveal: () => void;
  onCancel: () => void;
  onSave: (config: McpServerConfig) => void;
}) {
  const [draft, setDraft] = useState(config);
  const [transportType, setTransportType] = useState<McpTransport["type"]>(
    config.transport.type,
  );
  const [transportDrafts, setTransportDrafts] = useState(() =>
    createMcpTransportDrafts(config.transport),
  );
  const [argumentRows, setArgumentRows] = useState<ListRow[]>(() =>
    createMcpTransportDrafts(config.transport).stdio.args.map((value) => ({
      id: clientId(),
      value,
    })),
  );
  const [envRows, setEnvRows] = useState<StructuredRow[]>(() =>
    structuredRows(createMcpTransportDrafts(config.transport).stdio.env),
  );
  const [httpHeaderRows, setHttpHeaderRows] = useState<StructuredRow[]>(() =>
    structuredRows(
      createMcpTransportDrafts(config.transport).streamable_http.headers,
    ),
  );
  const [sseHeaderRows, setSseHeaderRows] = useState<StructuredRow[]>(() =>
    structuredRows(createMcpTransportDrafts(config.transport).sse.headers),
  );
  const [showSecrets, setShowSecrets] = useState(false);
  const baseTransport = transportDrafts[transportType];
  const transport: McpTransport =
    baseTransport.type === "stdio"
      ? {
          ...baseTransport,
          args: argumentRows
            .map((row) => row.value)
            .filter((value) => value.length > 0),
          env: structuredRecord(envRows),
        }
      : baseTransport.type === "streamable_http"
        ? { ...baseTransport, headers: structuredRecord(httpHeaderRows) }
        : { ...baseTransport, headers: structuredRecord(sseHeaderRows) };
  const activeMapRows =
    transport.type === "stdio"
      ? envRows
      : transport.type === "streamable_http"
        ? httpHeaderRows
        : sseHeaderRows;
  const valid =
    draft.name.trim() &&
    (transport.type === "stdio"
      ? transport.command.trim()
      : transport.url.trim()) &&
    !hasDuplicateStructuredKeys(activeMapRows);
  useEffect(() => {
    if (!revealedSecrets) return;
    setEnvRows((rows) =>
      structuredRows(mergeMcpSecrets(structuredRecord(rows), revealedSecrets)),
    );
    setHttpHeaderRows((rows) =>
      structuredRows(mergeMcpSecrets(structuredRecord(rows), revealedSecrets)),
    );
    setSseHeaderRows((rows) =>
      structuredRows(mergeMcpSecrets(structuredRecord(rows), revealedSecrets)),
    );
    setShowSecrets(true);
  }, [revealedSecrets]);
  const toggleSecrets = () => {
    if (showSecrets) setShowSecrets(false);
    else if (revealedSecrets) setShowSecrets(true);
    else onReveal();
  };
  const revealAction = draft.id ? (
    <button
      type="button"
      className="structured-field-visibility"
      title={showSecrets ? "Hide values" : "Reveal sensitive values"}
      aria-label={showSecrets ? "Hide values" : "Reveal sensitive values"}
      disabled={revealPending}
      onClick={toggleSecrets}
    >
      {showSecrets ? <EyeOff size={14} /> : <Eye size={14} />}
    </button>
  ) : undefined;
  return (
    <form
      className="mcp-editor"
      onSubmit={(event) => {
        event.preventDefault();
        if (!valid) return;
        const id =
          draft.id ||
          draft.name
            .trim()
            .toLowerCase()
            .replace(/[^a-z0-9_-]+/g, "_")
            .replace(/^_+|_+$/g, "") ||
          `server_${clientId()}`;
        onSave({ ...draft, id, transport });
      }}
    >
      <fieldset className="mcp-transport">
        <legend>Transport</legend>
        <div>
          {(["stdio", "streamable_http", "sse"] as const).map((type) => (
            <button
              type="button"
              aria-pressed={transportType === type}
              className={transportType === type ? "active" : ""}
              key={type}
              onClick={() => setTransportType(type)}
            >
              {mcpTransportLabel({ type } as McpTransport)}
            </button>
          ))}
        </div>
        <p>
          {transportType === "stdio"
            ? "Launch a local MCP process and communicate over stdin/stdout."
            : transportType === "streamable_http"
              ? "Recommended remote transport. One MCP endpoint may return JSON or an SSE stream."
              : "Compatibility mode for older servers with an SSE endpoint and a separate POST endpoint."}
        </p>
      </fieldset>
      <label>
        Name
        <input
          autoFocus
          value={draft.name}
          placeholder="GitHub"
          onChange={(event) => setDraft({ ...draft, name: event.target.value })}
        />
      </label>
      {draft.id && (
        <label>
          Server ID
          <input value={draft.id} disabled />
        </label>
      )}
      {transport.type === "stdio" ? (
        <>
          <label>
            Command
            <input
              value={transport.command}
              placeholder="npx"
              onChange={(event) =>
                setTransportDrafts((current) => ({
                  ...current,
                  stdio: { ...current.stdio, command: event.target.value },
                }))
              }
            />
          </label>
          <StructuredListEditor
            label="Arguments"
            description="每个命令参数单独一项，不需要手动编排多行格式。"
            rows={argumentRows}
            placeholder="例如：-y 或 @modelcontextprotocol/server-filesystem"
            addLabel="添加参数"
            onChange={setArgumentRows}
          />
          <StructuredKeyValueEditor
            label="Environment"
            description="环境变量使用独立的 Key / Value 输入。"
            rows={envRows}
            keyPlaceholder="例如：GITHUB_TOKEN"
            valuePlaceholder="Environment value"
            addLabel="添加环境变量"
            showValues={showSecrets || !draft.id}
            revealAction={revealAction}
            onChange={setEnvRows}
          />
        </>
      ) : (
        <>
          <label>
            {transport.type === "sse" ? "SSE URL" : "MCP endpoint URL"}
            <input
              value={transport.url}
              placeholder={
                transport.type === "sse"
                  ? "https://example.com/sse"
                  : "https://example.com/mcp"
              }
              onChange={(event) =>
                setTransportDrafts((current) => ({
                  ...current,
                  [transport.type]: {
                    ...current[transport.type],
                    url: event.target.value,
                  },
                }))
              }
            />
          </label>
          <StructuredKeyValueEditor
            label="Headers"
            description={`每个 Header 单独填写；Value 中可使用 ${"${NAME}"} 引用环境变量。`}
            rows={
              transport.type === "streamable_http"
                ? httpHeaderRows
                : sseHeaderRows
            }
            keyLabel="Name"
            keyPlaceholder="例如：Authorization"
            valuePlaceholder={`例如：Bearer ${"${MCP_TOKEN}"}`}
            addLabel="添加 Header"
            showValues={showSecrets || !draft.id}
            revealAction={revealAction}
            onChange={
              transport.type === "streamable_http"
                ? setHttpHeaderRows
                : setSseHeaderRows
            }
          />
        </>
      )}
      <label>
        Request timeout (ms)
        <input
          type="number"
          min={1}
          value={draft.request_timeout_ms}
          onChange={(event) =>
            setDraft({
              ...draft,
              request_timeout_ms: Math.max(1, Number(event.target.value) || 1),
            })
          }
        />
      </label>
      <div className="mcp-editor-actions">
        <button
          type="button"
          className="secondary"
          disabled={pending}
          onClick={onCancel}
        >
          Cancel
        </button>
        <button
          type="submit"
          className={`primary ${pending ? "pending" : ""}`}
          disabled={!valid || pending}
        >
          {pending ? <LoaderCircle size={14} /> : <Plug size={14} />} Save and
          connect
        </button>
      </div>
    </form>
  );
}

type SettingsSection = "appearance" | "endpoints" | "memory" | "toolgen";

type SettingsCenterProps = {
  panelRef: MutableRefObject<HTMLElement | null>;
  section: SettingsSection;
  onSectionChange: (section: SettingsSection) => void;
  appearance: Appearance;
  onAppearanceChange: (appearance: Appearance) => void;
  toolGenEnabled: boolean;
  toolGenToggleDisabled: boolean;
  onToolGenEnabledChange: (enabled: boolean) => void;
  memPath: string;
  connected: boolean;
  connectionLabel: string;
  retentionDays: 1 | 5 | 10 | null;
  temporaryCapacityBytes: number | null;
  conversationCapacityBytes: number | null;
  favoriteCapacity: ChatLibraryCapacity;
  retentionPending: boolean;
  conversationCapacityPending: boolean;
  favoriteCapacityPending: boolean;
  switchPending: boolean;
  temporaryItems: MemTemporaryItem[];
  temporaryItemsLoading: boolean;
  temporaryItemsDeleting: boolean;
  temporaryItemsError: string;
  endpoints: ModelEndpoint[];
  endpointEditor: ModelEndpoint | "new" | null;
  revealedEndpointApiKeys: Record<string, string>;
  revealedEndpointHeaders: Record<string, Record<string, string>>;
  revealedEndpointRequestFields: Record<string, Record<string, unknown>>;
  onClose: () => void;
  onSaveTemporaryPolicy: (
    days: 1 | 5 | 10 | null,
    maxBytes: number | null,
  ) => void;
  onSaveConversationCapacity: (maxBytes: number | null) => void;
  onSaveFavoriteCapacity: (maxBytes: number | null) => void;
  onSwitchMemory: (path: string) => void;
  onRefreshTemporaryItems: () => void;
  onDeleteTemporaryItems: (ids: string[]) => void;
  onEditEndpoint: (endpoint: ModelEndpoint | "new" | null) => void;
  onDeleteEndpoint: (endpoint: ModelEndpoint) => void;
  onRevealEndpoint: (endpointId: string) => void;
  onSaveEndpoint: (endpoint: ModelEndpointDraft) => void;
};

const SettingsCenter = memo(function SettingsCenter(
  props: SettingsCenterProps,
) {
  const {
    panelRef,
    section,
    onSectionChange,
    appearance,
    onAppearanceChange,
    toolGenEnabled,
    toolGenToggleDisabled,
    onToolGenEnabledChange,
    memPath,
    connected,
    connectionLabel,
    retentionDays,
    temporaryCapacityBytes,
    conversationCapacityBytes,
    favoriteCapacity,
    retentionPending,
    conversationCapacityPending,
    favoriteCapacityPending,
    switchPending,
    temporaryItems,
    temporaryItemsLoading,
    temporaryItemsDeleting,
    temporaryItemsError,
    endpoints,
    endpointEditor,
    revealedEndpointApiKeys,
    revealedEndpointHeaders,
    revealedEndpointRequestFields,
    onClose,
    onSaveTemporaryPolicy,
    onSaveConversationCapacity,
    onSaveFavoriteCapacity,
    onSwitchMemory,
    onRefreshTemporaryItems,
    onDeleteTemporaryItems,
    onEditEndpoint,
    onDeleteEndpoint,
    onRevealEndpoint,
    onSaveEndpoint,
  } = props;
  const [days, setDays] = useState<1 | 5 | 10 | null>(retentionDays);
  const [temporaryCapacity, setTemporaryCapacity] = useState<number | null>(
    temporaryCapacityBytes,
  );
  const [conversationCapacity, setConversationCapacity] = useState<
    number | null
  >(conversationCapacityBytes);
  const [favoriteCapacityLimit, setFavoriteCapacityLimit] = useState<
    number | null
  >(favoriteCapacity.limit_bytes ?? null);
  const [path, setPath] = useState(memPath);
  const [memoryPage, setMemoryPage] = useState<"overview" | "switch">(
    "overview",
  );
  const [temporaryDeleteMode, setTemporaryDeleteMode] = useState(false);
  const [selectedTemporaryIds, setSelectedTemporaryIds] = useState<Set<string>>(
    () => new Set(),
  );
  const busy =
    retentionPending ||
    conversationCapacityPending ||
    favoriteCapacityPending ||
    switchPending ||
    temporaryItemsDeleting;
  const cleanedPath = path.trim();
  const pathUnchanged = cleanedPath === memPath;
  const deletableTemporaryItems = useMemo(
    () => temporaryItems.filter((item) => item.deletable !== false),
    [temporaryItems],
  );
  const selectedTemporaryBytes = useMemo(
    () =>
      deletableTemporaryItems
        .filter((item) => selectedTemporaryIds.has(item.id))
        .reduce((total, item) => total + item.bytes, 0),
    [deletableTemporaryItems, selectedTemporaryIds],
  );
  const updateAppearance = <K extends keyof Appearance>(
    key: K,
    value: Appearance[K],
  ) => onAppearanceChange({ ...appearance, [key]: value });
  useEffect(() => setDays(retentionDays), [retentionDays]);
  useEffect(
    () => setTemporaryCapacity(temporaryCapacityBytes),
    [temporaryCapacityBytes],
  );
  useEffect(
    () => setConversationCapacity(conversationCapacityBytes),
    [conversationCapacityBytes],
  );
  useEffect(
    () => setFavoriteCapacityLimit(favoriteCapacity.limit_bytes ?? null),
    [favoriteCapacity.limit_bytes],
  );
  useEffect(() => {
    setPath(memPath);
    setMemoryPage("overview");
  }, [memPath]);
  useEffect(() => {
    const availableIds = new Set(
      deletableTemporaryItems.map((item) => item.id),
    );
    setSelectedTemporaryIds((current) => {
      if (
        current.size === 0 ||
        Array.from(current).every((id) => availableIds.has(id))
      )
        return current;
      return new Set(Array.from(current).filter((id) => availableIds.has(id)));
    });
    if (temporaryItemsDeleting) return;
    if (temporaryItems.length === 0) setTemporaryDeleteMode(false);
  }, [deletableTemporaryItems, temporaryItems.length, temporaryItemsDeleting]);
  const cancelTemporaryDelete = () => {
    setTemporaryDeleteMode(false);
    setSelectedTemporaryIds(new Set());
  };
  const toggleTemporaryItem = (id: string) =>
    setSelectedTemporaryIds((current) => {
      const next = new Set(current);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  const closeIfIdle = () => {
    if (!busy) onClose();
  };
  const selectSettingsSection = (next: SettingsSection) => {
    if (switchPending) return;
    if (next === "memory") setMemoryPage("overview");
    onSectionChange(next);
  };
  return createPortal(
    <div
      className="settings-center-backdrop"
      role="presentation"
      aria-label="Dismiss settings"
      onClick={closeIfIdle}
    >
      <section
        id="settings-center"
        ref={panelRef}
        className="settings-center"
        role="dialog"
        aria-modal="true"
        aria-labelledby="settings-center-title"
        tabIndex={-1}
        onClick={(event) => event.stopPropagation()}
      >
        <header className="settings-center-header">
          <div>
            <span className="eyebrow">SETTINGS</span>
            <h2 id="settings-center-title">Settings</h2>
            <div
              className="settings-runtime-status"
              role="status"
              aria-live="polite"
              title={connectionLabel}
            >
              <span
                className={`connection ${connected ? "online" : "offline"}`}
              />
              <span>{connectionLabel}</span>
            </div>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close settings"
            aria-label="Close settings"
            disabled={busy}
            onClick={closeIfIdle}
          >
            <X size={17} />
          </button>
        </header>
        <div className="settings-center-layout">
          <nav className="settings-center-nav" aria-label="Settings categories">
            <button
              type="button"
              className={section === "appearance" ? "active" : ""}
              aria-current={section === "appearance" ? "page" : undefined}
              disabled={switchPending}
              onClick={() => selectSettingsSection("appearance")}
            >
              <Palette size={16} />
              <span>
                <strong>Appearance</strong>
              </span>
            </button>
            <button
              type="button"
              className={section === "endpoints" ? "active" : ""}
              aria-current={section === "endpoints" ? "page" : undefined}
              disabled={switchPending}
              onClick={() => selectSettingsSection("endpoints")}
            >
              <Sparkles size={16} />
              <span>
                <strong>Model Endpoints</strong>
              </span>
            </button>
            <button
              type="button"
              className={section === "memory" ? "active" : ""}
              aria-current={section === "memory" ? "page" : undefined}
              disabled={switchPending}
              onClick={() => selectSettingsSection("memory")}
            >
              <Database size={16} />
              <span>
                <strong>Memory</strong>
              </span>
            </button>
            <button
              type="button"
              className={section === "toolgen" ? "active" : ""}
              aria-current={section === "toolgen" ? "page" : undefined}
              disabled={switchPending}
              onClick={() => selectSettingsSection("toolgen")}
            >
              <Wrench size={16} />
              <span>
                <strong>ToolGen</strong>
              </span>
            </button>
          </nav>
          <div className="settings-center-content">
            {section === "appearance" && (
              <section
                className="settings-pane appearance-settings-pane"
                aria-labelledby="appearance-settings-title"
              >
                <div className="settings-pane-heading">
                  <h3 id="appearance-settings-title">Appearance</h3>
                  <Palette size={19} aria-hidden="true" />
                </div>
                <fieldset>
                  <legend>Theme</legend>
                  <div className="segmented-control">
                    {(["dark", "light"] as const).map((theme) => (
                      <button
                        type="button"
                        title={`Use ${theme} theme`}
                        className={appearance.theme === theme ? "active" : ""}
                        aria-pressed={appearance.theme === theme}
                        key={theme}
                        onClick={() => updateAppearance("theme", theme)}
                      >
                        {theme === "dark" ? "Dark" : "Light"}
                      </button>
                    ))}
                  </div>
                </fieldset>
                <section
                  className="appearance-role-fonts"
                  aria-labelledby="appearance-user-fonts-title"
                >
                  <h4 id="appearance-user-fonts-title">User</h4>
                  <div className="appearance-font-selects">
                    <label>
                      <span>汉语字体</span>
                      <select
                        value={appearance.userChineseFont}
                        aria-label="User Chinese font"
                        onChange={(event) =>
                          updateAppearance(
                            "userChineseFont",
                            event.target.value as Appearance["userChineseFont"],
                          )
                        }
                      >
                        <option value="system">系统</option>
                        <option value="heiti">黑体</option>
                        <option value="kaiti">楷体</option>
                        <option value="songti">宋体</option>
                      </select>
                    </label>
                    <label>
                      <span>其他语言字体</span>
                      <select
                        value={appearance.userFont}
                        aria-label="User other language font"
                        onChange={(event) =>
                          updateAppearance(
                            "userFont",
                            event.target.value as Appearance["userFont"],
                          )
                        }
                      >
                        <option value="system">System</option>
                        <option value="serif">Serif</option>
                        <option value="mono">Mono</option>
                      </select>
                    </label>
                  </div>
                  <label className="appearance-bold-option">
                    <input
                      type="checkbox"
                      checked={appearance.userBold}
                      onChange={(event) =>
                        updateAppearance("userBold", event.target.checked)
                      }
                    />
                    <span className="appearance-checkbox" aria-hidden="true">
                      <Check size={12} strokeWidth={3} />
                    </span>
                    <span>粗体</span>
                  </label>
                </section>
                <section
                  className="appearance-role-fonts"
                  aria-labelledby="appearance-agent-fonts-title"
                >
                  <h4 id="appearance-agent-fonts-title">Agent</h4>
                  <div className="appearance-font-selects">
                    <label>
                      <span>汉语字体</span>
                      <select
                        value={appearance.agentChineseFont}
                        aria-label="Agent Chinese font"
                        onChange={(event) =>
                          updateAppearance(
                            "agentChineseFont",
                            event.target
                              .value as Appearance["agentChineseFont"],
                          )
                        }
                      >
                        <option value="system">系统</option>
                        <option value="heiti">黑体</option>
                        <option value="kaiti">楷体</option>
                        <option value="songti">宋体</option>
                      </select>
                    </label>
                    <label>
                      <span>其他语言字体</span>
                      <select
                        value={appearance.agentFont}
                        aria-label="Agent other language font"
                        onChange={(event) =>
                          updateAppearance(
                            "agentFont",
                            event.target.value as Appearance["agentFont"],
                          )
                        }
                      >
                        <option value="system">System</option>
                        <option value="serif">Serif</option>
                        <option value="mono">Mono</option>
                      </select>
                    </label>
                  </div>
                  <label className="appearance-bold-option">
                    <input
                      type="checkbox"
                      checked={appearance.agentBold}
                      onChange={(event) =>
                        updateAppearance("agentBold", event.target.checked)
                      }
                    />
                    <span className="appearance-checkbox" aria-hidden="true">
                      <Check size={12} strokeWidth={3} />
                    </span>
                    <span>粗体</span>
                  </label>
                </section>
                <fieldset>
                  <legend>Text size</legend>
                  <div className="segmented-control text-size-control">
                    {(["small", "medium", "large"] as const).map((size) => (
                      <button
                        type="button"
                        title={`Use ${size === "medium" ? "default" : size} text size`}
                        className={appearance.textSize === size ? "active" : ""}
                        aria-pressed={appearance.textSize === size}
                        key={size}
                        onClick={() => updateAppearance("textSize", size)}
                      >
                        {size === "small"
                          ? "Small"
                          : size === "medium"
                            ? "Default"
                            : "Large"}
                      </button>
                    ))}
                  </div>
                </fieldset>
              </section>
            )}
            {section === "endpoints" && (
              <EndpointSettingsPane
                endpoints={endpoints}
                endpointEditor={endpointEditor}
                revealedEndpointApiKeys={revealedEndpointApiKeys}
                revealedEndpointHeaders={revealedEndpointHeaders}
                revealedEndpointRequestFields={revealedEndpointRequestFields}
                onEdit={onEditEndpoint}
                onDelete={onDeleteEndpoint}
                onReveal={onRevealEndpoint}
                onSave={onSaveEndpoint}
              />
            )}
            {section === "toolgen" && (
              <section
                className="settings-pane toolgen-settings-pane"
                aria-labelledby="toolgen-settings-title"
              >
                <div className="settings-pane-heading">
                  <h3 id="toolgen-settings-title">ToolGen</h3>
                  <Wrench size={19} aria-hidden="true" />
                </div>
                <section className="settings-group toolgen-beta-card">
                  <div className="settings-group-heading">
                    <div>
                      <strong>Enable ToolGen</strong>
                      <p>
                        When enabled, completed answers show a ToolGen action
                        and can start the generation workflow. This preference
                        is stored only in this browser.
                      </p>
                    </div>
                    <button
                      type="button"
                      role="switch"
                      className="settings-feature-switch"
                      aria-checked={toolGenEnabled}
                      aria-label="Enable ToolGen Beta"
                      disabled={toolGenToggleDisabled}
                      onClick={() => onToolGenEnabledChange(!toolGenEnabled)}
                    >
                      <span className="settings-feature-switch-thumb" />
                    </button>
                  </div>
                  <div
                    className="toolgen-beta-status"
                    role="status"
                    aria-live="polite"
                  >
                    <span className={toolGenEnabled ? "enabled" : "disabled"} />
                    <strong>
                      {toolGenEnabled ? "Enabled" : "Disabled by default"}
                    </strong>
                    <small>
                      {toolGenToggleDisabled
                        ? "A ToolGen task is active; wait for it to finish before changing this setting."
                        : toolGenEnabled
                          ? "ToolGen actions and generation UI are available."
                          : "ToolGen actions and generation UI are hidden."}
                    </small>
                  </div>
                </section>
                <section className="toolgen-beta-note">
                  <TriangleAlert size={16} />
                  <div>
                    <strong>Beta capability</strong>
                    <p>
                      Generated tools should be reviewed before relying on them
                      in important workflows.
                    </p>
                  </div>
                </section>
              </section>
            )}
            {section === "memory" && memoryPage === "overview" && (
              <section
                className="settings-pane memory-settings-pane"
                aria-labelledby="memory-settings-title"
              >
                <div className="settings-pane-heading">
                  <h3 id="memory-settings-title">Memory</h3>
                  <Database size={19} aria-hidden="true" />
                </div>
                <section
                  className="memory-identity-card"
                  aria-label="Current MEM"
                >
                  <div className="memory-identity-icon" aria-hidden="true">
                    <Database size={20} />
                  </div>
                  <div className="memory-identity-copy">
                    <span>Current MEM</span>
                    <strong>Active workspace</strong>
                    <code title={memPath}>{memPath || "…"}</code>
                  </div>
                  <button
                    type="button"
                    className="memory-switch-entry"
                    disabled={busy}
                    onClick={() => {
                      setPath(memPath);
                      setMemoryPage("switch");
                    }}
                  >
                    <span>Switch MEM</span>
                    <ChevronRight size={15} />
                  </button>
                </section>
                <div className="memory-overview-label">
                  <span>Temporary data</span>
                  <small>Policies and deletable artifacts for this MEM</small>
                </div>
                <section className="settings-group settings-retention-card">
                  <div className="settings-group-heading">
                    <div>
                      <strong>Temporary-data policy</strong>
                      <p>
                        Age removes expired transient history, finished jobs,
                        and API audit events. Capacity rolls over deletable
                        temporary files and finished jobs as complete items.
                        Limited tiers include a 4 MiB safe-write reserve.
                      </p>
                    </div>
                  </div>
                  <div className="settings-capacity-grid">
                    <label className="settings-field">
                      <span>Retention period</span>
                      <select
                        value={days === null ? "unlimited" : String(days)}
                        disabled={retentionPending || switchPending}
                        onChange={(event) =>
                          setDays(
                            event.target.value === "unlimited"
                              ? null
                              : (Number(event.target.value) as 1 | 5 | 10),
                          )
                        }
                      >
                        <option value="1">Most recent day</option>
                        <option value="5">Most recent 5 days</option>
                        <option value="10">Most recent 10 days</option>
                        <option value="unlimited">Unlimited</option>
                      </select>
                    </label>
                    <label className="settings-field">
                      <span>Maximum capacity</span>
                      <select
                        value={
                          temporaryCapacity === null
                            ? "unlimited"
                            : String(temporaryCapacity)
                        }
                        disabled={retentionPending || switchPending}
                        onChange={(event) =>
                          setTemporaryCapacity(
                            event.target.value === "unlimited"
                              ? null
                              : Number(event.target.value),
                          )
                        }
                      >
                        <option value={128 * 1024 * 1024}>128 MB</option>
                        <option value={256 * 1024 * 1024}>256 MB</option>
                        <option value={512 * 1024 * 1024}>512 MB</option>
                        <option value={1024 * 1024 * 1024}>1 GB</option>
                        <option value={5 * 1024 * 1024 * 1024}>5 GB</option>
                        <option value="unlimited">Unlimited</option>
                      </select>
                    </label>
                  </div>
                  <div className="settings-group-actions">
                    <span
                      className="settings-status"
                      role="status"
                      aria-live="polite"
                    >
                      {retentionPending
                        ? "Saving and applying temporary-data policy…"
                        : ""}
                    </span>
                    <button
                      type="button"
                      className={`primary compact ${retentionPending ? "sending" : ""}`}
                      disabled={
                        busy ||
                        (days === retentionDays &&
                          temporaryCapacity === temporaryCapacityBytes)
                      }
                      onClick={() =>
                        onSaveTemporaryPolicy(days, temporaryCapacity)
                      }
                    >
                      {retentionPending && <LoaderCircle size={14} />}{" "}
                      {retentionPending ? "Saving…" : "Save policy"}
                    </button>
                  </div>
                </section>
                <div className="memory-overview-label">
                  <span>Conversation data</span>
                  <small>Complete Turns are the eviction boundary</small>
                </div>
                <section className="settings-group settings-capacity-card">
                  <div className="settings-group-heading">
                    <div>
                      <strong>Conversation capacity</strong>
                      <p>
                        When the limit is exceeded, Timem removes the oldest
                        complete Turns. Running and queued Sessions remain
                        protected. Limited tiers reserve 4 MiB for safe rolling
                        writes.
                      </p>
                    </div>
                  </div>
                  <label className="settings-field">
                    <span>Maximum capacity</span>
                    <select
                      value={
                        conversationCapacity === null
                          ? "unlimited"
                          : String(conversationCapacity)
                      }
                      disabled={conversationCapacityPending || switchPending}
                      onChange={(event) =>
                        setConversationCapacity(
                          event.target.value === "unlimited"
                            ? null
                            : Number(event.target.value),
                        )
                      }
                    >
                      <option value={128 * 1024 * 1024}>128 MB</option>
                      <option value={512 * 1024 * 1024}>512 MB</option>
                      <option value={1024 * 1024 * 1024}>1 GB</option>
                      <option value={5 * 1024 * 1024 * 1024}>5 GB</option>
                      <option value={20 * 1024 * 1024 * 1024}>20 GB</option>
                      <option value="unlimited">Unlimited</option>
                    </select>
                  </label>
                  <div className="settings-group-actions">
                    <span
                      className="settings-status"
                      role="status"
                      aria-live="polite"
                    >
                      {conversationCapacityPending
                        ? "Saving and applying conversation capacity…"
                        : ""}
                    </span>
                    <button
                      type="button"
                      className={`primary compact ${conversationCapacityPending ? "sending" : ""}`}
                      disabled={
                        busy ||
                        conversationCapacity === conversationCapacityBytes
                      }
                      onClick={() =>
                        onSaveConversationCapacity(conversationCapacity)
                      }
                    >
                      {conversationCapacityPending && (
                        <LoaderCircle size={14} />
                      )}{" "}
                      {conversationCapacityPending
                        ? "Saving…"
                        : "Save capacity"}
                    </button>
                  </div>
                </section>
                <div className="memory-overview-label">
                  <span>Favorites</span>
                  <small>Stored in physical 4 MiB slices</small>
                </div>
                <section className="settings-group settings-capacity-card">
                  <div className="settings-group-heading">
                    <div>
                      <strong>Favorites capacity</strong>
                      <p>
                        New favorites roll over the oldest complete favorites
                        when space is full; favorite content is never truncated.
                        Limited tiers reserve 4 MiB for safe rolling writes.
                      </p>
                    </div>
                  </div>
                  <label className="settings-field">
                    <span>Maximum capacity</span>
                    <select
                      value={
                        favoriteCapacityLimit === null
                          ? "unlimited"
                          : String(favoriteCapacityLimit)
                      }
                      disabled={favoriteCapacityPending || switchPending}
                      onChange={(event) =>
                        setFavoriteCapacityLimit(
                          event.target.value === "unlimited"
                            ? null
                            : Number(event.target.value),
                        )
                      }
                    >
                      <option value={256 * 1024 * 1024}>256 MB</option>
                      <option value={1024 * 1024 * 1024}>1 GB</option>
                      <option value="unlimited">Unlimited</option>
                    </select>
                    <small>
                      Currently using{" "}
                      {formatFavoriteCapacityUsed(favoriteCapacity.used_bytes)}.
                    </small>
                  </label>
                  <div className="settings-group-actions">
                    <span
                      className="settings-status"
                      role="status"
                      aria-live="polite"
                    >
                      {favoriteCapacityPending
                        ? "Saving and reorganizing favorite slices…"
                        : ""}
                    </span>
                    <button
                      type="button"
                      className={`primary compact ${favoriteCapacityPending ? "sending" : ""}`}
                      disabled={
                        busy ||
                        favoriteCapacityLimit ===
                          (favoriteCapacity.limit_bytes ?? null)
                      }
                      onClick={() =>
                        onSaveFavoriteCapacity(favoriteCapacityLimit)
                      }
                    >
                      {favoriteCapacityPending && <LoaderCircle size={14} />}{" "}
                      {favoriteCapacityPending ? "Saving…" : "Save capacity"}
                    </button>
                  </div>
                </section>
                <section className="settings-group settings-temporary-files">
                  <div className="settings-group-heading">
                    <div>
                      <strong>Largest temporary items</strong>
                      <p>
                        Top 100 deletable items, largest first. Persistent data
                        and running jobs stay protected.
                      </p>
                    </div>
                    <div className="settings-inline-actions">
                      {temporaryDeleteMode && (
                        <button
                          type="button"
                          className="secondary compact"
                          disabled={temporaryItemsDeleting}
                          onClick={cancelTemporaryDelete}
                        >
                          Cancel
                        </button>
                      )}
                      <button
                        type="button"
                        className={`danger compact ${temporaryDeleteMode ? "confirm" : ""}`}
                        disabled={
                          temporaryItemsLoading ||
                          temporaryItemsDeleting ||
                          temporaryItems.length === 0 ||
                          (temporaryDeleteMode &&
                            selectedTemporaryIds.size === 0)
                        }
                        onClick={() => {
                          if (!temporaryDeleteMode) {
                            setTemporaryDeleteMode(true);
                            setSelectedTemporaryIds(new Set());
                          } else
                            onDeleteTemporaryItems(
                              Array.from(selectedTemporaryIds),
                            );
                        }}
                      >
                        {temporaryItemsDeleting ? (
                          <LoaderCircle size={14} />
                        ) : (
                          <Trash2 size={14} />
                        )}{" "}
                        {temporaryItemsDeleting
                          ? "Deleting…"
                          : temporaryDeleteMode
                            ? `Delete ${selectedTemporaryIds.size}`
                            : "Delete"}
                      </button>
                      <button
                        type="button"
                        className="secondary compact icon-only"
                        title="Refresh temporary files"
                        aria-label="Refresh temporary files"
                        disabled={
                          temporaryItemsLoading || temporaryItemsDeleting
                        }
                        onClick={onRefreshTemporaryItems}
                      >
                        {temporaryItemsLoading ? (
                          <LoaderCircle size={14} />
                        ) : (
                          <RefreshCw size={14} />
                        )}
                      </button>
                    </div>
                  </div>
                  <div
                    className="settings-temporary-list"
                    role="list"
                    aria-label="Largest temporary files"
                  >
                    {temporaryItemsLoading && temporaryItems.length === 0 ? (
                      <div className="settings-temporary-empty">
                        <LoaderCircle size={15} /> Loading temporary files…
                      </div>
                    ) : temporaryItemsError ? (
                      <div className="settings-temporary-empty error">
                        <TriangleAlert size={15} /> {temporaryItemsError}
                      </div>
                    ) : temporaryItems.length === 0 ? (
                      <div className="settings-temporary-empty">
                        No deletable temporary files.
                      </div>
                    ) : (
                      temporaryItems.map((item, index) => {
                        const selected = selectedTemporaryIds.has(item.id);
                        return (
                          <button
                            type="button"
                            role="listitem"
                            className={`settings-temporary-row ${temporaryDeleteMode ? "selecting" : ""} ${selected ? "selected" : ""}`}
                            disabled={
                              !temporaryDeleteMode || temporaryItemsDeleting
                            }
                            aria-pressed={
                              temporaryDeleteMode ? selected : undefined
                            }
                            key={item.id}
                            onClick={() => toggleTemporaryItem(item.id)}
                          >
                            <span className="settings-temporary-rank">
                              {index + 1}
                            </span>
                            <span
                              className="settings-temporary-select"
                              aria-hidden="true"
                            >
                              {selected && <Check size={12} />}
                            </span>
                            <span className="settings-temporary-copy">
                              <strong title={item.path}>{item.path}</strong>
                              <small>
                                {item.kind === "shell_job"
                                  ? "Finished shell job"
                                  : "Temporary file"}
                              </small>
                            </span>
                            <span className="settings-temporary-size">
                              {formatBytes(item.bytes)}
                            </span>
                          </button>
                        );
                      })
                    )}
                  </div>
                  <div className="settings-temporary-summary">
                    <span>
                      {temporaryItems.length} item
                      {temporaryItems.length === 1 ? "" : "s"}
                      {temporaryItems.length === 100 ? " · Top 100" : ""}
                    </span>
                    <span>
                      {temporaryDeleteMode && selectedTemporaryIds.size > 0
                        ? `${formatBytes(selectedTemporaryBytes)} selected`
                        : "Largest first"}
                    </span>
                  </div>
                </section>
              </section>
            )}
            {section === "memory" && memoryPage === "switch" && (
              <section
                className="settings-pane memory-switch-pane"
                aria-labelledby="memory-switch-title"
              >
                <button
                  type="button"
                  className="settings-back-link"
                  disabled={switchPending}
                  onClick={() => {
                    setPath(memPath);
                    setMemoryPage("overview");
                  }}
                >
                  <ChevronLeft size={15} />
                  <span>Memory</span>
                </button>
                <div className="memory-switch-hero">
                  <div className="memory-switch-hero-icon" aria-hidden="true">
                    <ArrowLeftRight size={22} />
                  </div>
                  <div>
                    <span className="eyebrow">MEM WORKSPACE</span>
                    <h3 id="memory-switch-title">Switch MEM</h3>
                    <p>Move Timem to another memory workspace on this host.</p>
                  </div>
                </div>
                <div
                  className="memory-switch-route"
                  aria-label="Memory switch route"
                >
                  <div>
                    <span>Current</span>
                    <code title={memPath}>{memPath || "…"}</code>
                  </div>
                  <ChevronRight size={17} aria-hidden="true" />
                  <div className={cleanedPath && !pathUnchanged ? "ready" : ""}>
                    <span>Next</span>
                    <code title={cleanedPath}>
                      {cleanedPath || "Enter a destination below"}
                    </code>
                  </div>
                </div>
                <section className="memory-switch-form-card">
                  <label className="settings-field">
                    <span>Destination MEM directory</span>
                    <input
                      autoFocus
                      value={path}
                      disabled={switchPending}
                      spellCheck={false}
                      placeholder="/absolute/path/to/.test_mem"
                      onChange={(event) => setPath(event.target.value)}
                      onKeyDown={(event) => {
                        if (
                          event.key === "Enter" &&
                          !event.nativeEvent.isComposing &&
                          !switchPending &&
                          cleanedPath &&
                          !pathUnchanged
                        ) {
                          event.preventDefault();
                          onSwitchMemory(cleanedPath);
                        }
                      }}
                    />
                    <small>
                      Enter an existing absolute directory on the machine
                      running Timem Web.
                    </small>
                  </label>
                  <div className="memory-switch-impact">
                    <strong>What changes</strong>
                    <ul>
                      <li>
                        Current sessions and workspace state are replaced by the
                        destination MEM.
                      </li>
                      <li>
                        Model endpoints, Memory policy, and stored sessions load
                        from that MEM.
                      </li>
                      <li>
                        Any running work in the current MEM must be stopped
                        before switching; it will be marked interrupted.
                      </li>
                      <li>
                        The current MEM remains on disk and can be switched back
                        to later.
                      </li>
                    </ul>
                  </div>
                  <div className="memory-switch-actions">
                    <span
                      className="settings-status"
                      role="status"
                      aria-live="polite"
                    >
                      {switchPending
                        ? "Switching MEM and loading its sessions…"
                        : !cleanedPath
                          ? "Enter an absolute MEM directory."
                          : pathUnchanged
                            ? "This is already the active MEM."
                            : "Ready to switch."}
                    </span>
                    <div>
                      <button
                        type="button"
                        className="secondary compact"
                        disabled={switchPending}
                        onClick={() => {
                          setPath(memPath);
                          setMemoryPage("overview");
                        }}
                      >
                        Cancel
                      </button>
                      <button
                        type="button"
                        className={`primary compact memory-switch-confirm ${switchPending ? "sending" : ""}`}
                        disabled={
                          switchPending || !cleanedPath || pathUnchanged
                        }
                        onClick={() => onSwitchMemory(cleanedPath)}
                      >
                        {switchPending ? (
                          <LoaderCircle size={14} />
                        ) : (
                          <ArrowLeftRight size={14} />
                        )}{" "}
                        {switchPending ? "Switching…" : "Switch MEM"}
                      </button>
                    </div>
                  </div>
                </section>
              </section>
            )}
          </div>{" "}
        </div>
      </section>
    </div>,
    document.body,
  );
});

function EndpointSettingsPane({
  endpoints,
  endpointEditor,
  revealedEndpointApiKeys,
  revealedEndpointHeaders,
  revealedEndpointRequestFields,
  onEdit,
  onDelete,
  onReveal,
  onSave,
}: {
  endpoints: ModelEndpoint[];
  endpointEditor: ModelEndpoint | "new" | null;
  revealedEndpointApiKeys: Record<string, string>;
  revealedEndpointHeaders: Record<string, Record<string, string>>;
  revealedEndpointRequestFields: Record<string, Record<string, unknown>>;
  onEdit: (endpoint: ModelEndpoint | "new" | null) => void;
  onDelete: (endpoint: ModelEndpoint) => void;
  onReveal: (endpointId: string) => void;
  onSave: (endpoint: ModelEndpointDraft) => void;
}) {
  const [deleteMode, setDeleteMode] = useState(false);
  const [selectedEndpointId, setSelectedEndpointId] = useState("");
  useEffect(() => {
    if (
      selectedEndpointId &&
      !endpoints.some((endpoint) => endpoint.id === selectedEndpointId)
    )
      setSelectedEndpointId("");
    if (deleteMode && endpoints.length === 0) setDeleteMode(false);
  }, [deleteMode, endpoints, selectedEndpointId]);
  if (endpointEditor)
    return (
      <section
        className="settings-pane endpoint-settings-pane editing"
        aria-label="Model endpoint editor"
      >
        <ModelEndpointEditor
          endpoint={endpointEditor === "new" ? undefined : endpointEditor}
          revealedApiKey={
            endpointEditor === "new"
              ? ""
              : revealedEndpointApiKeys[endpointEditor.id]
          }
          revealedHeaders={
            endpointEditor === "new"
              ? {}
              : revealedEndpointHeaders[endpointEditor.id]
          }
          revealedRequestFields={
            endpointEditor === "new"
              ? {}
              : revealedEndpointRequestFields[endpointEditor.id]
          }
          onClose={() => onEdit(null)}
          onSave={onSave}
        />
      </section>
    );
  const selected = endpoints.find(
    (endpoint) => endpoint.id === selectedEndpointId,
  );
  return (
    <section
      className="settings-pane endpoint-settings-pane"
      aria-labelledby="endpoint-settings-title"
    >
      <div className="settings-pane-heading">
        <h3 id="endpoint-settings-title">Model Endpoints</h3>
        <Sparkles size={19} aria-hidden="true" />
      </div>
      <div className="endpoint-settings-toolbar">
        <span>
          {endpoints.length} endpoint{endpoints.length === 1 ? "" : "s"}
        </span>
        <div>
          {deleteMode && (
            <button
              type="button"
              className="secondary compact"
              onClick={() => {
                setDeleteMode(false);
                setSelectedEndpointId("");
              }}
            >
              Cancel
            </button>
          )}
          <button
            type="button"
            className={`danger compact ${deleteMode ? "confirm" : ""}`}
            disabled={endpoints.length === 0 || (deleteMode && !selected)}
            onClick={() => {
              if (!deleteMode) {
                setDeleteMode(true);
                setSelectedEndpointId("");
              } else if (selected) onDelete(selected);
            }}
          >
            {deleteMode ? <Check size={14} /> : <Trash2 size={14} />}{" "}
            {deleteMode ? "Delete selected" : "Delete"}
          </button>
          <button
            type="button"
            className="primary compact"
            disabled={deleteMode}
            onClick={() => onEdit("new")}
          >
            <Plus size={14} /> Add endpoint
          </button>
        </div>
      </div>
      <div className="endpoint-settings-list">
        {endpoints.length === 0 ? (
          <div className="endpoint-empty">
            No model endpoints yet. Add one to configure model access.
          </div>
        ) : (
          endpoints.map((endpoint) => {
            const selectedForDelete = selectedEndpointId === endpoint.id;
            return (
              <div
                className={`endpoint-settings-row ${deleteMode ? "delete-selecting" : ""} ${selectedForDelete ? "delete-selected" : ""}`}
                key={endpoint.id}
              >
                <button
                  type="button"
                  className="endpoint-settings-select"
                  aria-pressed={deleteMode ? selectedForDelete : undefined}
                  onClick={() => {
                    if (deleteMode)
                      setSelectedEndpointId((current) =>
                        current === endpoint.id ? "" : endpoint.id,
                      );
                    else {
                      onEdit(endpoint);
                      if (
                        (endpoint.api_key_configured ||
                          Object.keys(endpoint.http_headers).length > 0 ||
                          Object.keys(endpoint.request_fields).length > 0) &&
                        revealedEndpointApiKeys[endpoint.id] === undefined
                      )
                        onReveal(endpoint.id);
                    }
                  }}
                >
                  <span>
                    <strong>{endpoint.name}</strong>
                    {deleteMode && (
                      <span className="endpoint-delete-select">
                        {selectedForDelete && <Check size={13} />}
                      </span>
                    )}
                  </span>
                  <small>
                    {endpoint.model} · {endpoint.api_protocol} ·{" "}
                    {endpoint.max_llm_input_tokens / 1_000}K /{" "}
                    {endpoint.max_llm_output_tokens / 1_000}K
                  </small>
                  <code title={endpoint.base_url}>{endpoint.base_url}</code>
                </button>
                {!deleteMode && (
                  <button
                    type="button"
                    className="endpoint-settings-edit"
                    title={`Edit ${endpoint.name}`}
                    aria-label={`Edit ${endpoint.name}`}
                    onClick={() => {
                      onEdit(endpoint);
                      if (
                        (endpoint.api_key_configured ||
                          Object.keys(endpoint.http_headers).length > 0 ||
                          Object.keys(endpoint.request_fields).length > 0) &&
                        revealedEndpointApiKeys[endpoint.id] === undefined
                      )
                        onReveal(endpoint.id);
                    }}
                  >
                    <Pencil size={14} />
                  </button>
                )}
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}

function fencedCode(language: string, code: string) {
  let fence = "```";
  while (code.includes(fence)) fence += "`";
  return `${fence}${language}\n${code}\n${fence}`;
}

function CompletionCard({
  completion,
  toolGenPending = false,
  toolGenBlocked = false,
  onToolGen,
  answerActions,
}: {
  completion: NonNullable<ChatMessage["completion"]>;
  toolGenPending?: boolean;
  toolGenBlocked?: boolean;
  onToolGen?: () => void;
  answerActions?: React.ReactNode;
}) {
  const stats = completion.stats ?? {};
  const cancelled = completion.stop_reason?.toLowerCase() === "cancelledbyuser";
  const toolGenLabel = toolGenPending
    ? "Starting ToolGen"
    : toolGenBlocked
      ? "ToolGen busy"
      : "ToolGen";
  const toolGenTitle = toolGenPending
    ? "ToolGen is starting for this task..."
    : toolGenBlocked
      ? "Another ToolGen task is already running in this session"
      : "Extract reusable tool from this task";
  const facts = [
    [
      cancelled ? "Cancelled" : "Completed",
      formatDuration(completion.elapsed_ms),
    ],
    ["LLM", stats.llm_calls],
    ["Input", formatOptionalTokens(stats.prompt_tokens)],
    ["Output", formatOptionalTokens(stats.completion_tokens)],
    ["KVC read", formatOptionalTokens(stats.cached_tokens)],
    ["KVC created", formatOptionalTokens(stats.cache_created_tokens)],
    ["Tools", stats.tool_calls],
    ["Repair", stats.repair_calls],
    ["Memory", formatMemoryOps(stats.mem_reads, stats.mem_writes)],
    ["Compact", formatOptionalTokens(stats.shrunk_tokens)],
  ].filter(
    ([label, value]) =>
      label === "Completed" ||
      label === "Cancelled" ||
      (value !== undefined && value !== null && value !== "" && value !== 0),
  ) as Array<[string, string | number | undefined]>;
  return (
    <div className="completion-card" aria-label="Turn completion statistics">
      {facts.map(([label, value]) => (
        <span
          key={label}
          title={
            completionFactTitle(label, completion, stats) ??
            (value === undefined || value === "" ? label : `${label}: ${value}`)
          }
        >
          <b>{label}</b> {value}
        </span>
      ))}
      {!cancelled && isNotableStopReason(completion.stop_reason) && (
        <span className="completion-status">
          <b>Status</b> {completion.stop_reason}
        </span>
      )}
      {completion.repair_issue && (
        <span className="completion-status warning">
          <b>Last repair</b> {completion.repair_issue}
        </span>
      )}
      {onToolGen && !cancelled && (
        <button
          className={`completion-toolgen ${toolGenPending ? "sending" : ""}`}
          type="button"
          title={toolGenTitle}
          aria-label={toolGenTitle}
          aria-busy={toolGenPending || undefined}
          disabled={toolGenPending || toolGenBlocked}
          onClick={onToolGen}
        >
          {toolGenPending ? <LoaderCircle size={12} /> : <Wrench size={12} />}
          <span aria-live="polite">{toolGenLabel}</span>
        </button>
      )}
      {answerActions}
    </div>
  );
}

function completionFactTitle(
  label: string,
  completion: NonNullable<ChatMessage["completion"]>,
  stats: Record<string, number | undefined>,
) {
  if (label === "Completed" || label === "Cancelled")
    return completion.elapsed_ms === undefined
      ? undefined
      : `${completion.elapsed_ms} ms`;
  if (label === "Input")
    return stats.prompt_tokens === undefined
      ? undefined
      : `${stats.prompt_tokens} input tokens`;
  if (label === "Output")
    return stats.completion_tokens === undefined
      ? undefined
      : `${stats.completion_tokens} output tokens`;
  if (label === "KVC read")
    return stats.cached_tokens === undefined
      ? undefined
      : `${stats.cached_tokens} cached input tokens`;
  if (label === "KVC created")
    return stats.cache_created_tokens === undefined
      ? undefined
      : `${stats.cache_created_tokens} cache-created input tokens`;
  if (label === "Compact")
    return stats.shrunk_tokens === undefined
      ? undefined
      : `${stats.shrunk_tokens} compacted tokens`;
  if (label === "Memory")
    return `${stats.mem_reads ?? 0} memory reads / ${stats.mem_writes ?? 0} memory writes`;
  return undefined;
}

function isNotableStopReason(reason: string | null | undefined) {
  if (!reason) return false;
  return !["finished", "completed", "all_finished", "final_answer"].includes(
    reason.toLowerCase(),
  );
}

function formatOptionalTokens(value: number | undefined) {
  return value ? formatTokens(value) : undefined;
}

function formatRemainingDuration(remainingMs: number) {
  const seconds = Math.max(0, Math.ceil(remainingMs / 1000));
  if (seconds < 60) return `${seconds}s`;
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainingSeconds = seconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainingSeconds).padStart(2, "0")}`
    : `${minutes}:${String(remainingSeconds).padStart(2, "0")}`;
}

function formatClockDuration(elapsedMs: number) {
  const seconds = Math.max(0, Math.floor(elapsedMs / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remainingSeconds = seconds % 60;
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainingSeconds).padStart(2, "0")}`
    : `${String(minutes).padStart(2, "0")}:${String(remainingSeconds).padStart(2, "0")}`;
}

function formatDuration(elapsedMs: number | undefined) {
  if (elapsedMs === undefined) return undefined;
  const seconds = Math.max(0, Math.round(elapsedMs / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m${String(seconds % 60).padStart(2, "0")}s`;
}

function formatMemoryOps(
  reads: number | undefined,
  writes: number | undefined,
) {
  if (!reads && !writes) return undefined;
  return `${reads ?? 0}R/${writes ?? 0}W`;
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.ceil(bytes / 1024)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function ModelEndpointPanel({
  panelRef,
  server,
  session,
  onEdit,
  onApply,
}: {
  panelRef: MutableRefObject<HTMLElement | null>;
  server: Snapshot["server"] | null;
  session: Session | undefined;
  onEdit: () => void;
  onApply: (endpointId: string) => void;
}) {
  const endpoints = server?.model_endpoints ?? [];
  const selected = endpoints.find((endpoint) =>
    endpointMatchesProfile(endpoint, session?.runtime_profile),
  );
  return (
    <section
      id="runtime-panel"
      ref={panelRef}
      className="runtime-card endpoint-menu"
      tabIndex={-1}
    >
      <div className="endpoint-menu-heading">
        <div>
          <span className="eyebrow">MODEL ENDPOINTS</span>
          <strong>
            {session ? `用于 ${session.display_name}` : "选择 Session 后应用"}
          </strong>
        </div>
        <button type="button" className="endpoint-menu-edit" onClick={onEdit}>
          <Pencil size={13} />
          <span>编辑</span>
        </button>
      </div>
      <div className="endpoint-list">
        {endpoints.length === 0 ? (
          <div className="endpoint-empty">
            还没有可用接入点。请在 Settings 中添加后再选择。
          </div>
        ) : (
          endpoints.map((endpoint) => {
            const active = selected?.id === endpoint.id;
            return (
              <div
                className={`endpoint-row ${active ? "active" : ""}`}
                key={endpoint.id}
              >
                <button
                  type="button"
                  className="endpoint-select"
                  aria-pressed={active}
                  disabled={!session || session.state === "working"}
                  onClick={() => onApply(endpoint.id)}
                >
                  <span className="endpoint-copy">
                    <span className="endpoint-name-line">
                      <strong>{endpoint.name}</strong>
                    </span>
                    <small className="endpoint-model-summary">
                      <Sparkles
                        size={10}
                        className="session-model-icon"
                        aria-hidden="true"
                      />
                      <span>
                        {endpoint.model} · {endpoint.api_protocol}
                        {endpoint.stream ? " · stream" : ""} ·{" "}
                        {endpoint.max_llm_input_tokens === 1_000_000
                          ? "1M"
                          : `${endpoint.max_llm_input_tokens / 1_000}K`}{" "}
                        / {endpoint.max_llm_output_tokens / 1_000}K
                      </span>
                    </small>
                    <small title={endpoint.base_url}>{endpoint.base_url}</small>
                  </span>
                  <span
                    className={`endpoint-choice-box ${active ? "selected" : ""}`}
                    aria-hidden="true"
                  >
                    {active && <Check size={11} strokeWidth={3} />}
                  </span>
                </button>
              </div>
            );
          })
        )}
      </div>
      {session?.state === "working" && (
        <p className="endpoint-note">
          当前 Session 工作中，结束或停止任务后才能切换接入点。
        </p>
      )}
    </section>
  );
}

function ModelEndpointEditor({
  endpoint,
  revealedApiKey,
  revealedHeaders,
  revealedRequestFields,
  onClose,
  onSave,
}: {
  endpoint?: ModelEndpoint;
  revealedApiKey?: string;
  revealedHeaders?: Record<string, string>;
  revealedRequestFields?: Record<string, unknown>;
  onClose: () => void;
  onSave: (endpoint: ModelEndpointDraft) => void;
}) {
  const [draft, setDraft] = useState<ModelEndpointDraft>(() => ({
    id: endpoint?.id,
    name: endpoint?.name ?? "",
    model: endpoint?.model ?? "",
    api_protocol: endpoint?.api_protocol ?? "openai-compatible",
    response_protocol: endpoint?.response_protocol ?? "xml",
    base_url: endpoint?.base_url ?? "",
    max_llm_input_tokens: endpoint?.max_llm_input_tokens ?? 100_000,
    max_llm_output_tokens: endpoint?.max_llm_output_tokens ?? 10_000,
    stream: endpoint?.stream ?? true,
    api_key: revealedApiKey,
    http_headers: endpoint?.http_headers ?? {},
    request_fields: endpoint?.request_fields ?? {},
  }));
  const [headerRows, setHeaderRows] = useState<StructuredRow[]>(() =>
    structuredRows(endpoint?.http_headers ?? {}),
  );
  const [requestRows, setRequestRows] = useState<StructuredRow[]>(() =>
    requestFieldRows(endpoint?.request_fields ?? {}),
  );
  const [showApiKey, setShowApiKey] = useState(false);
  const [showHeaders, setShowHeaders] = useState(!endpoint);
  const [showRequestFields, setShowRequestFields] = useState(!endpoint);
  useEffect(() => {
    if (revealedApiKey !== undefined)
      setDraft((current) => ({ ...current, api_key: revealedApiKey }));
  }, [revealedApiKey]);
  useEffect(() => {
    if (!revealedHeaders) return;
    setHeaderRows((rows) =>
      rows.map((row) => ({
        ...row,
        value: revealedHeaders[row.key] ?? row.value,
      })),
    );
  }, [revealedHeaders]);
  useEffect(() => {
    if (!revealedRequestFields) return;
    setRequestRows((rows) =>
      rows.map((row) => ({
        ...row,
        value: Object.hasOwn(revealedRequestFields, row.key)
          ? JSON.stringify(revealedRequestFields[row.key])
          : row.value,
      })),
    );
  }, [revealedRequestFields]);
  useEffect(() => {
    setShowApiKey(false);
    setShowHeaders(!endpoint);
    setShowRequestFields(!endpoint);
  }, [endpoint?.id]);
  const apiKey = draft.api_key ?? "";
  const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(
    apiKey,
    {
      idle: "复制 API Key",
      copied: "API Key 已复制",
      failed: "API Key 复制失败",
    },
  );
  const apiKeyVisibilityLabel = showApiKey ? "隐藏 API Key" : "显示 API Key";
  const headers = structuredRecord(headerRows);
  const parsedRequestFields = parseRequestFieldRows(requestRows);
  const endpointDraft = {
    ...draft,
    http_headers: headers,
    request_fields: parsedRequestFields.value,
  };
  const duplicateHeaderNames = hasDuplicateStructuredKeys(headerRows);
  const duplicateRequestNames = hasDuplicateStructuredKeys(requestRows);
  const saveDisabled =
    duplicateHeaderNames ||
    duplicateRequestNames ||
    !!parsedRequestFields.error ||
    !endpointDraftValid(endpointDraft);
  const save = () => {
    if (!saveDisabled) onSave(endpointDraft);
  };
  return (
    <div className="endpoint-editor">
      <div className="endpoint-editor-heading">
        <strong>{endpoint ? "编辑接入点" : "新增接入点"}</strong>
        <button
          type="button"
          aria-label="Close endpoint editor"
          onClick={onClose}
        >
          <X size={14} />
        </button>
      </div>
      <div className="endpoint-editor-grid">
        <label>
          名称
          <input
            autoFocus
            value={draft.name}
            placeholder="例如：生产环境 GPT"
            onChange={(event) =>
              setDraft({ ...draft, name: event.target.value })
            }
          />
        </label>
        <label>
          模型
          <input
            value={draft.model}
            placeholder="gpt-4.1"
            onChange={(event) =>
              setDraft({ ...draft, model: event.target.value })
            }
          />
        </label>
        <div className="endpoint-api-protocol">
          <label>
            API 协议
            <select
              value={draft.api_protocol}
              onChange={(event) => {
                const api_protocol = event.target.value;
                setDraft({
                  ...draft,
                  api_protocol,
                  stream: api_protocol === "openai-compatible",
                });
              }}
            >
              <option value="openai-compatible">openai-compatible</option>
              <option value="openai-responses">openai-responses</option>
              <option value="anthropic">anthropic</option>
            </select>
          </label>
          <label
            className="endpoint-stream-toggle"
            title="以流式 SSE 接收 OpenAI-compatible 响应"
          >
            <input
              type="checkbox"
              checked={draft.stream}
              disabled={draft.api_protocol !== "openai-compatible"}
              onChange={(event) =>
                setDraft({ ...draft, stream: event.target.checked })
              }
            />
            <span>Stream</span>
          </label>
        </div>
        <label>
          响应协议
          <select
            value={draft.response_protocol}
            onChange={(event) =>
              setDraft({ ...draft, response_protocol: event.target.value })
            }
          >
            <option value="xml">xml</option>
            <option value="json">json</option>
          </select>
        </label>
        <label className="wide">
          Base URL
          <input
            value={draft.base_url}
            placeholder="https://api.example.com/v1"
            onChange={(event) =>
              setDraft({ ...draft, base_url: event.target.value })
            }
          />
        </label>
        <label>
          最大上下文窗口
          <select
            value={draft.max_llm_input_tokens}
            onChange={(event) =>
              setDraft({
                ...draft,
                max_llm_input_tokens: Number(event.target.value),
              })
            }
          >
            {MODEL_CONTEXT_WINDOW_OPTIONS.map((tokens) => (
              <option key={tokens} value={tokens}>
                {tokens === 1_000_000 ? "1M" : `${tokens / 1_000}K`}
              </option>
            ))}
          </select>
        </label>
        <label>
          最大输出
          <select
            value={draft.max_llm_output_tokens}
            onChange={(event) =>
              setDraft({
                ...draft,
                max_llm_output_tokens: Number(event.target.value),
              })
            }
          >
            {MODEL_OUTPUT_TOKEN_OPTIONS.map((tokens) => (
              <option key={tokens} value={tokens}>
                {tokens / 1_000}K
              </option>
            ))}
          </select>
        </label>
        <label className="wide">
          API Key
          <div className="endpoint-api-key">
            <input
              type={showApiKey ? "text" : "password"}
              autoComplete="new-password"
              spellCheck={false}
              value={apiKey}
              placeholder={
                endpoint?.api_key_configured && revealedApiKey === undefined
                  ? "正在读取…"
                  : "可留空"
              }
              onChange={(event) =>
                setDraft({ ...draft, api_key: event.target.value })
              }
            />
            <div className="endpoint-api-key-actions">
              <button
                type="button"
                className={copyClass}
                title={copyLabel}
                aria-label={copyLabel}
                disabled={!apiKey}
                onClick={() => void copy()}
              >
                {copyState === "copied" ? (
                  <CheckCheck size={12} />
                ) : (
                  <Copy size={12} />
                )}
              </button>
              <button
                type="button"
                title={apiKeyVisibilityLabel}
                aria-label={apiKeyVisibilityLabel}
                onClick={() => setShowApiKey((visible) => !visible)}
              >
                {showApiKey ? <EyeOff size={13} /> : <Eye size={13} />}
              </button>
            </div>
          </div>
        </label>
        <div className="wide endpoint-structured-headers">
          <StructuredKeyValueEditor
            label="Headers"
            description="可选。每个 HTTP Header 单独填写，无需输入 JSON 或多行格式文本。"
            rows={headerRows}
            keyLabel="Name"
            keyPlaceholder="Header name"
            valuePlaceholder="Header value"
            addLabel="添加 Header"
            showValues={showHeaders}
            revealAction={
              headerRows.length > 0 ? (
                <button
                  type="button"
                  className="structured-field-visibility"
                  title={
                    showHeaders ? "隐藏 Header Value" : "显示 Header Value"
                  }
                  aria-label={
                    showHeaders ? "隐藏 Header Value" : "显示 Header Value"
                  }
                  onClick={() => setShowHeaders((visible) => !visible)}
                >
                  {showHeaders ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              ) : undefined
            }
            onChange={(rows) => {
              setShowHeaders(true);
              setHeaderRows(rows);
            }}
          />
        </div>
        <div className="wide endpoint-structured-headers">
          <StructuredKeyValueEditor
            label="Request Fields"
            description={
              '可选。作为 JSON 请求体顶层字段发送；字符串需写成 "fast"，也支持数字、布尔值、数组和对象。'
            }
            rows={requestRows}
            keyLabel="Field"
            keyPlaceholder="例如：service_tier"
            valuePlaceholder={'例如："fast"'}
            addLabel="Add Req Field"
            showValues={showRequestFields}
            revealAction={
              requestRows.length > 0 ? (
                <button
                  type="button"
                  className="structured-field-visibility"
                  title={
                    showRequestFields
                      ? "隐藏 Request Field Value"
                      : "显示 Request Field Value"
                  }
                  aria-label={
                    showRequestFields
                      ? "隐藏 Request Field Value"
                      : "显示 Request Field Value"
                  }
                  onClick={() => setShowRequestFields((visible) => !visible)}
                >
                  {showRequestFields ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              ) : undefined
            }
            onChange={(rows) => {
              setShowRequestFields(true);
              setRequestRows(rows);
            }}
          />
          {parsedRequestFields.error && !duplicateRequestNames && (
            <small className="structured-field-error" role="alert">
              {parsedRequestFields.error}
            </small>
          )}
        </div>
      </div>
      <div className="endpoint-editor-buttons">
        <button type="button" className="secondary compact" onClick={onClose}>
          取消
        </button>
        <button
          type="button"
          className="primary compact"
          disabled={saveDisabled}
          onClick={save}
        >
          保存接入点
        </button>
      </div>
    </div>
  );
}

function RuntimeSettingsPanel({
  panelRef,
  server,
  session,
  pendingKeys,
  credentialPending,
  revealedApiKey,
  onUpdate,
  onApiKeyUpdate,
  onApiKeyReveal,
}: {
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
  const keyConfigured = session?.runtime_profile?.api_key_configured ?? false;
  const sessionWorking = session?.state === "working";
  const runtimeOptions = useMemo(
    () =>
      sessionRuntimeOptions(
        session?.runtime_profile,
        server?.runtime_options ?? [],
      ),
    [server?.runtime_options, session?.runtime_profile],
  );
  useEffect(() => setDrafts({}), [session?.session_id]);
  useEffect(
    () =>
      setDrafts((current) => reconcileRuntimeDrafts(current, runtimeOptions)),
    [runtimeOptions],
  );
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
    const sessionId = session?.session_id;
    if (
      !shouldAutoRevealSessionApiKey({
        sessionId,
        configured: keyConfigured,
        revealedApiKey,
        pending: credentialPending,
        requestedSessionId: autoRevealSessionRef.current,
      })
    )
      return;
    if (!sessionId) return;
    autoRevealSessionRef.current = sessionId;
    onApiKeyReveal();
  }, [
    credentialPending,
    keyConfigured,
    onApiKeyReveal,
    revealedApiKey,
    session?.session_id,
  ]);
  if (!server)
    return (
      <section
        id="runtime-panel"
        ref={panelRef}
        className="runtime-card"
        tabIndex={-1}
      >
        <Cpu size={16} />
        <span>Loading runtime settings…</span>
      </section>
    );
  const pendingRuntimeLabel = pendingKeys.size
    ? `Applying runtime setting${pendingKeys.size === 1 ? "" : "s"}: ${Array.from(pendingKeys).map(runtimeOptionLabel).join(", ")}`
    : "";
  const bindLabel = `${server.bind_host || "127.0.0.1"}:${server.port}`;
  const apiKeyDirty =
    revealedApiKey === undefined
      ? apiKeyDraft.length > 0
      : apiKeyDraft !== revealedApiKey;
  const canSaveApiKey =
    !!session && apiKeyDirty && !credentialPending && !sessionWorking;
  const toggleApiKey = () => {
    if (showApiKey) setShowApiKey(false);
    else if (
      revealedApiKey !== undefined ||
      !keyConfigured ||
      apiKeyDraft.length > 0
    )
      setShowApiKey(true);
    else onApiKeyReveal();
  };
  return (
    <section
      id="runtime-panel"
      ref={panelRef}
      className="runtime-card runtime-settings"
      tabIndex={-1}
    >
      <div className="runtime-summary">
        <Cpu size={16} />
        <span>Timem {server.version}</span>
        <span>topic protocol v{server.protocol_version}</span>
        <span>
          <FolderOpen size={14} />
          {bindLabel}
        </span>
        {server.public_access && <span>public · token required</span>}
      </div>
      <div className="session-credential-settings">
        <div className="session-credential-heading">
          <KeyRound size={15} />
          <div>
            <strong>Session API key</strong>
            <small>
              {session
                ? session.display_name
                : "Create or select a session first"}
            </small>
          </div>
        </div>
        <div className="session-credential-control">
          <div className="secret-input">
            <input
              type={showApiKey ? "text" : "password"}
              value={apiKeyDraft}
              autoComplete="new-password"
              spellCheck={false}
              aria-label="API key for current session"
              placeholder={
                credentialPending && keyConfigured
                  ? "Loading API key…"
                  : "Enter API key"
              }
              disabled={!session || credentialPending}
              readOnly={sessionWorking}
              onChange={(event) => setApiKeyDraft(event.target.value)}
              onKeyDown={(event) => {
                if (
                  event.key === "Enter" &&
                  !event.nativeEvent.isComposing &&
                  canSaveApiKey
                ) {
                  event.preventDefault();
                  onApiKeyUpdate(apiKeyDraft);
                }
              }}
            />
            <button
              type="button"
              title={showApiKey ? "Hide API key" : "Show API key"}
              aria-label={showApiKey ? "Hide API key" : "Show API key"}
              disabled={!session || credentialPending}
              onClick={toggleApiKey}
            >
              {showApiKey ? <EyeOff size={15} /> : <Eye size={15} />}
            </button>
          </div>
          <button
            type="button"
            className="primary compact"
            disabled={!canSaveApiKey}
            onClick={() => onApiKeyUpdate(apiKeyDraft)}
          >
            {credentialPending ? "Working…" : "Save key"}
          </button>
        </div>
        {sessionWorking && (
          <small className="session-credential-note">
            API key is read-only while working; you can still reveal and copy
            it. Finish or stop the active task before changing credentials.
          </small>
        )}
      </div>
      <p>
        {session
          ? `Runtime settings for ${session.display_name}. Changes apply only to this Session.`
          : "Create or select a Session to configure its runtime."}
      </p>
      <div className="runtime-options">
        {runtimeOptions.map((option) => {
          const value = drafts[option.key] ?? option.value;
          const pending = pendingKeys.has(option.key);
          const dirty = value !== option.value;
          const optionLabel = runtimeOptionLabel(option.key);
          const inputLabel = `${optionLabel} current value`;
          const applyLabel = pending
            ? `Applying ${optionLabel}`
            : dirty
              ? `Apply ${optionLabel}`
              : `${optionLabel} has no changes`;
          const resetDraft = () =>
            setDrafts((current) => {
              const { [option.key]: _removed, ...rest } = current;
              return rest;
            });
          const options = runtimeSelectOptions(option.key);
          return (
            <label key={option.key}>
              <span>{optionLabel}</span>
              <div>
                {options ? (
                  <select
                    value={value}
                    title={inputLabel}
                    aria-label={inputLabel}
                    disabled={pending}
                    onChange={(event) =>
                      setDrafts((current) => ({
                        ...current,
                        [option.key]: event.target.value,
                      }))
                    }
                    onKeyDown={(event) => {
                      if (
                        event.key === "Enter" &&
                        !event.nativeEvent.isComposing &&
                        dirty &&
                        !pending
                      ) {
                        event.preventDefault();
                        onUpdate(option.key, value);
                      }
                      if (event.key === "Escape" && dirty) {
                        event.preventDefault();
                        resetDraft();
                      }
                    }}
                  >
                    {options.map((choice) => (
                      <option value={choice} key={choice}>
                        {choice === "unlimited" ? "Unlimited" : choice}
                      </option>
                    ))}
                  </select>
                ) : (
                  <input
                    value={value}
                    title={inputLabel}
                    aria-label={inputLabel}
                    disabled={pending}
                    onChange={(event) =>
                      setDrafts((current) => ({
                        ...current,
                        [option.key]: event.target.value,
                      }))
                    }
                    onKeyDown={(event) => {
                      if (
                        event.key === "Enter" &&
                        !event.nativeEvent.isComposing &&
                        dirty &&
                        !pending
                      ) {
                        event.preventDefault();
                        onUpdate(option.key, value);
                      }
                      if (event.key === "Escape" && dirty) {
                        event.preventDefault();
                        resetDraft();
                      }
                    }}
                  />
                )}{" "}
                {dirty && (
                  <button
                    type="button"
                    className="secondary compact runtime-reset"
                    title={`Reset ${optionLabel} to current value`}
                    aria-label={`Reset ${optionLabel} to current value`}
                    disabled={pending}
                    onClick={resetDraft}
                  >
                    Reset
                  </button>
                )}
                <button
                  type="button"
                  className="secondary compact"
                  title={applyLabel}
                  aria-label={applyLabel}
                  disabled={pending || !dirty}
                  onClick={() => onUpdate(option.key, value)}
                >
                  {pending ? "Applying…" : "Apply"}
                </button>
              </div>
            </label>
          );
        })}
      </div>
      {(pendingRuntimeLabel || credentialPending) && (
        <p className="runtime-pending-status" role="status" aria-live="polite">
          {credentialPending
            ? "Saving the Session API key…"
            : pendingRuntimeLabel}
        </p>
      )}
    </section>
  );
}

function runtimeSelectOptions(key: string): readonly string[] | null {
  switch (key) {
    case "TIMEM_API_PROTOCOL":
      return ["openai-compatible", "openai-responses", "anthropic"];
    case "TIMEM_RESPONSE_PROTOCOL":
      return ["xml", "json"];
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

const FAVORITE_CAPACITY_OPTIONS = [
  { label: "256 MB", bytes: 256 * 1024 * 1024 },
  { label: "1 GB", bytes: 1024 * 1024 * 1024 },
  { label: "不限", bytes: null },
] as const;

function FavoriteCapacityDialog({
  notice,
  currentCapacity,
  updating,
  onClose,
  onSelectLimit,
}: {
  notice: { capacity: ChatLibraryCapacity; full: boolean };
  currentCapacity: ChatLibraryCapacity;
  updating: boolean;
  onClose: () => void;
  onSelectLimit: (maxBytes: number | null) => void;
}) {
  const capacity = notice.capacity;
  const percent = capacity.used_percent ?? 0;
  const limitLabel = formatFavoriteCapacityLimit(capacity.limit_bytes);
  const title = notice.full ? "收藏夹已满" : "收藏夹空间快满了";
  const message = notice.full
    ? `这条回复还没有收藏。当前收藏夹上限为 ${limitLabel}，已使用 ${percent}%。请扩大空间或删除一些收藏后再试。`
    : `这条回复已收藏。当前收藏夹上限为 ${limitLabel}，已使用 ${percent}%。建议现在扩大空间，或删除不再需要的收藏。`;
  return (
    <div
      className="modal-backdrop favorite-capacity-backdrop"
      role="presentation"
      onClick={() => {
        if (!updating) onClose();
      }}
    >
      <section
        className="decision-modal favorite-capacity-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="favorite-capacity-title"
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape" && !updating) {
            event.preventDefault();
            onClose();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">收藏夹空间</span>
            <h2 id="favorite-capacity-title">
              <Star size={19} fill="currentColor" /> {title}
            </h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="关闭"
            aria-label="关闭"
            disabled={updating}
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </div>
        <p>{message}</p>
        <div
          className="favorite-capacity-meter"
          aria-label={`收藏夹已使用 ${percent}%`}
        >
          <span style={{ width: `${Math.min(100, percent)}%` }} />
        </div>
        <div className="favorite-capacity-usage">
          <strong>{percent}%</strong>
          <span>
            已使用约 {formatFavoriteCapacityUsed(capacity.used_bytes)}
          </span>
        </div>
        <fieldset disabled={updating}>
          <legend>扩大收藏夹空间</legend>
          <div className="favorite-capacity-options">
            {FAVORITE_CAPACITY_OPTIONS.map((option) => {
              const selected =
                (currentCapacity.limit_bytes ?? null) === option.bytes;
              return (
                <button
                  type="button"
                  className={selected ? "selected" : ""}
                  disabled={selected || updating}
                  key={option.label}
                  onClick={() => onSelectLimit(option.bytes)}
                >
                  <span>{option.label}</span>
                  {selected && <small>当前</small>}
                </button>
              );
            })}
          </div>
        </fieldset>
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={updating}
            onClick={onClose}
          >
            {notice.full ? "稍后处理" : "知道了"}
          </button>
          {updating && (
            <span className="favorite-capacity-updating" role="status">
              <LoaderCircle size={14} />
              正在调整…
            </span>
          )}
        </div>
      </section>
    </div>
  );
}

function formatFavoriteCapacityLimit(bytes?: number | null) {
  if (bytes == null) return "不限";
  return bytes >= 1024 * 1024 * 1024 ? "1 GB" : "256 MB";
}

function formatFavoriteCapacityUsed(bytes: number) {
  if (bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  if (mb < 1) return `${Math.max(0.1, Math.round(mb * 10) / 10)} MB`;
  if (mb < 1024) return `${Math.round(mb)} MB`;
  return `${(mb / 1024).toFixed(1)} GB`;
}

function RuntimeUnavailableDialog({
  detail,
  onClose,
}: {
  detail: string;
  onClose: () => void;
}) {
  return (
    <div
      className="modal-backdrop runtime-unavailable-backdrop"
      role="presentation"
    >
      <section
        className="decision-modal runtime-unavailable-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="runtime-unavailable-title"
        aria-describedby="runtime-unavailable-description"
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            onClose();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">RUNTIME STATUS</span>
            <h2 id="runtime-unavailable-title">
              <TriangleAlert size={20} aria-hidden="true" /> Runtime unavailable
            </h2>
          </div>
          <button
            type="button"
            className="icon-button"
            autoFocus
            title="Close runtime unavailable alert"
            aria-label="Close runtime unavailable alert"
            onClick={onClose}
          >
            <X size={16} />
          </button>
        </div>
        <p id="runtime-unavailable-description">{detail}</p>
        <p className="runtime-unavailable-hint">
          Timem cannot send messages or change sessions until the runtime
          reconnects. After you close this dialog, the warning banner will
          remain visible.
        </p>
      </section>
    </div>
  );
}

function NewSessionDialog({
  workspaces,
  runtimeDefaults,
  creating,
  memSwitching,
  onClose,
  onCreate,
}: {
  workspaces: string[];
  runtimeDefaults: Snapshot["server"]["session_env_defaults"];
  creating: boolean;
  memSwitching: boolean;
  onClose: () => void;
  onCreate: (
    command: Extract<ClientCommand, { type: "session_create" }>,
  ) => void;
}) {
  const [displayName, setDisplayName] = useState("");
  const [workspaceDir, setWorkspaceDir] = useState(workspaces[0] ?? "");
  const [env, setEnv] = useState<Record<string, string>>({});
  const updateEnv = (key: string, value: string) =>
    setEnv((current) => ({ ...current, [key]: value }));
  const resetEnv = (key: string) =>
    setEnv((current) => {
      const { [key]: _removed, ...rest } = current;
      return rest;
    });
  const createDecision = sessionCreateDecision(
    displayName,
    workspaceDir,
    env,
    creating,
    memSwitching,
  );
  const canCreateSession = createDecision.kind === "send";
  const closeIfIdle = () => {
    if (!creating) onClose();
  };
  const submit = () => {
    if (createDecision.kind === "send") onCreate(createDecision.command);
  };
  const descriptionId = "new-session-dialog-description";
  const statusId = "new-session-dialog-status";
  const describedBy = creating ? `${descriptionId} ${statusId}` : descriptionId;
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      aria-label="Dismiss create session"
      onClick={closeIfIdle}
    >
      <section
        className="decision-modal session-modal"
        role="dialog"
        aria-modal="true"
        aria-label="Create session"
        aria-describedby={describedBy}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeIfIdle();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">NEW SESSION</span>
            <h2>Start a session</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close create session"
            aria-label="Close create session"
            disabled={creating}
            onClick={closeIfIdle}
          >
            <X size={16} />
          </button>
        </div>
        <p id={descriptionId}>
          Select a known workspace or enter an existing absolute directory for
          this session.
        </p>
        {creating && (
          <p
            id={statusId}
            className="mem-validation"
            role="status"
            aria-live="polite"
          >
            Creating session…
          </p>
        )}
        <div className="session-modal-scroll">
          <label>
            Display name
            <input
              autoFocus
              value={displayName}
              placeholder="Optional name"
              disabled={creating}
              onChange={(event) => setDisplayName(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  submit();
                }
              }}
            />
          </label>
          <label>
            Workspace directory
            <input
              type="text"
              list="new-session-workspaces"
              value={workspaceDir}
              disabled={creating}
              placeholder="/absolute/path/to/workspace"
              autoComplete="off"
              onChange={(event) => setWorkspaceDir(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  submit();
                }
              }}
            />
            <datalist id="new-session-workspaces">
              {workspaces.map((workspace) => (
                <option value={workspace} key={workspace}>
                  {tailPath(workspace, 64)}
                </option>
              ))}
            </datalist>
          </label>
          <p className="mem-hint">
            Choose a suggested workspace or type an absolute directory path that
            exists on the Timem host.
          </p>
          <details className="session-runtime-overrides">
            <summary>Runtime environment</summary>
            <div className="session-runtime-grid">
              {SESSION_RUNTIME_FIELDS.map(([key, label, kind]) => (
                <label key={key}>
                  <span>
                    {label}
                    <small>{key}</small>
                  </span>
                  <div className="session-runtime-control">
                    {kind === "api_protocol" ? (
                      <select
                        value={env[key] ?? ""}
                        disabled={creating}
                        onChange={(event) => updateEnv(key, event.target.value)}
                      >
                        <option value="">
                          Inherit · {runtimeDefaults[key] ?? "default"}
                        </option>
                        <option value="openai-compatible">
                          openai-compatible
                        </option>
                        <option value="openai-responses">
                          openai-responses
                        </option>
                        <option value="anthropic">anthropic</option>
                      </select>
                    ) : kind === "response_protocol" ? (
                      <select
                        value={env[key] ?? ""}
                        disabled={creating}
                        onChange={(event) => updateEnv(key, event.target.value)}
                      >
                        <option value="">
                          Inherit · {runtimeDefaults[key] ?? "xml"}
                        </option>
                        <option value="xml">xml</option>
                        <option value="json">json</option>
                      </select>
                    ) : kind === "bash_approval" ? (
                      <select
                        value={env[key] ?? ""}
                        disabled={creating}
                        onChange={(event) => updateEnv(key, event.target.value)}
                      >
                        <option value="">
                          Inherit · {runtimeDefaults[key] ?? "ask"}
                        </option>
                        <option value="ask">ask</option>
                        <option value="approve">approve</option>
                      </select>
                    ) : kind === "work_instructions" ? (
                      <select
                        value={env[key] ?? ""}
                        disabled={creating}
                        onChange={(event) => updateEnv(key, event.target.value)}
                      >
                        <option value="">
                          Inherit · {runtimeDefaults[key] ?? "silent"}
                        </option>
                        <option value="silent">silent</option>
                        <option value="ask">ask</option>
                        <option value="off">off</option>
                      </select>
                    ) : kind === "boolean" ? (
                      <select
                        value={env[key] ?? ""}
                        disabled={creating}
                        onChange={(event) => updateEnv(key, event.target.value)}
                      >
                        <option value="">
                          Inherit · {runtimeDefaults[key] ?? "false"}
                        </option>
                        <option value="true">true</option>
                        <option value="false">false</option>
                      </select>
                    ) : (
                      <input
                        type={kind}
                        value={env[key] ?? ""}
                        min={kind === "number" ? 1 : undefined}
                        disabled={creating}
                        autoComplete={
                          kind === "password" ? "new-password" : undefined
                        }
                        placeholder={
                          kind === "password"
                            ? "Optional session-only key"
                            : `Inherit · ${runtimeDefaults[key] ?? "default"}`
                        }
                        onChange={(event) => updateEnv(key, event.target.value)}
                      />
                    )}{" "}
                    {env[key] !== undefined && (
                      <button
                        type="button"
                        className="session-runtime-reset"
                        title={`Reset ${label} to inherited value`}
                        aria-label={`Reset ${label} to inherited value`}
                        disabled={creating}
                        onClick={() => resetEnv(key)}
                      >
                        Reset
                      </button>
                    )}
                  </div>
                </label>
              ))}
            </div>
          </details>
        </div>
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={creating}
            onClick={closeIfIdle}
          >
            Cancel
          </button>
          <button
            type="button"
            className={`primary ${creating ? "sending" : ""}`}
            disabled={!canCreateSession}
            onClick={submit}
          >
            {creating ? <LoaderCircle size={16} /> : <Plus size={16} />}{" "}
            {creating ? "Creating…" : "Create session"}
          </button>
        </div>
      </section>
    </div>
  );
}

function MemSwitchConfirmDialog({
  candidate,
  pending,
  onClose,
  onConfirm,
}: {
  candidate: MemSwitchCandidate;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const closeIfIdle = () => {
    if (!pending) onClose();
  };
  const descriptionId = "mem-switch-confirm-description";
  const statusId = "mem-switch-confirm-status";
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      aria-label="Dismiss MEM switch confirmation"
      onClick={closeIfIdle}
    >
      <section
        className="decision-modal mem-switch-confirm-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="mem-switch-confirm-title"
        aria-describedby={
          pending ? `${descriptionId} ${statusId}` : descriptionId
        }
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeIfIdle();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">STOP AND SWITCH</span>
            <h2 id="mem-switch-confirm-title">Stop current MEM work?</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close MEM switch confirmation"
            aria-label="Close MEM switch confirmation"
            disabled={pending}
            onClick={closeIfIdle}
          >
            <X size={16} />
          </button>
        </div>
        <p id={descriptionId}>
          Switching MEM will stop all work in the current MEM.{" "}
          {candidate.runningSessionCount} running Session
          {candidate.runningSessionCount === 1 ? "" : "s"} will be marked
          interrupted and will not continue in the background.
        </p>
        <p className="mem-switch-alternative">
          To keep the current work running, start a separate instance for the
          destination MEM instead:{" "}
          <code>
            timem --space {shellQuoteCommandArgument(candidate.path)}
          </code>
        </p>
        <code className="mem-switch-confirm-path" title={candidate.path}>
          {candidate.path}
        </code>
        {pending && (
          <p
            id={statusId}
            className="session-delete-status"
            role="status"
            aria-live="polite"
          >
            Stopping current MEM workers and switching…
          </p>
        )}
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={pending}
            onClick={closeIfIdle}
          >
            Keep current MEM
          </button>
          <button
            type="button"
            className={`danger ${pending ? "sending" : ""}`}
            disabled={pending}
            onClick={onConfirm}
          >
            {pending ? <LoaderCircle size={15} /> : <CircleStop size={15} />}{" "}
            {pending ? "Stopping and switching…" : "Stop work and switch"}
          </button>
        </div>
      </section>
    </div>
  );
}

function ModelEndpointDeleteDialog({
  endpoint,
  onClose,
  onConfirm,
}: {
  endpoint: ModelEndpoint;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <div
      className="modal-backdrop endpoint-delete-backdrop"
      role="presentation"
      onClick={onClose}
    >
      <section
        className="decision-modal session-delete-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={`Delete ${endpoint.name}`}
        onClick={(event) => event.stopPropagation()}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">DELETE ENDPOINT</span>
            <h2>Delete “{endpoint.name}”?</h2>
          </div>
          <button type="button" className="icon-button" onClick={onClose}>
            <X size={16} />
          </button>
        </div>
        <p>
          This removes the shared endpoint from every Session dropdown. Existing
          Session settings are not changed.
        </p>
        <div className="decision-actions">
          <button type="button" className="secondary" onClick={onClose}>
            Cancel
          </button>
          <button type="button" className="danger" onClick={onConfirm}>
            <Trash2 size={15} /> Delete endpoint
          </button>
        </div>
      </section>
    </div>
  );
}

function SessionDeleteDialog({
  session,
  pending,
  onClose,
  onConfirm,
}: {
  session: Session;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const descriptionId = "delete-session-dialog-description";
  const statusId = "delete-session-dialog-status";
  const closeIfIdle = () => {
    if (!pending) onClose();
  };
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      aria-label="Dismiss delete session confirmation"
      onClick={closeIfIdle}
    >
      <section
        className="decision-modal session-delete-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={`Delete ${session.display_name}`}
        aria-describedby={
          pending ? `${descriptionId} ${statusId}` : descriptionId
        }
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeIfIdle();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">DELETE SESSION</span>
            <h2>Delete “{session.display_name}”?</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close delete session confirmation"
            aria-label="Close delete session confirmation"
            disabled={pending}
            onClick={closeIfIdle}
          >
            <X size={16} />
          </button>
        </div>
        <p id={descriptionId}>
          This permanently deletes the session, its stored task history,
          settings, and session tools.{" "}
          {session.state === "working" && "Current work will be stopped."} This
          cannot be undone.
        </p>
        {pending && (
          <p
            id={statusId}
            className="session-delete-status"
            role="status"
            aria-live="polite"
          >
            Stopping workers and deleting session…
          </p>
        )}
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={pending}
            onClick={closeIfIdle}
          >
            Cancel
          </button>
          <button
            type="button"
            className={`danger ${pending ? "sending" : ""}`}
            disabled={pending}
            onClick={onConfirm}
          >
            {pending ? <LoaderCircle size={16} /> : <Trash2 size={15} />}{" "}
            {pending ? "Deleting…" : "Delete session"}
          </button>
        </div>
      </section>
    </div>
  );
}

function ChatMessageDeleteDialog({
  candidate,
  pending,
  onClose,
  onConfirm,
}: {
  candidate: ChatMessageDeleteCandidate;
  pending: boolean;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const descriptionId = "chat-message-delete-description";
  const statusId = "chat-message-delete-status";
  const closeIfIdle = () => {
    if (!pending) onClose();
  };
  const roleLabel =
    candidate.role === "user" ? "user message" : "assistant answer";
  const normalizedPreview = candidate.preview.trim().replace(/\s+/g, " ");
  const preview = normalizedPreview.slice(0, 180);
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      aria-label="Dismiss delete message confirmation"
      onClick={closeIfIdle}
    >
      <section
        className="decision-modal chat-message-delete-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={`Delete ${roleLabel}`}
        aria-describedby={
          pending ? `${descriptionId} ${statusId}` : descriptionId
        }
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeIfIdle();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">DELETE MESSAGE</span>
            <h2>Delete this {roleLabel}?</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close delete message confirmation"
            aria-label="Close delete message confirmation"
            disabled={pending}
            onClick={closeIfIdle}
          >
            <X size={16} />
          </button>
        </div>
        <p id={descriptionId}>
          This permanently removes the content from the conversation and its raw
          chat log. Runtime activity records for the task are retained. This
          cannot be undone.
        </p>
        {preview && (
          <blockquote className="chat-message-delete-preview">
            {preview}
            {normalizedPreview.length > 180 ? "…" : ""}
          </blockquote>
        )}
        {pending && (
          <p
            id={statusId}
            className="session-delete-status"
            role="status"
            aria-live="polite"
          >
            Deleting message and rewriting raw chat history…
          </p>
        )}
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={pending}
            onClick={closeIfIdle}
          >
            Cancel
          </button>
          <button
            type="button"
            className={`danger ${pending ? "sending" : ""}`}
            disabled={pending}
            onClick={onConfirm}
          >
            {pending ? <LoaderCircle size={16} /> : <Trash2 size={15} />}{" "}
            {pending ? "Deleting…" : "Delete message"}
          </button>
        </div>
      </section>
    </div>
  );
}

function ToolGenDialog({
  pending,
  onClose,
  onSubmit,
}: {
  pending: boolean;
  onClose: () => void;
  onSubmit: (text: string) => void;
}) {
  const [instruction, setInstruction] = useState("");
  const closeIfIdle = () => {
    if (!pending) onClose();
  };
  const submit = () => {
    if (!pending) onSubmit(instruction.trim());
  };
  const descriptionId = "toolgen-dialog-description";
  const statusId = "toolgen-dialog-status";
  const describedBy = pending ? `${descriptionId} ${statusId}` : descriptionId;
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      aria-label="Dismiss ToolGen dialog"
      onClick={closeIfIdle}
    >
      <section
        className="decision-modal toolgen-dialog"
        role="dialog"
        aria-modal="true"
        aria-label="Generate reusable tool"
        aria-describedby={describedBy}
        onClick={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            event.stopPropagation();
            closeIfIdle();
          }
        }}
      >
        <div className="modal-titlebar">
          <div>
            <span className="eyebrow">TOOLGEN</span>
            <h2>Extract reusable tool</h2>
          </div>
          <button
            type="button"
            className="icon-button"
            title="Close ToolGen dialog"
            aria-label="Close ToolGen dialog"
            disabled={pending}
            onClick={closeIfIdle}
          >
            <X size={16} />
          </button>
        </div>
        <p id={descriptionId}>
          Timem will preserve reusable work from the completed task as one or
          more standalone script tools. Add optional guidance below.
        </p>
        {pending && (
          <p
            id={statusId}
            className="toolgen-dialog-status"
            role="status"
            aria-live="polite"
          >
            Starting ToolGen and opening a generating-tools task…
          </p>
        )}
        <label>
          Additional guidance
          <textarea
            autoFocus
            value={instruction}
            disabled={pending}
            placeholder="Optional: preferred interface, language, scope, or reusable workflow…"
            onChange={(event) => setInstruction(event.target.value)}
            onKeyDown={(event) => {
              if (
                (event.metaKey || event.ctrlKey) &&
                event.key === "Enter" &&
                !event.nativeEvent.isComposing
              ) {
                event.preventDefault();
                submit();
              }
            }}
          />
          <small className="toolgen-dialog-hint">
            Cmd/Ctrl+Enter to generate; Escape closes before it starts.
          </small>
        </label>
        <div className="decision-actions">
          <button
            type="button"
            className="secondary"
            disabled={pending}
            onClick={closeIfIdle}
          >
            Cancel
          </button>
          <button
            type="button"
            className={`primary ${pending ? "sending" : ""}`}
            disabled={pending}
            onClick={submit}
          >
            {pending ? <LoaderCircle size={16} /> : <Wrench size={15} />}{" "}
            {pending ? "Starting…" : "Generate tool"}
          </button>
        </div>
      </section>
    </div>
  );
}

function toolKey(sessionId: string, toolId: string) {
  return `${sessionId}:${toolId}`;
}

function pendingToolIdsForSession(
  pending: ReadonlySet<string>,
  sessionId: string,
) {
  const prefix = `${sessionId}:`;
  return new Set(
    Array.from(pending)
      .filter((key) => key.startsWith(prefix))
      .map((key) => key.slice(prefix.length)),
  );
}

function removeToolKeysForSession(
  pending: ReadonlySet<string>,
  sessionId: string,
) {
  const prefix = `${sessionId}:`;
  return new Set(Array.from(pending).filter((key) => !key.startsWith(prefix)));
}

function toolgenRequestKey(sessionId: string, turnId: string) {
  return `${sessionId}:${turnId}`;
}

function hasPendingToolgenForSession(
  pending: ReadonlySet<string>,
  sessionId: string,
) {
  const prefix = `${sessionId}:`;
  return Array.from(pending).some((key) => key.startsWith(prefix));
}

function pendingToolgenTurnIds(
  pending: ReadonlySet<string>,
  sessionId: string,
) {
  const prefix = `${sessionId}:`;
  return new Set(
    Array.from(pending)
      .filter((key) => key.startsWith(prefix))
      .map((key) => key.slice(prefix.length)),
  );
}

function removeToolgenRequestsForSession(
  pending: ReadonlySet<string>,
  sessionId: string,
) {
  const prefix = `${sessionId}:`;
  return new Set(Array.from(pending).filter((key) => !key.startsWith(prefix)));
}

function InlineDecision({
  decision,
  pending,
  locked,
  position,
  total,
  onReply,
}: {
  decision: Decision;
  pending: boolean;
  locked: boolean;
  position: number;
  total: number;
  onReply: (decision: "accept" | "decline" | "always_allow") => void;
}) {
  const disabled = pending || locked;
  const status = pending
    ? "Sending decision…"
    : locked
      ? "Session interaction is temporarily locked."
      : "";
  const canAlwaysAllow =
    decision.event.topic.name === "core.user.approval.request";
  const denyLabel = pending
    ? "Waiting for the current reply to finish"
    : locked
      ? "Decision is locked while the session changes"
      : "Deny this runtime request";
  const allowLabel = pending
    ? "Sending decision"
    : locked
      ? "Decision is locked while the session changes"
      : "Allow this runtime request";
  const alwaysAllowLabel = pending
    ? "Sending decision"
    : locked
      ? "Decision is locked while the session changes"
      : "Allow and stop asking for this session";
  return (
    <section
      className="inline-decision"
      aria-label="Decision required"
      aria-busy={pending}
    >
      <div className="inline-decision-heading">
        <span className="eyebrow">
          RUNTIME REQUEST{total > 1 ? ` · ${position} OF ${total}` : ""}
        </span>
        <h2>{decision.title}</h2>
      </div>
      <pre>{decision.detail}</pre>
      {status && (
        <span
          className="inline-decision-status"
          role="status"
          aria-live="polite"
        >
          {status}
        </span>
      )}
      <div className="decision-actions">
        <button
          type="button"
          className="secondary"
          title={denyLabel}
          aria-label={denyLabel}
          disabled={disabled}
          onClick={() => onReply("decline")}
        >
          Deny
        </button>
        <button
          type="button"
          className="primary"
          title={allowLabel}
          aria-label={allowLabel}
          disabled={disabled}
          onClick={() => onReply("accept")}
        >
          Allow
        </button>
        {canAlwaysAllow && (
          <button
            type="button"
            className="primary always-allow"
            title={alwaysAllowLabel}
            aria-label={alwaysAllowLabel}
            disabled={disabled}
            onClick={() => onReply("always_allow")}
          >
            Always Allow
          </button>
        )}
      </div>
    </section>
  );
}

export default function Root() {
  return <TimemApp />;
}

import { createRoot } from "react-dom/client";
createRoot(document.getElementById("root")!).render(<Root />);
