import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const appearanceSource = readFileSync(new URL("../src/appearance.ts", import.meta.url), "utf8");
const viewModelSource = readFileSync(new URL("../src/view_model.ts", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

describe("assistant-ui thread integration", () => {
  it("uses assistant-ui thread primitives for the primary conversation surface", () => {
    expect(source).toContain("ThreadPrimitive.Root");
    expect(source).toContain("ThreadPrimitive.Viewport");
    expect(source).toContain("ThreadPrimitive.ViewportFooter");
    expect(source).toContain("ComposerPrimitive.Root");
    expect(source).toContain("<TurnInteraction");
  });

  it("keeps the assistant-ui viewport scrollable while the composer is docked", () => {
    expect(styles).toContain(".aui-thread { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }");
    expect(styles).toContain(".chat-scroll { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow-y: auto;");
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*position:\s*sticky;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*bottom:\s*0;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*z-index:\s*3;/);
    expect(source).toContain("ThreadPrimitive.ScrollToBottom");
    expect(source).toContain("scrollToBottomOnThreadSwitch");
    expect(source).toContain("followThreadLatest.current = isNearScrollBottom");
    expect(source).toContain("viewport.scrollTop = viewport.scrollHeight");
    expect(source).toContain("[latestTurn?.turn_id]");
  });

  it("progressively mounts long task history and preserves the reading position", () => {
    expect(source).toContain("INITIAL_RENDERED_TURNS = 24");
    expect(source).toContain("TURN_HISTORY_PAGE_SIZE = 24");
    expect(source).toContain("previousScrollMetrics.current");
    expect(source).toContain("preservePrependScrollTop(previous, viewport.scrollHeight)");
    expect(source).toContain("canLoadStoredHistory");
    expect(source).toContain('sendCommand({ type: "history_page"');
    expect(source).toContain("Loading earlier history…");
    expect(styles).toContain(".load-history");
  });

  it("keeps multi-session navigation reachable on mobile", () => {
    expect(source).toContain('aria-label="Sessions"');
    expect(source).toContain('className={`sidebar ${showMobileSessions ? "mobile-open" : ""}`}');
    expect(styles).toContain(".icon-button.mobile-session-button, .mobile-sidebar-backdrop { display: none; }");
    expect(styles).toContain(".sidebar.mobile-open { visibility: visible; transform: translateX(0);");
    expect(styles).toContain(".icon-button.mobile-session-button { display: grid;");
  });

  it("defaults the diagnostic activity panel to hidden", () => {
    expect(source).toContain("const [showActivity, setShowActivity] = useState(false);");
  });

  it("does not expose internal model transport bookkeeping or duplicate activity labels", () => {
    expect(source).toContain('kind !== "model_request" && kind !== "model_response"');
    expect(source).not.toContain("Model completed a response");
    expect(source).not.toContain("LIVE ACTIVITY");
    expect(source).not.toContain("Working view");
    expect(source).not.toContain("renderToolInvocation");
    expect(viewModelSource).not.toContain('title: "Work instructions"');
  });

  it("uses the Markdown highlighter for final answers and Bash activity commands", () => {
    expect(source).toContain('import rehypeHighlight from "rehype-highlight";');
    expect(source).toContain("rehypePlugins={[rehypeHighlight]}");
    expect(source).toContain("fencedCode(activity.code_language ?? \"text\", activity.code)");
  });

  it("renders completion telemetry below final answers", () => {
    expect(source).toContain("attachTurnCompletion(session, event.outcome.message_id");
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain("<CompletionCard completion={turn.completion}");
    expect(styles).toContain(".completion-card");
    expect(styles).toContain(".turn-final-delivery");
    expect(source).toContain('["Compact", formatOptionalTokens(stats.shrunk_tokens)]');
    expect(source).not.toContain('["Shrunk", formatTokens(stats.shrunk_tokens)]');
  });

  it("binds assistant-ui running state to the authoritative session lifecycle", () => {
    expect(source).toContain('isRunning: activeSession?.state === "working"');
    expect(source).toContain('cancelled ? "Cancelled" : "Completed"');
    expect(viewModelSource).toContain('worker.worker_id === session.primary_worker_id');
  });

  it("deduplicates rapid cancel clicks and clears the guard when a turn finishes", () => {
    expect(source).toContain("const cancellingSessionIds = useRef<Set<string>>(new Set());");
    expect(source).toContain("const [cancellingSessionIdSet");
    expect(source).toContain('if (cancellingSessionIds.current.has(activeSession.session_id)) return;');
    expect(source).toContain('cancellingSessionIds.current.add(activeSession.session_id);');
    expect(source).toContain('cancellingSessionIds.current.delete(event.session_id);');
    expect(source).toContain('{isCancelling ? "Stopping…" : "Stop"}');
  });

  it("blocks send while cancellation is still in flight", () => {
    const start = source.indexOf("const sendText = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(sendText).toContain("cancellingSessionIds.current.has(activeSession.session_id)");
    expect(sendText).toContain("Cancellation in progress");
    expect(sendText).toContain("return;");
  });

  it("uses synchronous pending guards for rapid repeated browser clicks", () => {
    expect(source).toContain("creatingSessionRef.current");
    expect(source).toContain("pendingAttachmentRemoveIdsRef");
    expect(source).toContain("pendingDecisionKeysRef");
    expect(source).toContain("pendingRenameSessionIdsRef");
    expect(source).toContain("pendingRuntimeKeysRef");
    expect(source).toContain("addPendingKey(");
    expect(source).toContain("clearAllPendingCommands");
  });

  it("clears stale pending browser guards when a reconnect snapshot arrives", () => {
    const helloStart = source.indexOf('if (event.type === "hello")');
    const helloEnd = source.indexOf('if (event.type === "session_created")', helloStart);
    const helloBranch = source.slice(helloStart, helloEnd);
    expect(helloBranch).toContain("clearAllPendingCommands();");
    expect(helloBranch).toContain("setDecisions([]);");
    expect(helloBranch).toContain("applySnapshot(event.snapshot);");
  });

  it("renders live task usage and session context without replacing final telemetry", () => {
    expect(source).toContain("<ContextUsageBar session={activeSession}");
    expect(source).toContain("<LiveTurnUsage turn={turn}");
    expect(source).toContain('aria-label="Current task token usage"');
    expect(source).toContain('aria-label="Context usage"');
    expect(source).toContain("!turn.final_answer && turn.completion");
    expect(viewModelSource).toContain("turnLiveUsage");
    expect(viewModelSource).toContain("sessionContextUsage");
    expect(styles).toContain(".context-usage-bar");
    expect(styles).toContain(".live-turn-usage");
  });

  it("supports agent rename and a distinct animated working state", () => {
    expect(source).toContain('type: "session_rename"');
    expect(source).toContain('event.type === "session_renamed"');
    expect(source).toContain("session-working-icon");
    expect(source).toContain("session-rename-input");
    expect(styles).toContain("@keyframes session-working-glow");
  });

  it("expands each session into its scoped worker status list", () => {
    expect(source).toContain("expandedSessionIds");
    expect(source).toContain("session-expand");
    expect(source).toContain("worker-list");
    expect(source).toContain("worker.display_name || `ID${worker.ordinal}`");
    expect(styles).toContain(".worker-row");
    expect(styles).toContain(".worker-state-dot.working");
  });

  it("shows the live session cwd in navigation and above the composer", () => {
    expect(source).toContain('className="session-cwd">{tailPath(session.current_dir)}');
    expect(source).toContain('className="composer-cwd" title={activeSession.current_dir}');
    expect(viewModelSource).toContain("context_state");
    expect(styles).toContain(".session-cwd");
    expect(styles).toContain(".composer-cwd");
  });

  it("uses session terminology consistently for the creation workflow", () => {
    expect(source).toContain("New session");
    expect(source).toContain('aria-label="Create session"');
    expect(source).toContain('creating ? "Creating…" : "Create session"');
    expect(source).toContain("disabled={creating}");
    expect(source).not.toContain("New agent");
  });

  it("creates sessions with independent runtime environment overrides", () => {
    expect(source).toContain("SESSION_RUNTIME_FIELDS");
    expect(source).toContain('TIMEM_GATEWAY_PROVIDER');
    expect(source).toContain('TIMEM_MODEL');
    expect(source).toContain('TIMEM_API_KEY');
    expect(source).toContain('type={kind}');
    expect(source).toContain('env }))');
    expect(source).toContain('session.runtime_profile.provider');
    expect(source).toContain('session.runtime_profile.model');
    expect(styles).toContain('.session-runtime-grid');
    expect(styles).toContain('.session-profile');
  });

  it("renders context compaction outside chat messages with a reduced-motion fallback", () => {
    expect(source).toContain("<ContextCompactNotice");
    expect(styles).toContain(".context-compact-notice");
    expect(styles).toContain("prefers-reduced-motion: reduce");
  });

  it("persists theme, font, and text-size appearance without changing core state", () => {
    expect(appearanceSource).toContain('APPEARANCE_STORAGE_KEY = "timem-web-appearance-v1"');
    expect(appearanceSource).toContain('root.dataset.theme = appearance.theme');
    expect(appearanceSource).toContain('root.dataset.font = appearance.font');
    expect(appearanceSource).toContain('root.dataset.textSize = appearance.textSize');
    expect(source).toContain('aria-label="Appearance"');
    expect(source).toContain('<AppearancePanel appearance={appearance}');
    expect(styles).toContain(':root[data-theme="light"]');
    expect(styles).toContain(':root[data-font="serif"]');
    expect(styles).toContain(':root[data-text-size="large"]');
    expect(html).toContain('localStorage.getItem("timem-web-appearance-v1")');
    expect(html).toContain('document.documentElement.dataset.theme');
  });

  it("renders GFM and highlighted code with a copy affordance", () => {
    expect(source).toContain('import remarkGfm from "remark-gfm"');
    expect(source).toContain('remarkPlugins={[remarkGfm]}');
    expect(source).toContain('pre: CodeBlock');
    expect(source).toContain('navigator.clipboard.writeText(code)');
    expect(styles).toContain('.markdown-body blockquote');
    expect(styles).toContain('.code-block figcaption');
  });

  it("moves submitted files from the composer into a compact user attachment list", () => {
    expect(source).toContain("consumedAttachmentIds");
    expect(source).toContain('className="turn-entry-attachments"');
    expect(source).toContain("entry.attachments.map");
    expect(styles).toContain(".turn-entry-attachments > span");
  });

  it("lets users remove pending attachments without losing access to long file names", () => {
    expect(source).toContain('type: "attachment_remove"');
    expect(source).toContain('className="pending-attachment-name"');
    expect(source).toContain('title={attachment.name}');
    expect(source).toContain("pendingAttachmentRemoveIds.has");
    expect(source).toContain("disabled={removing}");
    expect(source).toContain('aria-label={removing ? `Removing ${attachment.name}` : `Remove ${attachment.name}`}');
    expect(styles).toContain(".pending-attachment-name");
    expect(styles).toContain("text-overflow: ellipsis");
  });

  it("keeps working-turn input visually consistent with a normal send", () => {
    expect(source).toContain('activeSession?.state === "working" ? "继续输入…"');
    expect(source).toContain('title="Send message" aria-label="Send message"');
    expect(source).not.toContain('>Supplement</span>');
    expect(source).not.toContain('title="Send supplement"');
  });

  it("removes the access token from the visible URL while retaining the session credential", () => {
    expect(source).toContain('const TOKEN_STORAGE_KEY = "timem-web-access-token";');
    expect(source).toContain("window.sessionStorage.setItem(TOKEN_STORAGE_KEY, query)");
    expect(source).toContain("window.history.replaceState");
  });

  it("does not create an optimistic ghost turn when the WebSocket send fails", () => {
    const start = source.indexOf("const sendText = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(sendText).toContain("if (!sendCommand(command))");
    expect(sendText).not.toContain("setSessions((current)");
    expect(sendText).toContain("return;");
  });

  it("groups each task into user input, bounded process, and separate final delivery", () => {
    expect(source).toContain('className="turn-user-frame"');
    expect(source).toContain('className={`turn-assistant-frame ${turn.state}`}');
    expect(source).toContain('className="turn-work-scroll"');
    expect(source).toContain('className="turn-final-delivery"');
    expect(styles).toContain(".turn-work-scroll { max-height:");
    expect(styles).toContain("overflow-y: auto;");
    expect(source).toContain("followLatest.current = remaining < 36");
    expect(source).toContain('className="turn-new-updates"');
  });

  it("uses frame styling without repeating user or session identity labels", () => {
    expect(source).not.toContain('<div className="message-label">You</div>');
    expect(source).not.toContain('className="message-label">{assistantName}');
    expect(source).not.toContain("assistantName={activeSession?.display_name");
    expect(source).not.toContain('<span className="eyebrow">SESSION');
    expect(source).not.toContain('activeSession?.display_name ?? "Starting Timem…"');
    expect(source).toContain('className="header-model"');
  });

  it("coalesces tool lifecycles and renders tools as compact subordinate rows", () => {
    expect(source).toContain("coalesceActionLifecycle(turn.events)");
    expect(source).toContain("<ToolActivity activity={activity}/>");
    expect(source).toContain("tool-activity-status");
    expect(styles).toContain(".tool-activity");
  });

  it("uses an explicit session-created event and session-scoped inline decisions", () => {
    expect(source).toContain('event.type === "session_created"');
    expect(source).toContain("enqueueDecision(current, pendingDecision)");
    expect(source).toContain("decision.event.session_id === activeSession?.session_id");
    expect(source).toContain("<InlineDecision");
    expect(source).not.toContain("<DecisionDialog");
    expect(styles).toContain(".inline-decision");
  });

  it("keeps blocking requests in the session flow when their reply cannot be sent", () => {
    expect(source).toContain('if (sendCommand({ type: "topic_reply"');
    expect(source).toContain("worker_id: event.worker_id ?? undefined");
    expect(source).toContain("current.filter((candidate) => candidate !== decision)");
    expect(source).toContain('if (sendCommand({ type: "session_create"');
  });

  it("backs off and reconnects the WebSocket instead of only changing the label", () => {
    expect(source).toContain("const connect = () =>");
    expect(source).toContain("Math.min(10_000, 500 * 2 ** Math.min(retryAttempt, 5))");
    expect(source).toContain("window.setTimeout(connect, delay)");
    expect(source).toContain("window.clearTimeout(retryTimer)");
  });

  it("shows host and session errors outside the default-hidden diagnostic panel", () => {
    expect(source).toContain('className="host-error-banner" role="alert"');
    expect(source).toContain('title="Dismiss error"');
    expect(styles).toContain(".host-error-banner");
  });
});
