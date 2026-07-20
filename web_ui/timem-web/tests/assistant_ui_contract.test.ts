import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const appearanceSource = readFileSync(new URL("../src/appearance.ts", import.meta.url), "utf8");
const viewModelSource = readFileSync(new URL("../src/view_model.ts", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const viteConfig = readFileSync(new URL("../vite.config.ts", import.meta.url), "utf8");

describe("assistant-ui thread integration", () => {
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
    expect(source).toContain("scrollToBottomOnThreadSwitch");
    expect(source).toContain("followThreadLatest.current = isNearScrollBottom");
    expect(source).toContain("viewport.scrollTop = viewport.scrollHeight");
    expect(source).toContain("[latestTurn?.turn_id]");
  });

  it("keeps the composer usable on narrow screens while stop and tool buttons are visible", () => {
    expect(styles).toContain("@media (max-width: 520px)");
    expect(styles).toContain(".composer-actions { align-items: flex-start; gap: 8px; justify-content: space-between; }");
    expect(styles).toContain(".composer-actions > span { min-width: 0; flex: 1 1 auto; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }");
    expect(styles).toContain(".composer-buttons { width: 100%; flex-wrap: wrap; justify-content: flex-end; }");
    expect(styles).toContain(".attachment-strip { align-items: stretch; }");
    expect(styles).toContain(".pending-attachment { width: 100%; max-width: none; }");
    expect(styles).toContain(".completion-card span { white-space: normal; }");
    expect(source).toContain('{activeSession?.state === "working" && <button className={`stop-button ${isCancelling ? "sending" : ""}`');
  });

  it("makes disabled high-frequency controls visibly non-interactive", () => {
    expect(styles).toContain("button:disabled { cursor: not-allowed; }");
    expect(styles).toContain(".composer textarea:disabled { opacity: .62; cursor: not-allowed; }");
    expect(styles).toContain(".send-button:disabled, .stop-button:disabled, .attach-button:disabled, .toolrepo-toggle:disabled, .new-session:disabled, .load-history:disabled, .decision-actions button:disabled, .completion-toolgen:disabled");
    expect(styles).toContain(".send-button:disabled:hover");
    expect(styles).toContain(".attach-button:disabled:hover");
    expect(styles).toContain(".toolrepo-toggle:disabled:hover");
    expect(styles).toContain(':root[data-theme="light"] .send-button:disabled:hover');
    expect(styles).toContain(':root[data-theme="light"] .attach-button:disabled:hover');
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-toggle:disabled:hover');
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

  it("labels working-turn input as a supplement instead of a fresh send", () => {
    expect(source).toContain('const sendLabel = isCancelling ? "Cancellation in progress" : activeSession?.state === "working" ? "Send supplement" : "Send message";');
    expect(source).toContain('const missingSessionHint = activeSession ? "" : "Create a session before using Timem";');
    expect(source).toContain('const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";');
    expect(source).toContain('`${uploadingAttachmentText} · send is paused until it finishes`');
    expect(source).toContain('const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);');
    expect(source).toContain('const composerHintId = `composer-hint-${activeSessionId || "empty"}`;');
    expect(source).toContain("if (uploadingAttachment || sessionInteractionLocked) return;");
    expect(source).toContain('placeholder={!activeSession ? "Create a session to start…"');
    expect(source).toContain('aria-describedby={composerHintId}');
    expect(source).toContain('title={composerHint}');
    expect(source).toContain('<div className="composer-actions"><span id={composerHintId} role="status" aria-live="polite">{composerHint}</span>');
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
  });

  it("progressively mounts long task history and preserves the reading position", () => {
    expect(source).toContain("INITIAL_RENDERED_TURNS = 24");
    expect(source).toContain("TURN_HISTORY_PAGE_SIZE = 24");
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

  it("defaults the diagnostic activity panel to hidden", () => {
    expect(source).toContain("const [showActivity, setShowActivity] = useState(false);");
    expect(source).toContain("if (!showActivity) return;");
    expect(source).toContain("const sidePanelButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const closeSidePanel = useCallback(() => {");
    expect(source).toContain("sidePanelButtonRef.current?.focus({ preventScroll: true });");
    expect(source).toContain('if (event.key === "Escape") closeSidePanel()');
  });

  it("lets the session tools side panel collapse from the header, Escape key, and narrow-screen backdrop", () => {
    expect(source).toContain('const sessionActivityCount = sessionActivities.length;');
    expect(source).toContain('const sidePanelLabel = `${showActivity ? "Close" : "Open"} session tools and activity${sessionActivityCount ? `, ${sessionActivityCount} updates` : ""}`;');
    expect(source).toContain('aria-expanded={showActivity}');
    expect(source).toContain('aria-expanded={showActivity} aria-controls="session-side-panel"');
    expect(source).toContain('title={sidePanelLabel} aria-label={sidePanelLabel}');
    expect(source).toContain('className={`icon-button side-panel-button ${showActivity ? "selected" : ""}`}');
    expect(source).toContain('{sessionActivityCount > 0 && <span className="activity-count-badge" aria-hidden="true">{sessionActivityCount > 99 ? "99+" : sessionActivityCount}</span>}');
    expect(source).toContain('ref={sidePanelButtonRef} title={sidePanelLabel}');
    expect(source).toContain('setShowAppearance(false); setShowRuntime(false); if (showActivity) closeSidePanel(); else setShowActivity(true);');
    expect(source).toContain("const switchSidePanelTabFromKeyboard = (event: React.KeyboardEvent<HTMLDivElement>)");
    expect(source).toContain('const tabButton = tab === "tools" ? toolsTabRef.current : activityTabRef.current;');
    expect(source).toContain('tabButton?.focus({ preventScroll: true });');
    expect(source).toContain('if (event.key === "ArrowLeft" || event.key === "Home")');
    expect(source).toContain("toolsTabRef.current?.focus();");
    expect(source).toContain('} else if (event.key === "ArrowRight" || event.key === "End")');
    expect(source).toContain("activityTabRef.current?.focus();");
    expect(source).toContain('role="tablist" aria-label="Session side panel sections" onKeyDown={switchSidePanelTabFromKeyboard}');
    expect(source).toContain('ref={toolsTabRef} type="button" id="side-panel-tab-tools" role="tab" aria-label={`ToolRepo, ${session?.tools.length ?? 0} tools`} aria-controls="side-panel-tools"');
    expect(source).toContain('ref={activityTabRef} type="button" id="side-panel-tab-activity" role="tab" aria-label={`Activity, ${activities.length} updates`} aria-controls="side-panel-activity"');
    expect(source).toContain('const activityTabCount = activities.length > 99 ? "99+" : String(activities.length);');
    expect(source).toContain('>Activity<small aria-hidden="true">{activityTabCount}</small></button>');
    expect(source).toContain('ToolRepo{session && <> <small aria-hidden="true">{session.tools.length}</small></>}');
    expect(source).toContain('tabIndex={tab === "tools" ? 0 : -1}');
    expect(source).toContain('tabIndex={tab === "activity" ? 0 : -1}');
    expect(source).toContain('onClearActivities={() => {');
    expect(source).toContain('setActivities((current) => current.filter((activity) => activity.sessionId !== sessionId));');
    expect(source).toContain('tab === "activity" && activities.length > 0 && <button type="button" className="side-panel-clear"');
    expect(source).toContain('aria-label={`Clear ${activities.length} current session activity updates`}');
    expect(source).toContain('type="button" className="icon-button" title="Close side panel"');
    expect(source).toContain('id="side-panel-tools" className="toolrepo-panel" role="tabpanel" aria-labelledby="side-panel-tab-tools"');
    expect(source).toContain('id="side-panel-activity" className="activity-list" role="tabpanel" aria-labelledby="side-panel-tab-activity"');
    expect(source).toContain('<button type="button" className="side-panel-backdrop" aria-label="Close session tools and activity" onClick={closeSidePanel}');
    expect(source).toContain('onClose={closeSidePanel}');
    expect(source).toContain('aria-label="Close session tools and activity"');
    expect(source).toContain('id="session-side-panel" ref={panelRef} className="activity-panel session-side-panel" aria-label="Session tools and activity panel" tabIndex={-1}');
    expect(source).toContain('const sidePanelRef = useRef<HTMLElement | null>(null);');
    expect(source).toContain('sidePanelRef.current?.focus({ preventScroll: true });');
    expect(source).toContain('panelRef={sidePanelRef}');
    expect(source).toContain('ref={panelRef} className="activity-panel session-side-panel" aria-label="Session tools and activity panel" tabIndex={-1}');
    expect(styles).toContain(".side-panel-backdrop");
    expect(styles).toContain("z-index: 3");
    expect(styles).toContain(".app-shell, .app-shell:has(.activity-panel)");
    expect(styles).toContain(".activity-panel { position: fixed; z-index: 4;");
    expect(styles).toContain(".side-panel-header-actions");
    expect(styles).toContain(".side-panel-clear");
    expect(styles).toContain(".side-panel-button { position: relative; }");
    expect(styles).toContain(".activity-count-badge");
    expect(styles).toContain(':root[data-theme="light"] .activity-count-badge');
  });

  it("keeps narrow-screen panels as overlays so the chat and composer stay usable", () => {
    expect(styles).toContain("@media (max-width: 1050px) { .app-shell, .app-shell:has(.activity-panel) { grid-template-columns: 214px minmax(0, 1fr); }");
    expect(styles).toContain(".activity-panel { position: fixed; z-index: 4; right: 0; top: 0; bottom: 0; width: min(360px, 88vw); }");
    expect(styles).toContain(".side-panel-backdrop { display: block; position: fixed; z-index: 3; inset: 0;");
    expect(styles).toContain("@media (max-width: 720px) { .app-shell, .app-shell:has(.activity-panel) { grid-template-columns: 1fr; }");
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
    expect(source).toContain('title={runtimeLabel} aria-label={runtimeLabel}');
    expect(source).toContain('aria-expanded={showRuntime}');
    expect(source).toContain('aria-expanded={showRuntime} aria-controls="runtime-panel"');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card"');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings"');
    expect(source).toContain('const inputLabel = `${option.key} current value`;');
    expect(source).toContain('const applyLabel = pending ? `Applying ${option.key}` : dirty ? `Apply ${option.key}` : `${option.key} has no changes`;');
    expect(source).toContain('title={inputLabel} aria-label={inputLabel}');
    expect(source).toContain('title={applyLabel} aria-label={applyLabel}');
    expect(source).toContain('setShowAppearance(false); setShowActivity(false); if (showRuntime) closeRuntimePanel(); else setShowRuntime(true);');
  });

  it("opens ToolRepo from the composer and keeps the tool count inside the control", () => {
    expect(source).toContain('const [sidePanelTab, setSidePanelTab] = useState<"tools" | "activity">("tools")');
    expect(source).toContain('const toolRepoTitle = missingSessionHint || lockedControlHint || `Open ToolRepo · ${toolCount} tools`;');
    expect(source).toContain('const toolRepoLabel = missingSessionHint || lockedControlHint || `Open ToolRepo with ${toolCount} tools`;');
    expect(source).toContain('title={toolRepoTitle}');
    expect(source).toContain('aria-label={toolRepoLabel}');
    expect(source).toContain('onClick={onOpenToolRepo}');
    expect(source).toContain('onOpenToolRepo={() => { setShowAppearance(false); setShowRuntime(false); setSidePanelTab("tools"); setShowActivity(true); }}');
    expect(source).not.toContain('type: "toolgen_set"');
    expect(source).not.toContain('aria-pressed={toolgenEnabled}');
    expect(source).toContain('event.type === "tool_repo_updated"');
    expect(source).toContain('event.session_id !== activeSessionIdRef.current');
    expect(source).toContain('event.query !== toolSearchQueryRef.current');
    expect(styles).toContain(".toolrepo-toggle");
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
    expect(source).toContain('function CompletionCard({ completion, toolGenPending = false, toolGenBlocked = false, onToolGen }');
    expect(source).toContain('onToolGen={isToolGenTurn ? undefined : () => onRequestToolGen(turn.turn_id)}');
    expect(source).toContain('const toolGenLabel = toolGenPending ? "Starting ToolGen" : toolGenBlocked ? "ToolGen busy" : "ToolGen";');
    expect(source).toContain('const toolGenTitle = toolGenPending ? "ToolGen is starting for this task..." : toolGenBlocked ? "Another ToolGen task is already running in this session" : "Extract reusable tool from this task";');
    expect(source).toContain('className={`completion-toolgen ${toolGenPending ? "sending" : ""}`}');
    expect(source).toContain('title={toolGenTitle} aria-label={toolGenTitle}');
    expect(source).toContain('aria-busy={toolGenPending || undefined}');
    expect(source).toContain('disabled={toolGenPending || toolGenBlocked}');
    expect(source).toContain('<span aria-live="polite">{toolGenLabel}</span>');
    expect(source).toContain('isToolGenTurn ? "Generating tools…" : "working"');
    expect(source).toContain('isToolGenTurn ? "Generating tools…" : "Waiting for the first runtime update…"');
    expect(styles).toContain(".working-chip.toolgen-working");
    expect(styles).toContain(".completion-toolgen { display: inline-flex; align-items: center; gap: 4px; margin-left: auto; padding: 0 3px 0 9px; border: 0; border-left: 1px solid #333;");
    expect(styles).toContain(".completion-toolgen:hover { color: #8ebce0; border-left-color: #4f6474; }");
    expect(styles).toContain(':root[data-theme="light"] .completion-toolgen { border-left-color: #d5dde2; color: #437ba8; }');
    expect(styles).toContain(".completion-toolgen.sending svg");
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
    expect(source).toContain("Use a simple mem space name without slashes or '..'.");
    expect(source).toContain("This is the current mem space.");
    expect(source).toContain('const descriptionId = "mem-switch-dialog-description";');
    expect(source).toContain('const statusId = "mem-switch-dialog-status";');
    expect(source).toContain('const describedBy = validationText ? `${descriptionId} ${statusId}` : descriptionId;');
    expect(source).toContain('aria-label="Switch memory space" aria-describedby={describedBy}');
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
    expect(source).toContain('useEffect(() => {\n    setRenameToolId("");\n    setRenameValue("");\n    setContextMenu(null);\n  }, [session?.session_id, tab]);');
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
    expect(source).toContain('const activityEmptyTitle = session ? "No activity yet" : "No active session";');
    expect(source).toContain("Select or create a session to inspect runtime activity.");
    expect(source).toContain('className="activity-empty" aria-label={`${activityEmptyTitle}. ${activityEmptyText}`}');
    expect(source).toContain("<strong>{activityEmptyTitle}</strong><span>{activityEmptyText}</span>");
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

  it("shows a quiet empty state for the activity tab", () => {
    expect(source).toContain('const activityEmptyTitle = session ? "No activity yet" : "No active session";');
    expect(source).toContain("Runtime updates will appear here while this session works.");
    expect(source).toContain("Select or create a session to inspect runtime activity.");
    expect(source).toContain('className="activity-empty" aria-label={`${activityEmptyTitle}. ${activityEmptyText}`}');
    expect(source).toContain("<strong>{activityEmptyTitle}</strong><span>{activityEmptyText}</span>");
    expect(source).toContain("activities.length === 0");
    expect(styles).toContain(".activity-empty strong");
    expect(styles).toContain(".activity > div, .activity summary > div, .activity-body, .activity-detail, .activity .message-content { min-width: 0; overflow-wrap: anywhere; }");
    expect(styles).toContain(".activity-detail { max-height: min(320px, 45vh); overflow: auto;");
  });

  it("lets expanded activity details collapse again", () => {
    expect(source).toContain("function ActivityListItem");
    expect(source).toContain("const [open, setOpen] = useState(false);");
    expect(source).toContain('return <details className={`activity ${activity.tone}`');
    expect(source).toContain('onToggle={(event) => setOpen(event.currentTarget.open)}');
    expect(source).toContain('const summaryLabel = `${open ? "收起" : "展开"} Activity 详情${activity.title ? `：${activity.title}` : ""}`;');
    expect(source).toContain('aria-label={summaryLabel}');
    expect(source).toContain('className="activity-expand-label">{open ? "收起" : "展开"}</span>');
    expect(source).toContain('className="activity-collapse top" title="Collapse activity details" aria-label="Collapse activity details" onClick={collapse}>收起详情</button>');
    expect(source).toContain('className="activity-collapse" title="Collapse activity details" aria-label="Collapse activity details" onClick={collapse}>收起详情</button>');
    expect(styles).toContain(".activity:is(details) { display: block; }");
    expect(styles).toContain(".activity summary { display: grid;");
    expect(styles).toContain(".activity-collapse");
    expect(styles).toContain(':root[data-theme="light"] .activity-collapse');
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
    expect(source).toContain("const hasExpandableDetail = !!activity.detail?.trim() || !!activity.code?.trim();");
    expect(source).toContain('if (!hasExpandableDetail) return <div className={`tool-activity tool-activity-static ${running ? "running" : "settled"}`} aria-busy={running || undefined}>');
    expect(source).toContain("const toolName = toolDisplayName(activity.tool_name || activity.title);");
    expect(source).toContain('const summaryLabel = `${open ? "收起" : "展开"}工具详情：${toolName}`;');
    expect(source).toContain("const summaryContent = <>");
    expect(source).toContain('open={open} onToggle={(event) => setOpen(event.currentTarget.open)}');
    expect(source).toContain('aria-busy={running || undefined} open={open}');
    expect(source).toContain('aria-label={summaryLabel}');
    expect(source).toContain('className="tool-activity-collapse top" title={`Collapse ${toolName} details`} aria-label={`Collapse ${toolName} details`} onClick={collapse}>收起详情</button>');
    expect(source).toContain('className="tool-activity-collapse" title={`Collapse ${toolName} details`} aria-label={`Collapse ${toolName} details`} onClick={collapse}>收起详情</button>');
    expect(styles).toContain(".tool-activity-collapse");
    expect(styles).toContain(".tool-activity summary:focus-visible { background: #1f1f1f; box-shadow: inset 2px 0 0 #4d8fd7; }");
    expect(styles).toContain(':root[data-theme="light"] .tool-activity summary:focus-visible { background: #edf4f7; box-shadow: inset 2px 0 0 #2c7bbf; }');
    expect(source).toContain("toolDisplayName(activity.tool_name || activity.title)");
    expect(source).toContain("{invocationPreview && <code title={invocationPreview}>{invocationPreview}</code>}");
    expect(source).toContain('if (status === "background_running") return "background running";');
    expect(source).toContain('if (status === "timeout") return "timed out";');
    expect(styles).toContain(".tool-activity-static");
    expect(styles).toContain("grid-template-columns: 16px max-content max-content minmax(0, 1fr);");
    expect(viewModelSource).toContain('if (name === "run_bash") return "Bash";');
    expect(viewModelSource).toContain('if (name === "memmgr") return "MemMgr";');
    expect(viewModelSource).toContain('if (name === "capmgr") return "CapMgr";');
    expect(viewModelSource).toContain('if (name === "self_tool") return "Self tool";');
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
    expect(styles).toContain(':root[data-theme="light"] .toolgen-collapse');
    expect(source).not.toContain("turn.completion?.toolgen_retrospect");
  });

  it("does not expose internal model transport bookkeeping or duplicate activity labels", () => {
    expect(source).toContain('kind !== "model_request" && kind !== "model_response"');
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

  it("renders completion telemetry below final answers", () => {
    expect(source).toContain("attachTurnCompletion(session, event.outcome.message_id");
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain("<FinalAnswerDelivery text={turn.final_answer}");
    expect(source).toContain('className="turn-final-toolbar"');
    expect(source).toContain('const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(text, {');
    expect(source).toContain('copied: "Answer copied"');
    expect(source).toContain('failed: "Copy answer failed"');
    expect(source).toContain('const copyClass = copyState === "copied" ? "copy-success" : copyState === "failed" ? "copy-failed" : "";');
    expect(source).toContain('className={`final-copy ${copyClass}`}');
    expect(source).toContain('aria-label={copyLabel}');
    expect(source).toContain('title={copyLabel}');
    expect(source).toContain('<span aria-live="polite">{copyLabel}</span></button></div>');
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
    expect(styles).toContain(".turn-final-toolbar");
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
  });

  it("blocks send while cancellation is still in flight", () => {
    const start = source.indexOf("const sendText = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(sendText).toContain("cancellingSessionIds.current.has(activeSession.session_id)");
    expect(sendText).toContain("Cancellation in progress");
    expect(sendText).toContain("return false;");
  });

  it("keeps sending enabled during a working turn by bypassing assistant-ui Send", () => {
    const start = source.indexOf("const sendText = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(source).toContain("composerSendDecision");
    expect(viewModelSource).toContain('session.state === "working"');
    expect(viewModelSource).toContain('{ type: "turn_supplement"');
    expect(sendText).toContain("composerSendDecision(");
    expect(source).toContain('value={draft}');
    expect(source).toContain('onSubmit={(event) => { event.preventDefault(); void submitDraft(); }}');
    expect(source).toContain('type="submit" title={effectiveSendLabel}');
    expect(source).not.toContain("ComposerPrimitive.Send");
  });

  it("uses synchronous pending guards for rapid repeated browser clicks", () => {
    expect(source).toContain("creatingSessionRef.current");
    expect(source).toContain("const [draftsBySession, setDraftsBySession]");
    expect(source).toContain("const submittingDraftSessionIdsRef = useRef<Set<string>>(new Set());");
    expect(source).toContain("reserveSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId, draftsBySession)");
    expect(source).toContain("finishSessionDraftSubmission(submittingDraftSessionIdsRef, current, reserved.sessionId, reserved.text, sent)");
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
    expect(source).toContain('disabled={runtimeLocked || renamingSession} onClick={() => beginRename(session)}');
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
    expect(helloBranch).toContain("setDecisions([]);");
    expect(helloBranch).toContain("applySnapshot(event.snapshot);");
    expect(helloBranch).toContain("setSnapshotReady(true);");
    expect(source).toContain('if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return false;');
    expect(source).toContain('ws.onopen = () => { retryAttempt = 0; setConnected(true); setSnapshotReady(false); };');
    expect(source).toContain('setConnected(false);\n        setSnapshotReady(false);');
  });

  it("moves active selection to a live session when a reconnect or mem snapshot swaps sessions", () => {
    expect(viewModelSource).toContain("resolveActiveSessionId");
    expect(source).toContain("resolveActiveSessionId(current, snapshot.sessions)");
    expect(source).not.toContain("current || snapshot.sessions[0]?.session_id");
  });

  it("renders live task usage and session context without replacing final telemetry", () => {
    expect(source).toContain("<ContextUsageBar session={activeSession}");
    expect(source).toContain("<LiveTurnUsage turn={turn}");
    expect(source).toContain('aria-label="Current task token usage"');
    expect(source).toContain('const level = ratio >= 90 ? "critical" : ratio >= 75 ? "warning" : "normal";');
    expect(source).toContain('className={`context-usage-bar ${level}`}');
    expect(source).toContain('const contextUsageLabel = usage && limit');
    expect(source).toContain('`Context usage ${ratio}% · ${formatTokens(usage.prompt_tokens)} / ${formatTokens(limit)} input tokens`');
    expect(source).toContain('title={contextUsageLabel} aria-label={contextUsageLabel}');
    expect(source).toContain('role="status" aria-live="polite"');
    expect(source).toContain('className={`turn-work-scroll ${pendingUpdates > 0 ? "has-pending-updates" : ""}`} role="region" aria-label={isToolGenTurn ? "ToolGen work stream" : "Task work stream"}');
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
    expect(styles).toContain(".context-usage-bar");
    expect(styles).toContain(".context-usage-bar.warning strong");
    expect(styles).toContain(".context-usage-bar.critical strong");
    expect(styles).toContain(':root[data-theme="light"] .context-usage-bar.warning strong');
    expect(styles).toContain(':root[data-theme="light"] .context-usage-bar.critical strong');
    expect(styles).toContain(".turn-work-scroll.has-pending-updates");
    expect(styles).toContain(".live-turn-usage");
  });

  it("supports session rename and a distinct animated working state", () => {
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
    expect(source).toContain("aria-busy={renamingSession || undefined}");
    expect(source).toContain("Saving name...");
    expect(source).toContain("disabled={runtimeLocked || renamingSession}");
    expect(styles).toContain("@keyframes session-working-glow");
    expect(styles).toContain(".session-row.renaming-session");
    expect(styles).toContain(".session-pending");
    expect(styles).toContain(':root[data-theme="light"] .session-row.renaming-session');
    expect(styles).toContain(".sr-only { position: absolute; width: 1px; height: 1px;");
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

  it("shows the live session cwd in navigation and above the composer", () => {
    expect(source).toContain('className={`session ${session.session_id === activeSession?.session_id ? "active" : ""}`}');
    expect(source).toContain('className="session-name" title={session.display_name}');
    expect(source).toContain('className="session-cwd" title={session.current_dir}>{tailPath(session.current_dir)}');
    expect(source).toContain('className="session-profile" title={`${session.runtime_profile.provider}:${session.runtime_profile.model}`}');
    expect(source).toContain('className="session-working-icon" size={15} aria-label="Session working"');
    expect(source).not.toContain('className="session-state">busy</span>');
    expect(styles).not.toContain(".session-state");
    expect(source).toContain('className="composer-cwd" title={activeSession.current_dir} aria-label={`Current working directory: ${activeSession.current_dir}`}');
    expect(viewModelSource).toContain("context_state");
    expect(styles).toContain(".session-cwd");
    expect(styles).toContain(".composer-cwd");
  });

  it("announces runtime connection state and explains mem switch availability", () => {
    expect(source).toContain('const connectionLabel = !connected ? "Reconnecting to runtime…" : snapshotReady ? "Runtime connected" : "Syncing runtime…";');
    expect(source).toContain('const memSwitchTitle = !runtimeReady ? "Wait for the runtime snapshot before switching mem" : pendingMemSwitch ? "Mem switch is in progress" : "Switch mem space";');
    expect(source).toContain('setSnapshotReady(false)');
    expect(source).toContain('setSnapshotReady(true)');
    expect(source).toContain("const memSwitchButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const closeMemSwitchDialog = useCallback((restoreFocus = true) => {");
    expect(source).toContain("if (restoreFocus) memSwitchButtonRef.current?.focus({ preventScroll: true });");
    expect(source).toContain('className="connection-row" role="status" aria-live="polite" title={connectionLabel}');
    expect(source).toContain('className="connection-label">{connectionLabel}</span>');
    expect(source).toContain('ref={memSwitchButtonRef} className="mem-switch-button"');
    expect(source).toContain('title={memSwitchTitle} aria-label={memSwitchTitle}');
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
    expect(source).toContain('import { activityFromTopic');
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
    expect(source).toContain('TIMEM_GATEWAY_PROVIDER');
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
    expect(source).toContain('session.runtime_profile.provider');
    expect(source).toContain('session.runtime_profile.model');
    expect(styles).toContain('.session-runtime-grid');
    expect(styles).toContain('.session-runtime-control');
    expect(styles).toContain('.session-runtime-reset');
    expect(styles).toContain(':root[data-theme="light"] .session-runtime-reset');
    expect(styles).toContain('.session-profile');
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
    expect(source).toContain('title={runtimeLabel} aria-label={runtimeLabel}');
    expect(source).toContain('aria-expanded={showRuntime}');
    expect(source).toContain('if (showRuntime) closeRuntimePanel(); else setShowRuntime(true);');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card" tabIndex={-1}');
    expect(source).toContain('id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings" tabIndex={-1}');
  });

  it("lets runtime setting drafts reset to the latest server snapshot value", () => {
    expect(source).toContain("useEffect(() => setDrafts({}), [server?.runtime_options]);");
    expect(source).toContain('const pendingRuntimeLabel = pendingKeys.size ? `Applying runtime setting${pendingKeys.size === 1 ? "" : "s"}: ${Array.from(pendingKeys).join(", ")}` : "";');
    expect(source).toContain("const dirty = value !== option.value;");
    expect(source).toContain('className="secondary compact runtime-reset"');
    expect(source).toContain('title={`Reset ${option.key} to current value`}');
    expect(source).toContain('aria-label={`Reset ${option.key} to current value`}');
    expect(source).toContain("const resetDraft = () => setDrafts((current) => { const { [option.key]: _removed, ...rest } = current; return rest; });");
    expect(source).toContain('if (event.key === "Enter" && !event.nativeEvent.isComposing && dirty && !pending) { event.preventDefault(); onUpdate(option.key, value); }');
    expect(source).toContain('if (event.key === "Escape" && dirty) { event.preventDefault(); resetDraft(); }');
    expect(source).toContain("onClick={resetDraft}");
    expect(source).toContain('disabled={pending || !dirty}');
    expect(source).toContain('pendingRuntimeLabel && <p className="runtime-pending-status" role="status" aria-live="polite">{pendingRuntimeLabel}</p>');
    expect(styles).toContain(".runtime-options label > div input { flex: 1 1 auto; }");
    expect(styles).toContain(".runtime-reset { flex: none; }");
    expect(styles).toContain(".runtime-pending-status");
  });

  it("renders context compaction outside chat messages with a reduced-motion fallback", () => {
    expect(source).toContain("<ContextCompactNotice");
    expect(styles).toContain(".context-compact-notice");
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
    expect(appearanceSource).toContain('root.dataset.font = appearance.font');
    expect(appearanceSource).toContain('root.dataset.textSize = appearance.textSize');
    expect(source).toContain('const appearanceLabel = showAppearance ? "Close appearance settings" : "Open appearance settings";');
    expect(source).toContain("const appearanceButtonRef = useRef<HTMLButtonElement | null>(null);");
    expect(source).toContain("const appearancePanelRef = useRef<HTMLElement | null>(null);");
    expect(source).toContain('title={appearanceLabel} aria-label={appearanceLabel}');
    expect(source).toContain('ref={appearanceButtonRef}');
    expect(source).toContain('aria-expanded={showAppearance} aria-controls="appearance-panel"');
    expect(source).toContain('<AppearancePanel panelRef={appearancePanelRef} appearance={appearance}');
    expect(source).toContain("aria-pressed={appearance.theme === theme}");
    expect(source).toContain("aria-pressed={appearance.font === font}");
    expect(source).toContain("aria-pressed={appearance.textSize === size}");
    expect(source).toContain('title={`Use ${theme} theme`}');
    expect(source).toContain('title={`Use ${font} font for chat reading`}');
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
    expect(source).toContain('<p id={descriptionId}>Adjust theme, font, and message text size for this browser.</p>');
    expect(source).toContain('setShowRuntime(false); setShowActivity(false); if (showAppearance) closeAppearancePanel(); else setShowAppearance(true);');
    expect(styles).toContain(".appearance-panel header p");
    expect(styles).toContain(':root[data-theme="light"]');
    expect(styles).toContain(':root[data-font="serif"]');
    expect(styles).toContain(':root[data-text-size="large"]');
    expect(html).toContain('localStorage.getItem("timem-web-appearance-v1")');
    expect(html).toContain('document.documentElement.dataset.theme');
  });

  it("keeps the active session label readable in light theme after style overrides", () => {
    expect(styles).toContain(':root[data-theme="light"] .session-row.active { background: #e8e8e8; box-shadow: none; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session.active { background: transparent; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session { color: #202020; }');
    expect(styles).toContain(':root[data-theme="light"] .session-row.active .session-cwd { color: #626262; }');
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
    expect(styles).toContain("scrollbar-gutter: stable;");
    expect(styles).toContain(".table-scroll:focus-visible");
    expect(styles).toContain(':root[data-theme="light"] .table-scroll');
    expect(styles).toContain('.code-block figcaption');
    expect(styles).toContain(".code-block figcaption > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }");
    expect(styles).toContain(".code-block figcaption button { flex: none;");
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
    expect(styles).toContain(".toolrepo-toggle.count-pulse > span, .attach-button.uploading:disabled, .attach-button.uploading svg, .toolrepo-search-pending, .toolrepo-empty.searching svg, .upload-dot");
    expect(styles).toContain(".send-button.sending svg");
    expect(styles).toContain(".completion-toolgen.sending svg");
    expect(styles).toContain(".worker-state-dot.working");
    expect(styles).toContain("animation: none;");
  });

  it("keeps working-turn input visually consistent with a normal send", () => {
    expect(source).toContain('placeholder={!activeSession ? "Create a session to start…" : sessionInteractionLocked ? sessionInteractionLockReason : activeSession.state === "working" ? "继续输入…"');
    expect(source).toContain('"Ask Timem to investigate, write, or work with you."');
    expect(source).not.toContain("Ask Timem anything about this workspace");
    expect(source).toContain('activeSession?.state === "working" ? "Send supplement" : "Send message"');
    expect(source).toContain('title={effectiveSendLabel} aria-label={effectiveSendLabel}');
    expect(source).not.toContain('>Supplement</span>');
  });

  it("shows long current directories by their tail while preserving the full path tooltip", () => {
    expect(source).toContain('<span className="session-cwd" title={session.current_dir}>{tailPath(session.current_dir)}</span>');
    expect(source).toContain('className="composer-cwd" title={activeSession.current_dir} aria-label={`Current working directory: ${activeSession.current_dir}`}');
    expect(source).toContain('<FolderOpen size={13} aria-hidden="true"/>');
    expect(source).toContain('<span>{tailPath(activeSession.current_dir, 64)}</span>');
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
    expect(sendText).toContain("if (!sendCommand(decision.command))");
    expect(sendText).not.toContain("setSessions((current)");
    expect(sendText).toContain("return false;");
    expect(source).toContain("setDraftsBySession((current) => finishSessionDraftSubmission(submittingDraftSessionIdsRef, current, reserved.sessionId, reserved.text, sent));");
    expect(source).not.toContain("setDraft(\"\");");
  });

  it("surfaces failed user operations instead of silently restoring local pending state", () => {
    expect(source).toContain("const pushActivity = useCallback");
    expect(source).toContain("candidate.sessionId === activity.sessionId");
    expect(source).toContain("candidate.title === activity.title");
    expect(source).toContain("candidate.detail === activity.detail");
    expect(source).toContain("const withoutExisting = existingIndex >= 0");
    expect(source).toContain("const reportUiError = useCallback");
    expect(source).toContain("pushActivity({ id: crypto.randomUUID(), sessionId, tone: \"error\", title, detail, createdAt: Date.now() });");
    expect([...source.matchAll(/pushActivity\(activity\);/g)].length).toBeGreaterThanOrEqual(10);
    expect(source).toContain("Load history failed");
    expect(source).toContain("Reconnect to Timem Web before loading earlier history.");
    expect(source).toContain("Cancel failed");
    expect(source).toContain("Reconnect to Timem Web before cancelling this turn.");
    expect(source).toContain("Remove attachment failed");
    expect(source).toContain("Reconnect to Timem Web before removing this attachment.");
    expect(source).toContain("File upload failed");
    expect(source).toContain("Open Timem Web using the authenticated URL before attaching files.");
    expect(source).toContain("Runtime update failed");
    expect(source).toContain("Reconnect to Timem Web before applying runtime configuration.");
    expect(source).toContain("Decision reply failed");
    expect(source).toContain("Reconnect to Timem Web before replying to this runtime request.");
    expect(source).toContain("Create session failed");
    expect(source).toContain("Reconnect to Timem Web before creating a new session.");
    expect(source).toContain("Mem switch failed");
    expect(source).toContain("Reconnect to Timem Web before switching memory space.");
    expect(source).toContain("ToolGen start failed");
    expect(source).toContain("Reconnect to Timem Web before generating a reusable tool.");
    const ordinaryActivityAppend = [...source.matchAll(/setActivities\(\(current\) => \[activity,/g)].map((match) => source.slice(Math.max(0, match.index - 80), match.index + 120));
    expect(ordinaryActivityAppend).toEqual([
      expect.stringContaining("activity.kind === \"toolgen\""),
    ]);
  });

  it("groups each task into user input, bounded process, and separate final delivery", () => {
    expect(source).toContain('className="turn-user-frame"');
    expect(source).toContain('className={`turn-assistant-frame ${turn.state} ${showWorkStream ? "" : "collapsed-work"}`}');
    expect(source).toContain('sessionId={activeSession?.session_id ?? ""}');
    expect(source).toContain('function TurnInteraction({ sessionId, turn, decisions');
    expect(source).toContain('<TurnEventView key={event.event_id} event={event} sessionId={sessionId}/>');
    expect(source).not.toContain('<TurnEventView key={event.event_id} event={event} sessionId={turn.turn_id}/>');
    expect(source).toContain('className={`turn-work-scroll ${pendingUpdates > 0 ? "has-pending-updates" : ""}`}');
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain("const [showCompletedWork, setShowCompletedWork] = useState(true);");
    expect(source).toContain('const canCollapseCompletedWork = turn.state !== "working" && !!turn.final_answer;');
    expect(source).toContain('const showWorkStream = !canCollapseCompletedWork || showCompletedWork;');
    expect(source).toContain('className="work-collapse-toggle"');
    expect(source).toContain('aria-expanded={showCompletedWork}');
    expect(source).toContain('onClick={() => setShowCompletedWork((visible) => !visible)}');
    expect(source).toContain('{showWorkStream && <div className={`turn-work-scroll');
    expect(source).toContain('showWorkStream && pendingUpdates > 0');
    expect(styles).toContain(".turn-work-scroll { max-height:");
    expect(styles).toContain(".turn-work-scroll.has-pending-updates");
    expect(styles).toContain(".work-collapse-toggle");
    expect(styles).toContain(".turn-assistant-frame.collapsed-work");
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
    expect(source).toContain('const headerModelLabel = activeSession?.runtime_profile ? `${activeSession.runtime_profile.provider}:${activeSession.runtime_profile.model}` : "";');
    expect(source).toContain('className="header-model" title={headerModelLabel}>{headerModelLabel}</span>');
    expect(styles).toContain(".chat-header { flex: none; min-width: 0;");
    expect(styles).toContain(".header-model { min-width: 0; overflow: hidden;");
    expect(styles).toContain("text-overflow: ellipsis; white-space: nowrap;");
    expect(styles).toContain(".header-actions { flex: none;");
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

  it("shows inline decision submission state instead of silently disabling controls", () => {
    expect(source).toContain('const status = pending ? "Sending decision…" : locked ? "Session interaction is temporarily locked." : "";');
    expect(source).toContain('aria-busy={pending}');
    expect(source).toContain('className="inline-decision-status" role="status" aria-live="polite"');
    expect(source).toContain('title={declineLabel} aria-label={declineLabel} disabled={disabled}');
    expect(source).toContain('className={`primary ${pending ? "sending" : ""}`} title={acceptLabel} aria-label={acceptLabel} disabled={disabled}');
    expect(source).toContain('{pending ? <LoaderCircle size={16}/> : <Check size={16}/>} {pending ? "Sending…" : "Continue"}');
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

  it("backs off and reconnects the WebSocket instead of only changing the label", () => {
    expect(source).toContain("const connect = () =>");
    expect(source).toContain("Math.min(10_000, 500 * 2 ** Math.min(retryAttempt, 5))");
    expect(source).toContain("window.setTimeout(connect, delay)");
    expect(source).toContain("window.clearTimeout(retryTimer)");
  });

  it("shows host and session errors outside the default-hidden diagnostic panel", () => {
    expect(source).toContain("const visibleErrors = activities.filter");
    expect(source).toContain('const sessionActivities = activities.filter((activity) => activity.sessionId === activeSession?.session_id || activity.sessionId === "system");');
    expect(source).toContain("const visibleErrorText = visibleError ? `${visibleError.title}${visibleError.detail ? ` · ${visibleError.detail}` : \"\"}` : \"\";");
    expect(source).toContain("const visibleErrorCount = visibleErrors.length;");
    expect(source).toContain("const hiddenErrorCount = Math.max(0, visibleErrorCount - 1)");
    expect(source).toContain('const errorDetailsLabel = visibleErrorCount === 1 ? "Show this error in Activity" : `Show ${visibleErrorCount} errors in Activity`;');
    expect(source).toContain('const dismissErrorLabel = visibleError ? `Dismiss ${visibleError.title}` : "Dismiss error";');
    expect(source).toContain('className="host-error-banner" role="alert"');
    expect(source).toContain('className="host-error-text" title={visibleErrorText}');
    expect(source).toContain('className="host-error-detail"');
    expect(source).toContain("more hidden error");
    expect(source).toContain('title={dismissErrorLabel}');
    expect(source).toContain('aria-label={dismissErrorLabel}');
    expect(source).toContain('className="host-error-actions"');
    expect(source).toContain('className="host-error-details"');
    expect(source).toContain('onClick={() => { setShowAppearance(false); setShowRuntime(false); setSidePanelTab("activity"); setShowActivity(true); }}');
    expect(source).toContain('title={errorDetailsLabel}');
    expect(source).toContain('aria-label={errorDetailsLabel}');
    expect(source).toContain('aria-controls="session-side-panel" aria-expanded={showActivity && sidePanelTab === "activity"}');
    expect(source).toContain('setSidePanelTab("activity"); setShowActivity(true);');
    expect(source).toContain('className="host-error-dismiss-all"');
    expect(source).toContain('aria-label="Dismiss all visible errors"');
    expect(source).toContain('activity.tone !== "error" || (activity.sessionId !== activeSession?.session_id && activity.sessionId !== "system")');
    expect(styles).toContain(".host-error-banner");
    expect(styles).toContain(".host-error-text { flex: 1 1 auto;");
    expect(styles).toContain("-webkit-line-clamp: 2;");
    expect(styles).toContain(".host-error-detail");
    expect(styles).toContain(".host-error-banner em");
    expect(styles).toContain(".host-error-actions");
    expect(styles).toContain(".host-error-actions { flex: none; display: inline-flex; align-items: center; justify-content: flex-end; flex-wrap: wrap;");
    expect(styles).toContain(".host-error-actions .icon-button { flex: none; }");
    expect(styles).toContain(".host-error-details");
    expect(styles).toContain(".host-error-dismiss-all");
    expect(styles).toContain(':root[data-theme="light"] .host-error-banner em');
    expect(styles).toContain(':root[data-theme="light"] .host-error-dismiss-all');
    expect(styles).toContain(':root[data-theme="light"] .host-error-details');
    expect(styles).toContain("@media (max-width: 720px) { .host-error-banner { align-items: flex-start; flex-wrap: wrap;");
    expect(styles).toContain(".host-error-actions { width: 100%; justify-content: flex-end;");
  });
});
