import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const appearanceSource = readFileSync(new URL("../src/appearance.ts", import.meta.url), "utf8");
const preloadSource = readFileSync(new URL("../src/preload.ts", import.meta.url), "utf8");
const viewModelSource = readFileSync(new URL("../src/view_model.ts", import.meta.url), "utf8");
const protocolSource = readFileSync(new URL("../src/protocol.ts", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const logo = readFileSync(new URL("../public/timem_logo.png", import.meta.url));
const viteConfig = readFileSync(new URL("../vite.config.ts", import.meta.url), "utf8");

describe("Agent Core live-state delivery", () => {
  it("uses the explicit Core turn_started event for turn and worker working state", () => {
    expect(protocolSource).toContain('type: "turn_started"');
    expect(source).toContain('if (event.type === "turn_started")');
    expect(source).toContain('updateSessionWorkerState(upsertTurn(session, event.turn), event.worker_id, "working")');
  });

  it("does not infer completion from pending, restored, or other turn updates", () => {
    expect(source).not.toContain('event.turn.state !== "working"');
    expect(source).toMatch(/if \(event\.type === "turn_finished"\)[\s\S]*?setCompletedTurnKey/);
  });
});

describe("per-message worker role selection", () => {
  it("supports multiple selected roles and clears them only after a successful send", () => {
    expect(source).toContain("useState<Record<string, string[]>>({})");
    expect(source).toContain("role_ids: [...new Set(roleIds)]");
    expect(source).toContain("if (sent && selectedRoleIds.length > 0) onRolesConsumed(reserved.sessionId)");
    expect(source).toContain("selectedRoleIds.includes(role.id)");
  });

  it("shows role annotations on queued and sent user messages", () => {
    expect(source).toContain('className="queued-message-roles"');
    expect(source).toContain('className="turn-entry-roles"');
    expect(source).toContain("entry.worker_roles");
    expect(styles).toContain(".turn-entry-roles");
  });
});

describe("user message selection copying", () => {
  it("normalizes trailing DOM line breaks only for a selection contained in one user message", () => {
    expect(source).toContain('onCopy={(event) => {');
    expect(source).toContain("event.currentTarget.contains(selection.anchorNode)");
    expect(source).toContain("event.currentTarget.contains(selection.focusNode)");
    expect(source).toContain("normalizeCopiedUserMessageText(selection.toString())");
    expect(source).toContain('event.clipboardData.setData("text/plain", copiedText);');
    expect(source).toContain("event.preventDefault();");
    expect(viewModelSource).toContain('return text.replace(/(?:\\r?\\n)+$/, "");');
  });
});

describe("assistant-ui thread integration", () => {
  it("keeps a visible boot state before the React bundle mounts", () => {
    expect(html).toContain('<div id="root">');
    expect(html).toContain("Timem is loading...");
  });

  it("uses the Timem logo as the browser tab icon", () => {
    expect(html).toContain('<link rel="icon" type="image/png" href="/timem_logo.png" />');
    expect(Array.from(logo.subarray(0, 8))).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  });

  it("does not require crypto.randomUUID on an HTTP public-IP origin", () => {
    expect(protocolSource).toContain("export function clientId");
    expect(source).not.toContain("crypto.randomUUID()");
    expect(viewModelSource).not.toContain("crypto.randomUUID()");
  });

  it("keeps the brand concise and describes collaboration without a local-only qualifier", () => {
    expect(source).toContain("Ask Timem to investigate, write, or work with you.");
    expect(source).not.toContain("work with your local environment");
    expect(source).not.toContain("<small>local</small>");
  });

  it("uses assistant-ui thread primitives for the primary conversation surface", () => {
    expect(source).toContain("ThreadPrimitive.Root");
    expect(source).toContain("ThreadPrimitive.Viewport");
    expect(source).toContain("ThreadPrimitive.ViewportFooter");
    expect(source).toContain('form className="composer"');
    expect(source).toContain("<TurnInteraction");
  });

  it("keeps the assistant-ui viewport scrollable while the composer is docked", () => {
    expect(source).toContain("const EMPTY_CHAT_MESSAGES: ChatMessage[] = [];");
    expect(source).toContain("const activeMessages = activeSession?.messages ?? EMPTY_CHAT_MESSAGES;");
    expect(source).not.toContain("const activeMessages = activeSession?.messages ?? [];");
    expect(styles).toContain(".aui-thread { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }");
    expect(styles).toContain(".chat-scroll { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow-y: auto;");
    expect(styles).toContain("padding: 34px max(26px, calc((100% - 840px)/2)) 24px;");
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*position:\s*sticky;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*bottom:\s*0;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*z-index:\s*3;/);
    expect(source).toContain("ThreadPrimitive.ScrollToBottom");
    expect(source).toContain('title="Scroll to latest message" aria-label="Scroll to latest message"');
    expect(source).toContain("autoScroll={false}");
    expect(source).toContain("scrollToBottomOnInitialize={false}");
    expect(source).toContain("scrollToBottomOnRunStart={false}");
    expect(source).toContain("scrollToBottomOnThreadSwitch={false}");
    expect(styles).toMatch(/\.chat-scroll\s*\{[^}]*overflow-anchor:\s*none;/);
    expect(source).toContain("sessionScrollPositionsRef");
    expect(source).toContain('viewport.style.scrollBehavior = "auto";');
    expect(source).toContain("restoreSessionScrollTop(position, viewport.scrollHeight)");
    expect(source).toContain("followThreadLatest.current = isNearScrollBottom");
    expect(source).toContain("viewport.scrollTop = viewport.scrollHeight");
    expect(source).toContain("[activeSessionId, latestTurn?.turn_id]");
  });

  it("prioritizes a focused multiline composer before scrolling the chat viewport", () => {
    expect(source).toContain("const composerTextareaRef = useRef<HTMLTextAreaElement | null>(null);");
    expect(source).toContain("ref={composerTextareaRef}");
    expect(source).toContain('textarea.addEventListener("wheel", prioritizeComposerScroll, { passive: false });');
    expect(source).toContain('return () => textarea.removeEventListener("wheel", prioritizeComposerScroll);');
    expect(source).toContain("if (document.activeElement !== textarea) return;");
    expect(source).toContain("const deltaY = wheelDeltaPixels(event.deltaY, event.deltaMode, textarea.clientHeight);");
    expect(source).toContain("if (!canScrollInDirection(textarea, deltaY)) return;");
    expect(source).toContain("event.preventDefault();");
    expect(source).toContain("event.stopPropagation();");
    expect(source).toContain("textarea.scrollTop += deltaY;");
    expect(source).not.toContain("onWheel={(event) =>");
    expect(styles).toContain(".composer textarea { resize: none; overflow-y: auto; }");
  });

  it("renders durable runtime restarts as accessible chat timeline dividers", () => {
    expect(protocolSource).toContain('role: "user" | "assistant" | "system"');
    expect(source).toContain('message.kind === "runtime_restart"');
    expect(source).toContain("function RuntimeRestartDivider");
    expect(source).toContain('className="runtime-restart-divider" role="separator"');
    expect(source).toContain('.filter((message): message is ChatMessage & { role: "user" | "assistant" } => message.role !== "system")');
    expect(styles).toContain(".runtime-restart-divider {");
    expect(styles).toContain(':root[data-theme="light"] .runtime-restart-divider');
  });

  it("keeps the composer usable on narrow screens while stop and tool buttons are visible", () => {
    expect(styles).toContain("@media (max-width: 520px)");
    expect(styles).toContain(".composer-actions { align-items: flex-start; gap: 8px; justify-content: space-between; }");
    expect(styles).toContain(".composer-paths { min-width: 0; flex: 1 1 auto; }");
    expect(styles).toContain(".composer-buttons { width: 100%; flex-wrap: wrap; justify-content: flex-end; }");
    expect(styles).toContain(".attachment-strip { align-items: stretch; }");
    expect(styles).toContain(".pending-attachment { width: 100%; max-width: none; }");
    expect(styles).toContain(".completion-card span { white-space: normal; }");
    expect(source).toContain('{activeSession?.state === "working" && <button className={`stop-button ${isCancelling ? "sending" : ""}`');
  });

  it("makes disabled high-frequency controls visibly non-interactive", () => {
    expect(styles).toContain("button:disabled { cursor: not-allowed; }");
    expect(styles).toContain(".composer textarea:disabled { opacity: .62; cursor: not-allowed; }");
    expect(styles).toContain(".send-button:disabled, .stop-button:disabled, .attach-button:disabled, .new-session:disabled, .load-history:disabled, .decision-actions button:disabled, .completion-toolgen:disabled");
    expect(styles).toContain(".mem-card:disabled");
    expect(styles).toContain(".send-button:disabled:hover");
    expect(styles).toContain(".attach-button:disabled:hover");
    expect(styles).toContain(':root[data-theme="light"] .send-button:disabled:hover');
    expect(styles).toContain(':root[data-theme="light"] .attach-button:disabled:hover');
    expect(styles).toContain(':root[data-theme="light"] .load-history:disabled:hover');
  });

  it("uses valid light-theme root selectors", () => {
    expect(styles).toContain(':root[data-theme="light"]');
    expect(styles).not.toContain("::root");
  });

  it("declares button types explicitly so action controls cannot become accidental form submits", () => {
    const untypedButtons = [...source.matchAll(/<button(?![^>]*\btype=)[^>]*>/g)].map((match) => match[0]);
    expect(untypedButtons).toEqual([]);
    expect(source).toContain('type="submit"');
  });

  it("keeps keyboard focus visible across buttons and form controls", () => {
    expect(styles).toContain(":where(button, input, textarea, select, summary):focus-visible");
    expect(styles).toContain("outline: 2px solid #72d7c2");
    expect(styles).toContain(":root[data-theme=\"light\"] :where(button, input, textarea, select, summary):focus-visible");
    expect(styles).toContain("outline-color: #167669");
  });

  it("queues working-turn input while keeping an explicit supplement escape hatch", () => {
    expect(source).toContain('const sendLabel = isCancelling ? "Cancellation in progress" : activeSession?.state === "working" ? "Queue message" : "Send message";');
    expect(source).toContain('const missingSessionHint = activeSession ? "" : "Create a session before using Timem";');
    expect(source).toContain('const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";');
    expect(source).toContain('`${uploadingAttachmentText} · send is paused until it finishes`');
    expect(source).toContain('const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);');
    expect(source).toContain('const composerHintId = `composer-hint-${activeSessionId || "empty"}`;');
    expect(source).toContain("if (uploadingAttachment || sessionInteractionLocked) return;");
    expect(source).toContain('placeholder={!activeSession ? "Create a session to start…"');
    expect(source).toContain('aria-describedby={composerHintId}');
    expect(source).toContain('title={composerHint}');
    expect(source).toContain('<div className="composer-actions"><div className="composer-paths">');
    expect(source).toContain('<span id={composerHintId} className="sr-only" role="status" aria-live="polite">{composerHint}</span>');
    expect(source).toContain('<span className="composer-cwd-inline" title={activeSession.current_dir}><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></span>');
    expect(source).toContain('title={effectiveSendLabel}');
    expect(source).toContain('aria-label={effectiveSendLabel}');
    expect(source).toContain('className={`send-button ${submittingDraft ? "sending" : ""}`}');
    expect(source).toContain('{submittingDraft ? <LoaderCircle size={17}/> : <Send size={17}/>}');
    expect(styles).toContain(".send-button.sending svg");
    expect(source).toContain('className={`stop-button ${isCancelling ? "sending" : ""}`}');
    expect(source).toContain('{isCancelling ? <LoaderCircle size={17}/> : <CircleStop size={17}/>} {isCancelling ? "Stopping…" : "Stop"}');
    expect(styles).toContain(".stop-button.sending svg");
    expect(styles).toContain(".send-button.sending svg, .stop-button.sending svg");
    expect(source).toContain('aria-label={isCancelling ? "Cancellation requested" : lockedControlHint || "Cancel current turn"}');
  expect(source).toContain("const submitDraftAsSupplement = () => {");
 expect(source).toContain('event.key !== "Enter" || event.nativeEvent.isComposing');
 expect(source).toContain("event.metaKey || event.ctrlKey");
 expect(source).toContain("submitDraftAsSupplement();");
 expect(source).toContain('clientId("supplement")');
 expect(source).toContain("availableAttachments.map((attachment) => attachment.id)");
 expect(source).toContain("attachmentIds?: readonly string[], forceSupplement = false");
 expect(source).toContain("attachmentIds,\n forceSupplement,");
 });

  it("loads older stored history explicitly and preserves the reading position", () => {
    expect(source).toContain("STORED_HISTORY_PAGE_SIZE = 200");
    expect(source).toContain("previousScrollMetrics.current");
    expect(source).toContain("preservePrependScrollTop(previous, viewport.scrollHeight)");
    expect(source).toContain("canLoadStoredHistory");
    expect(source).toContain('sendCommand({ type: "history_page"');
    expect(source).toContain("limit: STORED_HISTORY_PAGE_SIZE");
    expect(source).toContain('const historyButtonLabel = sessionInteractionLocked');
    expect(source).toContain('`${sessionInteractionLockReason} · earlier history is locked`');
    expect(source).toContain("Loading earlier history…");
    expect(source).toContain("Load ${STORED_HISTORY_PAGE_SIZE} older stored tasks");
    expect(source).toContain('className={`load-history ${loadingHistory ? "loading" : ""}`} title={historyButtonLabel} aria-label={historyButtonLabel} aria-live="polite" aria-busy={loadingHistory || undefined}');
    expect(source).toContain('{loadingHistory && <LoaderCircle size={13} aria-hidden="true"/>}');
    expect(source).toContain("<span>{historyButtonLabel}</span>");
    expect(styles).toContain(".load-history");
    expect(styles).toContain(".load-history.loading svg");
    expect(styles).toContain(".load-history.loading svg, .send-button.sending svg");
    expect(source).not.toContain("event.currentTarget.scrollTop <= 48");
  });

  it("keeps multi-session navigation reachable on mobile", () => {
    expect(source).toContain('const mobileSessionsLabel = showMobileSessions ? "Close session navigation" : "Open session navigation";');
    expect(source).toContain("const mobileSessionButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const mobileSidebarRef = useRef<HTMLElement | null>(null);");
    expect(source).toContain("const closeMobileSidebar = useCallback((restoreFocus = true) => {");
    expect(source).toContain("if (restoreFocus) mobileSessionButtonRef.current?.focus({ preventScroll: true });");
    expect(source).toContain("mobileSidebarRef.current?.focus({ preventScroll: true });");
    expect(source).toContain('id="session-navigation" ref={mobileSidebarRef} className={`sidebar ${showMobileSessions ? "mobile-open" : ""}`} aria-label="Session navigation" tabIndex={-1}');
    expect(source).toContain('ref={mobileSessionButtonRef} title={mobileSessionsLabel} aria-label={mobileSessionsLabel}');
    expect(source).toContain('<button type="button" className="mobile-sidebar-backdrop" aria-label="Close session navigation" onClick={() => closeMobileSidebar()}');
    expect(source).toContain('aria-label="Close sessions" onClick={() => closeMobileSidebar()}');
    expect(source).toContain('setShowNewSession(true); closeMobileSidebar(false);');
    expect(source).toContain("if (!showMobileSessions) return;");
    expect(source).toContain('if (event.key === "Escape") closeMobileSidebar()');
    expect(source).toContain('setActiveSessionId(session.session_id); closeMobileSidebar();');
    expect(source).toContain('aria-current={session.session_id === activeSession?.session_id ? "page" : undefined}');
    expect(styles).toContain(".icon-button.mobile-session-button");
    expect(styles).toContain(".mobile-sidebar-backdrop");
    expect(styles).toContain(".sidebar.mobile-open { visibility: visible; transform: translateX(0);");
    expect(styles).toContain(".icon-button.mobile-session-button { display: grid;");
  });

  it("keeps ToolRepo as a dedicated panel without the diagnostic Activity feed", () => {
    expect(source).toContain("const [showToolRepo, setShowToolRepo] = useState(false);");
    expect(source).toContain("const toolRepoButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const toolRepoPanelRef = useRef<HTMLElement | null>(null);");
    expect(source).toContain("const closeToolRepoPanel = useCallback(() => {");
    expect(source).toContain("toolRepoButtonRef.current?.focus({ preventScroll: true });");
    expect(source).toContain('if (event.key === "Escape") closeToolRepoPanel()');
    expect(source).toContain('const activeToolCount = activeSession?.tools.length ?? 0;');
    expect(source).toContain('const toolRepoLabel = showToolRepo ? "Close ToolRepo" : `Open ToolRepo · ${activeToolCount} reusable tools`;');
    expect(source).toContain('aria-expanded={showToolRepo} aria-controls="toolrepo-panel"');
    expect(source).toContain('ref={toolRepoButtonRef} title={toolRepoLabel} aria-label={toolRepoLabel}');
    expect(source).toContain('<Wrench size={17}/><span className="toolrepo-header-count" aria-hidden="true">{activeToolCount}</span>');
    expect(source).toContain('<button type="button" className="side-panel-backdrop" aria-label="Close ToolRepo" onClick={closeToolRepoPanel}');
    expect(source).toContain('function ToolRepoPanel');
    expect(source).toContain('id="toolrepo-panel" ref={panelRef} className="toolrepo-side-panel session-side-panel" aria-label="ToolRepo" tabIndex={-1}');
    expect(source).toContain('<strong>ToolRepo</strong>');
    expect(source).toContain('<div className="side-panel-title"><Wrench size={15}/><strong>ToolRepo</strong></div>');
    expect(source).not.toContain('<strong>ToolRepo</strong>{session && <small>');
    expect(source).not.toContain('side-panel-tab-activity');
    expect(source).not.toContain('>Activity<');
    expect(source).not.toContain('function ActivityListItem');
    expect(source).not.toContain('activity-count-badge');
    expect(styles).toContain(".side-panel-backdrop");
    expect(styles).toContain("z-index: 3");
    expect(styles).toContain(".app-shell, .app-shell:has(.toolrepo-side-panel)");
    expect(styles).toContain(".toolrepo-side-panel { position: fixed; z-index: 4;");
  });

  it("keeps narrow-screen panels as overlays so the chat and composer stay usable", () => {
    expect(styles).toContain("@media (max-width: 1050px) { .app-shell, .app-shell:has(.toolrepo-side-panel) { grid-template-columns: 214px minmax(0, 1fr); }");
    expect(styles).toContain(".toolrepo-side-panel { position: fixed; z-index: 4; right: 0; top: 0; bottom: 0; width: min(360px, 88vw); }");
    expect(styles).toContain(".side-panel-backdrop { display: block; position: fixed; z-index: 3; inset: 0;");
    expect(styles).toContain("@media (max-width: 720px) { .app-shell, .app-shell:has(.toolrepo-side-panel) { grid-template-columns: 1fr; }");
    expect(styles).toContain(".sidebar { display: flex; visibility: hidden; position: fixed; z-index: 12;");
    expect(styles).toContain(".mobile-sidebar-backdrop { display: block; position: fixed; z-index: 11;");
    expect(styles).toContain(".chat-scroll { padding: 24px 17px; }");
    expect(styles).toContain(".composer-wrap { padding: 12px 17px 16px; }");
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*position:\s*sticky;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*bottom:\s*0;/);
    expect(styles).toMatch(/\.turn-work-scroll\s*\{[^}]*max-height:\s*52vh;/);
  });

  it("labels the runtime settings control for assistive and contract testing", () => {
    expect(source).toContain('const runtimeLabel = showRuntime ? "Close runtime information" : "Open runtime information";');
    expect(source).toContain('aria-label={`${runtimeLabel}: ${headerModelLabel}`}');
    expect(source).toContain('aria-expanded={showRuntime}');
    expect(source).toContain('aria-expanded={showRuntime} aria-controls="runtime-panel"');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card"');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings"');
    expect(source).toContain('const inputLabel = `${optionLabel} current value`;');
    expect(source).toContain('const applyLabel = pending ? `Applying ${optionLabel}` : dirty ? `Apply ${optionLabel}` : `${optionLabel} has no changes`;');
    expect(source).toContain('title={inputLabel} aria-label={inputLabel}');
    expect(source).toContain('title={applyLabel} aria-label={applyLabel}');
    expect(source).toContain('setShowAppearance(false); setShowMcp(false); setShowToolRepo(false); if (showRuntime) closeRuntimePanel(); else setShowRuntime(true);');
  });

  it("shows the runtime bind host and public-token mode from the server snapshot", () => {
    expect(protocolSource).toContain("bind_host: string;");
    expect(protocolSource).toContain("public_access: boolean;");
    expect(source).toContain('const bindLabel = `${server.bind_host || "127.0.0.1"}:${server.port}`;');
    expect(source).toContain("{bindLabel}");
    expect(source).toContain("public · token required");
    expect(source).not.toContain("localhost:{server.port}");
  });

  it("opens ToolRepo from the header and keeps the composer focused on message actions", () => {
    expect(source).toContain('const [showToolRepo, setShowToolRepo] = useState(false);');
    expect(source).toContain('aria-label={toolRepoLabel}');
    expect(source).toContain('className={`icon-button toolrepo-header-button ${showToolRepo ? "selected" : ""} ${toolCountPulseSessionId === activeSession?.session_id ? "count-pulse" : ""}`}');
    expect(source).not.toContain('className={`toolrepo-toggle');
    expect(source).not.toContain('onOpenToolRepo: () => void;');
    expect(source).not.toContain('type: "toolgen_set"');
    expect(source).not.toContain('aria-pressed={toolgenEnabled}');
    expect(source).toContain('event.type === "tool_repo_updated"');
    expect(source).toContain('event.session_id !== activeSessionIdRef.current');
    expect(source).toContain('event.query !== toolSearchQueryRef.current');
    expect(styles).toContain(".toolrepo-header-button");
    expect(styles).toContain(".toolrepo-header-count");
    expect(styles).toContain("@keyframes tool-count-pulse");
  });

  it("starts ToolGen manually from an exact completed turn with optional guidance", () => {
    expect(source).toContain('manualToolGenCommand(request.sessionId, request.turnId, text)');
    expect(source).toContain('const pendingToolgenRequestsRef = useRef<Set<string>>(new Set());');
    expect(source).toContain('if (pendingToolgenRequestsRef.current.has(requestKey)) return;');
    expect(source).toContain('pendingToolgenRequestsRef.current.add(requestKey);');
    expect(source).toContain('setPendingToolgenRequests(new Set(pendingToolgenRequestsRef.current));');
    expect(source).toContain('pendingToolgenRequestsRef.current.delete(requestKey);');
    expect(source).toContain('pendingToolgenRequestsRef.current = removeToolgenRequestsForSession(pendingToolgenRequestsRef.current, event.session_id);');
    expect(source).toContain('pendingToolgenRequestsRef.current.clear();');
    expect(source).toContain('function ToolGenDialog');
    expect(source).toContain('const descriptionId = "toolgen-dialog-description";');
    expect(source).toContain('const statusId = "toolgen-dialog-status";');
    expect(source).toContain('const describedBy = pending ? `${descriptionId} ${statusId}` : descriptionId;');
    expect(source).toContain('aria-describedby={describedBy}');
    expect(source).toContain("Extract reusable tool");
    expect(source).toContain("preserve reusable work from the completed task");
    expect(source).toContain("Optional: preferred interface, language, scope, or reusable workflow…");
    expect(source).toContain('Additional guidance');
    expect(source).toContain('event.key === "Enter" && !event.nativeEvent.isComposing');
    expect(source).toContain('pendingToolGenTurnIds={activeSession ? pendingToolgenTurnIds(pendingToolgenRequests, activeSession.session_id) : new Set()}');
    expect(source).toContain('toolGenSessionBusy={!!activeSession && hasPendingToolgenForSession(pendingToolgenRequests, activeSession.session_id)}');
    expect(source).toContain('toolGenPending={pendingToolGenTurnIds.has(turn.turn_id)}');
    expect(source).toContain('toolGenBlocked={toolGenSessionBusy && !pendingToolGenTurnIds.has(turn.turn_id)}');
    expect(source).toContain('function CompletionCard({ completion, toolGenPending = false, toolGenBlocked = false, onToolGen, answerActions }');
    expect(source).toContain('onToolGen={isToolGenTurn ? undefined : () => onRequestToolGen(turn.turn_id)}');
    expect(source).toContain('const toolGenLabel = toolGenPending ? "Starting ToolGen" : toolGenBlocked ? "ToolGen busy" : "ToolGen";');
    expect(source).toContain('const toolGenTitle = toolGenPending ? "ToolGen is starting for this task..." : toolGenBlocked ? "Another ToolGen task is already running in this session" : "Extract reusable tool from this task";');
    expect(source).toContain('className={`completion-toolgen ${toolGenPending ? "sending" : ""}`}');
    expect(source).toContain('title={toolGenTitle} aria-label={toolGenTitle}');
    expect(source).toContain('aria-busy={toolGenPending || undefined}');
    expect(source).toContain('disabled={toolGenPending || toolGenBlocked}');
    expect(source).toContain('<span aria-live="polite">{toolGenLabel}</span>');
    expect(source).toContain('isToolGenTurn ? "Generating tools…" : <span className="working-label">working</span>');
    expect(source).toContain('isToolGenTurn ? "Generating tools…" : "Waiting for the first runtime update…"');
    expect(styles).toContain(".working-chip.toolgen-working");
    expect(styles).toContain(".completion-toolgen { display: inline-flex; align-items: center; gap: 4px; margin-left: auto; padding: 0 3px 0 9px; border: 0; border-left: 1px solid #333;");
    expect(styles).toContain(".completion-toolgen:hover { color: #8ebce0; border-left-color: #4f6474; }");
    expect(styles).toContain(':root[data-theme="light"] .completion-toolgen { border-left-color: #d5dde2; color: #437ba8; }');
    expect(styles).toContain(".completion-toolgen.sending svg");
  });

  it("labels completed normal and ToolGen work frames with minimal text-only titles", () => {
    expect(source).toContain('isToolGenTurn ? "ToolGen" : "Thought/Action"');
    expect(source).not.toContain('<span className="work-title-dot" aria-hidden="true"/>');
    expect(source).toContain("completed-work-title");
    expect(source).toContain("toolgen-completed-title");
    expect(styles).toContain(".working-chip.completed-work-title { color: #d4d4d4; }");
    expect(styles).toContain(".working-chip.work-title-chip { min-height: 0; padding: 0; border: 0; border-radius: 0; background: transparent; }");
    expect(styles).not.toContain(".work-title-dot");
    expect(styles).toContain(':root[data-theme="light"] .working-chip.completed-work-title { color: #465a63; }');
  });

  it("identifies restored ToolGen work by topic rather than event source", () => {
    expect(source).toContain('(event.payload.topic as { name?: string } | undefined)?.name === "core.toolgen"');
    expect(source).not.toContain('event.source === "core_topic" && (event.payload.topic');
  });

  it("lets modal backdrops dismiss dialogs without closing while editing inside them", () => {
    expect(source).toContain('className="modal-backdrop" role="presentation" aria-label="Dismiss create session" onClick={closeIfIdle}');
    expect(source).toContain('className="modal-backdrop" role="presentation" aria-label="Dismiss ToolGen dialog" onClick={closeIfIdle}');
    expect(source).toContain('className="modal-backdrop" role="presentation" aria-label="Dismiss mem switch" onClick={closeIfIdle}');
    expect(source).toContain('onClick={(event) => event.stopPropagation()}');
    expect(source).toContain('const closeIfIdle = () => { if (!creating) onClose(); };');
    expect(source).toContain('const closeIfIdle = () => { if (!pending) onClose(); };');
    expect(source).toContain("const newSessionButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const FOCUSABLE_DIALOG_SELECTOR =");
    expect(source).toContain("textarea:not([disabled]), summary, [tabindex]");
    expect(source).toContain("function useDialogFocusTrap()");
    expect(source).toContain("activeElement.closest<HTMLElement>");
    expect(source).toContain('document.addEventListener("keydown", containFocus, true);');
    expect(source).toContain("useDialogFocusTrap();");
    expect(source).toContain("const closeNewSessionDialog = useCallback((restoreFocus = true) => {");
    expect(source).toContain('window.getComputedStyle(newSessionButton).visibility !== "hidden"');
    expect(source).toContain('newSessionButton.focus({ preventScroll: true });');
    expect(source).toContain('mobileSessionButtonRef.current?.focus({ preventScroll: true });');
    expect(source).toContain('const descriptionId = "new-session-dialog-description";');
    expect(source).toContain('const statusId = "new-session-dialog-status";');
    expect(source).toContain('const describedBy = creating ? `${descriptionId} ${statusId}` : descriptionId;');
    expect(source).toContain('aria-label="Create session" aria-describedby={describedBy}');
    expect(source).toContain('<p id={descriptionId}>Choose a workspace and optional runtime overrides for this session.</p>');
    expect(source).toContain('{creating && <p id={statusId} className="mem-validation" role="status" aria-live="polite">Creating session…</p>}');
    expect(source).toContain('onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); closeIfIdle(); } }}');
    expect(source).toContain('onClose={() => { if (!creatingSessionRef.current) closeNewSessionDialog(); }}');
    expect(source).toContain('onClose={() => { if (!pendingToolgenRequests.has(toolgenRequestKey(toolgenDialog.sessionId, toolgenDialog.turnId))) setToolgenDialog(null); }}');
    expect(source).toContain('onClose={() => { if (!pendingMemSwitch) closeMemSwitchDialog(); }}');
    expect(source).toContain('closeMemSwitchDialog();');
    expect(source).toContain("const validationText = pending");
    expect(source).toContain("Enter an absolute mem directory path on the Timem host.");
    expect(source).toContain("This is the current mem directory.");
    expect(source).toContain('const descriptionId = "mem-switch-dialog-description";');
    expect(source).toContain('const statusId = "mem-switch-dialog-status";');
    expect(source).toContain('const describedBy = validationText ? `${descriptionId} ${statusId}` : descriptionId;');
    expect(source).toContain('aria-label="Switch memory directory" aria-describedby={describedBy}');
    expect(source).toContain('<p id={descriptionId}>Switching mem stops current workers');
    expect(source).toContain('id={statusId} className="mem-validation" role="status" aria-live="polite"');
    expect(source).toContain('title={validationText || "Switch mem"}');
    expect(source).toContain('aria-label={validationText || "Switch mem"}');
    expect(source).toContain('if (event.key === "Enter" && !event.nativeEvent.isComposing && !pending && !invalid) { event.preventDefault(); onSwitch(cleaned); }');
    expect(source).toContain('className="modal-titlebar"');
    expect(source).toContain('aria-label="Close create session" disabled={creating} onClick={closeIfIdle}');
    expect(source).toContain('aria-label="Close ToolGen dialog" disabled={pending} onClick={closeIfIdle}');
    expect(source).toContain('<p id={descriptionId}>Timem will preserve reusable work');
    expect(source).toContain('id={statusId} className="toolgen-dialog-status" role="status" aria-live="polite"');
    expect(source).toContain("Starting ToolGen and opening a generating-tools task…");
    expect(source).toContain('aria-label="Close mem switch" disabled={pending} onClick={closeIfIdle}');
    expect(source).toContain('className={`primary ${creating ? "sending" : ""}`}');
    expect(source).toContain("const createDecision = sessionCreateDecision(displayName, workspaceDir, env, creating, memSwitching);");
    expect(source).toContain('closeNewSessionDialog();');
    expect(source).toContain('memSwitching={runtimeLocked}');
    expect(source).toContain("const submit = () => { if (createDecision.kind === \"send\") onCreate(createDecision.command); };");
    expect(source).toContain('if (event.key === "Enter" && !event.nativeEvent.isComposing)');
    expect(source).toContain('{creating ? <LoaderCircle size={16}/> : <Plus size={16}/>} {creating ? "Creating…" : "Create session"}');
    expect(source).toContain("const submit = () => { if (!pending) onSubmit(instruction.trim()); };");
    expect(source).toContain('if ((event.metaKey || event.ctrlKey) && event.key === "Enter" && !event.nativeEvent.isComposing)');
    expect(source).toContain("Cmd/Ctrl+Enter to generate; Escape closes before it starts.");
    expect(source).toContain('className={`primary ${pending ? "sending" : ""}`} disabled={pending} onClick={submit}');
    expect(source).toContain('{pending ? <LoaderCircle size={16}/> : <Wrench size={15}/>} {pending ? "Starting…" : "Generate tool"}');
    expect(source).toContain('className={`primary ${pending ? "sending" : ""}`} disabled={pending || invalid || cleaned === current}');
    expect(source).toContain('{pending && <LoaderCircle size={16}/>} {pending ? "Switching…" : "Switch mem"}');
    expect(styles).toContain(".decision-modal { width: min(520px, 100%); max-height: calc(100vh - 40px); display: flex; flex-direction: column; overflow: hidden;");
    expect(styles).toContain(".modal-titlebar { flex: none; min-width: 0; display: flex;");
    expect(styles).toContain(".modal-titlebar .icon-button { flex: none;");
    expect(styles).toContain(".decision-actions { flex: none; display: flex; flex-wrap: wrap;");
    expect(styles).toContain(".decision-actions button { min-width: 96px;");
    expect(styles).toContain(".decision-actions .primary { display: inline-flex; align-items: center; justify-content: center;");
    expect(styles).toContain(".decision-actions .primary.sending svg");
    expect(styles).toContain(".decision-actions button { flex: 1 1 130px; }");
    expect(styles).toContain(".session-modal-scroll { flex: 1; min-height: 0; overflow-y: auto;");
    expect(styles).toContain('.session-runtime-overrides summary::after { content: "Show";');
    expect(styles).toContain('.session-runtime-overrides[open] summary::after { content: "Hide";');
    expect(styles).toContain(".toolgen-dialog label { min-height: 0;");
    expect(styles).toContain(".toolgen-dialog textarea { min-height: 112px; max-height: min(260px, 38vh);");
    expect(styles).toContain(".toolgen-dialog-status");
    expect(styles).toContain(".toolgen-dialog-hint");
    expect(styles).toContain(".mem-validation");
    expect(styles).toContain(':root[data-theme="light"] .mem-validation');
  });

  it("renders ToolRepo browsing, search, rename and terminal-open controls", () => {
    expect(source).toContain('placeholder={session ? "Search names and code" : "Select a session first"}');
    expect(source).toContain('aria-label="Clear ToolRepo search"');
    expect(source).toContain('onClick={() => onSearchQueryChange("")}');
    expect(source).toContain('if (event.key === "Escape" && searchQuery)');
    expect(source).toContain("event.preventDefault(); event.stopPropagation(); onSearchQueryChange(\"\");");
    expect(source).toContain('const sortLabel = sort === "time" ? "recent update" : sort;');
    expect(source).toContain('const sortControlLabel = `Sort ToolRepo by ${sortLabel}`;');
    expect(source).toContain('title={sortControlLabel} aria-label={sortControlLabel}');
    expect(source).toContain('type: "tool_repo_detail"');
    expect(source).toContain('type: "tool_repo_rename"');
    expect(source).toContain('type: "tool_repo_open_terminal"');
    expect(source).toContain('const [pendingToolDetailKey, setPendingToolDetailKey] = useState("");');
    expect(source).toContain('const [pendingToolRenameKeys, setPendingToolRenameKeys] = useState<Set<string>>(() => new Set());');
    expect(source).toContain('pendingToolRenameIds={activeSession ? pendingToolIdsForSession(pendingToolRenameKeys, activeSession.session_id) : new Set()}');
    expect(source).toContain('setPendingToolRenameKeys((current) => removeToolKeysForSession(current, event.session_id));');
    expect(source).toContain('pendingToolDetailId={activeSession && pendingToolDetailKey.startsWith(`${activeSession.session_id}:`) ? pendingToolDetailKey.slice(activeSession.session_id.length + 1) : ""}');
    expect(source).toContain("const pendingTool = pendingToolDetailId ? sortedTools.find((tool) => tool.tool_id === pendingToolDetailId) : undefined;");
    expect(source).toContain('const loadingDetail = pendingToolDetailId === tool.tool_id;');
    expect(source).toContain('const renamingTool = pendingToolRenameIds.has(tool.tool_id);');
    expect(source).toContain('useEffect(() => {\n    setRenameToolId("");\n    setRenameValue("");\n    setContextMenu(null);\n  }, [session?.session_id]);');
    expect(source).toContain('useEffect(() => {\n    setContextMenu(null);\n  }, [searchQuery, sort, selectedTool?.summary.tool_id, tools.length]);');
    expect(source).toContain('const pendingToolDetailLabel = pendingTool ? `Loading ${pendingTool.name} tool directory` : "";');
    expect(source).toContain('aria-busy={loadingDetail || renamingTool || undefined}');
    expect(source).toContain('renamingTool ? "Renaming..." : loadingDetail ? "Loading details..."');
    expect(source).toContain('disabled={renamingTool}');
    expect(source).toContain('className="toolrepo-detail loading" aria-busy="true" aria-label={pendingToolDetailLabel}');
    expect(source).toContain('Reading tool directory…');
    expect(source).toContain('title={`Stop viewing ${pendingTool.name} details`}');
    expect(source).toContain('aria-label={`Stop viewing ${pendingTool.name} details`}');
    expect(source).toContain('className="toolrepo-detail-loading" role="status" aria-live="polite" aria-label={pendingToolDetailLabel}');
    expect(source).toContain('Reading directory tree...');
    expect(source).toContain('role="treeitem" tabIndex={0} aria-selected={selectedTool?.summary.tool_id === tool.tool_id} aria-expanded={expanded}');
    expect(source).toContain('setPendingToolDetailKey(`${activeSession.session_id}:${toolId}`);');
    expect(source).toContain('setPendingToolDetailKey((key) => key === `${event.session_id}:${event.detail.summary.tool_id}` ? "" : key);');
    expect(source).toContain("Tool detail failed");
    expect(source).toContain("Reconnect to Timem Web before opening tool details.");
    expect(source).toContain("Tool rename failed");
    expect(source).toContain("Open terminal failed");
    expect(source).toContain("Reconnect to Timem Web before renaming this tool.");
    expect(source).toContain("Reconnect to Timem Web before opening a tool directory.");
    expect(source).toContain("if (name && name !== tool.name && !onRenameTool(tool.tool_id, name)) return;");
    expect(source).toContain('if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); finishToolRename(tool); }');
    expect(source).toContain('if (event.key === "Escape") { event.preventDefault(); setRenameToolId(""); setRenameValue(""); }');
    expect(source).toContain('const renameKey = toolKey(activeSession.session_id, toolId);');
    expect(source).toContain('setPendingToolRenameKeys((current) => new Set(current).add(renameKey));');
    expect(source).toContain('setPendingToolRenameKeys((current) => { const next = new Set(current); next.delete(renameKey); return next; });');
    expect(source).toContain("在命令行中打开目录");
    expect(source).toContain("selectedTool?.summary.tool_id === toolId");
    expect(source).toContain("setSelectedTool(null)");
    expect(source).toContain("const expanded = selectedTool?.summary.tool_id === tool.tool_id;");
    expect(source).toContain('aria-expanded={expanded}');
    expect(source).toContain('onClick={() => { if (expanded) onCollapseTool(); else onSelectTool(tool.tool_id); }}');
    expect(source).toContain("const toolToggleLabel = expanded ? `收起 ${tool.name} 详情` : `展开 ${tool.name} 详情`;");
    expect(source).toContain('aria-label={toolToggleLabel}');
    expect(source).toContain('title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`}');
    expect(source).toContain('className="toolrepo-toggle-state">{expanded ? "收起" : "展开"}</em>');
    expect(source).toContain('const [pendingToolSearchKey, setPendingToolSearchKey] = useState("");');
    expect(source).toContain("setPendingToolSearchKey((key) => key === `${event.session_id}:${event.query}` ? \"\" : key);");
    expect(source).toContain("setPendingToolSearchKey(searchKey);");
    expect(source).toContain('if (!sendCommand({ type: "tool_repo_search", session_id: activeSession.session_id, query: toolSearchQuery, limit: 200 }))');
    expect(source).toContain('setPendingToolSearchKey((key) => key === searchKey ? "" : key);');
    expect(source).toContain('reportUiError("ToolRepo search failed", "Reconnect to Timem Web before searching saved tools.", activeSession.session_id);');
    expect(source).toContain('searchPending={!!activeSession && pendingToolSearchKey === `${activeSession.session_id}:${toolSearchQuery}`}');
    expect(source).toContain('className={searchPending ? "searching" : ""} aria-busy={searchPending}');
    expect(source).toContain('searchPending && <span className="toolrepo-search-pending" aria-hidden="true"/>');
    expect(source).toContain("event.session_id === activeSessionIdRef.current && toolSearchQueryRef.current.trim()");
    expect(source).toContain("return { ...current, [event.session_id]: event.tools };");
    expect(source).toContain('event.type === "tool_repo_search_result"');
    expect(source).toContain("!event.tools.some((tool) => tool.tool_id === selected.summary.tool_id)");
    expect(source).toContain("selectedTool.files.map");
    expect(source).toContain('title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`}');
    expect(source).toContain("title={selectedTool.summary.synopsis}");
    expect(source).toContain('title={`${file.path} · ${formatBytes(file.bytes)}`}');
    expect(source).toContain("if (selectedTool?.summary.tool_id === toolId)");
    expect(source).toContain("setSelectedTool(null);");
    expect(source).toContain('const toolRepoEmptyTitle = !session ? "No active session" : searchPending ? "Searching ToolRepo…" : hasToolSearch ? "No matching tools" : "No reusable tools yet";');
    expect(source).toContain('Searching tool names and file contents. Results will update automatically.');
    expect(source).toContain('className={`toolrepo-empty ${searchPending ? "searching" : ""}`} aria-label={`${toolRepoEmptyTitle}. ${toolRepoEmptyText}`} aria-busy={searchPending || undefined}');
    expect(source).toContain("const toolRepoResultText = !session");
    expect(source).toContain('searchPending');
    expect(source).toContain('"Searching..."');
    expect(source).toContain('`${sortedTools.length} of ${session.tools.length} tools`');
    expect(source).toContain('`${sortedTools.length} tool${sortedTools.length === 1 ? "" : "s"}`');
    expect(source).toContain('className="toolrepo-result-count" aria-live="polite"');
    expect(source).toContain("Select or create a session to browse its ToolRepo.");
    expect(source).toContain('placeholder={session ? "Search names and code" : "Select a session first"}');
    expect(source).toContain("disabled={!session} onChange");
    expect(source).toContain("clear search to show all saved tools");
    expect(source).toContain('aria-label="Tool directory tree"');
    expect(source).toContain('aria-label="Collapse tool detail"');
    expect(source).toContain('if (event.key === "Escape") setContextMenu(null);');
    expect(source).toContain('const contextMenuActionRef = useRef<HTMLButtonElement>(null);');
    expect(source).toContain('contextMenuActionRef.current?.focus({ preventScroll: true });');
    expect(source).toContain("Math.max(8, Math.min(event.clientX, window.innerWidth - 220))");
    expect(source).toContain("Math.max(8, Math.min(event.clientY, window.innerHeight - 76))");
    expect(source).toContain('className="toolrepo-context-menu" role="menu" aria-label="Tool actions"');
    expect(source).toContain('onKeyDownCapture={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setContextMenu(null); } }}');
    expect(source).toContain('<button ref={contextMenuActionRef} type="button" role="menuitem" onClick={() => { onOpenTerminal(contextMenu.toolId); setContextMenu(null); }}>');
    expect(source).toContain('className="toolrepo-detail-collapse"');
    expect(source).toContain(">收起详情</button>");
    expect(source).not.toContain('className="toolrepo-detail-footer"');
    expect(source).not.toContain("<MarkdownContent text={selectedTool.readme}");
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-item-main > svg");
    expect(styles).toContain(".toolrepo-toggle-state");
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-toggle-state");
    expect(styles).toContain(".toolrepo-item.loading-detail");
    expect(styles).toContain(".toolrepo-item.renaming-tool");
    expect(styles).toContain(".toolrepo-edit:disabled");
    expect(styles).toContain(".toolrepo-item.loading-detail .toolrepo-item-main small");
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-open");
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-edit");
    expect(styles).toContain(".toolrepo-controls label button");
    expect(styles).toContain(".toolrepo-controls label.searching");
    expect(styles).toContain(".toolrepo-search-pending");
    expect(styles).toContain(".toolrepo-empty.searching svg");
    expect(styles).toContain(".toolrepo-result-count { flex: none; padding: 0 12px 8px;");
    expect(styles).toContain(".toolrepo-browser { min-height: 0; flex: 1; display: flex; flex-direction: column; overflow: hidden;");
    expect(styles).toContain(".toolrepo-list { min-height: 0; flex: 1 1 auto; display: grid; align-content: start; overflow: auto;");
    expect(styles).toContain(".toolrepo-detail { flex: none; min-height: 0; max-height: 260px;");
    expect(styles).toContain(".toolrepo-detail.loading");
    expect(styles).toContain(".toolrepo-detail-loading");
    expect(styles).toContain(".toolrepo-detail button { flex: none; min-height: 26px;");
    expect(styles).toContain(".toolrepo-detail > header button:not(.toolrepo-detail-collapse)");
    expect(styles).toContain(".toolrepo-detail button.toolrepo-detail-collapse { width: auto; padding: 0 8px; }");
    expect(styles).toContain(".toolrepo-files { flex: none; display: grid; max-height: 180px;");
    expect(styles).not.toContain(".toolrepo-detail-footer");
    expect(styles).toContain(".toolrepo-context-menu { position: fixed; z-index: 40; max-width: min(260px, calc(100vw - 16px));");
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-empty');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-empty.searching svg');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-result-count');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-controls label.searching');
    expect(styles).toContain("@keyframes search-pending-pulse");
    expect(styles).toContain(".toolrepo-empty.searching svg, .upload-dot");
    expect(styles).toContain(".activity-empty strong");
    expect(styles).toContain(':root[data-theme="light"] .activity-empty strong');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-detail > header strong');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-toggle-state');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-item.selected .toolrepo-toggle-state');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-detail-loading');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-item.loading-detail');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-files > div');
    expect(styles).not.toContain(".toolrepo-readme");
  });

  it("makes ToolRepo tree items keyboard navigable without hijacking nested controls", () => {
    expect(source).toContain('role="treeitem" tabIndex={0}');
    expect(source).toContain('event.target.closest("button, input, select, textarea")');
    expect(source).toContain('event.key === "Enter" || event.key === " "');
    expect(source).toContain('event.key === "ArrowRight" && !expanded');
    expect(source).toContain('event.key === "ArrowLeft" && expanded');
    expect(source).toContain('event.key === "Escape" && expanded');
    expect(styles).toContain(".toolrepo-item:focus-visible");
  });

  it("provides a keyboard reachable ToolRepo terminal action on each tool row", () => {
    expect(source).toContain('className="toolrepo-open"');
    expect(source).toContain('title={`Open ${tool.name} directory in terminal`}');
    expect(source).toContain('aria-label={`Open ${tool.name} directory in terminal`}');
    expect(source).toContain("onClick={() => onOpenTerminal(tool.tool_id)}");
    expect(styles).toContain("grid-template-columns: minmax(0, 1fr) 26px 26px;");
    expect(styles).toContain(".toolrepo-open, .toolrepo-edit");
    expect(styles).toContain(".toolrepo-open:focus-visible");
  });

  it("shows readable tool names and invocation previews in the working pane", () => {
    expect(source).toContain("function toolInvocationPreview");
    expect(source).toContain("activity.detail?.split");
    expect(source).toContain("const detail = activity.detail?.trim();"); expect(source).toContain("const code = activity.code?.trim();"); expect(source).toContain("const hasExpandableDetail = !!detail || !!code;");
    expect(source).toContain('const running = status === "running" || status === "background_running";');
    expect(source).toContain("const [open, setOpen] = useState(false);");
    expect(source).toContain('if (!hasExpandableDetail) return <div className={`tool-activity tool-activity-static ${bashActivity ? "bash-activity" : ""} ${running ? "running" : "settled"}`} aria-busy={running || undefined}>');
    expect(source).toContain("const toolName = toolDisplayName(activity.tool_name || activity.title);");
    expect(source).toContain('const summaryLabel = `${open ? "收起" : "展开"}工具详情：${toolName}`;');
    expect(source).toContain("const summaryContent = <>");
    expect(source).toContain('className="tool-activity-command" title={invocationPreview}');
    expect(source).not.toContain("!hasExpandableDetail && invocationPreview");
    expect(source).toContain('open={open} onToggle={(event) => setOpen(event.currentTarget.open)}');
    expect(source).toContain('aria-busy={running || undefined} open={open}');
    expect(source).toContain('aria-label={summaryLabel}');
    expect(source).not.toContain("tool-activity-collapse");
    expect(styles).not.toContain(".tool-activity-collapse");
    expect(styles).toContain(".tool-activity-body { max-height: 280px; overflow: auto; margin: 0 0 5px 22px; padding: 0; border: 0; }");
    expect(styles).toContain(".tool-activity-body .turn-work-detail { padding: 2px 0 3px; }"); expect(styles).toContain(".tool-activity-body .code-block { margin: 0;"); expect(source).toContain('{detail && <div className="turn-work-detail"><MarkdownContent text={detail}/></div>}'); expect(source).toContain('<MarkdownContent text={fencedCode(activity.code_language ?? "text", code)} />');
    expect(styles).toContain(".tool-activity summary:focus-visible { background: #1f1f1f; box-shadow: inset 2px 0 0 #4d8fd7; }");
    expect(styles).toContain(':root[data-theme="light"] .tool-activity summary:focus-visible { background: #edf4f7; box-shadow: inset 2px 0 0 #2c7bbf; }');
    expect(source).toContain("toolDisplayName(activity.tool_name || activity.title)");
    expect(source).toContain('if (status === "background_running") return "background running";');
    expect(source).toContain('if (status === "timeout") return "timed out";');
    expect(styles).toContain(".tool-activity-static");
    expect(styles).toContain("grid-template-columns: 16px max-content max-content minmax(0, 1fr);");
    expect(viewModelSource).toContain('if (name === "run_bash") return "Bash";');
    expect(viewModelSource).toContain('if (name === "memmgr") return "MemMgr";');
    expect(viewModelSource).toContain('if (name === "capmgr") return "CapMgr";');
    expect(viewModelSource).toContain('if (name === "self_tool") return "Self Tool";');
  });

  it("carries the live working marker into the completed Thought Action chip", () => {
    expect(styles).toContain(".turn-assistant-frame.working .working-chip { font-size: 14px; font-weight: 720; color: #7ebce8; letter-spacing: 0; }");
    expect(styles).toContain(".turn-assistant-frame.working .working-chip .pulse { width: 8px; height: 8px; background: #3485dc; box-shadow: 0 0 0 4px #3485dc24; }");
    expect(source).toContain("working-chip work-title-chip work-collapse-toggle");
    expect(source).toContain('turn.state === "working" ? " active-work-title" : " completed-work-title"');
    expect(source).not.toContain('<span className="work-title-dot" aria-hidden="true"/>');
    expect(source).not.toContain('isToolGenTurn ? <Wrench size={11}/> : <span className="pulse"/>');
    expect(styles).toContain(".work-collapse-toggle:hover { color: #f0f0f0; }");
    expect(styles).not.toContain(".work-collapse-toggle:hover { border-color:");
    expect(styles).not.toContain(".working-chip.interrupted-work-title { border-color:");
    expect(styles).not.toContain(".working-chip.completed-work-title.toolgen-completed-title { border-color:");
    expect(styles).toContain(".working-chip.work-title-chip { min-height: 0; padding: 0; border: 0; border-radius: 0; background: transparent; }");
    expect(styles).toContain(".turn-assistant-frame.working .working-chip.active-work-title { min-width: 0; color: #8fc9f1; font-size: 11px; font-weight: 700; letter-spacing: 0; }");
    expect(source).toContain('<span className="working-label">working</span>');
    expect(styles).toContain(".turn-assistant-frame.working .working-label {");
    expect(styles).toContain("background-size: 320% 100%;");
    expect(styles).toContain("animation: working-label-sweep 2.8s linear infinite;");
    expect(styles).toContain("@keyframes working-label-sweep");
    expect(styles).toContain("from { background-position: 100% 50%; }");
    expect(styles).toContain("to { background-position: -100% 50%; }");
    expect(styles).toContain("will-change: background-position;");
    expect(styles).not.toContain("70%, 100% { background-position: -100% 50%; }");
    expect(styles).toContain(':root[data-theme="light"] .turn-assistant-frame.working .working-label {');
    expect(styles).toContain("@media (prefers-reduced-motion: reduce) {");
    expect(styles).toContain("color: #8fc9f1;");
    expect(styles).toContain("background: none;");
    expect(styles).toContain("animation: none;");
    expect(styles).toContain(':root[data-theme="light"] .turn-assistant-frame.working .working-label { color: #286a9b; }');
    expect(styles).toContain(".turn-work-item { grid-template-columns: 16px minmax(0, 1fr); gap: 6px; padding: 6px 6px; color: #aaa; font-size: 12px;");
    expect(source).toContain('<span className="activity-thinking-dot" aria-hidden="true"/>');
    expect(source).not.toContain('activity.tone === "thinking" ? "💡"');
    expect(source).toContain('activity.kind === "free_talk" ? " free-talk" : ""');
    expect(styles).toContain(".activity-thinking-dot { width: 5px; height: 5px; border-radius: 50%; background: #111; }");
    expect(styles).toContain(".turn-work-item.free-talk .turn-work-detail { font-size: 90%; }");
    expect(styles).toContain(".turn-work-item.free-talk .turn-work-detail .message-content { font-size: inherit; }");
    expect(styles).toContain(".worker-role-editor input::placeholder, .worker-role-editor textarea::placeholder { font-size: inherit; }");
expect(source).toContain('className={`worker-role-editor ${editingId ? "editing" : "creating"}`}');
expect(styles).toContain(".worker-role-editor input { height: 34px; padding: 0 9px; font-size: var(--content-size); line-height: 1.4; }");
expect(styles).toContain(".worker-role-editor textarea { min-height: 112px; resize: vertical; padding: 9px; font-size: var(--content-size); line-height: 1.5; }");
expect(styles).toContain(".worker-role-editor > div button { min-height: 29px; padding: 0 10px; font-size: var(--content-size); }");
expect(styles).not.toContain("font: 12px/1.45 var(--ui-font);");
expect(styles).toContain(".worker-role-editor.editing textarea { height: clamp(160px, 30dvh, 360px);");
expect(source).toContain('className="worker-role-action worker-role-edit"');
expect(source).toContain('className={`worker-role-action worker-role-delete ${deleteConfirmId === role.id ? "confirm-delete" : ""}`}');
expect(styles).toContain(".worker-role-panel .worker-role-action {");
expect(styles).toContain(".worker-role-panel .worker-role-edit:hover:not(:disabled)");
expect(styles).toContain(".worker-role-panel .worker-role-delete:hover:not(:disabled)");
expect(styles).toContain(':root[data-theme="light"] .worker-role-panel .worker-role-action');
expect(styles).toContain(':root[data-theme="light"] .worker-role-panel .worker-role-delete.confirm-delete');
  });

  it("shows a live elapsed duration only while the turn is working", () => {
    expect(source).toContain("const [workingElapsedMs, setWorkingElapsedMs] = useState(() => Math.max(0, Date.now() - turn.created_at_ms));");
    expect(source).toContain('if (turn.state !== "working") return;');
    expect(source).toContain("const timer = window.setInterval(updateElapsed, 1_000);");
    expect(source).toContain("return () => window.clearInterval(timer);");
    expect(source).toContain('className="working-elapsed" aria-hidden="true"');
    expect(styles).toContain(".working-elapsed { min-width: 3.5ch;");
    expect(styles).toContain("font-variant-numeric: tabular-nums;");
  });

  it("renders Thought Action as an independent trigger attached to a softly tinted process panel", () => {
    expect(styles).toContain(".turn-assistant-frame { position: relative; overflow: visible; padding-left: 0; border: 0; border-radius: 0; background: transparent;");
    expect(styles).toContain('.turn-user-frame { width: fit-content; max-width: min(86%, 680px); margin: 0 0 11px auto; }');
    expect(styles).not.toContain('.turn-user-content::after');
    expect(styles).toContain('.turn-work-panel { position: relative; z-index: 1; margin-top: -4px; overflow: hidden; border-radius: 11px; background: #353535; }');
    expect(styles).not.toContain('.turn-work-panel::before');
    expect(styles).not.toContain('.turn-assistant-heading::after');
    expect(styles).toContain('.turn-work-scroll { padding: 14px 10px 7px; }');
    expect(styles).toContain(':root[data-theme="light"] .turn-work-panel { background: #fafbfb; }');
    expect(styles).not.toContain(':root[data-theme="light"]\n:root[data-theme="light"] .turn-work-panel');
    expect(styles).not.toContain(':root[data-theme="light"] :root[data-theme="light"] .turn-work-panel');
    expect(styles).not.toContain(':root[data-theme="light"] .turn-user-content::after');
    expect(source).toContain('{workStreamVisible && <div className="turn-work-panel">');
  });

  it("keeps ToolGen retrospective attached to its final delivery", () => {
    expect(source).toContain("function ToolGenNotice");
    expect(source).toContain('<details className={`toolgen-notice');
    expect(source).toContain("const [open, setOpen] = useState(false);");
    expect(source).toContain("const collapse = () => setOpen(false);");
    expect(source).toContain("onToggle={(event) => setOpen(event.currentTarget.open)}");
    expect(source).toContain('const summaryLabel = `${open ? "收起" : "展开"} ToolGen 详情${activity.title ? `：${activity.title}` : ""}`;');
    expect(source).toContain('aria-label={summaryLabel}');
    expect(source).toContain('className="toolgen-collapse"');
    expect(source).toContain('className="toolgen-collapse top" title="Collapse ToolGen details" aria-label="Collapse ToolGen details" onClick={collapse}>收起详情</button>');
    expect(source).toContain('className="toolgen-collapse" title="Collapse ToolGen details" aria-label="Collapse ToolGen details" onClick={collapse}>收起详情</button>');
    expect(styles).toContain(".toolgen-notice[open] summary svg");
    expect(styles).toContain('content: "收起"');
    expect(styles).toContain(".toolgen-collapse");
    expect(styles).toContain(".toolgen-collapse.top");
    expect(styles).toContain(':root[data-theme="light"] .toolgen-notice');
    expect(styles).toContain(".toolgen-notice.published");
    expect(styles).toContain(".toolgen-notice.published summary::before");
    expect(styles).toContain(':root[data-theme="light"] .toolgen-notice.published');
    expect(styles).toContain(':root[data-theme="light"] .toolgen-collapse');
    expect(source).not.toContain("turn.completion?.toolgen_retrospect");
  });

  it("shows refined transient recovery controls beside the working label", () => {
    expect(source).toContain(
      "const modelRetryStatus = useMemo(() => activeModelRetryStatus(turn), [turn]);",
    );
    expect(source).toContain("model-retry-status");
    expect(source).toContain("modelRetryStatus.label");
    expect(source).toContain("modelRetryStatus.progress");
    expect(source).toContain(
      'kind === "model_request" || kind === "model_response" || kind === "model_retry"',
    );
    expect(source).toContain(
      'kind === "model_request" || kind === "model_response" || kind === "model_retry"',
    );
    expect(viewModelSource).toContain('case "core.model.repair":');
    expect(styles).toContain(".model-retry-status");
    expect(styles).toContain(".model-retry-detail");
    expect(styles).toContain("font-size: 10px");
    expect(styles).toContain("font-size: 9px");
    expect(styles).toContain("font-size: 11px");
    expect(styles).toContain("min-height: 20px");
  });

  it("uses dnd-kit sortable motion for queued-message reordering", () => {
    expect(source).toContain('from "@dnd-kit/core"');
    expect(source).toContain('from "@dnd-kit/sortable"');
    expect(source).toContain("<DndContext");
    expect(source).toContain("<SortableContext");
    expect(source).toContain("<DragOverlay");
    expect(source).toContain("onDragOver={previewQueuedMessageDrag}");
    expect(source).toContain("queuedMessageOverId");
    expect(source).toContain("draggedQueuedMessagePosition");
    expect(source).toContain("verticalListSortingStrategy");
    expect(source).toContain("sortableKeyboardCoordinates");
    expect(source).toContain("useSortable");
    expect(source).toContain("CSS.Transform.toString");
    expect(source).not.toContain("draggable=");
    expect(source).not.toContain("dataTransfer");
    expect(styles).toContain(".queued-message.dragging");
    expect(styles).toContain(".queued-message-overlay");
    expect(styles).toContain("will-change: transform");
    expect(styles).toContain("@media (prefers-reduced-motion: reduce)");
  });

  it("does not expose internal model transport bookkeeping or duplicate activity labels", () => {
    expect(source).toContain('if (kind === "model_request" || kind === "model_response" || kind === "model_retry") return null;');
    expect(source).not.toContain('setActivities((current) => [activity');
    expect(source).not.toContain("Model completed a response");
    expect(source).not.toContain("LIVE ACTIVITY");
    expect(source).not.toContain("Working view");
    expect(source).not.toContain("renderToolInvocation");
    expect(viewModelSource).not.toContain('title: "Work instructions"');
    expect(source).toContain('activity.tone === "warning" ? "⚠️"');
    expect(source).not.toContain('activity.tone === "warning" ? "!"');
  });

  it("uses the Markdown highlighter for final answers and Bash activity commands", () => {
    expect(source).toContain('import rehypeHighlight from "rehype-highlight";');
    expect(source).toContain("rehypePlugins={[rehypeHighlight]}");
    expect(source).toContain("fencedCode(activity.code_language ?? \"text\", activity.code)");
    expect(viteConfig).toContain('highlighting: ["highlight.js", "rehype-highlight"]');
  });

  it("renders Bash activity commands with the interface font at normal weight", () => {
    expect(source).toContain('const bashActivity = activity.tool_name === "run_bash";');
    expect(source).toContain('bashActivity ? "bash-activity" : ""');
    expect(styles).toContain(".tool-activity.bash-activity .tool-activity-command");
    expect(styles).toContain(".tool-activity.bash-activity .tool-activity-body .code-block code *");
    expect(styles).toContain(".tool-activity-command { min-width: 0; grid-column: 4; justify-self: start; overflow: hidden; color: #737373;");
    expect(styles).toContain("text-overflow: ellipsis; white-space: nowrap;");
    expect(styles).toContain("font-family: var(--ui-font);");
    expect(styles).toContain("font-weight: 400;");
  });

  it("renders completion telemetry below final answers", () => {
    expect(source).toContain("attachTurnCompletion(session, event.outcome.message_id");
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain("<FinalAnswerDelivery text={turn.final_answer}");
    expect(source).not.toContain('className="turn-final-toolbar"');
    expect(source).toContain('className="final-answer-actions"');
    expect(source).toContain('const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(text, {');
    expect(source).toContain('copied: "Answer copied"');
    expect(source).toContain('failed: "Copy answer failed"');
    expect(source).toContain('const copyClass = copyState === "copied" ? "copy-success" : copyState === "failed" ? "copy-failed" : "";');
    expect(source).toContain('className={`final-copy ${copyClass}`}');
    expect(source).toContain('aria-label={copyLabel}');
    expect(source).toContain('title={copyLabel}');
    expect(source).toContain('aria-label={copyLabel} onClick={() => void copy()}>{copyState === "copied"');
    expect(source).not.toContain('<span aria-live="polite">{copyLabel}</span>');
    expect(source).toContain('answerActions={answerActions}');
    expect(source).toContain('{answerActions}');
    expect(source).toContain('className="chat-message-delete assistant-message-delete"');
    expect(source).toContain('<figcaption><span title={language}>{language}</span>');
    expect(source).toContain("navigator.clipboard.writeText(text)");
    expect(source).toContain("async function copyTextToClipboard(text: string)");
    expect(source).toContain('document.createElement("textarea")');
    expect(source).toContain('textarea.setAttribute("readonly", "true")');
    expect(source).toContain('document.execCommand("copy")');
    expect(source).toContain("document.body.removeChild(textarea)");
    expect(source).toContain("window.getSelection()?.removeAllRanges()");
    expect(source).toContain("window.clearTimeout(resetTimerRef.current)");
    expect(source).toContain('setCopyState("idle");\n  }, [text]);');
    expect(source).toContain("<CompletionCard completion={completion}");
    expect(styles).toContain(".completion-card");
    expect(styles).toContain(".final-answer-actions");
    expect(styles).not.toContain(".turn-final-toolbar");
    expect(styles).toContain(".final-copy");
    expect(styles).toContain(".final-copy.copy-success, .code-block figcaption button.copy-success");
    expect(styles).toContain(".final-copy.copy-failed, .code-block figcaption button.copy-failed");
    expect(styles).toContain(':root[data-theme="light"] .final-copy');
    expect(styles).toContain(':root[data-theme="light"] .final-copy.copy-success');
    expect(styles).toContain(':root[data-theme="light"] .final-copy.copy-failed');
    expect(styles).not.toContain("::root");
    expect(styles).toContain(".completion-card { gap: 0 7px;");
    expect(styles).toContain("font-size: 10px; overflow-wrap: anywhere;");
    expect(styles).toContain(".completion-card span { min-width: 0; padding: 0; border: 0; white-space: normal; }");
    expect(styles).toContain(".completion-card .completion-status { white-space: normal; overflow-wrap: anywhere; }");
    expect(styles).toContain(".turn-final-delivery");
    expect(source).toContain("function completionFactTitle");
    expect(source).toContain('title={completionFactTitle(label, completion, stats) ?? `${label}: ${value}`}');
    expect(source).toContain('`${stats.prompt_tokens} input tokens`');
    expect(source).toContain('`${stats.completion_tokens} output tokens`');
    expect(source).toContain('`${stats.cached_tokens} cached input tokens`');
    expect(source).toContain('["Compact", formatOptionalTokens(stats.shrunk_tokens)]');
    expect(source).not.toContain('["Shrunk", formatTokens(stats.shrunk_tokens)]');
  });

  it("binds assistant-ui running state to the authoritative session lifecycle", () => {
    expect(source).toContain('isRunning: activeSession?.state === "working"');
    expect(source).toContain('cancelled ? "Cancelled" : "Completed"');
    expect(viewModelSource).toContain('worker.state === "working"');
  });

  it("deduplicates rapid cancel clicks and clears the guard when a turn finishes", () => {
    expect(source).toContain("const cancellingSessionIds = useRef<Set<string>>(new Set());");
    expect(source).toContain("const [cancellingSessionIdSet");
    expect(source).toContain('if (cancellingSessionIds.current.has(activeSession.session_id)) return;');
    expect(source).toContain('cancellingSessionIds.current.add(activeSession.session_id);');
    expect(source).toContain('cancellingSessionIds.current.delete(event.session_id);');
    expect(source).toContain('{isCancelling ? "Stopping…" : "Stop"}');
    expect(source).toContain("const cancelActiveSessionTurn = async () =>");
    expect(source).toContain('pauseQueuedMessages(activeSessionId, "用户停止了当前任务", "user")');
    expect(source).not.toContain("clearSessionQueuedMessages(previous, activeSessionId)");
    expect(source).toContain("releaseSessionQueuedMessageClaims(queuedMessageClaimsRef.current, sessionId)");
    expect(source).toContain("onClick={() => void cancelActiveSessionTurn()}");
  });

  it("blocks send while cancellation is still in flight", () => {
    const start = source.indexOf("const sendTextForSession = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(sendText).toContain("cancellingSessionIds.current.has(targetSession.session_id)");
    expect(sendText).toContain("Cancellation in progress");
    expect(sendText).toContain("return false;");
  });

  it("keeps sending enabled during a working turn by bypassing assistant-ui Send", () => {
    const start = source.indexOf("const sendTextForSession = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(source).toContain("composerSendDecision");
    expect(viewModelSource).toContain('session.state === "working"');
    expect(viewModelSource).toContain('{ type: "turn_supplement"');
    expect(sendText).toContain("composerSendDecision(");
    expect(source).toContain('value={draft}');
    expect(source).toContain('onSubmit={(event) => { event.preventDefault(); submitDraft(); }}');
    expect(source).toContain('type="submit" title={effectiveSendLabel}');
    expect(source).not.toContain("ComposerPrimitive.Send");
  });

  it("uses synchronous pending guards for rapid repeated browser clicks", () => {
    expect(source).toContain("creatingSessionRef.current");
    expect(source).toContain("const [draftsBySession, setDraftsBySession]");
    expect(source).toContain("const submittingDraftSessionIdsRef = useRef<Set<string>>(new Set());");
    expect(source).toContain("const directSubmissionsRef = useRef<Map<string, {");
    expect(source).toContain("reserveSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId, draftsBySession)");
    expect(source).toContain("finishSessionDraftSubmission(submittingDraftSessionIdsRef, draftsBySession, reserved.sessionId, reserved.text, sent)");
    expect(source).toContain("directSubmissionsRef.current.set(reserved.sessionId, {");
    expect(source).toContain('event.command_id.startsWith("submit-") && event.status === "rejected"');
    expect(source).toContain("rejectedDirectDrafts.set(sessionId, submission.text)");
    expect(source).toContain("onRolesConsumed(session.session_id, submission.roleIds)");
    expect(source).toContain("sent = !!reliableStorageScope\n        && saveQueuedMessages(window.localStorage, reliableStorageScope, nextQueues, queuedMessagesBySessionRef.current);");
    expect(source).toMatch(/if \(sent\) \{[\s\S]*?updateQueuedMessages\(\(\) => nextQueues\);[\s\S]*?\}/);
    expect(source).toContain("shouldDirectManualMessage(activeSession.state, existingQueue.length, !!queuedMessagesPause)");
    const submitDraftStart = source.indexOf("const submitDraft = () =>");
 const submitDraftEnd = source.indexOf("const submitDraftAsSupplement = () =>", submitDraftStart);
 const submitDraftSource = source.slice(submitDraftStart, submitDraftEnd);
 expect(submitDraftSource).not.toContain("resumeQueuedMessages()");
    expect(source).toContain("sessionIds={sessions.map((session) => session.session_id)}");
    expect(source).toContain("pruneSessionDrafts(current, sessionIds)");
    expect(source).toContain("pruneSessionSubmissionLocks(submittingDraftSessionIdsRef, sessionIds)");
    expect(source).toContain("disabled={!activeSession || !draft.trim() || submittingDraft || uploadingAttachment || sessionInteractionLocked}");
    expect(source).toContain("pendingAttachmentRemoveIdsRef");
    expect(source).toContain("pendingDecisionKeysRef");
    expect(source).toContain("pendingRenameSessionIdsRef");
    expect(source).toContain("pendingRuntimeKeysRef");
    expect(source).toContain("addPendingKey(");
    expect(source).toContain("clearAllPendingCommands");
    expect(source).toContain('setPendingToolSearchKey("");');
    expect(source).toContain('setPendingToolDetailKey("");');
    expect(source).toContain("setSelectedTool(null);");
  });

  it("exposes earlier-history loading as a busy button state", () => {
    expect(source).toContain('className={`load-history ${loadingHistory ? "loading" : ""}`}');
    expect(source).toContain('aria-label={historyButtonLabel} aria-live="polite" aria-busy={loadingHistory || undefined}');
    expect(source).toContain('disabled={loadingHistory || sessionInteractionLocked}');
    expect(source).toContain('loadingHistory && <LoaderCircle size={13} aria-hidden="true"/>');
  });

  it("locks old-session interactions while a mem switch snapshot is pending", () => {
    expect(source).toContain("sessionInteractionLocked={runtimeLocked}");
    expect(source).toContain("disabled={runtimeLocked}");
    expect(source).toContain("if (pendingMemSwitch) return;");
    expect(source).toContain('reason === "mem_switching"');
    expect(source).toContain("disabled={!activeSession || sessionInteractionLocked}");
    expect(source).toContain("disabled={!activeSession || !draft.trim() || submittingDraft || uploadingAttachment || sessionInteractionLocked}");
    expect(source).toContain("disabled={loadingHistory || sessionInteractionLocked}");
    expect(source).toContain("disabled={removing || sessionInteractionLocked}");
    expect(source).toContain("const disabled = pending || locked;");
    expect(source).toContain("disabled={disabled}");
    expect(source).toContain('const runtimeReady = connected && snapshotReady;');
    expect(source).toContain('const runtimeLocked = pendingMemSwitch || !runtimeReady;');
    expect(source).toContain('const newSessionLabel = runtimeLocked ? "Session controls are temporarily locked" : "New session";');
    expect(source).toContain('ref={newSessionButtonRef} className="new-session" title={newSessionLabel} aria-label={newSessionLabel} disabled={runtimeLocked}');
    expect(source).toContain('title={runtimeLocked ? "Session controls are temporarily locked" : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers`}');
    expect(source).toContain('aria-label={runtimeLocked ? `Workers locked while the runtime synchronizes for ${session.display_name}`');
    expect(source).toContain('aria-expanded={expandedSessionIds.has(session.session_id)} disabled={runtimeLocked}');
    expect(source).toContain('aria-label={runtimeLocked ? `${session.display_name} locked while the runtime synchronizes` : renamingSession ? `${session.display_name} rename is being saved` : undefined}');
    expect(source).toContain('disabled={runtimeLocked} onClick={() => { setActiveSessionId(session.session_id);');
    expect(source).toContain('onDoubleClick={() => { if (!runtimeLocked && renamingSessionId !== session.session_id) beginRename(session); }}');
    expect(source).toContain("sessionRenameDecision(");
    expect(styles).toContain(".session:disabled, .session-expand:disabled");
    expect(styles).toContain(".session:disabled:hover, .session-expand:disabled:hover");
    expect(viewModelSource).toContain('"mem_switching"');
    expect(viewModelSource).toContain('"already_pending"');
  });

  it("clears stale pending browser guards when a reconnect snapshot arrives", () => {
    const helloStart = source.indexOf('if (event.type === "hello")');
    const helloEnd = source.indexOf('if (event.type === "session_created")', helloStart);
    const helloBranch = source.slice(helloStart, helloEnd);
    expect(helloBranch).toContain("clearAllPendingCommands();");
    expect(helloBranch).toContain("setDecisions(decisionsFromSessions(event.snapshot.sessions));");
    expect(helloBranch).toContain("applySnapshot(event.snapshot);");
    expect(helloBranch).toContain("setSnapshotReady(true);");
    expect(source).toContain('if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return reliable;');
    expect(source).toContain("hasConnectedOnce = true;");
    expect(source).toContain("disconnectNoticeShown = false;");
    expect(source).toContain("retryAttempt = 0;");
    expect(source).toContain("setConnected(true);");
    expect(source).toContain("setRuntimeEverConnected(true);");
    expect(source).toContain("setSnapshotReady(false);");
    expect(source).toContain('setConnected(false);\n        setSnapshotReady(false);');
  });

  it("moves active selection to a live session when a reconnect or mem snapshot swaps sessions", () => {
    expect(viewModelSource).toContain("resolveActiveSessionId");
    expect(source).toContain("resolveActiveSessionId(current, snapshot.sessions)");
    expect(source).not.toContain("current || snapshot.sessions[0]?.session_id");
  });

  it("remounts the assistant-ui thread root when switching sessions", () => {
    expect(source).toContain('<ThreadPrimitive.Root key={activeSessionId ?? "no-session"} className="aui-thread">');
  });

  it("renders live task usage and session context without replacing final telemetry", () => {
    expect(source).toContain("<HeaderContextUsage session={activeSession}");
    expect(source).toContain("<LiveTurnUsage turn={turn}");
    expect(source).toContain('aria-label="Current task token usage"'); expect(styles).toContain("animation: live-turn-usage-breathe 2.8s ease-in-out infinite;"); expect(styles).toContain("@keyframes live-turn-usage-breathe { 50% { opacity: .48; } }"); expect(styles).toContain(".live-turn-usage, .pulse, .connection.offline");
    expect(source).toContain('const level = ratio >= 90 ? "critical" : ratio >= 75 ? "warning" : "normal";');
    expect(source).toContain('className={`header-context ${level}`}');
    expect(source).toContain('const ratio = limit ? Math.min(100, Math.ceil((usage?.prompt_tokens ?? 0) * 100 / limit)) : 0;');
    expect(source).toContain('const contextUsageLabel = limit');
    expect(source).toContain('`Context usage ${ratio}% · ${formatTokens(usage?.prompt_tokens ?? 0)} / ${formatTokens(limit)} input tokens`');
    expect(source).toContain('title={contextUsageLabel} aria-label={contextUsageLabel}');
    expect(source).toContain('className="header-context-meter"');
    expect(source).toContain('style={{ width: `${ratio}%` }}');
    expect(source).toContain('`${ratio}%/${formatTokens(limit)}`');
    expect(source).toContain('{limit ? `${ratio}%/${formatTokens(limit)}` : "—"}');
    expect(source).toContain('role="status" aria-live="polite"');
    expect(source).toContain('className={`turn-work-scroll ${pendingUpdates > 0 ? "has-pending-updates" : ""}${visibleItems.length === 0 && decisions.length === 0 ? " empty" : " has-content"}`} role="region" aria-label={isToolGenTurn ? "ToolGen work stream" : "Task work stream"}');
    expect(source).toContain("const persistentToolGenItems = useMemo(() => visibleItems.filter");
    expect(source).toContain('activity.toolgen_phase === "published"');
    expect(source).toContain("const scrollItems = useMemo(() => visibleItems.filter");
    expect(source).toContain('className="turn-persistent-toolgen" aria-label="ToolGen result"');
    expect(source).toContain("scrollItems.map((item, index)");
    expect(styles).toContain(".turn-persistent-toolgen");
    expect(source).toContain('title="Scroll to latest work update"');
    expect(source).toContain('aria-label={`${pendingUpdates} new work update${pendingUpdates === 1 ? "" : "s"}; scroll to latest`}');
    expect(source).toContain('scroll.scrollTo({ top: scroll.scrollHeight, behavior: prefersReducedMotion() ? "auto" : "smooth" });');
    expect(source).toContain('function prefersReducedMotion()');
    expect(source).toContain('window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false');
    expect(source).toContain('<ArrowDown size={13} aria-hidden="true"/>');
    expect(styles).toContain(".turn-new-updates:focus-visible, .scroll-to-bottom:focus-visible");
    expect(source).toContain("!turn.final_answer && turn.completion");
    expect(viewModelSource).toContain("turnLiveUsage");
    expect(viewModelSource).toContain("sessionContextUsage");
    expect(viewModelSource).toContain('message.kind === "runtime_restart"');
    expect(viewModelSource).toContain("turnLiveUsageSince(turn, runtimeRestartAtMs)");
    expect(styles).toContain(".header-context-meter");
    expect(styles).toContain(".header-context.warning .header-context-meter > span");
    expect(styles).toContain(".header-context.critical .header-context-meter > span");
    expect(styles).toContain(':root[data-theme="light"] .header-context.warning .header-context-meter > span');
    expect(styles).toContain(':root[data-theme="light"] .header-context.critical .header-context-meter > span');
    expect(styles).not.toContain(".context-usage-bar");
    expect(styles).toContain(".turn-work-scroll.has-pending-updates");
    expect(styles).toContain(".live-turn-usage");
  });

  it("supports agent rename and a distinct animated working state", () => {
    expect(viewModelSource).toContain('type: "session_rename"');
    expect(viewModelSource).toContain("sessionRenameDecision");
    expect(source).toContain('event.type === "session_renamed"');
    expect(source).toContain("Rename session failed");
    expect(source).toContain("Reconnect to Timem Web before renaming this session.");
    expect(source).toContain("session-working-icon");
    expect(source).toContain('aria-label="Session working"');
    expect(source).toContain('aria-hidden="true"');
    expect(source).toContain('className="sr-only">Session state: {session.state}</span>');
    expect(source).not.toContain("Agent working");
    expect(source).toContain("session-rename-input");
    expect(source).toContain('if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); finishRename(session.session_id); }');
    expect(source).toContain('if (event.key === "Escape") { event.preventDefault(); setRenamingSessionId(""); setRenameDraft(""); }');
    expect(source).toContain("const renamingSession = pendingRenameSessionIds.has(session.session_id);");
    expect(source).toContain('renamingSession ? "renaming-session" : ""');
    expect(source).toContain("aria-busy={renamingSession || deletingSession || undefined}");
    expect(source).toContain("Saving name...");
    expect(source).toContain('onDoubleClick={() => { if (!runtimeLocked && renamingSessionId !== session.session_id) beginRename(session); }}');
    expect(styles).toContain("@keyframes session-working-glow");
    expect(styles).toContain(".session-row.renaming-session");
    expect(styles).toContain(".session-pending");
    expect(styles).toContain(':root[data-theme="light"] .session-row.renaming-session');
    expect(styles).toContain(".sr-only { position: absolute; width: 1px; height: 1px;");
  });

  it("requires confirmation before permanently deleting a session", () => {
    expect(protocolSource).toContain('{ type: "session_delete"; session_id: string }');
    expect(protocolSource).toContain('{ type: "session_deleted"; session_id: string }');
    expect(source).toContain("SessionDeleteDialog");
    expect(source).toContain('className={`session-delete ${deletingSession ? "deleting" : ""}`}');
    expect(source).toContain('sendCommand({ type: "session_delete", session_id: sessionId })');
    expect(source).toContain("This permanently deletes the session, its stored task history, settings, and session tools.");
    expect(source).toContain("This cannot be undone.");
    expect(source).toContain('event.type === "session_deleted"');
    expect(styles).toContain(".session-delete-dialog");
    expect(styles).toContain(".decision-actions .danger");
  });

  it("expands each session into its scoped worker status list", () => {
    expect(source).toContain("expandedSessionIds");
    expect(source).toContain("session-expand");
    expect(source).toContain("worker-list");
    expect(source).toContain('aria-label={`Workers for ${session.display_name}: ${session.workers.length} worker${session.workers.length === 1 ? "" : "s"}`}');
    expect(source).toContain('className={`worker-state-dot ${worker.state}`} aria-hidden="true"');
    expect(source).toContain("worker.display_name || `ID${worker.ordinal}`");
    expect(styles).toContain(".worker-row");
    expect(styles).toContain(".worker-state-dot.working");
  });

  it("shows the live session cwd in navigation and the composer footer", () => {
    expect(source).toContain('className={`session ${session.session_id === activeSession?.session_id ? "active" : ""}`}');
    expect(source).toContain('className="session-name" title={session.display_name}');
    expect(source).toContain('className="session-detail session-cwd" title={session.current_dir}><FolderOpen size={11} aria-hidden="true"/><span className="path-tail">{workspacePathLabel(session.current_dir)}</span>');
    expect(source).toContain('className="session-sub"><span className="session-detail session-cwd"');
    expect(source).toContain('className="session-detail session-profile" title={modelDisplayName(session)}');
    expect(source).toContain('className="session-working-icon" size={15} aria-label="Session working"');
    expect(source).not.toContain('className="session-state">busy</span>');
    expect(styles).not.toContain(".session-state");
    expect(source).toContain('className="composer-cwd-inline"');
    expect(source).toContain('<span className="composer-cwd-inline" title={activeSession.current_dir}><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></span>');
    expect(source.indexOf('className="queued-message-list"')).toBeLessThan(source.indexOf('<form className="composer"'));
    expect(source.indexOf('aria-label="Message Timem"')).toBeLessThan(source.lastIndexOf('className="composer-cwd-inline"'));
    expect(styles).toContain(".path-tail { direction: rtl; text-align: left; unicode-bidi: plaintext; }");
    expect(viewModelSource).toContain("context_state");
    expect(styles).toContain(".session-cwd");
    expect(styles).toContain(".composer-cwd-inline");
    expect(source).toContain('activeSession?.debug_dir && <span className="composer-cwd-inline composer-debug-inline" title={activeSession.debug_dir}><b>DEBUG:</b><span>{activeSession.debug_dir}</span></span>');
    expect(source).not.toContain("tailPath(activeSession.debug_dir, 64)");
    expect(styles).toContain('.composer-paths { min-width: 0; flex: 1 1 auto; display: grid; gap: 2px; overflow: hidden; }');
    expect(styles).toContain(".composer-debug-inline { align-items: flex-start; overflow: visible; }");
    expect(styles).toContain(".composer-debug-inline span { overflow: visible; text-overflow: clip; white-space: normal; overflow-wrap: anywhere; user-select: text; }");
    expect(styles).toContain('font-family: "SFMono-Regular", Consolas, monospace;');
  });

  it("announces runtime connection state and explains mem switch availability", () => {
    expect(source).toContain('const [runtimeEverConnected, setRuntimeEverConnected] = useState(false);');
    expect(source).toContain('const [reconnectAttempt, setReconnectAttempt] = useState(0);');
    expect(source).toContain('setRuntimeEverConnected(true)');
    expect(source).toContain("const connectionLabel = runtimeConnectionLabel(connected, snapshotReady, runtimeEverConnected, reconnectAttempt);");
    expect(viewModelSource).toContain("export function runtimeConnectionLabel");
    expect(source).toContain('const memSwitchTitle = !runtimeReady ? "Wait for the runtime snapshot before switching mem" : pendingMemSwitch ? "Mem switch is in progress" : "Switch mem directory";');
    expect(source).toContain('setSnapshotReady(false)');
    expect(source).toContain('setSnapshotReady(true)');
    expect(source).toContain("const memSwitchButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const closeMemSwitchDialog = useCallback((restoreFocus = true) => {");
    expect(source).toContain("if (restoreFocus) memSwitchButtonRef.current?.focus({ preventScroll: true });");
    expect(source).toContain('className="connection-row" role="status" aria-live="polite" title={connectionLabel}');
    expect(source).toContain('className="connection-label">{connectionLabel}</span>');
    expect(source).toContain("const runtimeDisconnected = runtimeEverConnected && !connected;");
    expect(source).toContain("const runtimeUnavailable = runtimeDisconnected && reconnectAttempt >= 3;");
    expect(source).toContain('const runtimeDisconnectedTitle = runtimeUnavailable ? "Runtime unavailable" : "Connection lost";');
    expect(source).toContain("sessionInteractionLockReasonForState(pendingMemSwitch, connected, runtimeEverConnected, reconnectAttempt)");
    expect(viewModelSource).toContain('return reconnectAttempt >= 3 ? "Runtime unavailable. Restart timem-web." : "Connection lost. Reconnecting…";');
    expect(source).toContain("sessionInteractionLockReason={sessionInteractionLockReason}");
    expect(source).toContain('className="runtime-disconnect-banner" role="alert"');
    expect(source).toContain("<strong>{runtimeDisconnectedTitle}</strong>");
    expect(source).toContain("<span>{runtimeDisconnectedDetail}</span>");
    expect(styles).toContain(".runtime-disconnect-banner");
    expect(styles).toContain(":root[data-theme=\"light\"] .runtime-disconnect-banner");
    expect(source).toContain('ref={memSwitchButtonRef} className="mem-card"');
    expect(source).toContain('<span className="mem-card-icon" aria-hidden="true"><Database size={15}/></span>');
    expect(source).toContain('<span className="mem-card-copy"><strong>Memory</strong>');
    expect(source).toContain('<small dir="rtl">{pendingMemSwitch ? "Switching…" : server?.mem?.space_dir ?? "…"}</small>');
    expect(source).not.toContain('tailPath(server?.mem?.space_dir');
    expect(styles).toContain(".mem-card-copy small { overflow: hidden; color: #858585; direction: rtl;");
    expect(styles).toContain("text-align: left; text-overflow: ellipsis; unicode-bidi: plaintext; white-space: nowrap;");
    expect(source).toContain('onClick={() => setShowMemSwitch(true)}');
    expect(styles).toContain(".mem-card");
    expect(styles).toContain(".mem-card-copy");
    expect(styles).toContain(".connection-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }");
    expect(styles).toContain(".connection.offline { background: #d77b75; box-shadow: 0 0 0 3px #d77b7522; animation: connection-retry 1.1s ease-in-out infinite; }");
    expect(styles).toContain("@keyframes connection-retry");
    expect(styles).toContain("@media (prefers-reduced-motion: reduce) { .chat-scroll { scroll-behavior: auto; }");
    expect(styles).toContain(".pulse, .connection.offline, .session-dot.working");
  });

  it("uses session terminology consistently for the creation workflow", () => {
    expect(source).toContain("New session");
    expect(source).toContain('const welcomeTitle = activeSession ? "Ready when you are." : "Create a session to start.";');
    expect(source).toContain('const welcomeText = activeSession ? "Ask Timem to investigate, write, or work with you." : "Use New session to choose a workspace and runtime profile.";');
    expect(source).toContain("<h2>{welcomeTitle}</h2><p>{welcomeText}</p>");
    expect(source).toContain('aria-label="Create session"');
    expect(source).toContain('creating ? "Creating…" : "Create session"');
    expect(source).toContain("disabled={creating}");
    expect(source).toContain('activeModelRetryStatus, activityFromTopic');
    expect(source).toContain('sessionCreateDecision');
    expect(source).toContain("const canCreateSession = createDecision.kind === \"send\";");
    expect(source).toContain("disabled={creating || workspaces.length === 0}");
    expect(source).toContain('workspaces.map((workspace) => <option value={workspace} key={workspace} title={workspace}>{tailPath(workspace, 64)}</option>)');
    expect(source).toContain("No workspace available");
    expect(source).toContain("No workspace is available from the runtime snapshot.");
    expect(source).toContain("disabled={!canCreateSession}");
    expect(source).not.toContain("New agent");
  });

  it("creates sessions with independent runtime environment overrides", () => {
    expect(source).toContain("SESSION_RUNTIME_FIELDS");
    expect(source).toContain('TIMEM_MODEL');
    expect(source).toContain('TIMEM_API_KEY');
    expect(source).toContain('TIMEM_ENABLE_THINKING');
    expect(source).toContain('TIMEM_REASONING_EFFORT');
    expect(source).toContain('TIMEM_STREAM');
    expect(source).toContain('kind === "boolean"');
    expect(source).toContain('type={kind}');
    expect(source).toContain("const resetEnv = (key: string)");
    expect(source).toContain('className="session-runtime-control"');
    expect(source).toContain('className="session-runtime-reset"');
    expect(source).toContain('title={`Reset ${label} to inherited value`}');
    expect(source).toContain('aria-label={`Reset ${label} to inherited value`}');
    expect(source).toContain('onClick={() => resetEnv(key)}>Reset</button>');
    expect(source).toContain('onCreate={(command) => {');
    expect(source).toContain('modelDisplayName(session)');
    expect(styles).toContain('.session-runtime-grid');
    expect(styles).toContain('.session-runtime-control');
    expect(styles).toContain('.session-runtime-reset');
    expect(styles).toContain(':root[data-theme="light"] .session-runtime-reset');
    expect(styles).toContain('.session-profile');
  });

  it("keeps model configuration and service failures visible with actionable guidance", () => {
    expect(source).toContain("const [modelServiceIssues, setModelServiceIssues]");
    expect(source).toContain("modelServiceIssues[activeSession.session_id] ?? sessionModelConfigurationIssue(activeSession)");
    expect(source).toContain('className="model-config-banner" role="alert"');
    expect(source).toContain("{activeModelServiceIssue.title}");
    expect(source).toContain("{activeModelServiceIssue.detail}");
    expect(source).toContain("Open Runtime settings");
    expect(source).toContain("setModelServiceIssues((current) => ({ ...current, [sessionId]: issue }))");
    expect(source).toContain("setModelServiceIssues((current) => ({ ...current, [event.session_id]: issue }))");
    expect(source).toContain("commandSessionId(completed?.command)");
    expect(source).toContain("isModelSubmissionCommand(completed?.command)");
    expect(source).toContain('kind === "model_error"');
    expect(styles).toContain(".model-config-banner");
  });

  it("lets an existing session edit, reveal, and clear its API key without snapshot leakage", () => {
    expect(source).toContain('Session API key');
    expect(source).toContain('aria-label="API key for current session"');
    expect(source).toContain('type={showApiKey ? "text" : "password"}');
    expect(source).toContain('autoComplete="new-password"');
    expect(source).toContain('session_api_key_update');
    expect(source).toContain('session_api_key_reveal');
    expect(source).toContain('event.type === "session_api_key_revealed"');
    expect(source).toContain('setRevealedSessionApiKeys({});');
    expect(source).toContain('showApiKey ? <EyeOff size={15}/> : <Eye size={15}/>');
    expect(source).toContain('setShowApiKey(false);');
    expect(source).toContain('shouldAutoRevealSessionApiKey({ sessionId, configured: keyConfigured');
    expect(source).toContain('placeholder={credentialPending && keyConfigured ? "Loading API key…" : "Enter API key"}');
    expect(source).toContain('<small>{session ? session.display_name : "Create or select a session first"}</small>');
    expect(source).not.toContain('keyConfigured ? "configured" : "not configured"');
    expect(source).not.toContain('placeholder={keyConfigured ? "API key configured"');
    expect(source).toContain('event.type === "session_runtime_updated"');
    expect(source).toContain('api_key_configured');
    expect(source).not.toContain('previousCredentialPending.current && !credentialPending && revealedApiKey === undefined');
    expect(source).toContain('const commandId = clientId("credential")');
    expect(source).toContain('pendingSessionApiKeyCommandsRef.current.set(commandId, { sessionId, timeoutId })');
    expect(source).toContain('finishPendingSessionApiKeyCommand(');
    expect(source).toContain('event.status === "committed"');
    expect(source).toContain('"API key update rejected"');
    expect(source).toContain('"API key update timed out"');
    expect(source).toContain('if (!connected || !snapshotReady)');
    expect(source).toContain('"API key update unavailable"');
    expect(source).toContain('SESSION_API_KEY_SAVE_TIMEOUT_MS');
    expect(source).toContain('cancelAllPendingSessionApiKeyCommands(');
    expect(source).toContain('Your input was kept; reconnect and try again.');
    expect(source).not.toContain('onApiKeyUpdate("")}>Clear</button>');
    expect(source).toContain('disabled={!session || credentialPending} readOnly={sessionWorking}');
    expect(source).toContain('disabled={!session || credentialPending} onClick={toggleApiKey}');
    expect(source).toContain('API key is read-only while working; you can still reveal and copy it.');
    expect(source).toContain('Finish or stop the active task before changing credentials.');
    expect(styles).toContain('.session-credential-settings');
    expect(styles).toContain('.session-credential-control');
    expect(protocolSource).toContain('api_key_configured: boolean');
    expect(protocolSource).toContain('{ type: "session_api_key_update"; session_id: string; api_key: string }');
    expect(protocolSource).toContain('{ type: "session_api_key_reveal"; session_id: string }');
    expect(protocolSource).not.toContain('runtime_profile: {\n    api_key: string');
  });

  it("dismisses the runtime configuration card on outside click or Escape", () => {
    expect(source).toContain('runtimePanelRef.current?.focus({ preventScroll: true });');
    expect(source).toContain('const closeRuntimePanel = useCallback((restoreFocus = true) => {');
    expect(source).toContain('if (restoreFocus) runtimeButtonRef.current?.focus({ preventScroll: true });');
    expect(source).toContain('document.addEventListener("pointerdown", dismissOnOutsidePointer)');
    expect(source).toContain('runtimeButtonRef.current?.contains(target)');
    expect(source).toContain('runtimePanelRef.current?.contains(target)');
    expect(source).toContain('closeRuntimePanel(false);');
    expect(source).toContain('if (event.key === "Escape") closeRuntimePanel()');
    expect(source).toContain('const runtimeLabel = showRuntime ? "Close runtime information" : "Open runtime information";');
    expect(source).toContain('aria-label={`${runtimeLabel}: ${headerModelLabel}`}');
    expect(source).toContain('aria-expanded={showRuntime}');
    expect(source).toContain('if (showRuntime) closeRuntimePanel(); else setShowRuntime(true);');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card" tabIndex={-1}');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings" tabIndex={-1}');
  });

  it("reconciles only applied runtime fields and preserves unrelated drafts", () => {
    expect(source).toContain("setDrafts((current) => reconcileRuntimeDrafts(current, runtimeOptions))");
    expect(source).toContain("sessionRuntimeOptions(session?.runtime_profile, server?.runtime_options ?? [])");
    expect(source).toContain('useEffect(() => setDrafts({}), [session?.session_id]);');
    expect(source).toContain('const pendingRuntimeLabel = pendingKeys.size ? `Applying runtime setting${pendingKeys.size === 1 ? "" : "s"}: ${Array.from(pendingKeys).map(runtimeOptionLabel).join(", ")}` : "";');
    expect(source).toContain("const dirty = value !== option.value;");
    expect(source).toContain("const optionLabel = runtimeOptionLabel(option.key);");
    expect(source).toContain("<span>{optionLabel}</span>");
    expect(source).toContain('className="secondary compact runtime-reset"');
    expect(source).toContain('title={`Reset ${optionLabel} to current value`}');
    expect(source).toContain('aria-label={`Reset ${optionLabel} to current value`}');
    expect(source).toContain("const resetDraft = () => setDrafts((current) => { const { [option.key]: _removed, ...rest } = current; return rest; });");
    expect(source).toContain('if (event.key === "Enter" && !event.nativeEvent.isComposing && dirty && !pending) { event.preventDefault(); onUpdate(option.key, value); }');
    expect(source).toContain('if (event.key === "Escape" && dirty) { event.preventDefault(); resetDraft(); }');
    expect(source).toContain("onClick={resetDraft}");
    expect(source).toContain('disabled={pending || !dirty}');
    expect(source).toContain('(pendingRuntimeLabel || credentialPending) && <p className="runtime-pending-status" role="status" aria-live="polite">');
    expect(styles).toContain(".runtime-options label > div input, .runtime-options label > div select { flex: 1 1 auto; }");
    expect(styles).toContain(".runtime-reset { flex: none; }");
    expect(styles).toContain(".runtime-settings { display: block; overflow: visible; padding: 14px; }");
    expect(styles).not.toContain("max-height: 360px; overflow: auto;");
    expect(styles).toContain(".runtime-pending-status");
  });

  it("uses select controls for runtime settings with predefined values", () => {
    expect(source).toContain("function runtimeSelectOptions(key: string): readonly string[] | null");
    expect(source).toContain('case "TIMEM_BASH_APPROVAL":');
    expect(source).toContain('return ["approve", "ask"];');
    expect(source).toContain('case "TIMEM_WORK_INSTRUCTIONS":');
    expect(source).toContain('return ["silent", "ask", "off"];');
    expect(source).toContain('case "TIMEM_API_PROTOCOL":');
    expect(source).toContain('return ["openai-compatible", "openai-responses", "anthropic"];');
    expect(source).toContain('case "TIMEM_RESPONSE_PROTOCOL":');
    expect(source).toContain('return ["xml", "json"];');
    expect(source).not.toContain('<option value="markdown">markdown</option>');
    expect(source).toContain('case "TIMEM_MAX_ROUNDS":');
    expect(source).toContain('return ["50", "200", "500", "unlimited"];');
    expect(source).toContain("options ? <select value={value}");
    expect(source).toContain('options.map((choice) => <option value={choice} key={choice}>{choice === "unlimited" ? "Unlimited" : choice}</option>)');
    expect(styles).toContain(".runtime-options input, .runtime-options select, .session-modal input, .session-modal select");
  });

  it("renders context compaction as a compact status pill with a reduced-motion fallback", () => {
    expect(source).toContain("<ContextCompactNotice");
    expect(source).toContain('<Gauge size={13}/>');
    expect(styles).toContain(".context-compact-notice");
    expect(styles).toContain("width: fit-content");
    expect(styles).toContain("grid-template-columns: 22px auto 72px");
    expect(styles).toContain("border-radius: 999px");
    expect(styles).toContain(".compact-meter { position: relative; width: 72px; height: 3px;");
    expect(styles).toContain("prefers-reduced-motion: reduce");
  });

  it("keeps routing identifiers out of the task work stream", () => {
    expect(source).toContain('["kind", "session_id", "context_id", "worker_id"].includes(key)');
  });

  it("persists theme, font, and text-size appearance without changing core state", () => {
    expect(appearanceSource).toContain('APPEARANCE_STORAGE_KEY = "timem-web-appearance-v1"');
    expect(appearanceSource).toContain('root.dataset.theme = appearance.theme');
    expect(styles).toContain(':root[data-theme="light"] { color-scheme: light; }');
    expect(styles).toContain(':root[data-theme="dark"] { color-scheme: dark; }');
    expect(appearanceSource).toContain('root.dataset.userFont = appearance.userFont');
    expect(appearanceSource).toContain('root.dataset.userChineseFont = appearance.userChineseFont');
    expect(appearanceSource).toContain('root.dataset.userBold = String(appearance.userBold)');
    expect(appearanceSource).toContain('root.dataset.agentFont = appearance.agentFont');
    expect(appearanceSource).toContain('root.dataset.agentChineseFont = appearance.agentChineseFont');
    expect(appearanceSource).toContain('root.dataset.agentBold = String(appearance.agentBold)');
    expect(appearanceSource).toContain('root.dataset.textSize = appearance.textSize');
    expect(source).toContain('const appearanceLabel = showAppearance ? "Close appearance settings" : "Open appearance settings";');
    expect(source).toContain("const appearanceButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const appearancePanelRef = useRef<HTMLElement | null>(null);");
    expect(source).toContain('title={appearanceLabel} aria-label={appearanceLabel}');
    expect(source).toContain('ref={appearanceButtonRef}');
    expect(source).toContain('aria-expanded={showAppearance} aria-controls="appearance-panel"');
    expect(source).toContain("<AppearancePanel");
    expect(source).toContain("panelRef={appearancePanelRef}");
    expect(source).toContain("appearance={appearance}");
    expect(source).toContain("aria-pressed={appearance.theme === theme}");
    expect(source).toContain('value={appearance.userChineseFont} aria-label="User Chinese font"');
    expect(source).toContain('value={appearance.userFont} aria-label="User other language font"');
    expect(source).toContain('checked={appearance.userBold}');
    expect(source).toContain('value={appearance.agentChineseFont} aria-label="Agent Chinese font"');
    expect(source).toContain('value={appearance.agentFont} aria-label="Agent other language font"');
    expect(source).toContain('checked={appearance.agentBold}');
    expect(source).toContain("aria-pressed={appearance.textSize === size}");
    expect(source).toContain('title={`Use ${theme} theme`}');
    expect(source).toContain('<option value="heiti">黑体</option><option value="kaiti">楷体</option><option value="songti">宋体</option>');
    expect(source).toContain('<option value="system">System</option><option value="serif">Serif</option><option value="mono">Mono</option>');
    expect(source).toContain('title={`Use ${size === "medium" ? "default" : size} text size`}');
    expect(source).toContain('if (!showAppearance) return;');
    expect(source).toContain('appearancePanelRef.current?.focus({ preventScroll: true });');
    expect(source).toContain('const closeAppearancePanel = useCallback((restoreFocus = true) => {');
    expect(source).toContain('if (restoreFocus) appearanceButtonRef.current?.focus({ preventScroll: true });');
    expect(source).toContain('appearanceButtonRef.current?.contains(target)');
    expect(source).toContain('appearancePanelRef.current?.contains(target)');
    expect(source).toContain('closeAppearancePanel(false);');
    expect(source).toContain('if (event.key === "Escape") closeAppearancePanel()');
    expect(source).toContain('const descriptionId = "appearance-panel-description";');
    expect(source).toContain('id="appearance-panel" ref={panelRef} className="appearance-panel" role="dialog" aria-modal="false" aria-label="Appearance settings" aria-describedby={descriptionId} tabIndex={-1} onKeyDown={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); onClose(); } }}');
    expect(source).toContain('<p id={descriptionId}>Adjust theme, language fonts, and message text size for this browser.</p>');
    expect(source).toContain('setShowRuntime(false); setShowMcp(false); setShowToolRepo(false); if (showAppearance) closeAppearancePanel(); else setShowAppearance(true);');
    expect(styles).toContain(".appearance-panel header p");
    expect(styles).toContain(':root[data-theme="light"]');
    expect(styles).toContain(':root[data-user-font="serif"]');
    expect(styles).toContain(':root[data-agent-font="serif"]');
    expect(styles).toContain(':root[data-user-chinese-font="heiti"]');
    expect(styles).toContain(':root[data-agent-chinese-font="kaiti"]');
    expect(styles).toContain(':root[data-user-bold="true"]');
    expect(styles).toContain(':root[data-agent-bold="true"]');
    expect(styles).toContain('.appearance-font-selects { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 10px; }');
    expect(styles).toContain('.turn-user-entry { font-family: var(--user-other-font), var(--user-chinese-font), sans-serif; font-weight: var(--user-font-weight); }');
    expect(styles).toContain('.turn-assistant-frame, .turn-final-delivery { font-family: var(--agent-other-font), var(--agent-chinese-font), sans-serif; font-weight: var(--agent-font-weight); }');
    expect(styles).toContain("--content-size: 13.5px;");
    expect(styles).toContain(':root[data-text-size="small"] { --content-size: 12.6px; }');
    expect(styles).toContain(':root[data-text-size="large"] { --content-size: 14.4px; }');
    expect(html).toContain('<script type="module" src="/src/preload.ts"></script>');
    expect(html).not.toContain("<script>\n");
    expect(preloadSource).toContain("applyAppearance(loadAppearance())");
    expect(appearanceSource).toContain('window.localStorage.getItem(APPEARANCE_STORAGE_KEY)');
    expect(appearanceSource).toContain('root.dataset.theme = appearance.theme');
    expect(appearanceSource).toContain('root.dataset.userChineseFont = appearance.userChineseFont');
    expect(appearanceSource).toContain('root.dataset.agentChineseFont = appearance.agentChineseFont');
  });

  it("keeps the active session label readable in light theme after style overrides", () => {
    expect(styles).toContain(':root[data-theme="light"] .session-row.active { background: #e8e8e8; box-shadow: none; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session.active { background: transparent; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session { color: #202020; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session-cwd { color: #666; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session-profile { color: #747474; }');
  });

  it("renders GFM and highlighted code with a copy affordance", () => {
    expect(source).toContain('import remarkGfm from "remark-gfm"');
    expect(source).toContain('remarkPlugins={[remarkGfm]}');
    expect(source).toContain('pre: CodeBlock');
    expect(source).toContain('className="table-scroll" role="region" tabIndex={0} aria-label="Scrollable table. Use horizontal scroll to inspect all columns."');
    expect(source).toContain('const codeCopySubject = `${language} code`;');
    expect(source).toContain('const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(code, {');
    expect(source).toContain('idle: `Copy ${codeCopySubject}`');
    expect(source).toContain('copied: `${codeCopySubject} copied`');
    expect(source).toContain('failed: `Copy ${codeCopySubject} failed`');
    expect(source).toContain('className={copyClass}');
    expect(source).toContain('aria-label={copyLabel}');
    expect(styles).toContain('.markdown-body blockquote');
    expect(styles).toContain(".table-scroll");
    expect(styles).toContain("scrollbar-gutter: auto;");
    expect(styles).toContain("scrollbar-gutter: stable;");
    expect(styles).toContain(".table-scroll:focus-visible");
    expect(styles).toContain(':root[data-theme="light"] .table-scroll');
    expect(styles).toContain('.code-block figcaption');
    expect(styles).toContain(".markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4 { margin: 1.1em 0 .4em;");
    expect(styles).toContain(".markdown-body p, .markdown-body ul, .markdown-body ol { margin: .48em 0; }");
    expect(styles).toContain(".markdown-body li + li { margin-top: .2em; }");
    expect(styles).toContain(".markdown-body blockquote { margin: .68em 0; padding: .15em .75em;");
    expect(styles).toContain(".table-scroll { width: 100%; margin: 11px 0;");
    expect(styles).toContain(".code-block { overflow: hidden; margin: .68em 0;");
    expect(styles).toContain(".code-block figcaption { min-width: 0; height: 26px;");
    expect(styles).toContain("border-radius: 0; padding: 10px 11px; background: #0d141b;");
    expect(styles).toContain(".code-block figcaption > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }");
    expect(styles).toContain(".code-block figcaption button { flex: none;");
    expect(styles).toContain(':root[data-theme="light"] .code-block { border-color: #cfddda; background: #f3f7f6; }');
    expect(styles).toContain(':root[data-theme="light"] .code-block figcaption { border-color: #d5e1de; background: #eaf1ef; color: #657b79; }');
    expect(styles).toContain(':root[data-theme="light"] .turn-final-delivery > .message-content .code-block pre,');
    expect(styles).toContain(':root[data-theme="light"] .code-block .hljs-comment,');
    expect(styles).toContain(':root[data-theme="light"] .code-block .hljs-keyword,');
    expect(styles).toContain(':root[data-theme="light"] .code-block .hljs-punctuation,');
  });

  it("moves submitted files from the composer into a compact user attachment list", () => {
    expect(source).toContain("consumedAttachmentIds");
    expect(source).toContain('className="turn-entry-attachments"');
    expect(source).toContain("entry.attachments.map");
    expect(styles).toContain(".turn-entry-attachments > span");
  });

  it("lets users remove pending attachments without losing access to long file names", () => {
    expect(source).toContain('type: "attachment_remove"');
    expect(source).toContain("const attachedFileCount = activeSession?.attachments.length ?? 0;");
    expect(source).toContain('const attachmentSummary = attachedFileCount === 1 ? "1 file attached" : `${attachedFileCount} files attached`;');
    expect(source).toContain('const attachmentStripLabel = uploadingAttachment');
    expect(source).toContain('? `${attachmentSummary}; ${uploadingAttachmentText}`');
    expect(source).toContain(': `Files attached to the next message; ${attachmentSummary}`;');
    expect(source).toContain('className="attachment-summary" title={attachmentSummary}');
    expect(source).toContain('className="pending-attachment-name"');
    expect(source).toContain('title={attachment.name}');
    expect(source).toContain("pendingAttachmentRemoveIds.has");
    expect(source).toContain("disabled={removing || sessionInteractionLocked}");
    expect(source).toContain("const removeLabel = removing ? `Removing ${attachment.name}` : sessionInteractionLocked ? `${sessionInteractionLockReason} · cannot remove ${attachment.name}` : `Remove ${attachment.name}`;");
    expect(source).toContain("title={removeLabel} aria-label={removeLabel}");
    expect(source).toContain("aria-busy={removing || undefined}");
    expect(styles).toContain(".attachment-summary");
    expect(styles).toContain(".pending-attachment-name");
    expect(styles).toContain("text-overflow: ellipsis");
  });

  it("guards file uploads with visible pending feedback and no-session disabled state", () => {
    expect(source).toContain("pendingUploadSessionIdsRef");
    expect(source).toContain("setPendingUploadSessionIds");
    expect(source).toContain("const [pendingUploadFiles, setPendingUploadFiles]");
    expect(source).toContain("setPendingUploadFiles((current) => ({ ...current, [activeSession.session_id]: { name: file.name, bytes: file.size } }));");
    expect(source).toContain("Upload already in progress");
    expect(source).toContain("removePendingKey(pendingUploadSessionIdsRef, setPendingUploadSessionIds, activeSession.session_id);");
    expect(source).toContain("uploadingAttachment={!!activeSession && pendingUploadSessionIds.has(activeSession.session_id)}");
    expect(source).toContain("uploadingAttachmentFile={activeSession ? pendingUploadFiles[activeSession.session_id] : undefined}");
    expect(source).toContain('const lockedControlHint = sessionInteractionLocked ? sessionInteractionLockReason : "";');
    expect(source).toContain('const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";');
    expect(source).toContain('const attachTitle = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");');
    expect(source).toContain('const attachLabel = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");');
    expect(source).toContain('const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);');
    expect(source).toContain('className={`attach-button ${uploadingAttachment ? "uploading" : ""}`}');
    expect(source).toContain('{uploadingAttachment ? <LoaderCircle size={17}/> : <Paperclip size={17}/>}');
    expect(source).toContain('title={attachTitle}');
    expect(source).toContain('aria-label={attachLabel}');
    expect(source).toContain("disabled={!activeSession || uploadingAttachment || sessionInteractionLocked}");
    expect(source).toContain("disabled={!activeSession || !draft.trim() || submittingDraft || uploadingAttachment || sessionInteractionLocked}");
    expect(source).toContain('aria-label={attachmentStripLabel} aria-live="polite" aria-busy={uploadingAttachment || undefined}');
    expect(source).toContain('uploadingAttachment && <div className="pending-attachment uploading" role="status"');
    expect(source).toContain('aria-label={uploadingAttachmentFile ? `${uploadingAttachmentText}, ${formatBytes(uploadingAttachmentFile.bytes)}` : uploadingAttachmentText}');
    expect(source).toContain("title={uploadingAttachmentFile?.name ?? uploadingAttachmentText}");
    expect(source).toContain('className="upload-dot" aria-hidden="true"');
    expect(source).toContain('uploadingAttachmentFile?.name ?? "Uploading file…"');
    expect(source).toContain("formatBytes(uploadingAttachmentFile.bytes)");
    expect(styles).toContain(".attach-button.uploading:disabled");
    expect(styles).toContain(".attach-button.uploading svg");
    expect(styles).toContain(".pending-attachment.uploading");
    expect(styles).toContain(".upload-dot");
    expect(styles).toContain("@keyframes upload-button-pulse");
    expect(styles).toContain("@keyframes upload-dot-pulse");
    expect(styles).toContain("@media (prefers-reduced-motion: reduce)");
    expect(styles).toContain(".toolrepo-header-button.count-pulse .toolrepo-header-count, .attach-button.uploading:disabled, .attach-button.uploading svg, .toolrepo-search-pending, .toolrepo-empty.searching svg, .upload-dot");
    expect(styles).toContain(".send-button.sending svg");
    expect(styles).toContain(".completion-toolgen.sending svg");
    expect(styles).toContain(".worker-state-dot.working");
    expect(styles).toContain("animation: none;");
  });

  it("keeps working-turn input visually consistent with a normal send", () => {
    expect(source).toContain('placeholder={!activeSession ? "Create a session to start…" : sessionInteractionLocked ? sessionInteractionLockReason : activeSession.state === "working" ? "继续输入…"');
    expect(source).toContain('"Ask Timem to investigate, write, or work with you."');
    expect(source).not.toContain("Ask Timem anything about this workspace");
    expect(source).toContain('activeSession?.state === "working" ? "Queue message" : "Send message"');
    expect(source).toContain('className={`queued-message-list ${queueExpanded ? "expanded" : "collapsed"} ${queuePanelCollapsed ? "summary-only" : ""} ${queuedMessagesPause ? "paused" : ""}`}');
    expect(source).toContain("自动发送已停止");
    expect(source).toContain('role="switch"');
    expect(source).toContain('className="queued-auto-send-switch"');
    expect(source).toContain("aria-checked={!queuedMessagesPause}");
    expect(source).toContain('aria-label={queuedMessagesPause ? "开启自动发送" : "停止自动发送"}');
    expect(source).toContain('if (queuedMessagesPause) resumeQueuedMessages(activeSessionId); else pauseQueuedMessages(activeSessionId, "用户关闭了自动发送", "user");');
    expect(source).toContain("const queuedMessagesPauseBySessionRef = useRef<Record<string, QueuedMessagesPauseState>>({});");
    expect(source).toContain("const pause = stopQueuedAutoSend(current, reason, source, Date.now());");
    expect(source).toContain("const queuedMessagesPause = activeSessionId ? queuedMessagesPauseBySession[activeSessionId] ?? null : null;");
    expect(source).toContain("new Set(Object.keys(queuedMessagesPauseBySessionRef.current))");
    expect(source).toContain("queuedMessagesPauseSessionId(reliableStorageScope, event.key)");
    expect(source).toContain("liveSessionIds.has(pauseSessionId)");
    expect(source).toContain("delete next[sessionId];");
    expect(source).not.toContain("手动发送仍可用");
    expect(source).toContain("const sendAsNewTurn = !!queuedMessagesPause;");
    expect(source).toContain('sendAsNewTurn ? "作为新消息开始任务" : "立即发送为当前任务的补充"');
    expect(source).toContain("messageRoleIds, sendAsNewTurn)");
    expect(source).toContain("forceNewTurn = false");
    expect(viewModelSource).toContain('!forceNewTurn && (forceSupplement || session.state === "working")');
    expect(source).not.toContain('onClick={resumeQueuedMessages}>继续发送</button>');
    expect(styles).toContain(".queued-auto-send-switch[aria-checked=\"true\"]");
    expect(styles).toContain("transform: translateX(14px)");
    expect(styles).toContain(".queued-auto-send-switch:focus-visible");
    expect(styles).toContain(".queued-auto-send-thumb");
    expect(source).toContain('className="queued-message-supplement"');
    expect(source).toContain('claimed ? "发送中…" : message.deliveryError ? "重试" : "立即"');
    expect(source).toContain('title={effectiveSendLabel} aria-label={effectiveSendLabel}');
    expect(source).not.toContain('>Supplement</span>');
  });

  it("bounds, expands, collapses, and reorders the queued message list", () => {
    expect(source).toContain("displayQueuedMessages.slice(0, COLLAPSED_QUEUE_LIMIT)");
    expect(source).toContain('queueExpanded ? "expanded" : "collapsed"');
    expect(source).toContain('aria-expanded={queueExpanded}');
    expect(source).toContain('queueExpanded ? "收起" : `展开 ${hiddenQueuedMessageCount} 条`');
    expect(source).toContain("const [collapsedQueuePanelSessionIds, setCollapsedQueuePanelSessionIds]");
    expect(source).toContain("const firstQueuedMessage = displayQueuedMessages[0];");
    expect(source).toContain('className={`queued-message-summary ${firstQueuedMessage?.deliveryError ? "delivery-error" : ""}`}');
    expect(source).toContain("<p>{firstQueuedMessage?.text}</p>");
    expect(source).toContain('className="queued-message-summary-attachments"');
    expect(source).toContain('className="queued-message-summary-count">{displayQueuedMessages.length} 条</small>');
    expect(source).toContain('className="queued-message-panel-toggle"');
    expect(source).toContain('title={queuePanelCollapsed ? "展开待发送队列" : "折叠待发送队列为一行"}');
    expect(source).toContain("{!queuePanelCollapsed && <DndContext");
    expect(source).toContain('className="queued-message-drag" disabled={dragDisabled}');
    expect(source).toContain("const finishQueuedMessageDrag = ({ active, over }: DragEndEvent)");
    expect(source).toContain("reorderQueuedMessages(");
    expect(styles).toContain(".queued-message-list.collapsed .queued-message-items { max-height: 224px; overflow: hidden; }");
    expect(styles).toContain(".queued-message-list.expanded .queued-message-items { max-height: min(50vh, 420px); overflow-y: auto;");
    expect(styles).toContain(".queued-message-list.summary-only { gap: 0; padding-block: 7px; }");
    expect(styles).toContain(".queued-message-header-actions { flex: 0 0 auto;");
    expect(styles).toContain(".queued-message-toggle, .queued-message-panel-toggle");
    expect(styles).toContain(".queued-message-list.summary-only > header { min-height: 26px; padding-bottom: 0; }");
    expect(styles).toContain(".queued-message-list > header small { min-width: 0; overflow: hidden;");
    expect(styles).toContain(".queued-message-summary { min-width: 0; flex: 1 1 auto;");
    expect(styles).toContain(".queued-message-summary p { min-width: 0; flex: 1 1 auto;");
    expect(styles).toContain(".queued-message-summary-count { padding-left: 6px; border-left: 1px solid");
    expect(styles).toContain(':root[data-theme="light"] .queued-message-summary p');
    expect(styles).toContain("@media (max-width: 520px) {\n  .queued-message-list > header { gap: 5px; }");
  });

  it("keeps queued worker roles in normal flow above long message previews", () => {
    expect(source).toContain('className="queued-message-preview"');
    expect(source).toContain('className="queued-message-roles" title={messageRoleNames.join(" | ")}');
    expect(source).toContain('className="queued-message-actions"');
    expect(styles).toContain(".queued-message-preview { min-width: 0; display: grid; justify-items: start; gap: 3px; overflow: hidden; }");
    expect(styles).toContain(".queued-message p { width: 100%; min-width: 0;");
    expect(styles).toContain(".queued-message-roles { max-width: 100%; display: inline-flex;");
    expect(source).toContain('className="queued-message-role-separator" aria-hidden="true">|</i>');
    expect(source).not.toContain('messageRoleNames.join("、")');
    expect(styles).toContain(".queued-message-role-names { min-width: 0; display: inline-flex;");
    expect(styles).toContain(".queued-message-role-separator { flex: 0 0 auto; margin: 0 4px; color: #6f9187;");
    expect(styles).toContain(':root[data-theme="light"] .queued-message-role-separator { color: #91aaa2; }');
    expect(styles).not.toContain(".queued-message-preview.has-roles");
    expect(styles).not.toContain(".queued-message-roles { position: absolute;");
    expect(source).not.toContain('messageRoleNames.length > 0 ? "has-roles" : ""');
  });

  it("claims each queued message before immediate or automatic dispatch", () => {
    expect(source).toContain("queuedMessageClaimsRef");
    expect(source).toContain("claimQueuedMessage(queuedMessageClaimsRef.current");
    expect(source).toContain("releaseQueuedMessageClaim(queuedMessageClaimsRef.current");
    expect(source).toContain("queuedMessagesBySessionRef.current");
    expect(source).toContain("unclaimedQueuedMessages(queuedMessages, queuedMessageClaims, activeSessionId)");
    expect(source).toContain("displayQueuedMessages.length > 0");
    expect(source).toContain("removeQueuedMessage(current[activeSession.session_id] ?? [], message.id, queuedMessageClaimsRef.current");
    expect(source).toContain('disabled={claimed || (!message.deliveryError && !sendAsNewTurn && activeSession.state !== "working")');
    expect(source).toContain('aria-busy={claimed || undefined}');
    expect(source).toContain('claimed ? "发送中…" : message.deliveryError ? "重试" : "立即"');
    expect(styles).toContain(".queued-message.sending");
  });

  it("lets queued messages be re-edited without changing their queue position", () => {
    expect(source).toContain('className="queued-message-edit"');
    expect(source).toContain('className="queued-message-editor" autoFocus');
    expect(source).toContain("message.id === edit.id ? { ...message, text, deliveryError: undefined } : message");
    expect(source).toContain('selectQueuedDispatches(sessions, queuedMessagesBySessionRef.current, queuedDispatchSessionIdsRef.current, editingQueuedMessage?.sessionId, new Set(Object.keys(queuedMessagesPauseBySessionRef.current)))');
    expect(source).toContain(">保存</button>");
    expect(source).toContain('className="queued-message-edit-cancel"');
    expect(source).toContain(">取消</button>");
    expect(styles).toContain(".queued-message.editing { grid-template-columns: 19px minmax(0, 1fr);");
    expect(styles).toContain(".queued-message.editing .queued-message-drag { display: none; }");
    expect(styles).toContain(".queued-message.editing .queued-message-preview { grid-column: 2; grid-row: 1; width: 100%;");
    expect(styles).toContain(".queued-message-editor { box-sizing: border-box; width: 100%;");
    expect(styles).toContain("min-height: 112px; max-height: min(42vh, 320px);");
    expect(styles).toContain(".queued-message.editing .queued-message-actions { grid-column: 2; grid-row: 2; justify-self: end;");
    expect(styles).toContain(".queued-message.editing .queued-message-edit-save");
    expect(styles).toContain("@media (max-width: 720px) {\n  .queued-message.editing");
  });

  it("keeps composer typing away from expensive turn history recomputation", () => {
    expect(source).toContain("memo(function VisibleTurnList");
    expect(source).toContain("memo(function TurnInteraction");
    expect(source).toContain("const decisionsByTurn = useMemo");
    expect(source).toContain("decisions={decisionsByTurn.get(sessionTurnKey(sessionId, turn.turn_id)) ?? EMPTY_DECISIONS}");
    expect(source).toContain("<VisibleTurnList");
    expect(source).not.toContain("decisions={decisions.filter");
    expect(source).toContain("const lifecycleEvents = useMemo(() => coalesceActionLifecycle(turn.events), [turn.events]);");
  });

  it("releases a stuck send affordance only from the authoritative turn completion", () => {
    expect(source).toContain('const completedKey = `${event.session_id}:${event.turn_id ?? ""}`;');
    expect(source).toContain("setCompletedTurnKey(completedKey);");
    expect(source).toContain("if (shouldPauseQueuedMessages(stopReason))");
    expect(source).not.toContain('if (event.turn.state !== "working") setCompletedTurnKey');
    expect(source).toContain('completedTurnKey.startsWith(`${activeSessionId}:`)');
    expect(source).toContain('releaseSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId)');
    expect(source).toContain('applyQueuedMessagesAck(nextQueues, ack.command_id, ack.status, ack.error, clientId("queued"))');
    expect(source).toContain('releaseQueuedMessageClaim(queuedMessageClaimsRef.current, sessionId, commandId);');
  });

  it("shows long current directories by their tail while preserving the full path tooltip", () => {
    expect(source).toContain('<span className="session-detail session-cwd" title={session.current_dir}><FolderOpen size={11} aria-hidden="true"/><span className="path-tail">{workspacePathLabel(session.current_dir)}</span></span>');
    expect(styles).toContain('.session-cwd span { text-overflow: clip; }');
    expect(styles).toContain('.session-detail::before');
    expect(styles).toContain('.session-detail:not(:last-child)::after');
    expect(styles).toContain('border-bottom-left-radius: 5px');
    expect(styles).toContain('.session-profile { display: inline-flex; align-items: center; gap: 6px;');
    expect(source).toContain('className="composer-cwd-inline"');
    expect(source).toContain('<span className="composer-cwd-inline" title={activeSession.current_dir}><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></span>');
  });

  it("removes the access token from the visible URL while retaining the session credential", () => {
    expect(source).toContain('const TOKEN_STORAGE_KEY = "timem-web-access-token";');
    expect(source).toContain("window.sessionStorage.setItem(TOKEN_STORAGE_KEY, query)");
    expect(source).toContain("window.history.replaceState");
    expect(source).toContain('if (token) query.set("token", token);');
    expect(source).toContain('if (eventCursorRef.current > 0) query.set("last_event_seq", String(eventCursorRef.current));');
    expect(source).toContain('saveEventCursor(window.sessionStorage');
    expect(source).toContain('loadEventCursor(window.sessionStorage');
    expect(source).toContain('new WebSocket(`${scheme}://${window.location.host}/ws${queryString}`)');
    expect(source).not.toContain("Access token missing");
    expect(source).not.toContain("if (!token) {\n      setActivities");
  });

  it("does not create an optimistic ghost turn when the WebSocket send fails", () => {
    const start = source.indexOf("const sendTextForSession = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(sendText).toContain("if (!sendCommand(command, commandId))");
    expect(sendText).not.toContain("setSessions((current)");
    expect(sendText).toContain("return false;");
    expect(source).toContain("const nextDrafts = finishSessionDraftSubmission(submittingDraftSessionIdsRef, draftsBySession, reserved.sessionId, reserved.text, sent);");
    expect(source).not.toContain("setDraft(\"\");");
  });

  it("surfaces failed user operations instead of silently restoring local pending state", () => {
    expect(source).toContain("const pushActivity = useCallback");
    expect(source).toContain('activity.sessionId === "system"');
    expect(source).toContain("appendActivityToCurrentTurn(session, { ...activity, sessionId: requestedSessionId })");
    expect(source).toContain("const reportUiError = useCallback");
    expect(source).toContain("pushActivity({ id: clientId(), sessionId, tone: \"error\", title, detail, createdAt: Date.now() });");
    expect([...source.matchAll(/pushActivity\(activity\);/g)].length).toBeGreaterThanOrEqual(10);
    expect(source).toContain("Load history failed");
    expect(source).toContain("Reconnect to Timem Web before loading earlier history.");
    expect(source).toContain("Runtime unavailable");
    expect(source).toContain("Timem Web runtime is not connected. Restart timem-web and reopen the authenticated URL before sending another message.");
    expect(source).toContain("Cancel failed");
    expect(source).toContain("Reconnect to Timem Web before cancelling this turn.");
    expect(source).toContain("Remove attachment failed");
    expect(source).toContain("Reconnect to Timem Web before removing this attachment.");
    expect(source).toContain("File upload failed");
    expect(source).toContain("const params = new URLSearchParams({ session_id: activeSession.session_id });");
    expect(source).toContain('if (token) params.set("token", token);');
    expect(source).toContain("Runtime update failed");
    expect(source).toContain("Reconnect to Timem Web before applying this Session configuration.");
    expect(source).toContain("Decision reply failed");
    expect(source).toContain("Reconnect to Timem Web before replying to this runtime request.");
    expect(source).toContain("Create session failed");
    expect(source).toContain("Reconnect to Timem Web before creating a new session.");
    expect(source).toContain("Mem switch failed");
    expect(source).toContain("Reconnect to Timem Web before switching the mem directory.");
    expect(source).toContain("ToolGen start failed");
    expect(source).toContain("Reconnect to Timem Web before generating a reusable tool.");
    expect(source).not.toContain("setActivities((current)");
    expect(source).toContain('event.source === "ui_activity"');
  });

  it("groups each task into user input, bounded process, and separate final delivery", () => {
    expect(source).toContain('className="turn-user-frame"');
    expect(source).toContain('className={`turn-assistant-frame ${turn.state} ${workStreamVisible ? "" : "collapsed-work"}`}');
    expect(source).toContain('sessionId={activeSession?.session_id ?? ""}');
    expect(source).toContain('function TurnInteraction({ sessionId, turn, decisions');
    expect(source).toContain('<ActivityView key={item.key} activity={activity}/>');
    expect(source).not.toContain("function TurnEventView(");
    expect(source).toContain('className={`turn-work-scroll ${pendingUpdates > 0 ? "has-pending-updates" : ""}${visibleItems.length === 0 && decisions.length === 0 ? " empty" : " has-content"}`}');
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain('const supplementItems = useMemo(() => turn.user_entries');
 expect(source).toContain('entry.kind === "supplement"');
 expect(source).toContain('kind: "user_supplement" as const');
 expect(source).toContain('title: "[用户补充]"');
 expect(source).toContain('const timelineItems = useMemo(() => [...lifecycleItems, ...supplementItems]');
 expect(source).toContain('left.createdAt - right.createdAt');
 expect(source).toContain('turn.events.length + supplementItems.length + decisions.length');
 expect(source).toContain('activity.kind === "user_supplement"');
 expect(source).toContain('<span className="activity-mark" aria-hidden="true">💡</span>');
 expect(source).toContain('<div className="user-supplement-line"><strong>{activity.title}</strong>');
 expect(styles).toContain(".turn-work-item.user-supplement");
 expect(styles).toContain(".user-supplement-line strong");
 expect(source).toContain('const lifecycleItems = useMemo(() => lifecycleEvents.map((event) => ({'); expect(source).toContain('const processActivities = useMemo(() => timelineItems'); expect(source).toContain("id: event.event_id,"); expect(source).toContain("createdAt: event.created_at_ms,"); expect(source).not.toContain("scrollEventActivities");
    expect(source).not.toContain('const hasOnlyFreeTalk = hasOnlyFreeTalkActivity(processActivities, decisions.length);');
    expect(source).toContain('const interruptedByUser = turn.completion?.stop_reason?.toLowerCase() === "cancelledbyuser";');
    expect(source).toContain('const [showWorkStream, setShowWorkStream] = useState(() => turn.state === "working");');
    expect(source).toContain('if (!wasWorking && turn.state === "working") setShowWorkStream(true);');
    expect(source).toContain('if (wasWorking && turn.state !== "working") setShowWorkStream(false);');
    expect(source).toContain('const canCollapseCompletedWork = turn.state !== "working" && (!!turn.final_answer || interruptedByUser);');
    expect(source).toContain('const canToggleWorkStream = turn.state === "working" || canCollapseCompletedWork;');
    expect(source).toContain('const workStreamVisible = !canToggleWorkStream || showWorkStream;');
    expect(source).toContain('className={`working-chip work-title-chip work-collapse-toggle');
    expect(source).toContain('turn.state === "working" ? " active-work-title" : " completed-work-title"');
    expect(source).toContain('className="work-collapse-arrow"');
    expect(source).toContain('aria-expanded={showWorkStream}');
    expect(source).toContain('onClick={() => setShowWorkStream((visible) => !visible)}');
    expect(source).toContain('<span className="work-title-status">(Interrupted)</span>');
    expect(styles).toContain('.working-chip.interrupted-work-title');
    expect(source).toContain('{workStreamVisible && <div className="turn-work-panel">');
    expect(source).toContain('{workStreamVisible && <div className="turn-work-panel">');
    expect(source).toContain('<div className={`turn-work-scroll');
    expect(source).toContain('{pendingUpdates > 0 && <button type="button" className="turn-new-updates"');
    expect(styles).toContain(".turn-work-scroll { max-height:");
    expect(styles).toContain(".turn-work-scroll.empty { min-height: 52px; }");
    expect(styles).toContain(".turn-work-scroll.has-pending-updates");
    expect(styles).toContain(".work-collapse-toggle");
    expect(styles).toContain('.work-collapse-toggle[aria-expanded="true"] .work-collapse-arrow { transform: rotate(90deg); }');
    expect(styles).toContain(".turn-assistant-frame.collapsed-work");
    expect(styles).toContain("overflow-y: auto;");
    expect(source).toContain("followLatest.current = isNearScrollBottom({"); expect(source).toContain("const observer = new ResizeObserver(() => {"); expect(source).toContain("if (!followLatest.current) return;"); expect(source).toContain("observer.observe(content);"); expect(source).toContain("return () => observer.disconnect();");
    expect(source).toContain('className="turn-new-updates"');
  });

  it("uses frame styling without repeating user or session identity labels", () => {
    expect(source).not.toContain('<div className="message-label">You</div>');
    expect(source).not.toContain('className="message-label">{assistantName}');
    expect(source).not.toContain("assistantName={activeSession?.display_name");
    expect(source).not.toContain('<span className="eyebrow">SESSION');
    expect(source).not.toContain('activeSession?.display_name ?? "Starting Timem…"');
    expect(source).toContain('const headerModelLabel = modelDisplayName(activeSession);');
    expect(source).toContain('className={`header-model ${showRuntime ? "selected" : ""}`}');
    expect(source).toContain('<span title={headerModelLabel}>{headerModelLabel}</span><ChevronDown');
    expect(source).not.toContain('<Settings size={17}/>');
    expect(styles).toContain(".chat-header { flex: none; min-width: 0;");
    expect(styles).toContain(".header-model { min-width: 0; max-width: min(42vw, 260px); flex: 0 1 auto;");
    expect(styles).toContain(".header-model { font-size: 14px; }");
    expect(styles).toContain("text-overflow: ellipsis; white-space: nowrap;");
    expect(styles).toContain(".header-actions { flex: none;");
  });

  it("coalesces tool lifecycles and renders tools as compact subordinate rows", () => {
  expect(source).toContain("coalesceActionLifecycle(turn.events)");
  expect(source).toContain('<ToolActivityGroup key={`tool-activity-group-${item.key}`} summary={summary}/>');
  expect(source).toContain("summarizeConsecutiveToolActivities(");
  expect(source).toContain('className={`tool-activity-group ${summary.status}`}');
  expect(source).toContain('className="tool-activity-group-counts"');
  expect(source).toContain("<strong>{count}</strong>");
  expect(source).toContain("{index > 0 && <i>|</i>}");
  expect(source).toContain("tool-activity-status");
  expect(styles).toContain(".tool-activity-group { margin: 3px 0; color: #aaa; font-size: 10px; }");
  expect(styles).toContain(".tool-activity-group-count > span { overflow: hidden; font-weight: 350; text-overflow: ellipsis; }");
  expect(styles).toContain(".tool-activity-group-count > strong { color: #c7c7c7; font-weight: 750; }");
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

  it("shows inline decision submission state instead of silently disabling controls", () => {
    expect(source).toContain('const status = pending ? "Sending decision…" : locked ? "Session interaction is temporarily locked." : "";');
    expect(source).toContain('aria-busy={pending}');
    expect(source).toContain('const canAlwaysAllow = decision.event.topic.name === "core.user.approval.request";');
    expect(source).toContain('className="inline-decision-status" role="status" aria-live="polite"');
    expect(source).toContain('title={denyLabel} aria-label={denyLabel} disabled={disabled}');
    expect(source).toContain('title={allowLabel} aria-label={allowLabel} disabled={disabled}');
    expect(source).toContain('title={alwaysAllowLabel} aria-label={alwaysAllowLabel} disabled={disabled}');
    expect(source).toContain('{canAlwaysAllow && <button type="button" className="primary always-allow"');
    expect(source).toContain('onClick={() => onReply("always_allow")}>Always Allow</button>');
    expect(styles).toContain(".inline-decision-status");
    expect(styles).toContain(".inline-decision pre { max-height: min(240px, 34vh); overflow: auto;");
    expect(styles).toContain(".decision-actions .primary.sending svg");
    expect(styles).toContain(':root[data-theme="light"] .inline-decision-status');
  });

  it("keeps blocking requests in the session flow when their reply cannot be sent", () => {
    expect(source).toContain('if (sendCommand({ type: "topic_reply"');
    expect(source).toContain("worker_id: event.worker_id ?? undefined");
    expect(source).toContain("current.filter((candidate) => candidate !== decision)");
    expect(source).toContain('onCreate={(command) => {');
    expect(source).toContain("if (sendCommand(command))");
  });

  it("keeps long-session scrolling stable and draft typing away from turn reconciliation", () => {
    expect(source).toContain("const VisibleTurnList = memo(function VisibleTurnList");
    expect(source).toContain("const TurnInteraction = memo(function TurnInteraction");
    expect(source).toContain("turns={turns}");
    expect(source).not.toContain("content-visibility");
    expect(styles).toContain(".turn-interaction.completed { contain: layout style; }");
    expect(styles).toContain("scroll-behavior: auto;");
  });

  it("keeps compact mobile controls and long content inside the viewport", () => {
    expect(styles).toContain(".chat-scroll { overflow-x: hidden; }");
    expect(styles).toContain(".turn-work-scroll { overflow-x: hidden; }");
    expect(styles).toContain("overscroll-behavior-y: auto;");
    expect(styles).toContain(".markdown-body .table-scroll { max-width: 100%; overflow-x: auto;");
    expect(styles).toContain(".header-context > span:first-child { display: none; }");
    expect(styles).toContain(".composer-buttons { flex: none; width: auto; flex-wrap: nowrap; }");
    expect(styles).toContain(".stop-button { width: 34px; height: 32px;");
    expect(styles).toContain(".session-cwd, .session-profile { font-size: 11px; }");
  });

  it("uses the shared worker-aware decision key for inline request pending state", () => {
    expect(source).toContain("decisionKey, decisionsFromSessions, draftForSession");
    expect(source).toContain("pendingDecisionKeys.has(decisionKey(decision))");
    expect(source).not.toContain("function decisionKey(decision: Decision)");
    expect(viewModelSource).toContain("decision.event.context_id ?? \"\"");
    expect(viewModelSource).toContain("decision.event.worker_id ?? \"\"");
  });

  it("backs off and reconnects the WebSocket instead of only changing the label", () => {
    expect(source).toContain("const connect = () =>");
    expect(source).toContain("Math.min(10_000, 500 * 2 ** Math.min(nextAttempt - 1, 5))");
    expect(source).toContain("window.setTimeout(connect, delay)");
    expect(source).toContain("window.clearTimeout(retryTimer)");
    expect(source).toContain("let hasConnectedOnce = false;");
    expect(source).toContain("let disconnectNoticeShown = false;");
    expect(source).toContain("hasConnectedOnce = true;");
    expect(source).toContain("Runtime disconnected");
    expect(source).toContain("Timem Web lost its runtime connection. If timem-web has exited, restart it and reopen the authenticated URL.");
  });

  it("manages session-scoped MCP servers with accessible and responsive controls", () => {
    expect(source).toContain('aria-label="Manage MCP servers"');
    expect(source).toContain('aria-label="MCP servers" tabIndex={-1}');
    expect(source).toContain('<h2><strong className="mcp-session-name">{session?.display_name ?? "Current session"}</strong> \'s Capabilities</h2>');
    expect(source).not.toContain('Capabilities of current session');
    expect(source).not.toContain('are injected into its model and executor');
    expect(source).toContain('mcpButtonRef.current?.contains(target) || mcpPanelRef.current?.contains(target)');
    expect(source).toContain('if (event.key === "Escape") closeMcpPanel();');
    expect(source).toContain('type: "mcp_session_toggle"');
    expect(source).toContain('type: "mcp_server_reconnect"');
    expect(source).toContain('type: "mcp_server_upsert"');
    expect(source).toContain('window.confirm(`Delete MCP server');
    expect(source).toContain('(["stdio", "streamable_http", "sse"] as const)');
    expect(source).toContain('const [transportDrafts, setTransportDrafts] = useState(() => createMcpTransportDrafts(config.transport));');
    expect(source).toContain('onClick={() => setTransportType(type)}');
    expect(source).toContain('One MCP endpoint may return JSON or an SSE stream.');
    expect(source).toContain('role="switch" aria-checked={active}');
    expect(source).toContain('const connectionState = !active ? "disabled" : server.state === "connected" ? "connected" : server.state === "error" || !!server.error ? "failed" : "connecting";');
    expect(source).toContain('className={`mcp-session-toggle ${connectionState}`}');
    expect(source).toContain('className="mcp-port-glyph"');
    expect(source).toContain('className="mcp-port-node left"');
    expect(source).toContain('connectionState === "failed" && <X className="mcp-port-failure"');
    expect(source).not.toContain('className="mcp-toggle-label"');
    expect(source).toContain('const pendingMcpKeysRef = useRef<Set<string>>(new Set());');
    expect(source).toContain('!addPendingKey(pendingMcpKeysRef, setPendingMcpKeys, key)');
    expect(source).toContain('removePendingKey(pendingMcpKeysRef, setPendingMcpKeys, key)');
    expect(source).toContain('pendingMcpKeysRef.current.clear();');
    expect(source).toContain("One argument per line. Spaces stay inside that argument.");
    expect(protocolSource).toContain('type: "mcp_updated"');
    expect(protocolSource).toContain('| { type: "sse"; url: string; headers: Record<string, string> };');
    expect(protocolSource).toContain('mcp_server_ids: string[]');
    expect(styles).toContain(':root[data-theme="light"] .mcp-panel');
    expect(styles).toContain('.mcp-panel { position: fixed; inset: 58px 8px 8px;');
    expect(styles).toContain('.mcp-button > svg { transform: rotate(90deg); }');
    expect(styles).toContain('.mcp-port-node { position: absolute; z-index: 2; top: 2px; width: 12px; height: 12px;');
    expect(styles).toContain('.mcp-session-toggle.connected .mcp-port-node { background: #63b5f2;');
    expect(styles).toContain('.mcp-session-toggle.failed .mcp-port-link { border-color: #bd6e68; }');
    expect(styles).toContain('grid-template-columns: repeat(3, minmax(0, 1fr))');
  });

  it("keeps only global runtime availability in a banner and puts other errors in the task work stream", () => {
    expect(source).toContain('className="runtime-disconnect-banner" role="alert"');
    expect(source).toContain('const runtimeDisconnectedTitle = runtimeUnavailable ? "Runtime unavailable" : "Connection lost";');
    expect(source).not.toContain('className="host-error-banner"');
    expect(source).not.toContain("const visibleErrors = activities.filter");
    expect(source).toContain('event.source === "ui_activity"');
    expect(source).toContain("appendActivityToCurrentTurn");
    expect(source).toContain('if (event.type === "runtime_notice")');
    expect(protocolSource).toContain('type: "runtime_notice"');
    expect(styles).toContain(".runtime-disconnect-banner");
    expect(styles).not.toContain(".host-error-banner");
  });
});
