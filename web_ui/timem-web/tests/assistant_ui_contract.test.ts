import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("../src/main.tsx", import.meta.url),
  "utf8",
);
const markdownSource = readFileSync(
  new URL("../src/markdown_render.tsx", import.meta.url),
  "utf8",
);
const clipboardSource = readFileSync(
  new URL("../src/clipboard_copy.ts", import.meta.url),
  "utf8",
);
const appearanceSource = readFileSync(
  new URL("../src/appearance.ts", import.meta.url),
  "utf8",
);
const preloadSource = readFileSync(
  new URL("../src/preload.ts", import.meta.url),
  "utf8",
);
const viewModelSource = readFileSync(
  new URL("../src/view_model.ts", import.meta.url),
  "utf8",
);
const toolStatusSource = readFileSync(
  new URL("../src/tool_status.ts", import.meta.url),
  "utf8",
);
const protocolSource = readFileSync(
  new URL("../src/protocol.ts", import.meta.url),
  "utf8",
);
const styles = readFileSync(
  new URL("../src/styles.css", import.meta.url),
  "utf8",
);
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");
const logo = readFileSync(new URL("../public/timem_logo.png", import.meta.url));
const viteConfig = readFileSync(
  new URL("../vite.config.ts", import.meta.url),
  "utf8",
);

describe("Agent Core live-state delivery", () => {
  it("uses the explicit Core turn_started event for turn and worker working state", () => {
    expect(protocolSource).toContain('type: "turn_started"');
    expect(source).toContain('if (event.type === "turn_started")');
    expect(source).toContain(
      'updateSessionWorkerState(upsertTurn(session, event.turn), event.worker_id, "working")',
    );
  });

  it("does not infer completion from pending, restored, or other turn updates", () => {
    expect(source).not.toContain('event.turn.state !== "working"');
    expect(source).toMatch(
      /if \(event\.type === "turn_finished"\)[\s\S]*?setCompletedTurnsBySession/,
    );
  });
});

describe("per-message worker role selection", () => {
  it("supports multiple selected roles and clears them only after a successful send", () => {
    expect(source).toContain("useState<Record<string, string[]>>({})");
    expect(source).toContain("role_ids: [...new Set(roleIds)]");
    expect(source).toContain(
      "if (sent && selectedRoleIds.length > 0) onRolesConsumed(reserved.sessionId)",
    );
    expect(source).toContain("selectedRoleIds.includes(role.id)");
  });

  it("shows role annotations on queued and sent user messages", () => {
    expect(source).toContain('className="queued-message-roles"');
    expect(source).toContain('className="turn-entry-roles"');
    expect(source).toContain("entry.worker_roles");
    expect(styles).toContain(".turn-entry-roles");
  });

  it("allows every role group, including ungrouped roles, to collapse", () => {
    expect(source).toContain(
      "const [collapsedRoleGroupIds, setCollapsedRoleGroupIds] = useState<Set<string>>(() => new Set());",
    );
    expect(source).toContain("const toggleRoleGroup = (groupId: string)");
    expect(source).toContain(
      'className="worker-role-group-toggle" aria-expanded={!collapsed}',
    );
    expect(source).toContain(
      "aria-controls={`worker-role-group-list-${group.id}`}",
    );
    expect(source).toContain('toggleRoleGroup("ungrouped")');
    expect(source).toContain(
      "!collapsed && <div id={`worker-role-group-list-${group.id}`}",
    );
    expect(styles).toContain(
      '.worker-role-panel .worker-role-group-toggle[aria-expanded="true"] svg { transform: rotate(90deg); }',
    );
    expect(styles).toContain(
      ".worker-role-group.collapsed { gap: 0; border-color: #292d2d; background: #191b1b; }",
    );
    expect(styles).toContain(
      ".worker-role-ungrouped { border-color: #2a2e2d; background: transparent; box-shadow: none; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-ungrouped { border-color: #e1e5e3; background: transparent; box-shadow: none; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-panel .worker-role-group-toggle { border-color: transparent; background: transparent; color: #52615d; }',
    );
  });
});

describe("user message bubble styling", () => {
  it("defaults the user bubble font to bold while keeping the appearance control configurable", () => {
    expect(appearanceSource).toContain("userBold: true,");
    expect(source).toContain("checked={appearance.userBold}");
    expect(styles).toContain(
      ':root[data-user-bold="true"] { --user-font-weight: 550; }',
    );
    expect(styles).toContain("font-weight: var(--user-font-weight)");
  });

  it("preserves soft line breaks entered in user message textareas", () => {
    expect(source).toContain("<MarkdownContent text={entry.text}/>");
    expect(styles).toContain(
      "/* Preserve textarea soft line breaks inside user-message Markdown paragraphs. */",
    );
    expect(styles).toContain(
      ".turn-user-entry .markdown-body :is(p, li) { white-space: pre-wrap; }",
    );
    expect(styles).not.toContain(
      ".message-content .markdown-body :is(p, li) { white-space: pre-wrap; }",
    );
  });

  it("uses a muted blue-gray bubble in dark mode and keeps the readable light-blue bubble in light mode", () => {
    expect(styles).toContain(
      ".turn-user-content {\n  background: #263746;\n  color: #e7f1f8;",
    );
    expect(styles).toContain(
      ".turn-user-entry .markdown-body pre { border-color: #456176; background: #111a22; }",
    );
    expect(styles).toContain(
      ".turn-user-entry .markdown-body :is(h1, h2, h3, h4, h5, h6) { color: #f1f7fb; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-user-content { background: #d9edff; color: #17324d; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-user-entry .markdown-body pre { border-color: #9fc6e3; background: #17324d; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-user-entry .markdown-body :is(h1, h2, h3, h4, h5, h6) { color: #17324d; }',
    );
  });

  it("keeps the composer surface neutral with a blue gradient border and coordinated actions", () => {
    expect(styles).toContain(
      ".composer { border: 1px solid transparent; border-radius: 24px; background: linear-gradient(#212121, #212121) padding-box, linear-gradient(135deg, #39556a 0%, #78a9ca 48%, #466b84 100%) border-box;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .composer { border-color: transparent; background: linear-gradient(#fff, #fff) padding-box, linear-gradient(135deg, #bdd5e6 0%, #669fc5 48%, #a7c8df 100%) border-box; }',
    );
    expect(styles).toContain(
      ".composer:focus-within { background: linear-gradient(#212121, #212121) padding-box, linear-gradient(135deg, #4c7390 0%, #9bc9e7 48%, #5b87a5 100%) border-box;",
    );
    expect(styles).toContain(
      ".attach-button { border-color: transparent; background: transparent; color: #9bb9cd; }",
    );
    expect(styles).toContain(
      ".send-button { border-radius: 50%; background: #b9dcf5; color: #142431;",
    );
    expect(styles).toContain(
      ".composer-text-field > .text-field-expand { top: 1px; right: 0; background: transparent; color: #9bb9cd; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .composer-text-field > .text-field-expand { background: transparent; color: #527894; }',
    );
    expect(styles).toContain(
      ".send-button:disabled:hover { background: #b9dcf5; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .attach-button:disabled:hover { background: transparent; color: #527894; }',
    );
  });
});

describe("user message selection copying", () => {
  it("normalizes trailing DOM line breaks only for a selection contained in one user message", () => {
    expect(source).toContain("onCopy={(event) => {");
    expect(source).toContain(
      "event.currentTarget.contains(selection.anchorNode)",
    );
    expect(source).toContain(
      "event.currentTarget.contains(selection.focusNode)",
    );
    expect(source).toContain(
      "normalizeCopiedUserMessageText(selection.toString())",
    );
    expect(source).toContain(
      'event.clipboardData.setData("text/plain", copiedText);',
    );
    expect(source).toContain("event.preventDefault();");
    expect(viewModelSource).toContain(
      'return text.replace(/(?:\\r?\\n)+$/, "");',
    );
  });
});

describe("assistant-ui thread integration", () => {
  it("keeps a visible boot state before the React bundle mounts", () => {
    expect(html).toContain('<div id="root">');
    expect(html).toContain("Timem is loading...");
  });

  it("uses the Timem logo as the browser tab icon", () => {
    expect(html).toContain(
      '<link rel="icon" type="image/png" href="/timem_logo.png" />',
    );
    expect(Array.from(logo.subarray(0, 8))).toEqual([
      137, 80, 78, 71, 13, 10, 26, 10,
    ]);
  });

  it("does not require crypto.randomUUID on an HTTP public-IP origin", () => {
    expect(protocolSource).toContain("export function clientId");
    expect(source).not.toContain("crypto.randomUUID()");
    expect(viewModelSource).not.toContain("crypto.randomUUID()");
  });

  it("keeps the brand concise and describes collaboration without a local-only qualifier", () => {
    expect(source).toContain(
      "Ask Timem to investigate, write, or work with you.",
    );
    expect(source).not.toContain("work with your local environment");
    expect(source).not.toContain("<small>local</small>");
    expect(styles).toContain(
      ".brand { display: grid; grid-template-columns: 41px minmax(0, 1fr); align-items: center; column-gap: 9px;",
    );
    expect(styles).not.toContain(
      ".brand { grid-template-columns: 41px minmax(0, 1fr) 26px; }",
    );
    expect(styles).toContain(
      ".brand > span { width: 100%; height: 41px; display: inline-flex; align-items: center; justify-content: center; padding-top: 2px;",
    );
    expect(styles).toContain(
      "font-size: 28px; font-weight: 800; line-height: 1; letter-spacing: .12em; text-align: center; text-shadow: 0 1px 0 #000, 0 3px 6px #000b, 0 0 12px #68b8a740; }",
    );
    expect(styles).toContain(
      ".brand { grid-template-columns: 41px minmax(0, 1fr) 30px; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .brand > span { text-shadow: 0 1px 0 #fff, 0 3px 6px #31564b45; }',
    );
  });

  it("keeps the Markdown outline anchor separate from navigation-item decoration", () => {
    expect(source).toContain(
      'className="final-answer-outline-toggle" aria-expanded={false}',
    );
    expect(source).toContain('aria-label="Show table of contents"');
    expect(source).toContain("onClick={() => setOutlineCollapsed(false)}");
    expect(source).toContain(
      '<Bookmark size={18} fill="currentColor" strokeWidth={1.6} aria-hidden="true"/><ChevronRight className="final-answer-outline-toggle-arrow"',
    );
    expect(source).not.toContain(
      '<Bookmark size={15} strokeWidth={1.8} aria-hidden="true"/><span>Contents</span>',
    );
    expect(styles).toContain(
      ".final-answer-outline-toggle { position: relative; display: inline-flex; flex: none; width: 34px; height: 52px; align-items: center; justify-content: center;",
    );
    expect(styles).toContain(
      ".final-answer-outline-toggle-arrow { position: absolute; top: 50%; right: 1px; visibility: hidden; opacity: 0;",
    );
    expect(styles).toContain(
      ".final-answer-outline-toggle:is(:hover, :focus-visible) .final-answer-outline-toggle-arrow { visibility: visible; opacity: 1;",
    );
    expect(styles).toContain("@keyframes final-outline-arrow-nudge");
    expect(styles).not.toContain(".final-answer-outline-toggle span {");
    expect(styles).toContain(
      ".final-answer-outline-card nav > button { position: relative;",
    );
    expect(styles).toContain(
      ".final-answer-outline-card nav > button::before { position: absolute;",
    );
    expect(styles).not.toContain(
      ".final-answer-outline-card button::before { position: absolute;",
    );
  });

  it("keeps resizable sidebar branding contained and mirrors borderless shadows around chat", () => {
    expect(styles).toContain(".sidebar { container-type: inline-size; }");
    expect(styles).toContain("font-size: clamp(20px, 12cqw, 28px);");
    expect(styles).toContain("letter-spacing: clamp(.06em, .5cqw, .12em);");
    expect(styles).toContain("white-space: nowrap;");
    expect(styles).toMatch(
      /@media \(min-width: 1051px\) \{[\s\S]*?\.chat-shell \{[\s\S]*?z-index: 0;[\s\S]*?border-right: 0;[\s\S]*?box-shadow: none;[\s\S]*?\.sidebar,[\s\S]*?\.worker-role-panel \{ z-index: 1; \}[\s\S]*?\.sidebar \{[\s\S]*?border-right: 0;[\s\S]*?box-shadow: 18px 0 30px -24px #000000b8;[\s\S]*?\.worker-role-panel,[\s\S]*?\.toolrepo-side-panel \{[\s\S]*?width: 100%;[\s\S]*?margin-left: 0;[\s\S]*?border-left: 0;/,
    );
    expect(styles).toContain(
      ".worker-role-panel { box-shadow: -18px 0 30px -24px #000000b8; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .sidebar { box-shadow: 18px 0 30px -24px #29443b52; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-panel { box-shadow: -18px 0 30px -24px #29443b52; }',
    );
    expect(styles).not.toContain(
      ".worker-role-panel { box-shadow: -3px 0 0 var(--management-panel); }",
    );
    expect(styles).not.toContain("box-shadow: inset -1px 0 #303b3875;");
    expect(styles).not.toContain("box-shadow: inset 1px 0 #303b3875;");
    expect(styles).toContain(
      ".toolrepo-side-panel { box-shadow: -3px 0 0 #10171f; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-side-panel { box-shadow: -3px 0 0 #f8fafb; }',
    );
    expect(styles).toContain(
      ".worker-role-panel > .sidebar-resize-handle:hover:not(:disabled),",
    );
    expect(styles).toContain(
      ".toolrepo-side-panel > .sidebar-resize-handle:focus-visible { appearance: none; border: 0; border-radius: 0; outline: 0; background: transparent; box-shadow: none; }",
    );
    expect(styles).toContain(
      ":where(.worker-role-panel) :where(button:not(.sidebar-resize-handle)),",
    );
    expect(styles).toContain(
      ':where(:root[data-theme="light"] .worker-role-panel) :where(button:not(.sidebar-resize-handle)),',
    );
    expect(styles).not.toContain(
      ".worker-role-panel button:not(.sidebar-resize-handle)",
    );
    expect(styles).not.toContain(
      ':root[data-theme="light"] .worker-role-panel button:not(.sidebar-resize-handle)',
    );
    expect(styles).not.toContain(
      ".worker-role-panel button,\n.mcp-panel-header-actions",
    );
    expect(styles).not.toContain(
      ':root[data-theme="light"] .worker-role-panel button,',
    );
    expect(styles).toContain(
      ".sidebar-resize-handle { position: absolute; z-index: 7; top: 0; bottom: 0; width: 10px; appearance: none; border: 0; border-radius: 0; padding: 0; outline: 0; background: transparent; box-shadow: none;",
    );
    expect(styles).not.toContain("width: calc(100% + 1px);");
    expect(styles).not.toContain("margin-left: -1px;");
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
    expect(source).toContain(
      "const activeMessages = activeSession?.messages ?? EMPTY_CHAT_MESSAGES;",
    );
    expect(source).not.toContain(
      "const activeMessages = activeSession?.messages ?? [];",
    );
    expect(styles).toContain(
      ".aui-thread { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow: hidden; }",
    );
    expect(styles).toContain(
      ".chat-scroll { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow-y: auto;",
    );
    expect(styles).toContain(
      "padding: 34px max(26px, calc((100% - 840px)/2 - 22px)) 24px max(26px, calc((100% - 840px)/2 + 22px));",
    );
    expect(styles).toContain(
      "padding: 16px max(26px, calc((100% - 840px)/2 - 22px)) 20px max(26px, calc((100% - 840px)/2 + 22px));",
    );
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*position:\s*sticky;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*bottom:\s*0;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*z-index:\s*3;/);
    expect(source).not.toContain("ThreadPrimitive.ScrollToBottom");
    expect(source).not.toContain(
      'title="Scroll to latest message" aria-label="Scroll to latest message"',
    );
    expect(source).toContain(
      'title={userMessageNavigation.next ? "下一条用户消息" : "导航至聊天最下方"}',
    );
    expect(source).toContain("autoScroll={false}");
    expect(source).toContain("scrollToBottomOnInitialize={false}");
    expect(source).toContain("scrollToBottomOnRunStart={false}");
    expect(source).toContain("scrollToBottomOnThreadSwitch={false}");
    expect(styles).toMatch(/\.chat-scroll\s*\{[^}]*overflow-anchor:\s*none;/);
    expect(source).toContain("sessionScrollPositionsRef");
    expect(source).toContain('viewport.style.scrollBehavior = "auto";');
    expect(source).toContain(
      "restoreSessionScrollTop(position, viewport.scrollHeight)",
    );
    expect(source).toContain("const nearBottom = isNearScrollBottom");
    expect(source).toContain("followThreadLatest.current = nearBottom;");
    expect(source).toContain("viewport.scrollTop = viewport.scrollHeight");
    expect(source).toContain("[activeSessionId, latestTurn?.turn_id]");
  });

  it("keeps the new-session action understated and makes session groups easier to scan", () => {
    expect(styles).toContain(
      ".new-session { justify-content: flex-start; border: 0; border-radius: 8px; background: transparent; color: #ececec; font-size: 12px; }",
    );
    expect(source).toContain(
      '<Folder className="session-group-folder" size={14}/>',
    );
    expect(source).toContain("<FolderPlus size={16}/>");
    expect(source).toContain(
      'className={`session-management-actions ${sessionDeleteMode ? "deleting" : ""}`}',
    );
    expect(styles).toContain(
      ".session-management-actions { min-height: 34px; display: flex; align-items: center; justify-content: space-between; padding: 0 2px 0 21px; }",
    );
    expect(styles).toContain(
      ".session-row.delete-selecting .session { padding-right: 34px; }",
    );
    expect(styles).toContain(
      "font-size: 13px; font-weight: 720; letter-spacing: .025em; }",
    );
    expect(source).toContain(
      'collapsed ? <Folder className="session-group-folder" size={14}/> : <FolderOpen className="session-group-folder" size={14}/>',
    );
    expect(source).not.toContain('className="session-group-chevron"');
  });

  it("shows an accessible animation in the session list until the runtime snapshot is ready", () => {
    expect(source).toContain(
      '<nav className="session-list" aria-label="Sessions" aria-busy={!snapshotReady}>',
    );
    expect(source).toContain(
      '!snapshotReady ? <div className="session-list-loading" role="status" aria-live="polite"><LoaderCircle size={18} aria-hidden="true"/><span>Loading sessions…</span></div> : sessionBuckets.map',
    );
    expect(styles).toContain(
      ".session-list-loading { min-height: 116px; display: flex; align-items: center; justify-content: center;",
    );
    expect(styles).toContain(
      ".session-list-loading svg { flex: none; color: #75aa9e; animation: spin 1.2s linear infinite; }",
    );
  });

  it("submits session group editors as forms and keeps empty groups visible as drop targets", () => {
    expect(source).toContain(
      '<form className="session-group-editor" onSubmit=',
    );
    expect(source).toContain(
      '<form className="session-group-editor inline" onSubmit=',
    );
    expect(source).toContain(
      'type="submit" disabled={!sessionGroupEditor.name.trim()}',
    );
    expect(source).toContain(
      "sessionGroups.map((group) => ({ id: group.id, group, sessions:",
    );
    expect(source).toContain(
      '{ id: "__ungrouped", group: undefined, sessions: ungroupedSessions }',
    );
    expect(source).toContain(
      'bucketSessions.length === 0 && <div className="session-group-drop-hint">拖动Session以归组</div>',
    );
    expect(styles).toContain(
      ".session-group-drop-hint { min-height: 28px; display: grid; place-items: center; box-sizing: border-box; margin: 1px 6px 3px;",
    );
  });

  it("supports dragging sessions between session groups like roles", () => {
    expect(source).toContain("function SortableSession(");
    expect(source).toContain("function SessionDropGroup(");
    expect(source).toContain("id: `session-group:${id}`");
    expect(source).toContain('className="session-drag"');
    expect(source).toContain("onDragEnd={finishSessionDrag}");
    expect(source).toContain(
      'type: "session_group_move", session_id: sessionId, group_id: targetGroupId',
    );
    expect(styles).toContain(".session-group.drop-target .session-group-list");
    expect(styles).toContain(".session-row.dragging");
    expect(styles).toContain(".session-drag {");
    expect(styles).toContain(
      ".session-group-list .session-row > .session-drag { margin-left: 0; }",
    );
    expect(source).toContain(
      'className={`session-endpoint-reveal ${renamingSession ? "pending" : ""}`}',
    );
    expect(source).toContain(
      'renamingSession ? <span className="session-pending">Saving name...</span> : <span>{sessionEndpointName}</span>',
    );
    expect(source).not.toContain(
      '<Sparkles size={9} className="session-model-icon"',
    );
    expect(styles).toContain(
      ".session-row:hover .session-endpoint-reveal, .session-row:has(:focus-visible) .session-endpoint-reveal, .session-endpoint-reveal.pending",
    );
    expect(styles).toContain(
      ".session-row.delete-selecting .session-endpoint-reveal { display: none; }",
    );
    expect(styles).toContain(
      "/* Compact Session rows: hidden endpoint labels never reserve name width. */",
    );
    expect(styles).toContain(
      ".session-row { min-height: 28px; border-radius: 4px; }",
    );
    expect(styles).toContain(".session-row::after {");
    expect(styles).toContain(
      ".session-row .session {\n  min-height: 28px;\n  padding: 1px 2px 1px 34px;",
    );
    expect(styles).toContain(
      ".session-drag,\n.session-expand {\n  position: absolute;",
    );
    expect(styles).toContain("opacity: 0;\n  pointer-events: none;");
    expect(styles).toContain(
      ".session-row:hover > :is(.session-drag, .session-expand.available),",
    );
    expect(styles).toContain(
      ".session-row:has(:focus-visible) > :is(.session-drag, .session-expand.available),",
    );
    expect(styles).not.toContain(".session-row:focus-within");
    expect(styles).toContain(
      ".session-row > :is(.session-drag, .session-expand.available):focus-visible {",
    );
    expect(styles).toContain(
      ".session-row:hover .session,\n.session-row:has(:focus-visible) .session { padding-left: 7px; padding-right: 58px; }",
    );
    expect(styles).toContain(
      ".session-row:hover .session-endpoint-reveal,\n.session-row:has(:focus-visible) .session-endpoint-reveal {\n  position: absolute;",
    );
    expect(styles).toContain(
      "right: 58px;\n  width: max-content;\n  max-width: min(132px, calc(100% - 72px));\n  flex: none;",
    );
    expect(styles).toContain(
      ".session-row:not(.has-workers):hover .session-endpoint-reveal,",
    );
    expect(styles).toContain(
      "right: 32px; max-width: min(156px, calc(100% - 46px));",
    );
    expect(styles).toContain(
      ".session-row:hover .session-endpoint-reveal::before,",
    );
    expect(styles).toContain(
      "background: linear-gradient(90deg, transparent, #242e2bea 78%, #242e2b 100%);",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .session-row:hover .session-endpoint-reveal::before,',
    );
    expect(styles).toContain(
      ".session-row:is(.delete-selecting, .controls-suppressed) > :is(.session-drag, .session-expand) { display: none; }",
    );
    expect(styles).toContain(
      ".session-row:is(.delete-selecting, .controls-suppressed) .session { padding-left: 7px; padding-right: 5px; }",
    );
    expect(styles).toContain(
      ".session-overlay .session-name { min-width: 0; font-size: 12px; font-weight: 400; }",
    );
    expect(source).toContain(
      'renamingSessionId === session.session_id || runtimeLocked || isDragging ? "controls-suppressed" : ""',
    );
    expect(source).toContain(
      'session.workers.length === 0 ? "No workers in this session"',
    );
    expect(source).toContain(
      'session-expand ${session.workers.length > 0 ? "available" : ""}',
    );
    expect(source).toContain(
      "server?.debug_mode && session.workers.length > 0 && expandedSessionIds.has(session.session_id)",
    );
    expect(styles).toContain(".session-group-list { padding-left: 0; }");
    expect(styles).toContain(".session-expand { right: 4px; width: 22px;");
    expect(styles).toContain(
      ".worker-name {\n  min-width: 0;\n  overflow: visible;",
    );
    expect(styles).toContain(
      "overflow-wrap: anywhere;\n  white-space: normal;",
    );
    expect(source).toContain(
      'className={`worker-row ${depth > 0 ? "child-worker" : "root-worker"}',
    );
    expect(source).toContain(
      'server?.debug_mode && session.workers.length > 0 ? "has-workers" : ""',
    );
    expect(source).not.toContain(
      "<span className={`worker-state ${worker.state}`}>{worker.state}</span>",
    );
    expect(styles).toContain("@media (hover: none), (pointer: coarse) {");
    expect(styles).toContain(
      ".session-row:not(.delete-selecting, .controls-suppressed) > :is(.session-drag, .session-expand.available)",
    );
    expect(styles).toContain(
      ".session-identity { flex: 1 1 48px; min-width: 40px; }",
    );
    expect(styles).toContain(".session-endpoint-reveal {\n  position: static;");
    expect(styles).toContain(
      "font-size: 10px;\n  transform: translateY(-50%);",
    );
    expect(styles).toContain(
      "max-width: calc(100% - 48px); flex: 0 1 auto; display: none;",
    );
    expect(styles).toContain(
      "display: inline-flex; opacity: 1; visibility: visible; transform: translateX(0);",
    );
    expect(styles).not.toContain("max-width: 34%");
  });

  it("keeps previous-message and thread-bottom navigation beside the thread", () => {
    expect(source).toContain(
      'className="turn-user-frame" data-user-message-anchor',
    );
    expect(source).not.toMatch(
      /className=\{`turn-user-entry \.\{entry\.kind\}`\} data-user-message-anchor/,
    );

    expect(source).toContain(
      'className={`user-message-navigation outline-overlap-${userMessageNavigationLayout.overlap}${userMessageNavigationHoverLocked ? " hover-locked" : ""}`}',
    );
    expect(source).toContain(
      "onPointerEnter={lockUserMessageNavigationLayout} onPointerLeave={unlockUserMessageNavigationLayout}",
    );
    expect(source).toContain(
      'title="上一条用户消息" aria-label="上一条用户消息"',
    );
    expect(source).toContain(
      'title={userMessageNavigation.next ? "下一条用户消息" : "导航至聊天最下方"}',
    );
    expect(source).toContain("disabled={!userMessageNavigation.previous}");
    expect(source).toContain(
      "disabled={!userMessageNavigation.next && !userMessageNavigation.bottom}",
    );
    expect(source).toContain(
      'const nextUserMessageAvailable = adjacentUserMessageIndex(anchorOffsets, navigationTop, "next") >= 0;',
    );
    expect(source).toContain(
      "bottom: !nextUserMessageAvailable && !isNearScrollBottom({",
    );
    expect(source).toContain(
      'if (userMessageNavigation.next) navigateUserMessage("next"); else navigateToThreadBottom();',
    );
    expect(source).toContain("top: viewport.scrollHeight");
    expect(source).toContain(
      'behavior: prefersReducedMotion() ? "auto" : "smooth"',
    );
    expect(source).not.toContain('className="scroll-to-bottom"');
    expect(styles).not.toContain(".scroll-to-bottom");
    expect(source).toContain("adjacentUserMessageIndex");
    expect(source).toContain("followThreadLatest.current = false;");
    expect(source).toContain("const durationMs = 180;");
    expect(source).toContain("requestAnimationFrame(animate)");
    expect(source).toContain("const eased = 1 - Math.pow(1 - progress, 3);");
    expect(source).not.toContain("userMessageNavigationVisible");
    expect(source).not.toContain("userMessageNavigationHideTimerRef");
    expect(source).toContain(
      '<ChevronUp size={14} strokeWidth={2.2} aria-hidden="true"/>',
    );
    expect(source).toContain(
      '<ChevronDown size={14} strokeWidth={2.2} aria-hidden="true"/>',
    );
    expect(source).not.toContain("user-message-navigation-triangle");

    expect(styles).toContain(
      ".user-message-navigation { position: absolute; z-index: 6; top: calc(50% + 38px); left: max(10px, calc((100% - 876px) / 2)); display: grid; gap: 7px;",
    );
    expect(source).toContain("markdownFloatingNavigationLayout(");
    expect(source).toContain(
      "outline-overlap-${userMessageNavigationLayout.overlap}",
    );
    expect(source).toContain(
      "pendingUserMessageNavigationLayoutRef.current = next;",
    );
    expect(source).toContain(
      "window.requestAnimationFrame(updateUserMessageNavigationLayout);",
    );
    expect(styles).toContain(
      ".user-message-navigation.hover-locked { transition: none; }",
    );
    expect(source.indexOf("className={`thread-working-away")).toBeGreaterThan(
      source.indexOf("onPointerEnter={lockUserMessageNavigationLayout}"),
    );
    expect(styles).toContain(
      ".user-message-navigation button { display: grid; place-items: center; width: 34px; height: 34px; padding: 0; border: 0; border-radius: 9px; background: #242424d9;",
    );
    expect(styles).toContain(
      "box-shadow: inset 0 1px 0 #ffffff0d, inset 0 -7px 14px #00000024, 0 9px 24px #0000002e, 0 0 10px #8db9ad0a;",
    );
    expect(styles).not.toContain(
      ".user-message-navigation button { display: grid; place-items: center; width: 34px; height: 34px; padding: 0; border: 1px",
    );
    expect(styles).not.toContain(
      "background: #242424d9; color: #8ea7a0; box-shadow: 0 9px 24px #0000002e, 0 0 0 1px #ffffff08;",
    );
    expect(styles).toContain(
      ".user-message-navigation button:disabled { opacity: .24; cursor: default; box-shadow: none; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .user-message-navigation button { background: #f7f9f9ed; color: #667d76;',
    );
    expect(styles).toContain(
      "@media (max-width: 720px) { .user-message-navigation { left: 6px; }",
    );
  });

  it("prioritizes a focused multiline composer before scrolling the chat viewport", () => {
    expect(source).toContain(
      "const composerTextareaRef = useRef<HTMLTextAreaElement | null>(null);",
    );
    expect(source).toContain("ref={composerTextareaRef}");
    expect(source).toContain(
      'textarea.addEventListener("wheel", prioritizeComposerScroll, { passive: false });',
    );
    expect(source).toContain(
      'return () => textarea.removeEventListener("wheel", prioritizeComposerScroll);',
    );
    expect(source).toContain(
      "if (document.activeElement !== textarea) return;",
    );
    expect(source).toContain(
      "const deltaY = wheelDeltaPixels(event.deltaY, event.deltaMode, textarea.clientHeight);",
    );
    expect(source).toContain(
      "if (!canScrollInDirection(textarea, deltaY)) return;",
    );
    expect(source).toContain("event.preventDefault();");
    expect(source).toContain("event.stopPropagation();");
    expect(source).toContain("textarea.scrollTop += deltaY;");
    expect(source).not.toContain("onWheel={(event) =>");
    expect(styles).toContain(
      ".composer textarea { resize: none; overflow-y: auto; }",
    );
  });

  it("renders durable runtime restarts as accessible chat timeline dividers", () => {
    expect(protocolSource).toContain('role: "user" | "assistant" | "system"');
    expect(source).toContain('message.kind === "runtime_restart"');
    expect(source).toContain("function RuntimeRestartDivider");
    expect(source).toContain(
      'className="runtime-restart-divider" role="separator"',
    );
    expect(source).toContain(
      '.filter((message): message is ChatMessage & { role: "user" | "assistant" } => message.role !== "system")',
    );
    expect(styles).toContain(".runtime-restart-divider {");
    expect(styles).toContain(
      ':root[data-theme="light"] .runtime-restart-divider',
    );
  });

  it("keeps the composer usable on narrow screens while stop and tool buttons are visible", () => {
    expect(styles).toContain("@media (max-width: 520px)");
    expect(styles).toContain(
      ".composer-actions { align-items: flex-start; gap: 8px; justify-content: space-between; }",
    );
    expect(styles).toContain(
      ".composer-paths { min-width: 0; flex: 1 1 auto; }",
    );
    expect(styles).toContain(
      ".composer-buttons { width: 100%; flex-wrap: wrap; justify-content: flex-end; }",
    );
    expect(styles).toContain(".attachment-strip { align-items: stretch; }");
    expect(styles).toContain(
      ".pending-attachment { width: 100%; max-width: none; }",
    );
    expect(styles).toContain(".completion-card span { white-space: normal; }");
    expect(source).toContain(
      '{showStopAction ? <button className={`stop-button ${isCancelling ? "sending" : ""}`',
    );
  });

  it("makes disabled high-frequency controls visibly non-interactive", () => {
    expect(styles).toContain("button:disabled { cursor: not-allowed; }");
    expect(styles).toContain(
      ".composer textarea:disabled { opacity: .62; cursor: not-allowed; }",
    );
    expect(styles).toContain(
      ".send-button:disabled, .stop-button:disabled, .attach-button:disabled, .new-session:disabled, .load-history:disabled, .decision-actions button:disabled, .completion-toolgen:disabled",
    );
    expect(styles).toContain(".mem-card:disabled");
    expect(styles).toContain(".send-button:disabled:hover");
    expect(styles).toContain(".attach-button:disabled:hover");
    expect(styles).toContain(
      ':root[data-theme="light"] .send-button:disabled:hover',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .attach-button:disabled:hover',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .load-history:disabled:hover',
    );
  });

  it("uses valid light-theme root selectors", () => {
    expect(styles).toContain(':root[data-theme="light"]');
    expect(styles).not.toContain("::root");
  });

  it("declares button types explicitly so action controls cannot become accidental form submits", () => {
    const untypedButtons = [
      ...source.matchAll(/<button(?![^>]*\btype=)[^>]*>/g),
    ].map((match) => match[0]);
    expect(untypedButtons).toEqual([]);
    expect(source).toContain('type="submit"');
  });

  it("keeps keyboard focus visible across buttons and form controls", () => {
    expect(styles).toContain(
      ":where(button, input, textarea, select, summary):focus-visible",
    );
    expect(styles).toContain("outline: 2px solid #72d7c2");
    expect(styles).toContain(
      ':root[data-theme="light"] :where(button, input, textarea, select, summary):focus-visible',
    );
    expect(styles).toContain("outline-color: #167669");
  });

  it("waits for authoritative TurnStarted before rendering ordinary submitted work", () => {
    expect(source).toContain("turnShouldRenderInTimeline");
    expect(source).toContain(
      "() => renderedSession.turns.filter(turnShouldRenderInTimeline)",
    );
    expect(source).toMatch(/visibleRuntimeRestartMarkers\(\s*visibleTurns,/);
    expect(source).toContain("turns={visibleTurns}");
    expect(viewModelSource).toContain(
      'if (turn.state !== "pending") return true;',
    );
    expect(viewModelSource).toContain(
      'turn.user_entries.some((entry) => entry.kind === "approval")',
    );
    expect(source).toContain(
      'const isWorking = turn.state === "working" && !isCancelling;',
    );
  });

  it("defines immediate Stop through an explicit recoverable turn state machine", () => {
    expect(source).toContain("const interactionPhase = turnInteractionPhase(");
    expect(source).toContain(
      "pendingDirectSubmission?.commandId ?? persistedSubmitCommandId",
    );
    expect(source).toContain(
      'composerPrimaryAction(interactionPhase, draft) === "stop"',
    );
    expect(source).toContain(
      'interactionPhase.kind === "idle" ? undefined : interactionPhase.commandId',
    );
    expect(source).toContain("target_command_id: cancelTargetCommandId");
    expect(source).toContain("activeSession.cancelling_turn_id");
    expect(protocolSource).toContain('type: "turn_cancelling"');
    expect(source).toContain('if (event.type === "turn_cancelling")');
    expect(source).toContain("targetedCancelStillApplies(");
    expect(source).toContain("persistedCancelTargetCommandIds");
  });

  it("queues working-turn input while keeping an explicit supplement escape hatch", () => {
    expect(source).toContain("const hasDraftText = !!draft.trim();");
    expect(source).toContain("const pendingDirectSubmission =");
    expect(source).toContain(
      "directSubmissionsRef.current.has(activeSessionId)",
    );
    expect(source).toMatch(
      /composerPrimaryAction\([\s\S]*?activeSession\?\.state,[\s\S]*?draft,[\s\S]*?isCancelling,[\s\S]*?pendingDirectSubmission,[\s\S]*?\) === "stop"/,
    );
    expect(source).toContain("await onCancel(pendingDirectSubmission);");
    expect(source).toContain(
      'const sendLabel = isCancelling ? "Send after stop" : activeSession?.state === "working" ? "Queue message" : "Send message";',
    );
    expect(source).toContain(
      'const missingSessionHint = activeSession ? "" : "Create a session before using Timem";',
    );
    expect(source).toContain(
      'const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";',
    );
    expect(source).toContain(
      "`${uploadingAttachmentText} · send is paused until it finishes`",
    );
    expect(source).toContain(
      'const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);',
    );
    expect(source).toContain(
      'const composerHintId = `composer-hint-${activeSessionId || "empty"}`;',
    );
    expect(source).toContain(
      "if (uploadingAttachment || sessionInteractionLocked) return;",
    );
    expect(source).toContain(
      'placeholder={!activeSession ? "Create a session to start…"',
    );
    expect(source).toContain("aria-describedby={composerHintId}");
    expect(source).toContain("title={composerHint}");
    expect(source).toContain(
      '<div className="composer-actions"><div className="composer-paths">',
    );
    expect(source).toContain(
      '<span id={composerHintId} className="sr-only" role="status" aria-live="polite">{composerHint}</span>',
    );
    expect(source).toContain(
      '<span className="composer-cwd-inline" title={activeSession.current_dir}><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></span>',
    );
    expect(source).toContain("title={effectiveSendLabel}");
    expect(source).toContain("aria-label={effectiveSendLabel}");
    expect(source).toContain(
      '{showStopAction ? <button className={`stop-button ${isCancelling ? "sending" : ""}`}',
    );
    expect(source).toContain(
      ': <button className={`send-button ${submittingDraft ? "sending" : ""}`}',
    );
    expect(source).toContain(
      "{submittingDraft ? <LoaderCircle size={17}/> : <Send size={17}/>}",
    );
    expect(styles).toContain(".send-button.sending svg");
    expect(source).toContain(
      "disabled={!activeSession || !hasDraftText || submittingDraft || uploadingAttachment || sessionInteractionLocked}",
    );
    expect(source).toContain("<CircleStop size={17}/> Stop");
    expect(styles).toContain(".stop-button.sending svg");
    expect(styles).toContain(
      ".send-button.sending svg, .stop-button.sending svg",
    );
    expect(source).toContain(
      'aria-label={isCancelling ? "Cancellation requested" : lockedControlHint || "Cancel current turn"}',
    );
    expect(source).toContain("const submitDraftAsSupplement = () => {");
    expect(source).toContain(
      'event.key !== "Enter" || event.nativeEvent.isComposing',
    );
    expect(source).toContain("event.metaKey || event.ctrlKey");
    expect(source).toContain("submitDraftAsSupplement();");
    expect(source).toContain('clientId("supplement")');
    expect(source).toContain(
      "availableAttachments.map((attachment) => attachment.id)",
    );
    expect(source).toContain(
      "attachmentIds?: readonly string[], forceSupplement = false",
    );
    expect(source).toContain("attachmentIds,\n forceSupplement,");
  });

  it("loads older stored history explicitly and preserves the reading position", () => {
    expect(source).toContain("STORED_HISTORY_PAGE_SIZE = 200");
    expect(source).toContain("previousScrollMetrics.current");
    expect(source).toContain(
      "preservePrependScrollTop(previous, viewport.scrollHeight)",
    );
    expect(source).toContain("canLoadStoredHistory");
    expect(source).toContain('sendCommand({ type: "history_page"');
    expect(source).toContain("limit: STORED_HISTORY_PAGE_SIZE");
    expect(source).toContain(
      "const historyButtonLabel = sessionInteractionLocked",
    );
    expect(source).toContain(
      "`${sessionInteractionLockReason} · earlier history is locked`",
    );
    expect(source).toContain("Loading earlier history…");
    expect(source).toContain(
      "Load ${STORED_HISTORY_PAGE_SIZE} older stored tasks",
    );
    expect(source).toContain(
      'className={`load-history ${loadingHistory ? "loading" : ""}`} title={historyButtonLabel} aria-label={historyButtonLabel} aria-live="polite" aria-busy={loadingHistory || undefined}',
    );
    expect(source).toContain(
      '{loadingHistory && <LoaderCircle size={13} aria-hidden="true"/>}',
    );
    expect(source).toContain("<span>{historyButtonLabel}</span>");
    expect(styles).toContain(".load-history");
    expect(styles).toContain(".load-history.loading svg");
    expect(styles).toContain(
      ".load-history.loading svg, .send-button.sending svg",
    );
    expect(source).not.toContain("event.currentTarget.scrollTop <= 48");
  });

  it("keeps multi-session navigation reachable on mobile", () => {
    expect(source).toContain(
      'const mobileSessionsLabel = showMobileSessions ? "Close session navigation" : "Open session navigation";',
    );
    expect(source).toContain(
      "const mobileSessionButtonRef = useRef<HTMLButtonElement | null>(null);",
    );
    expect(source).toContain(
      "const mobileSidebarRef = useRef<HTMLElement | null>(null);",
    );
    expect(source).toContain(
      "const closeMobileSidebar = useCallback((restoreFocus = true) => {",
    );
    expect(source).toContain(
      "if (restoreFocus) mobileSessionButtonRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      "mobileSidebarRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      'id="session-navigation" ref={mobileSidebarRef} className={`sidebar ${leftSidebarCollapsed ? "collapsed" : ""} ${showMobileSessions ? "mobile-open" : ""}`} aria-label="Session navigation" tabIndex={-1}',
    );
    expect(source).toContain(
      "ref={mobileSessionButtonRef} title={mobileSessionsLabel} aria-label={mobileSessionsLabel}",
    );
    expect(source).toContain(
      '<button type="button" className="mobile-sidebar-backdrop" aria-label="Close session navigation" onClick={() => closeMobileSidebar()}',
    );
    expect(source).toContain(
      'aria-label="Close sessions" onClick={() => closeMobileSidebar()}',
    );
    expect(source).toContain(
      "setShowNewSession(true); closeMobileSidebar(false);",
    );
    expect(source).toContain("if (!showMobileSessions) return;");
    expect(source).toContain(
      'if (event.key === "Escape") closeMobileSidebar()',
    );
    expect(source).toContain(
      "setActiveSessionId(session.session_id); closeMobileSidebar();",
    );
    expect(source).toContain(
      'aria-current={session.session_id === activeSession?.session_id ? "page" : undefined}',
    );
    expect(styles).toContain(".icon-button.mobile-session-button");
    expect(styles).toContain(".mobile-sidebar-backdrop");
    expect(styles).toContain(
      ".sidebar.mobile-open { visibility: visible; transform: translateX(0);",
    );
    expect(styles).toContain(
      ".icon-button.mobile-session-button { display: grid;",
    );
  });

  it("supports persistent, resizable desktop sidebars and a refined collapsed session rail", () => {
    expect(source).toContain(
      'const SIDEBAR_LAYOUT_STORAGE_KEY = "timem.sidebar-layout.v1";',
    );
    expect(source).toContain(
      "const [sidebarLayout, setSidebarLayout] = useState<SidebarLayout>(loadSidebarLayout);",
    );
    expect(source).toContain("saveSidebarLayout(sidebarLayout);");
    expect(source).toContain(
      'const startSidebarResize = useCallback((side: "left" | "right"',
    );
    expect(source).toContain(
      'window.matchMedia("(max-width: 1050px)").matches',
    );
    expect(source).toContain('className="sidebar-resize-handle left"');
    expect(source).toContain('className="sidebar-resize-handle right"');
    expect(source).toContain(
      'className="collapsed-brand brand-logo-toggle brand-logo-restore"',
    );
    expect(source).toContain(
      'className="brand-logo-toggle" title="Hide session navigation" aria-label="Hide session navigation"',
    );
    expect(source).toContain('className="brand-scale-corner top-right"');
    expect(source).toContain('className="brand-scale-corner bottom-left"');
    expect(source).toContain('className="brand-scale-corner top-left"');
    expect(source).toContain('className="brand-scale-corner bottom-right"');
    expect(source).not.toContain(
      'className="desktop-sidebar-toggle" title="Hide session navigation"',
    );
    expect(source).not.toContain("<span>TM</span>");
    expect(source).toContain(
      'className={`sidebar-settings-button ${showAppearance ? "active" : ""}`}',
    );
    expect(source).toContain(
      'className="desktop-sidebar-toggle worker-role-collapse"',
    );
    expect(styles).toContain(
      ".worker-role-panel .worker-role-collapse { border: 0; background: transparent; color: #718b83; box-shadow: none; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-panel .worker-role-collapse { border: 0; background: transparent; color: #698078; box-shadow: none; }',
    );
    expect(source).toContain(
      "const leftSidebarCollapsed = sidebarLayout.leftCollapsed && !showMobileSessions;",
    );
    expect(source).toContain(
      "const rightSidebarCollapsed = sidebarLayout.rightCollapsed && !showRoles;",
    );
    expect(source).toContain('className="header-session-cluster"');
    expect(source).toContain('{activeSession?.display_name ?? "No session"}');
    expect(styles).toContain(
      ".chat-header { position: relative; display: grid; grid-template-columns: auto minmax(0, 1fr) auto; column-gap: 7px; }",
    );
    expect(styles).toContain(
      ".header-session-cluster { width: fit-content; max-width: 152px; min-width: 0; display: grid; grid-column: 1; grid-row: 1;",
    );
    expect(styles).toContain(
      ".header-session-cluster > strong { width: auto; max-width: 152px; overflow: hidden;",
    );
    expect(styles).toContain(
      ".header-session-cluster .header-model-guide-anchor { width: fit-content; max-width: 152px; min-width: 0; }",
    );
    expect(styles).toContain(
      ".header-session-cluster .header-model { width: auto; max-width: 152px; min-height: 23px;",
    );
    expect(styles).not.toContain(
      ".app-shell.left-sidebar-collapsed .chat-header {",
    );
    expect(styles).not.toContain(
      ".app-shell.left-sidebar-collapsed .header-session-cluster .header-model",
    );
    expect(source).toContain(
      '<div className="header-session-cluster"><strong title={activeSession?.display_name ?? "No session"}>{activeSession?.display_name ?? "No session"}</strong><div className="header-model-guide-anchor">',
    );
    expect(styles).toContain(
      "grid-template-columns: var(--left-sidebar-width, 220px) minmax(0, 1fr) var(--right-sidebar-width, 286px);",
    );
    expect(styles).toContain(
      ".app-shell.left-sidebar-collapsed.right-sidebar-collapsed { grid-template-columns: 58px minmax(0, 1fr) 44px; }",
    );
    expect(styles).toContain(
      ".sidebar.collapsed > :not(.collapsed-brand, .sidebar-footer) { display: none; }",
    );
    expect(styles).toContain(
      ".collapsed-brand { display: grid; place-items: center; width: 42px; height: 42px;",
    );
    expect(styles).toContain(
      ".brand-logo-toggle { position: relative; width: 41px; height: 41px; display: grid; place-items: center;",
    );
    expect(styles).toContain(
      ".brand-scale-corner { position: absolute; width: 11px; height: 11px;",
    );
    expect(styles).toContain(
      ".brand-scale-corner.top-right { top: -5px; right: -5px; border-top: 2px solid #83b9af; border-right: 2px solid #83b9af; border-top-right-radius: 7px; }",
    );
    expect(styles).toContain(
      ".brand-scale-corner.bottom-left { bottom: -5px; left: -5px; border-bottom: 2px solid #83b9af; border-left: 2px solid #83b9af; border-bottom-left-radius: 7px; }",
    );
    expect(styles).toContain(
      ".brand-scale-corner.top-left { top: 1px; left: 1px; border-top: 2px solid #83b9af; border-left: 2px solid #83b9af; border-top-left-radius: 7px; }",
    );
    expect(styles).toContain(
      ".brand-scale-corner.bottom-right { right: 1px; bottom: 1px; border-right: 2px solid #83b9af; border-bottom: 2px solid #83b9af; border-bottom-right-radius: 7px; }",
    );
    expect(styles).toContain(
      ".brand-logo-toggle:not(.brand-logo-restore):is(:hover, :focus-visible) .brand-scale-corner.top-right { transform: translate(-3px, 3px); }",
    );
    expect(styles).toContain(
      ".brand-logo-toggle:not(.brand-logo-restore):is(:hover, :focus-visible) .brand-scale-corner.bottom-left { transform: translate(3px, -3px); }",
    );
    expect(styles).toContain(
      ".brand-logo-restore:is(:hover, :focus-visible) .brand-scale-corner.top-left { transform: translate(-4px, -4px); }",
    );
    expect(styles).toContain(
      ".brand-logo-restore:is(:hover, :focus-visible) .brand-scale-corner.bottom-right { transform: translate(4px, 4px); }",
    );
    expect(styles).toContain(".brand-logo-toggle { pointer-events: none; }");
    expect(styles).toContain(".brand-scale-corner { display: none; }");
    expect(styles).toContain(
      ".header-session-cluster { width: fit-content; max-width: 152px; min-width: 0; display: grid; grid-column: 1; grid-row: 1; justify-items: start;",
    );
  });

  it("keeps ToolRepo as a dedicated panel without the diagnostic Activity feed", () => {
    expect(source).toContain(
      "const [showToolRepo, setShowToolRepo] = useState(false);",
    );
    expect(source).toContain(
      "const toolRepoButtonRef = useRef<HTMLButtonElement | null>(null);",
    );
    expect(source).toContain(
      "const toolRepoPanelRef = useRef<HTMLElement | null>(null);",
    );
    expect(source).toContain("const closeToolRepoPanel = useCallback(() => {");
    expect(source).toContain(
      "toolRepoButtonRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      'if (event.key === "Escape") closeToolRepoPanel()',
    );
    expect(source).toContain(
      "const activeToolCount = activeSession?.tools.length ?? 0;",
    );
    expect(source).toContain(
      'const toolRepoLabel = showToolRepo ? "Close ToolRepo" : `Open ToolRepo · ${activeToolCount} reusable tools`;',
    );
    expect(source).toContain(
      'aria-expanded={showToolRepo} aria-controls="toolrepo-panel"',
    );
    expect(source).toContain(
      '{toolGenEnabled && <button type="button" ref={toolRepoButtonRef} title={toolRepoLabel} aria-label={toolRepoLabel}',
    );
    expect(source).toContain(
      '<Wrench size={17}/><span className="toolrepo-header-count" aria-hidden="true">{activeToolCount}</span>',
    );
    expect(source).toContain(
      '{toolGenEnabled && showToolRepo && <button type="button" className="side-panel-backdrop" aria-label="Close ToolRepo" onClick={closeToolRepoPanel}',
    );
    expect(source).toContain("function ToolRepoPanel");
    expect(source).toContain(
      'id="toolrepo-panel" ref={panelRef} className="toolrepo-side-panel session-side-panel" aria-label="ToolRepo" tabIndex={-1}',
    );
    expect(source).toContain("<strong>ToolRepo</strong>");
    expect(source).toContain(
      '<div className="side-panel-title"><Wrench size={15}/><strong>ToolRepo</strong></div>',
    );
    expect(source).not.toContain(
      "<strong>ToolRepo</strong>{session && <small>",
    );
    expect(source).not.toContain("side-panel-tab-activity");
    expect(source).not.toContain(">Activity<");
    expect(source).not.toContain("function ActivityListItem");
    expect(source).not.toContain("activity-count-badge");
    expect(styles).toContain(".side-panel-backdrop");
    expect(styles).toContain("z-index: 3");
    expect(styles).toContain(
      ".app-shell, .app-shell:has(.toolrepo-side-panel)",
    );
    expect(styles).toContain(
      ".toolrepo-side-panel { position: fixed; z-index: 4;",
    );
  });

  it("keeps narrow-screen panels as overlays so the chat and composer stay usable", () => {
    expect(styles).toContain(
      "@media (max-width: 1050px) { .app-shell, .app-shell:has(.toolrepo-side-panel) { grid-template-columns: 214px minmax(0, 1fr); }",
    );
    expect(styles).toContain(
      ".toolrepo-side-panel { position: fixed; z-index: 4; right: 0; top: 0; bottom: 0; width: min(360px, 88vw); }",
    );
    expect(styles).toContain(
      ".side-panel-backdrop { display: block; position: fixed; z-index: 3; inset: 0;",
    );
    expect(styles).toContain(
      "@media (max-width: 720px) { .app-shell, .app-shell:has(.toolrepo-side-panel) { grid-template-columns: 1fr; }",
    );
    expect(styles).toContain(
      ".sidebar { display: flex; visibility: hidden; position: fixed; z-index: 12;",
    );
    expect(styles).toContain(
      ".mobile-sidebar-backdrop { display: block; position: fixed; z-index: 11;",
    );
    expect(styles).toContain(".chat-scroll { padding: 24px 17px; }");
    expect(styles).toContain(".composer-wrap { padding: 12px 17px 16px; }");
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*position:\s*sticky;/);
    expect(styles).toMatch(/\.composer-wrap\s*\{[^}]*bottom:\s*0;/);
    expect(styles).toMatch(/\.turn-work-scroll\s*\{[^}]*max-height:\s*52vh;/);
  });

  it("labels the runtime settings control for assistive and contract testing", () => {
    expect(source).toContain(
      'const runtimeLabel = showRuntime ? "Close runtime information" : "Open runtime information";',
    );
    expect(source).toContain(
      "aria-label={`${runtimeLabel}: ${headerModelLabel}`}",
    );
    expect(source).toContain("aria-expanded={showRuntime}");
    expect(source).toContain(
      'aria-expanded={showRuntime} aria-controls="runtime-panel"',
    );
    expect(source).toContain(
      'id="runtime-panel" ref={panelRef} className="runtime-card"',
    );
    expect(source).toContain(
      'id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings"',
    );
    expect(source).toContain(
      "const inputLabel = `${optionLabel} current value`;",
    );
    expect(source).toContain(
      "const applyLabel = pending ? `Applying ${optionLabel}` : dirty ? `Apply ${optionLabel}` : `${optionLabel} has no changes`;",
    );
    expect(source).toContain("title={inputLabel} aria-label={inputLabel}");
    expect(source).toContain("title={applyLabel} aria-label={applyLabel}");
    expect(source).toContain(
      "setShowAppearance(false); setShowMcp(false); setShowToolRepo(false); if (showRuntime) closeRuntimePanel(); else setShowRuntime(true);",
    );
  });

  it("shows the runtime bind host and public-token mode from the server snapshot", () => {
    expect(protocolSource).toContain("bind_host: string;");
    expect(protocolSource).toContain("public_access: boolean;");
    expect(source).toContain(
      'const bindLabel = `${server.bind_host || "127.0.0.1"}:${server.port}`;',
    );
    expect(source).toContain("{bindLabel}");
    expect(source).toContain("public · token required");
    expect(source).not.toContain("localhost:{server.port}");
  });

  it("opens ToolRepo from the header and keeps the composer focused on message actions", () => {
    expect(source).toContain(
      "const [showToolRepo, setShowToolRepo] = useState(false);",
    );
    expect(source).toContain(
      '{toolGenEnabled && <button type="button" ref={toolRepoButtonRef} title={toolRepoLabel} aria-label={toolRepoLabel}',
    );
    expect(source).toContain(
      'className={`icon-button toolrepo-header-button ${showToolRepo ? "selected" : ""} ${toolCountPulseSessionId === activeSession?.session_id ? "count-pulse" : ""}`}',
    );
    expect(source).not.toContain("className={`toolrepo-toggle");
    expect(source).not.toContain("onOpenToolRepo: () => void;");
    expect(source).not.toContain('type: "toolgen_set"');
    expect(source).toContain('event.type === "tool_repo_updated"');
    expect(source).toContain("event.session_id !== activeSessionIdRef.current");
    expect(source).toContain("event.query !== toolSearchQueryRef.current");
    expect(styles).toContain(".toolrepo-header-button");
    expect(styles).toContain(".toolrepo-header-count");
    expect(styles).toContain("@keyframes tool-count-pulse");
  });

  it("keeps ToolGen behind a disabled-by-default Beta setting", () => {
    expect(source).toContain(
      'import { loadToolGenEnabled, saveToolGenEnabled } from "./beta_features";',
    );
    expect(source).toContain(
      "const [toolGenEnabled, setToolGenEnabled] = useState(loadToolGenEnabled);",
    );
    expect(source).toContain("saveToolGenEnabled(toolGenEnabled);");
    expect(source).toContain(
      'type SettingsSection = "appearance" | "endpoints" | "memory" | "toolgen";',
    );
    expect(source).toContain(
      "<Wrench size={16}/><span><strong>ToolGen</strong></span>",
    );
    expect(source).toContain(
      'onClick={() => selectSettingsSection("toolgen")}',
    );
    expect(source).toContain(
      'role="switch" className="settings-feature-switch" aria-checked={toolGenEnabled}',
    );
    expect(source).toContain("onToolGenEnabledChange(!toolGenEnabled)");
    expect(source).toContain("Disabled by default");
    expect(source).toContain(
      "const requestActiveToolGen = useCallback((turnId: string) => {",
    );
    expect(source).toContain("if (!toolGenEnabled || !activeSessionKey");
    expect(source).toContain(
      "onRequestToolGen={toolGenEnabled ? requestActiveToolGen : undefined}",
    );
    expect(source).toContain(
      "{toolGenEnabled && toolgenDialog && toolgenDialog.sessionId === activeSessionKey && <ToolGenDialog",
    );
    expect(source).toContain("if (!toolGenEnabled) return;");
    expect(source).toContain("if (!toolGenEnabled) {");
    expect(source).toContain("setShowToolRepo(false);");
    expect(source).toContain(
      '{toolGenEnabled && <button type="button" ref={toolRepoButtonRef}',
    );
    expect(source).toContain(
      "{toolGenEnabled && showToolRepo && <ToolRepoPanel",
    );
    expect(styles).toContain('.settings-feature-switch[aria-checked="true"]');
    expect(styles).toContain(
      '.settings-feature-switch[aria-checked="true"] .settings-feature-switch-thumb',
    );
    expect(styles).toContain(".toolgen-beta-note");
    expect(styles).toContain(':root[data-theme="light"] .toolgen-beta-card');
  });

  it("starts ToolGen manually from an exact completed turn with optional guidance", () => {
    expect(source).toContain(
      "manualToolGenCommand(request.sessionId, request.turnId, text)",
    );
    expect(source).toContain(
      "const pendingToolgenRequestsRef = useRef<Set<string>>(new Set());",
    );
    expect(source).toContain(
      "if (pendingToolgenRequestsRef.current.has(requestKey)) return;",
    );
    expect(source).toContain(
      "pendingToolgenRequestsRef.current.add(requestKey);",
    );
    expect(source).toContain(
      "setPendingToolgenRequests(new Set(pendingToolgenRequestsRef.current));",
    );
    expect(source).toContain(
      "pendingToolgenRequestsRef.current.delete(requestKey);",
    );
    expect(source).toContain(
      "pendingToolgenRequestsRef.current = removeToolgenRequestsForSession(pendingToolgenRequestsRef.current, event.session_id);",
    );
    expect(source).toContain("pendingToolgenRequestsRef.current.clear();");
    expect(source).toContain("function ToolGenDialog");
    expect(source).toContain(
      'const descriptionId = "toolgen-dialog-description";',
    );
    expect(source).toContain('const statusId = "toolgen-dialog-status";');
    expect(source).toContain(
      "const describedBy = pending ? `${descriptionId} ${statusId}` : descriptionId;",
    );
    expect(source).toContain("aria-describedby={describedBy}");
    expect(source).toContain("Extract reusable tool");
    expect(source).toContain("preserve reusable work from the completed task");
    expect(source).toContain(
      "Optional: preferred interface, language, scope, or reusable workflow…",
    );
    expect(source).toContain("Additional guidance");
    expect(source).toContain(
      'event.key === "Enter" && !event.nativeEvent.isComposing',
    );
    expect(source).toContain("const activePendingToolGenTurnIds = useMemo(");
    expect(source).toContain("const activeToolGenBusy = useMemo(");
    expect(source).toContain(
      "pendingToolGenTurnIds={activePendingToolGenTurnIds}",
    );
    expect(source).toContain("toolGenSessionBusy={activeToolGenBusy}");
    expect(source).toContain(
      "toolGenPending={pendingToolGenTurnIds.has(turn.turn_id)}",
    );
    expect(source).toContain(
      "toolGenBlocked={toolGenSessionBusy && !pendingToolGenTurnIds.has(turn.turn_id)}",
    );
    expect(source).toContain(
      "function CompletionCard({ completion, toolGenPending = false, toolGenBlocked = false, onToolGen, answerActions }",
    );
    expect(source).toContain(
      "onToolGen={isToolGenTurn || !onRequestToolGen ? undefined : () => onRequestToolGen(turn.turn_id)}",
    );
    expect(source).toContain(
      'const toolGenLabel = toolGenPending ? "Starting ToolGen" : toolGenBlocked ? "ToolGen busy" : "ToolGen";',
    );
    expect(source).toContain(
      'const toolGenTitle = toolGenPending ? "ToolGen is starting for this task..." : toolGenBlocked ? "Another ToolGen task is already running in this session" : "Extract reusable tool from this task";',
    );
    expect(source).toContain(
      'className={`completion-toolgen ${toolGenPending ? "sending" : ""}`}',
    );
    expect(source).toContain("title={toolGenTitle} aria-label={toolGenTitle}");
    expect(source).toContain("aria-busy={toolGenPending || undefined}");
    expect(source).toContain("disabled={toolGenPending || toolGenBlocked}");
    expect(source).toContain('<span aria-live="polite">{toolGenLabel}</span>');
    expect(source).toContain(
      'isToolGenTurn ? "Generating tools…" : <span className="working-label">working</span>',
    );
    expect(source).not.toContain("Waiting for the first runtime update…");
    expect(source).not.toContain("已接收，正在开始处理");
    expect(source).not.toContain("工具生成任务已接收，正在启动");
    expect(source).toContain(
      'turn.state === "working" && !hasVisibleProcess && <div className="turn-starting-status"',
    );
    expect(source).toContain(
      '<span className="turn-starting-mark" aria-hidden="true"/><span>working</span>',
    );
    expect(styles).toContain(".working-chip.toolgen-working");
    expect(styles).toContain(
      ".completion-toolgen { display: inline-flex; align-items: center; gap: 4px; margin-left: auto; padding: 0 3px 0 9px; border: 0; border-left: 1px solid #333;",
    );
    expect(styles).toContain(
      ".completion-toolgen:hover { color: #8ebce0; border-left-color: #4f6474; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .completion-toolgen { border-left-color: #d5dde2; color: #437ba8; }',
    );
    expect(styles).toContain(".completion-toolgen.sending svg");
  });

  it("labels completed normal and ToolGen work frames with minimal text-only titles", () => {
    expect(source).toContain('isToolGenTurn ? "ToolGen" : "Thought/Action"');
    expect(source).not.toContain(
      '<span className="work-title-dot" aria-hidden="true"/>',
    );
    expect(source).toContain("completed-work-title");
    expect(source).toContain("toolgen-completed-title");
    expect(styles).toContain(
      ".working-chip.completed-work-title { color: #d4d4d4; }",
    );
    expect(styles).toContain(
      ".working-chip.work-title-chip { min-height: 0; padding: 0; border: 0; border-radius: 0; background: transparent; }",
    );
    expect(styles).not.toContain(".work-title-dot");
    expect(styles).toContain(
      ':root[data-theme="light"] .working-chip.completed-work-title { color: #465a63; }',
    );
  });

  it("identifies restored ToolGen work by topic rather than event source", () => {
    expect(source).toContain(
      '(event.payload.topic as { name?: string } | undefined)?.name === "core.toolgen"',
    );
    expect(source).not.toContain(
      'event.source === "core_topic" && (event.payload.topic',
    );
  });

  it("lets modal backdrops dismiss dialogs without closing while editing inside them", () => {
    expect(source).toContain(
      'className="modal-backdrop" role="presentation" aria-label="Dismiss create session" onClick={closeIfIdle}',
    );
    expect(source).toContain(
      'className="modal-backdrop" role="presentation" aria-label="Dismiss ToolGen dialog" onClick={closeIfIdle}',
    );
    expect(source).toContain(
      'className="settings-center-backdrop" role="presentation" aria-label="Dismiss settings" onClick={closeIfIdle}',
    );
    expect(source).toContain("onClick={(event) => event.stopPropagation()}");
    expect(source).toContain(
      "const closeIfIdle = () => { if (!creating) onClose(); };",
    );
    expect(source).toContain(
      "const closeIfIdle = () => { if (!pending) onClose(); };",
    );
    expect(source).toContain("const FOCUSABLE_DIALOG_SELECTOR =");
    expect(source).toContain("function useDialogFocusTrap()");
    expect(source).toContain(
      'document.addEventListener("keydown", containFocus, true);',
    );
    expect(source).toContain("useDialogFocusTrap();");
    expect(source).toContain(
      'role="dialog" aria-modal="true" aria-labelledby="settings-center-title"',
    );
    expect(source).toContain("disabled={busy} onClick={closeIfIdle}");
    expect(source).toContain(
      'if (event.key === "Escape" && !pendingMemSwitch && !pendingMemRetention) closeAppearancePanel();',
    );
    expect(source).toContain(
      "const busy = retentionPending || conversationCapacityPending || favoriteCapacityPending || switchPending || temporaryItemsDeleting;",
    );
    expect(source).toContain(
      'if (event.key === "Enter" && !event.nativeEvent.isComposing && !switchPending && cleanedPath && !pathUnchanged)',
    );
    expect(source).toContain(
      'role="status" aria-live="polite">{switchPending ? "Switching MEM and loading its sessions…"',
    );
    expect(source).toContain(
      "const workspaceModalOpen = showAppearance || chatLibraryMode !== null;",
    );
    expect(source).toContain(
      "inert={workspaceModalOpen} aria-hidden={workspaceModalOpen || undefined}",
    );
    expect(source).toContain(
      'document.body.classList.toggle("workspace-modal-open", workspaceModalOpen)',
    );
    expect(source).toContain(
      'return createPortal(<div className="settings-center-backdrop"',
    );
    expect(source).toContain(
      'return createPortal(<div className="chat-library-center-backdrop"',
    );
    expect(styles).toContain("body.workspace-modal-open { overflow: hidden; }");
    expect(styles).toContain(
      ".settings-center-backdrop { position: fixed; z-index: 45;",
    );
    expect(styles).toContain(".settings-center { width: min(880px, 100%);");
    expect(styles).toContain(
      "/* Borderless surface pass: paint-cheap Settings navigation and a modal workspace veil. */",
    );
    expect(styles).toContain(
      ".settings-center-backdrop {\n  backdrop-filter: blur(9px) saturate(.78);\n  -webkit-backdrop-filter: blur(9px) saturate(.78);",
    );
    expect(styles).toContain(
      ".settings-center-nav button:hover:not(:disabled) {\n  background: #202824;\n  color: #f0f6f3;\n  box-shadow: none;\n  transform: none;",
    );
    expect(styles).toContain(
      ".decision-modal,\n.session-modal {\n  border: 0;",
    );
    expect(styles).toContain(".endpoint-menu,\n.mcp-panel {\n  border: 0;");
    expect(styles).toContain(
      ".mcp-list {\n  min-height: 0;\n  max-height: min(320px, calc(100vh - 210px));",
    );
    expect(styles).toContain(
      "overscroll-behavior: contain;\n  scrollbar-gutter: stable;",
    );
    expect(styles).toContain(
      ".worker-role-group-editor input,\n.worker-role-editor input,\n.worker-role-editor textarea {\n  border: 0;",
    );
  });

  it("renders ToolRepo browsing, search, rename and terminal-open controls", () => {
    expect(source).toContain(
      'placeholder={session ? "Search names and code" : "Select a session first"}',
    );
    expect(source).toContain('aria-label="Clear ToolRepo search"');
    expect(source).toContain('onClick={() => onSearchQueryChange("")}');
    expect(source).toContain('if (event.key === "Escape" && searchQuery)');
    expect(source).toContain(
      'event.preventDefault(); event.stopPropagation(); onSearchQueryChange("");',
    );
    expect(source).toContain(
      'const sortLabel = sort === "time" ? "recent update" : sort;',
    );
    expect(source).toContain(
      "const sortControlLabel = `Sort ToolRepo by ${sortLabel}`;",
    );
    expect(source).toContain(
      "title={sortControlLabel} aria-label={sortControlLabel}",
    );
    expect(source).toContain('type: "tool_repo_detail"');
    expect(source).toContain('type: "tool_repo_rename"');
    expect(source).toContain('type: "tool_repo_open_terminal"');
    expect(source).toContain(
      'const [pendingToolDetailKey, setPendingToolDetailKey] = useState("");',
    );
    expect(source).toContain(
      "const [pendingToolRenameKeys, setPendingToolRenameKeys] = useState<Set<string>>(() => new Set());",
    );
    expect(source).toContain(
      "pendingToolRenameIds={activeSession ? pendingToolIdsForSession(pendingToolRenameKeys, activeSession.session_id) : new Set()}",
    );
    expect(source).toContain(
      "setPendingToolRenameKeys((current) => removeToolKeysForSession(current, event.session_id));",
    );
    expect(source).toContain(
      'pendingToolDetailId={activeSession && pendingToolDetailKey.startsWith(`${activeSession.session_id}:`) ? pendingToolDetailKey.slice(activeSession.session_id.length + 1) : ""}',
    );
    expect(source).toContain(
      "const pendingTool = pendingToolDetailId ? sortedTools.find((tool) => tool.tool_id === pendingToolDetailId) : undefined;",
    );
    expect(source).toContain(
      "const loadingDetail = pendingToolDetailId === tool.tool_id;",
    );
    expect(source).toContain(
      "const renamingTool = pendingToolRenameIds.has(tool.tool_id);",
    );
    expect(source).toContain(
      'useEffect(() => {\n    setRenameToolId("");\n    setRenameValue("");\n    setContextMenu(null);\n  }, [session?.session_id]);',
    );
    expect(source).toContain(
      "useEffect(() => {\n    setContextMenu(null);\n  }, [searchQuery, sort, selectedTool?.summary.tool_id, tools.length]);",
    );
    expect(source).toContain(
      'const pendingToolDetailLabel = pendingTool ? `Loading ${pendingTool.name} tool directory` : "";',
    );
    expect(source).toContain(
      "aria-busy={loadingDetail || renamingTool || undefined}",
    );
    expect(source).toContain(
      'renamingTool ? "Renaming..." : loadingDetail ? "Loading details..."',
    );
    expect(source).toContain("disabled={renamingTool}");
    expect(source).toContain(
      'className="toolrepo-detail loading" aria-busy="true" aria-label={pendingToolDetailLabel}',
    );
    expect(source).toContain("Reading tool directory…");
    expect(source).toContain(
      "title={`Stop viewing ${pendingTool.name} details`}",
    );
    expect(source).toContain(
      "aria-label={`Stop viewing ${pendingTool.name} details`}",
    );
    expect(source).toContain(
      'className="toolrepo-detail-loading" role="status" aria-live="polite" aria-label={pendingToolDetailLabel}',
    );
    expect(source).toContain("Reading directory tree...");
    expect(source).toContain(
      'role="treeitem" tabIndex={0} aria-selected={selectedTool?.summary.tool_id === tool.tool_id} aria-expanded={expanded}',
    );
    expect(source).toContain(
      "setPendingToolDetailKey(`${activeSession.session_id}:${toolId}`);",
    );
    expect(source).toContain(
      'setPendingToolDetailKey((key) => key === `${event.session_id}:${event.detail.summary.tool_id}` ? "" : key);',
    );
    expect(source).toContain("Tool detail failed");
    expect(source).toContain(
      "Reconnect to Timem Web before opening tool details.",
    );
    expect(source).toContain("Tool rename failed");
    expect(source).toContain("Open terminal failed");
    expect(source).toContain(
      "Reconnect to Timem Web before renaming this tool.",
    );
    expect(source).toContain(
      "Reconnect to Timem Web before opening a tool directory.",
    );
    expect(source).toContain(
      "if (name && name !== tool.name && !onRenameTool(tool.tool_id, name)) return;",
    );
    expect(source).toContain(
      'if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); finishToolRename(tool); }',
    );
    expect(source).toContain(
      'if (event.key === "Escape") { event.preventDefault(); setRenameToolId(""); setRenameValue(""); }',
    );
    expect(source).toContain(
      "const renameKey = toolKey(activeSession.session_id, toolId);",
    );
    expect(source).toContain(
      "setPendingToolRenameKeys((current) => new Set(current).add(renameKey));",
    );
    expect(source).toContain(
      "setPendingToolRenameKeys((current) => { const next = new Set(current); next.delete(renameKey); return next; });",
    );
    expect(source).toContain("在命令行中打开目录");
    expect(source).toContain("selectedTool?.summary.tool_id === toolId");
    expect(source).toContain("setSelectedTool(null)");
    expect(source).toContain(
      "const expanded = selectedTool?.summary.tool_id === tool.tool_id;",
    );
    expect(source).toContain("aria-expanded={expanded}");
    expect(source).toContain(
      "onClick={() => { if (expanded) onCollapseTool(); else onSelectTool(tool.tool_id); }}",
    );
    expect(source).toContain(
      "const toolToggleLabel = expanded ? `收起 ${tool.name} 详情` : `展开 ${tool.name} 详情`;",
    );
    expect(source).toContain("aria-label={toolToggleLabel}");
    expect(source).toContain(
      "title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`}",
    );
    expect(source).toContain(
      'className="toolrepo-toggle-state">{expanded ? "收起" : "展开"}</em>',
    );
    expect(source).toContain(
      'const [pendingToolSearchKey, setPendingToolSearchKey] = useState("");',
    );
    expect(source).toContain(
      'setPendingToolSearchKey((key) => key === `${event.session_id}:${event.query}` ? "" : key);',
    );
    expect(source).toContain("setPendingToolSearchKey(searchKey);");
    expect(source).toContain(
      'if (!sendCommand({ type: "tool_repo_search", session_id: activeSession.session_id, query: toolSearchQuery, limit: 200 }))',
    );
    expect(source).toContain(
      'setPendingToolSearchKey((key) => key === searchKey ? "" : key);',
    );
    expect(source).toContain(
      'reportUiError("ToolRepo search failed", "Reconnect to Timem Web before searching saved tools.", activeSession.session_id);',
    );
    expect(source).toContain(
      "searchPending={!!activeSession && pendingToolSearchKey === `${activeSession.session_id}:${toolSearchQuery}`}",
    );
    expect(source).toContain(
      'className={searchPending ? "searching" : ""} aria-busy={searchPending}',
    );
    expect(source).toContain(
      'searchPending && <span className="toolrepo-search-pending" aria-hidden="true"/>',
    );
    expect(source).toContain(
      "event.session_id === activeSessionIdRef.current && toolSearchQueryRef.current.trim()",
    );
    expect(source).toContain(
      "return { ...current, [event.session_id]: event.tools };",
    );
    expect(source).toContain('event.type === "tool_repo_search_result"');
    expect(source).toContain(
      "!event.tools.some((tool) => tool.tool_id === selected.summary.tool_id)",
    );
    expect(source).toContain("selectedTool.files.map");
    expect(source).toContain(
      "title={`${toolToggleLabel} · ${tool.language} · ${tool.tool_type}`}",
    );
    expect(source).toContain("title={selectedTool.summary.synopsis}");
    expect(source).toContain(
      "title={`${file.path} · ${formatBytes(file.bytes)}`}",
    );
    expect(source).toContain("if (selectedTool?.summary.tool_id === toolId)");
    expect(source).toContain("setSelectedTool(null);");
    expect(source).toContain(
      'const toolRepoEmptyTitle = !session ? "No active session" : searchPending ? "Searching ToolRepo…" : hasToolSearch ? "No matching tools" : "No reusable tools yet";',
    );
    expect(source).toContain(
      "Searching tool names and file contents. Results will update automatically.",
    );
    expect(source).toContain(
      'className={`toolrepo-empty ${searchPending ? "searching" : ""}`} aria-label={`${toolRepoEmptyTitle}. ${toolRepoEmptyText}`} aria-busy={searchPending || undefined}',
    );
    expect(source).toContain("const toolRepoResultText = !session");
    expect(source).toContain("searchPending");
    expect(source).toContain('"Searching..."');
    expect(source).toContain(
      "`${sortedTools.length} of ${session.tools.length} tools`",
    );
    expect(source).toContain(
      '`${sortedTools.length} tool${sortedTools.length === 1 ? "" : "s"}`',
    );
    expect(source).toContain(
      'className="toolrepo-result-count" aria-live="polite"',
    );
    expect(source).toContain(
      "Select or create a session to browse its ToolRepo.",
    );
    expect(source).toContain(
      'placeholder={session ? "Search names and code" : "Select a session first"}',
    );
    expect(source).toContain("disabled={!session} onChange");
    expect(source).toContain("clear search to show all saved tools");
    expect(source).toContain('aria-label="Tool directory tree"');
    expect(source).toContain('aria-label="Collapse tool detail"');
    expect(source).toContain(
      'if (event.key === "Escape") setContextMenu(null);',
    );
    expect(source).toContain(
      "const contextMenuActionRef = useRef<HTMLButtonElement>(null);",
    );
    expect(source).toContain(
      "contextMenuActionRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      "Math.max(8, Math.min(event.clientX, window.innerWidth - 220))",
    );
    expect(source).toContain(
      "Math.max(8, Math.min(event.clientY, window.innerHeight - 76))",
    );
    expect(source).toContain(
      'className="toolrepo-context-menu" role="menu" aria-label="Tool actions"',
    );
    expect(source).toContain(
      'onKeyDownCapture={(event) => { if (event.key === "Escape") { event.preventDefault(); event.stopPropagation(); setContextMenu(null); } }}',
    );
    expect(source).toContain(
      '<button ref={contextMenuActionRef} type="button" role="menuitem" onClick={() => { onOpenTerminal(contextMenu.toolId); setContextMenu(null); }}>',
    );
    expect(source).toContain('className="toolrepo-detail-collapse"');
    expect(source).toContain(">收起详情</button>");
    expect(source).not.toContain('className="toolrepo-detail-footer"');
    expect(source).not.toContain("<MarkdownContent text={selectedTool.readme}");
    expect(styles).toContain(
      ".toolrepo-item.selected .toolrepo-item-main > svg",
    );
    expect(styles).toContain(".toolrepo-toggle-state");
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-toggle-state");
    expect(styles).toContain(".toolrepo-item.loading-detail");
    expect(styles).toContain(".toolrepo-item.renaming-tool");
    expect(styles).toContain(".toolrepo-edit:disabled");
    expect(styles).toContain(
      ".toolrepo-item.loading-detail .toolrepo-item-main small",
    );
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-open");
    expect(styles).toContain(".toolrepo-item.selected .toolrepo-edit");
    expect(styles).toContain(".toolrepo-controls label button");
    expect(styles).toContain(".toolrepo-controls label.searching");
    expect(styles).toContain(".toolrepo-search-pending");
    expect(styles).toContain(".toolrepo-empty.searching svg");
    expect(styles).toContain(
      ".toolrepo-result-count { flex: none; padding: 0 12px 8px;",
    );
    expect(styles).toContain(
      ".toolrepo-browser { min-height: 0; flex: 1; display: flex; flex-direction: column; overflow: hidden;",
    );
    expect(styles).toContain(
      ".toolrepo-list { min-height: 0; flex: 1 1 auto; display: grid; align-content: start; overflow: auto;",
    );
    expect(styles).toContain(
      ".toolrepo-detail { flex: none; min-height: 0; max-height: 260px;",
    );
    expect(styles).toContain(".toolrepo-detail.loading");
    expect(styles).toContain(".toolrepo-detail-loading");
    expect(styles).toContain(
      ".toolrepo-detail button { flex: none; min-height: 26px;",
    );
    expect(styles).toContain(
      ".toolrepo-detail > header button:not(.toolrepo-detail-collapse)",
    );
    expect(styles).toContain(
      ".toolrepo-detail button.toolrepo-detail-collapse { width: auto; padding: 0 8px; }",
    );
    expect(styles).toContain(
      ".toolrepo-files { flex: none; display: grid; max-height: 180px;",
    );
    expect(styles).not.toContain(".toolrepo-detail-footer");
    expect(styles).toContain(
      ".toolrepo-context-menu { position: fixed; z-index: 40; max-width: min(260px, calc(100vw - 16px));",
    );
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-empty');
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-empty.searching svg',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-result-count',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-controls label.searching',
    );
    expect(styles).toContain("@keyframes search-pending-pulse");
    expect(styles).toContain(".toolrepo-empty.searching svg, .upload-dot");
    expect(styles).toContain(".activity-empty strong");
    expect(styles).toContain(
      ':root[data-theme="light"] .activity-empty strong',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-detail > header strong',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-toggle-state',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-item.selected .toolrepo-toggle-state',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-detail-loading',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .toolrepo-item.loading-detail',
    );
    expect(styles).toContain(':root[data-theme="light"] .toolrepo-files > div');
    expect(styles).not.toContain(".toolrepo-readme");
  });

  it("makes ToolRepo tree items keyboard navigable without hijacking nested controls", () => {
    expect(source).toContain('role="treeitem" tabIndex={0}');
    expect(source).toContain(
      'event.target.closest("button, input, select, textarea")',
    );
    expect(source).toContain('event.key === "Enter" || event.key === " "');
    expect(source).toContain('event.key === "ArrowRight" && !expanded');
    expect(source).toContain('event.key === "ArrowLeft" && expanded');
    expect(source).toContain('event.key === "Escape" && expanded');
    expect(styles).toContain(".toolrepo-item:focus-visible");
  });

  it("provides a keyboard reachable ToolRepo terminal action on each tool row", () => {
    expect(source).toContain('className="toolrepo-open"');
    expect(source).toContain(
      "title={`Open ${tool.name} directory in terminal`}",
    );
    expect(source).toContain(
      "aria-label={`Open ${tool.name} directory in terminal`}",
    );
    expect(source).toContain("onClick={() => onOpenTerminal(tool.tool_id)}");
    expect(styles).toContain(
      "grid-template-columns: minmax(0, 1fr) 26px 26px;",
    );
    expect(styles).toContain(".toolrepo-open, .toolrepo-edit");
    expect(styles).toContain(".toolrepo-open:focus-visible");
  });

  it("shows readable tool names and invocation previews in the working pane", () => {
    expect(source).toContain("function toolInvocationPreview");
    expect(source).toContain("activity.detail?.split");
    expect(source).toContain("const detail = activity.detail?.trim();");
    expect(source).toContain("const code = activity.code?.trim();");
    expect(source).toContain("const hasExpandableDetail = !!detail || !!code;");
    expect(source).toContain("const running = isToolActivityRunning(status);");
    expect(source).toContain("const [open, setOpen] = useState(false);");
    expect(source).toContain(
      'if (!hasExpandableDetail) return <div className={`tool-activity tool-activity-static ${bashActivity ? "bash-activity" : ""}${pollingActivity ? " poll-activity" : ""} ${running ? "running" : "settled"}`} aria-busy={running || undefined}>',
    );
    expect(source).toContain(
      "const toolName = toolActivityDisplayName(activity.tool_name || activity.title, activity.tool_mode);",
    );
    expect(source).toContain(
      'const summaryLabel = `${open ? "收起" : "展开"}工具详情：${toolName}`;',
    );
    expect(source).toContain("const summaryContent = <>");
    expect(source).toContain(
      'className="tool-activity-command" title={invocationPreview}',
    );
    expect(source).not.toContain("!hasExpandableDetail && invocationPreview");
    expect(source).toContain(
      "open={open} onToggle={(event) => setOpen(event.currentTarget.open)}",
    );
    expect(source).toContain("aria-busy={running || undefined} open={open}");
    expect(source).toContain("aria-label={summaryLabel}");
    expect(source).not.toContain("tool-activity-collapse");
    expect(styles).not.toContain(".tool-activity-collapse");
    expect(styles).toContain(
      ".tool-activity-body { max-height: 280px; overflow: auto; margin: 0 0 5px 22px; padding: 0; border: 0; }",
    );
    expect(styles).toContain(
      ".tool-activity-body .turn-work-detail { padding: 2px 0 3px; }",
    );
    expect(styles).toContain(".tool-activity-body .code-block { margin: 0;");
    expect(source).toContain(
      '{detail && <div className="turn-work-detail"><MarkdownContent text={detail}/></div>}',
    );
    expect(source).toContain(
      '<MarkdownContent text={fencedCode(activity.code_language ?? "text", code)} />',
    );
    expect(styles).toContain(
      ".tool-activity summary:focus-visible { background: #1f1f1f; box-shadow: inset 2px 0 0 #4d8fd7; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .tool-activity summary:focus-visible { background: #edf4f7; box-shadow: inset 2px 0 0 #2c7bbf; }',
    );
    expect(source).toContain(
      "toolActivityDisplayName(activity.tool_name || activity.title, activity.tool_mode)",
    );
    expect(source).toContain('from "./tool_status"');
    expect(toolStatusSource).toContain(
      'if (status === TOOL_STATUS_BACKGROUND_RUNNING) return "background running";',
    );
    expect(toolStatusSource).toContain(
      'if (status === "timeout") return "timed out";',
    );
    expect(styles).toContain(".tool-activity-static");
    expect(styles).toContain(
      "grid-template-columns: 16px max-content max-content minmax(0, 1fr);",
    );
    expect(viewModelSource).toContain(
      'if (name === "run_bash") return "Bash";',
    );
    expect(viewModelSource).toContain(
      'if (name === "memmgr") return "MemMgr";',
    );
    expect(viewModelSource).toContain(
      'if (name === "capmgr") return "CapMgr";',
    );
    expect(viewModelSource).toContain(
      'if (name === "self_tool") return "Self Tool";',
    );
  });

  it("carries the live working marker into the completed Thought Action chip", () => {
    expect(styles).toContain(
      ".turn-assistant-frame.working .working-chip { font-size: 14px; font-weight: 720; color: #7ebce8; letter-spacing: 0; }",
    );
    expect(styles).toContain(
      ".turn-assistant-frame.working .working-chip .pulse { width: 8px; height: 8px; background: #3485dc; box-shadow: 0 0 0 4px #3485dc24; }",
    );
    expect(source).toContain(
      "working-chip work-title-chip work-collapse-toggle",
    );
    expect(source).toContain(
      'turn.state === "working" ? " active-work-title" : " completed-work-title"',
    );
    expect(source).not.toContain(
      '<span className="work-title-dot" aria-hidden="true"/>',
    );
    expect(source).not.toContain(
      'isToolGenTurn ? <Wrench size={11}/> : <span className="pulse"/>',
    );
    expect(styles).toContain(".work-collapse-toggle:hover { color: #f0f0f0; }");
    expect(styles).not.toContain(".work-collapse-toggle:hover { border-color:");
    expect(styles).not.toContain(
      ".working-chip.interrupted-work-title { border-color:",
    );
    expect(styles).not.toContain(
      ".working-chip.completed-work-title.toolgen-completed-title { border-color:",
    );
    expect(styles).toContain(
      ".working-chip.work-title-chip { min-height: 0; padding: 0; border: 0; border-radius: 0; background: transparent; }",
    );
    expect(styles).toContain(
      ".turn-assistant-frame.working .working-chip.active-work-title { min-width: 0; color: #8fc9f1; font-size: 11px; font-weight: 700; letter-spacing: 0; }",
    );
    expect(source).toContain('<span className="working-label">working</span>');
    expect(styles).toContain(".turn-assistant-frame.working .working-label {");
    expect(styles).toContain("color: #8fc9f1;");
    expect(styles).not.toContain("background-size: 320% 100%;");
    expect(styles).not.toContain("animation: working-label-sweep");
    expect(styles).not.toContain("@keyframes working-label-sweep");
    expect(styles).not.toContain("will-change: background-position;");
    expect(styles).not.toContain(
      "70%, 100% { background-position: -100% 50%; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-assistant-frame.working .working-label {',
    );
    expect(styles).toContain("@media (prefers-reduced-motion: reduce) {");
    expect(styles).toContain("color: #8fc9f1;");
    expect(styles).toContain("background: none;");
    expect(styles).toContain("animation: none;");
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-assistant-frame.working .working-label { color: #286a9b; }',
    );
    expect(styles).toContain(
      ".turn-work-item { grid-template-columns: 16px minmax(0, 1fr); gap: 6px; padding: 6px 6px; color: #aaa; font-size: 12px;",
    );
    expect(source).toContain(
      '<span className="activity-thinking-dot" aria-hidden="true"/>',
    );
    expect(source).not.toContain('activity.tone === "thinking" ? "💡"');
    expect(source).toContain(
      'activity.kind === "free_talk" ? " free-talk" : ""',
    );
    expect(styles).toContain(
      ".activity-thinking-dot { width: 5px; height: 5px; border-radius: 50%; background: #111; }",
    );
    expect(styles).toContain(
      ".turn-work-item.free-talk .turn-work-detail { font-size: 90%; }",
    );
    expect(styles).toContain(
      ".turn-work-item.free-talk .turn-work-detail .message-content { font-size: inherit; }",
    );
    expect(styles).toContain(
      ".worker-role-editor input::placeholder, .worker-role-editor textarea::placeholder { font-size: inherit; }",
    );
    expect(source).toContain(
      'className={`worker-role-editor ${editingId ? "editing" : "creating"}`}',
    );
    expect(styles).toContain(
      ".worker-role-editor input { height: 34px; padding: 0 9px; font-size: var(--worker-role-control-size); line-height: 1.4; }",
    );
    expect(styles).toContain(
      ".worker-role-editor textarea { min-height: 112px; resize: vertical; padding: 9px; font-size: var(--worker-role-control-size); line-height: 1.5; }",
    );
    expect(source).toContain(
      '<section className="appearance-role-fonts" aria-labelledby="appearance-user-fonts-title"><h4 id="appearance-user-fonts-title">User</h4>',
    );
    expect(source).toContain(
      '<section className="appearance-role-fonts" aria-labelledby="appearance-agent-fonts-title"><h4 id="appearance-agent-fonts-title">Agent</h4>',
    );
    expect(source).toContain(
      '<span className="appearance-checkbox" aria-hidden="true"><Check size={12} strokeWidth={3}/></span><span>粗体</span>',
    );
    expect(styles).toContain(
      "/* Appearance bold toggles use a conventional checkbox and explicit check mark. */",
    );
    expect(styles).toContain(
      '.appearance-bold-option input[type="checkbox"]:checked + .appearance-checkbox {',
    );
    expect(styles).toContain("background: #397b69;\n  color: #f2fff9;");
    expect(styles).toContain(
      '.appearance-bold-option input[type="checkbox"]:checked + .appearance-checkbox > svg { opacity: 1; transform: scale(1); }',
    );
    expect(styles).toContain(
      '.appearance-bold-option input[type="checkbox"]:focus-visible + .appearance-checkbox {',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .appearance-bold-option input[type="checkbox"]:checked + .appearance-checkbox {',
    );
    expect(source).not.toContain(
      '<fieldset className="appearance-role-fonts"><legend>User</legend>',
    );
    expect(source).not.toContain(
      '<fieldset className="appearance-role-fonts"><legend>Agent</legend>',
    );
    expect(styles).toContain(
      ".appearance-settings-pane .appearance-role-fonts {",
    );
    expect(styles).toContain(
      "width: 100%;\n  max-width: 100%;\n  min-width: 0;\n  box-sizing: border-box;\n  overflow: hidden;",
    );
    expect(styles).toContain(".appearance-role-fonts > h4 {");
    expect(styles).toContain("color: #dbe6e1;");
    expect(styles).toContain(
      ".appearance-role-fonts .appearance-font-selects label {",
    );
    expect(styles).toContain("color: #bac8c2;\n  font-weight: 650;");
    expect(styles).toContain(
      ':root[data-theme="light"] .appearance-role-fonts > h4 { color: #263b34; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .appearance-role-fonts .appearance-font-selects label { color: #465b53; }',
    );
    expect(styles).toContain(".appearance-settings-pane > fieldset > legend {");
    expect(styles).toContain("color: #d4e0db;\n  font-weight: 720;");
    expect(styles).toContain(
      ".appearance-settings-pane .segmented-control button {",
    );
    expect(styles).toContain("color: #b9c7c1;\n  font-weight: 650;");
    expect(styles).toContain(
      ':root[data-theme="light"] .appearance-settings-pane > fieldset > legend { color: #344b43; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .appearance-settings-pane .segmented-control button { color: #4a6058; }',
    );
    expect(styles).toContain(
      ".worker-role-editor > div button { min-height: 29px; padding: 0 10px; font-size: var(--worker-role-control-size); }",
    );
    expect(styles).not.toContain("font: 12px/1.45 var(--ui-font);");
    expect(styles).toContain(
      ".worker-role-editor.editing textarea { height: clamp(160px, 30dvh, 360px);",
    );
    expect(source).toContain('className="worker-role-action worker-role-edit"');
    expect(source).not.toContain("deleteConfirmId");
    expect(source).toContain(
      "const [roleDeleteMode, setRoleDeleteMode] = useState(false);",
    );
    expect(source).toContain(
      'const [selectedDeleteRoleId, setSelectedDeleteRoleId] = useState("");',
    );
    expect(source).toContain(
      'className={`worker-role-delete-manage ${roleDeleteMode ? "confirm" : ""}`}',
    );
    expect(source).toContain("勾选一个 Role，然后点击顶部对勾确认删除。");
    expect(source).toContain(
      "checked={roleDeleteMode ? selectedDeleteRoleId === role.id : selectedRoleIds.includes(role.id)}",
    );
    expect(source).toContain(
      'type: "worker_role_delete", session_id: session.session_id, role_id: selectedDeleteRoleId',
    );
    expect(source).toContain(
      'session && !roleDeleteMode && <form className="worker-role-group-editor"',
    );
    expect(source).toContain(
      "session && !roleDeleteMode && <form className={`worker-role-editor",
    );
    expect(styles).toContain(".worker-role-panel .worker-role-action {");
    expect(styles).toContain(
      ".worker-role-panel .worker-role-edit:hover:not(:disabled)",
    );
    expect(styles).toContain(
      ".worker-role-panel .worker-role-delete-manage.confirm",
    );
    expect(styles).toContain(
      ".worker-role-panel .worker-role-delete-manage.confirm { border-color: transparent; background: #a94b45; color: #fff;",
    );
    expect(styles).toContain(
      ".worker-role-panel .worker-role-delete-manage { border-color: transparent; background: transparent; color: #929da2; box-shadow: none; }",
    );
    expect(styles).toContain(
      ".worker-role-panel .worker-role-delete-cancel { border-color: transparent; background: #29312e; color: #aebbb6;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-panel .worker-role-delete-cancel { border-color: transparent; background: #e5ebe8; color: #596a64;',
    );
    expect(styles).not.toContain(
      "Roles delete controls keep their semantic surfaces after generic panel-button styling.",
    );
    expect(styles).not.toContain(
      ".worker-role-panel button.worker-role-delete-manage:not(.sidebar-resize-handle)",
    );
    expect(styles).toContain(
      ".worker-role-panel > header .worker-role-close { margin-left: 5px; }",
    );
    expect(source).toContain("<X size={14} strokeWidth={3}/>");
    expect(source).toContain("<Check size={14} strokeWidth={3}/>");
    expect(styles).toContain(
      ".worker-role-panel .worker-role-delete-manage { border-color: transparent; background: transparent; color: #929da2; box-shadow: none; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-panel .worker-role-delete-manage:hover:not(:disabled)',
    );
    expect(styles).toContain(".worker-role-item.delete-selected");
    expect(styles).toContain(
      ':root[data-theme="light"] .worker-role-item.delete-selected',
    );
  });

  it("updates only the core topic target session for high-frequency events", () => {
    expect(source).toContain(
      "const sessionIndex = current.findIndex((session) => session.session_id === topic.session_id);",
    );
    expect(source).toContain("if (sessionIndex < 0) return current;");
    expect(source).toContain("next[sessionIndex] = nextSession;");
    expect(source).not.toContain(
      "current.map((session) => applyCoreTopicToSession(",
    );
  });

  it("shows a live elapsed duration only while the turn is working", () => {
    expect(source).toContain(
      "const WorkingElapsed = memo(function WorkingElapsed",
    );
    expect(source).toContain(
      "const [elapsedMs, setElapsedMs] = useState(() =>",
    );
    expect(source).toContain("Math.max(0, Date.now() - createdAtMs)");
    expect(source).toContain(
      "const timer = window.setInterval(updateElapsed, 1_000);",
    );
    expect(source).toContain("return () => window.clearInterval(timer);");
    expect(source).toContain(
      '{turn.state === "working" && <WorkingElapsed createdAtMs={turn.created_at_ms}/>}',
    );
    expect(source).not.toContain(
      "const [workingElapsedMs, setWorkingElapsedMs]",
    );
    expect(source).toContain('className="working-elapsed" aria-hidden="true"');
    expect(styles).toContain(".working-elapsed { min-width: 3.5ch;");
    expect(styles).toContain("font-variant-numeric: tabular-nums;");
  });

  it("renders Thought Action as an independent trigger attached to a softly tinted process panel", () => {
    expect(styles).toContain(
      ".turn-assistant-frame { position: relative; overflow: visible; padding-left: 0; border: 0; border-radius: 0; background: transparent;",
    );
    expect(styles).toContain(
      ".turn-user-frame { width: fit-content; max-width: min(86%, 680px); margin: 0 0 11px auto; }",
    );
    expect(styles).not.toContain(".turn-user-content::after");
    expect(styles).toContain(
      ".turn-work-panel { position: relative; z-index: 1; margin-top: -4px; overflow: hidden; border-radius: 11px; background: #353535; }",
    );
    expect(styles).not.toContain(".turn-work-panel::before");
    expect(styles).not.toContain(".turn-assistant-heading::after");
    expect(styles).toContain(".turn-work-scroll { padding: 14px 10px 7px; }");
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-work-panel { background: #fafbfb; }',
    );
    expect(styles).not.toContain(
      ':root[data-theme="light"]\n:root[data-theme="light"] .turn-work-panel',
    );
    expect(styles).not.toContain(
      ':root[data-theme="light"] :root[data-theme="light"] .turn-work-panel',
    );
    expect(styles).not.toContain(
      ':root[data-theme="light"] .turn-user-content::after',
    );
    expect(source).toContain(
      '{workStreamVisible && <div className="turn-work-panel">',
    );
  });

  it("keeps ToolGen retrospective attached to its final delivery", () => {
    expect(source).toContain("function ToolGenNotice");
    expect(source).toContain("<details className={`toolgen-notice");
    expect(source).toContain("const [open, setOpen] = useState(false);");
    expect(source).toContain("const collapse = () => setOpen(false);");
    expect(source).toContain(
      "onToggle={(event) => setOpen(event.currentTarget.open)}",
    );
    expect(source).toContain(
      'const summaryLabel = `${open ? "收起" : "展开"} ToolGen 详情${activity.title ? `：${activity.title}` : ""}`;',
    );
    expect(source).toContain("aria-label={summaryLabel}");
    expect(source).toContain('className="toolgen-collapse"');
    expect(source).toContain(
      'className="toolgen-collapse top" title="Collapse ToolGen details" aria-label="Collapse ToolGen details" onClick={collapse}>收起详情</button>',
    );
    expect(source).toContain(
      'className="toolgen-collapse" title="Collapse ToolGen details" aria-label="Collapse ToolGen details" onClick={collapse}>收起详情</button>',
    );
    expect(styles).toContain(".toolgen-notice[open] summary svg");
    expect(styles).toContain('content: "收起"');
    expect(styles).toContain(".toolgen-collapse");
    expect(styles).toContain(".toolgen-collapse.top");
    expect(styles).toContain(':root[data-theme="light"] .toolgen-notice');
    expect(styles).toContain(".toolgen-notice.published");
    expect(styles).toContain(".toolgen-notice.published summary::before");
    expect(styles).toContain(
      ':root[data-theme="light"] .toolgen-notice.published',
    );
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
    expect(styles).toContain(
      ".turn-assistant-heading:has(.model-retry-status[open]) { z-index: 4; }",
    );
    expect(styles).toContain("font-size: 10px");
    expect(styles).toContain("font-size: 9px");
    expect(styles).toContain("font-size: 11px");
    expect(styles).toContain("min-height: 20px");
  });

  it("flushes live Thought/Action progress without waiting for the next animation frame", () => {
    expect(source).toContain(
      "function isLiveTurnProgressEvent(event: WireEvent): boolean",
    );
    expect(source).toContain('topicName === "core.model.response"');
    expect(source).toContain(
      'topicName === "core.action" && event.event.payload?.event === "start"',
    );
    expect(source).toContain(
      "inboundEvents.enqueue(event, isLiveTurnProgressEvent(event));",
    );
  });

  it("renders polling Bash with a clock, Poll label, live clock timer, and second-line command", () => {
    expect(source).toContain(
      'const pollingActivity = bashActivity && activity.tool_mode === "poll";',
    );
    expect(source).toContain('pollingActivity ? <Clock3 size={13}/> : ">_"');
    expect(source).toContain(
      "toolActivityDisplayName(activity.tool_name || activity.title, activity.tool_mode)",
    );
    expect(source).toContain(
      "pollingActivity ? formatClockDuration(displayedElapsedMs) : formatDuration(displayedElapsedMs)",
    );
    expect(source).toContain("window.setInterval(updateElapsed, 1_000)");
    expect(styles).toContain(
      ".tool-activity.poll-activity .tool-activity-command { grid-column: 2 / -1; grid-row: 2;",
    );
    expect(viewModelSource).toContain(
      'if (name === "run_bash" && mode === "poll") return "Poll";',
    );
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
    expect(source).toContain(
      'if (kind === "model_request" || kind === "model_response" || kind === "model_retry") return null;',
    );
    expect(source).not.toContain("setActivities((current) => [activity");
    expect(source).not.toContain("Model completed a response");
    expect(source).not.toContain("LIVE ACTIVITY");
    expect(source).not.toContain("Working view");
    expect(source).not.toContain("renderToolInvocation");
    expect(viewModelSource).not.toContain('title: "Work instructions"');
    expect(source).toContain('activity.tone === "warning" ? "⚠️"');
    expect(source).not.toContain('activity.tone === "warning" ? "!"');
  });

  it("uses the Markdown highlighter for final answers and Bash activity commands", () => {
    expect(markdownSource).toContain(
      'import rehypeHighlight from "rehype-highlight";',
    );
    expect(markdownSource).toContain(
      "rehypePlugins={[rehypeHighlight, rehypeKatex]}",
    );
    expect(source).toContain(
      'fencedCode(activity.code_language ?? "text", activity.code)',
    );
    expect(viteConfig).toContain(
      'highlighting: ["highlight.js", "rehype-highlight"]',
    );
  });

  it("renders Bash activity commands with the interface font at normal weight", () => {
    expect(source).toContain(
      'const bashActivity = activity.tool_name === "run_bash";',
    );
    expect(source).toContain('bashActivity ? "bash-activity" : ""');
    expect(styles).toContain(
      ".tool-activity.bash-activity .tool-activity-command",
    );
    expect(styles).toContain(
      ".tool-activity.bash-activity .tool-activity-body .code-block code *",
    );
    expect(styles).toContain(
      ".tool-activity-command { min-width: 0; grid-column: 4; justify-self: start; overflow: hidden; color: #737373;",
    );
    expect(styles).toContain("text-overflow: ellipsis; white-space: nowrap;");
    expect(styles).toContain("font-family: var(--ui-font);");
    expect(styles).toContain("font-weight: 400;");
  });

  it("renders completion telemetry below final answers", () => {
    expect(source).toContain(
      "attachTurnCompletion(session, event.outcome.message_id",
    );
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain("<TurnAnswerDelivery turn={turn}");
    expect(source).toContain(
      'if (availableKeys.length === 1 && selected === "final" && turn.final_answer)',
    );
    expect(source).toContain("<FinalAnswerDelivery text={turn.final_answer}");
    expect(source).toContain("<FinalAnswerContent text={text}/>");
    expect(source).toContain("<FinalAnswerContent text={turn.final_answer}/>");

    expect(source).toContain(
      'const chatShell = viewport?.closest<HTMLElement>(".chat-shell");',
    );
    expect(source).toContain(
      "const bodyInset = Math.max(0, contentRect.left - viewportRect.left);",
    );
    expect(source).toContain(
      "const nextTop = contentRect.top - viewportRect.top + viewport.scrollTop;",
    );
    expect(source).toContain(
      "setOutlineHost((current) => current === viewport ? current : viewport);",
    );
    expect(source).toContain(
      'getComputedStyle(root).getPropertyValue("--final-outline-width")',
    );
    expect(source).toContain(
      'setOutlinePlacement(bodyInset >= outlineWidth + FINAL_ANSWER_OUTLINE_EDGE_GUARD ? "docked" : "overlay")',
    );
    expect(source).toContain(
      "setShowOutline(finalAnswerNeedsOutline(content.offsetHeight, viewport.clientHeight, outline.length))",
    );
    expect(source).toContain("try { return extractMarkdownOutline(text); }");
    expect(source).toContain("catch { return []; }");
    expect(source).toContain(
      "headingIdPrefix={outline.length >= FINAL_ANSWER_OUTLINE_MIN_SECTIONS ? headingPrefix : undefined}",
    );
    expect(source).toContain('aria-label="Final answer table of contents"');
    expect(source).toContain(
      'const [outlinePlacement, setOutlinePlacement] = useState<"docked" | "overlay">("docked");',
    );
    expect(source).toContain(
      "const SessionTimelineActiveContext = createContext(false);",
    );
    expect(source).toContain(
      "<SessionTimelineActiveContext.Provider value={active}>",
    );
    expect(source).toContain(
      "const timelineActive = useContext(SessionTimelineActiveContext);",
    );
    expect(source).toContain(
      "if (!timelineActive || !root || !content || !viewport || !chatShell",
    );
    expect(source).toContain(
      "}, [outline, outlineCollapsed, text, timelineActive]);",
    );
    expect(source).toContain(
      "const outlineElement = timelineActive && showOutline && outlineHost ? createPortal(<aside",
    );
    expect(source).toContain("if (timelineActive) return;");
    expect(source).toContain("outlineHeadingOffsetsRef.current.clear();");
    expect(source).toContain("setShowOutline(false);");

    expect(source).not.toContain(
      "const [outlineBoundaryOffset, setOutlineBoundaryOffset] = useState(0);",
    );
    expect(source).toContain(
      "const [outlineGeometry, setOutlineGeometry] = useState({ top: 0, height: 0, stickyTop: 0 });",
    );
    expect(source).toContain(
      "const [outlineCollapsed, setOutlineCollapsed] = useState(false);",
    );
    expect(source).toContain(
      'setOutlineCollapsed(outlinePlacement === "overlay")',
    );
    expect(source).toContain('className="final-answer-outline-anchor"');
    expect(source).not.toContain("markdownOutlineAnchorOffset(");
    expect(source).not.toContain("outlinePositionTaskRef");
    expect(source).toContain(
      'className="final-answer-outline-toggle" aria-expanded={false}',
    );
    expect(source).toContain('aria-label="Show table of contents"');
    expect(source).toContain("onClick={() => setOutlineCollapsed(false)}");
    expect(source).toContain(
      'className="final-answer-outline-close" aria-label="Hide table of contents"',
    );
    expect(source).toContain("onClick={() => setOutlineCollapsed(true)}");
    expect(source).toContain(
      "const FINAL_ANSWER_OUTLINE_SCROLL_DURATION_MS = 180;",
    );
    expect(source).toContain(
      "markdownOutlineAnimationPosition(startTop, targetTop, elapsedMs, FINAL_ANSWER_OUTLINE_SCROLL_DURATION_MS)",
    );
    expect(source).toContain(
      "outlineNavigationAnimationRef.current = requestAnimationFrame(animate);",
    );
    expect(source).toContain(
      "if (elapsedMs < FINAL_ANSWER_OUTLINE_SCROLL_DURATION_MS)",
    );
    expect(source).toContain("MARKDOWN_OUTLINE_START_ID");
    expect(source).toContain(
      "markdownOutlineActiveId(outline, outlineHeadingOffsetsRef.current, threshold)",
    );
    expect(source).toContain(
      "const outlineNavRef = useRef<HTMLElement | null>(null);",
    );
    expect(source).toContain("<nav ref={outlineNavRef}><button");
    expect(source).toContain("markdownOutlineRailScrollTop(");
    expect(source).toContain(
      "nav?.querySelector<HTMLElement>('[aria-current=\"location\"]')",
    );
    expect(source).toContain(
      "if (Math.abs(nextScrollTop - nav.scrollTop) > .5) nav.scrollTop = nextScrollTop;",
    );
    expect(source).toContain("const navigateToStart = () => {");
    expect(source).toContain(
      "markdownOutlineTargetScrollTop(viewport.scrollTop, root.getBoundingClientRect().top, viewportTop, FINAL_ANSWER_OUTLINE_SCROLL_OFFSET)",
    );
    expect(source).toContain(
      'className={`final-answer-outline-start${activeId === MARKDOWN_OUTLINE_START_ID ? " active" : ""}`}',
    );
    expect(source).toContain('title="Go to the start of this answer"');
    expect(source).toContain("onClick={navigateToStart}");
    expect(styles).toContain(
      ".final-answer-outline-card nav > button.final-answer-outline-start { display: flex; align-items: center;",
    );
    expect(styles).toContain(
      ".final-answer-outline-card nav > button.final-answer-outline-start::before { display: none; }",
    );
    expect(styles).toContain(
      ".final-answer-outline-card nav { flex: 1 1 auto; min-height: 0; display: flex; flex-direction: column; overflow-x: hidden; overflow-y: auto;",
    );
    expect(styles).toContain(
      ".final-answer-outline-card { position: relative; width: 172px; min-height: 0; max-height: 65vh;",
    );
    expect(styles).toContain(
      ".final-answer-outline-anchor { position: sticky; top: var(--final-outline-sticky-top, 15vh);",
    );
    expect(styles).toContain("scrollbar-gutter: stable;");
    expect(styles).toContain(
      ".final-answer-outline-card nav > button { position: relative; flex: 0 0 auto;",
    );
    expect(styles).not.toContain("max-height: min(56vh, 472px)");
    expect(styles).not.toContain("height: min(calc(64vh - 42px), 498px)");
    expect(styles).toContain(".final-answer-outline { position: absolute;");
    expect(styles).toContain(
      ".final-answer-outline-anchor { position: sticky; top: var(--final-outline-sticky-top, 15vh);",
    );
    expect(styles).toContain(
      ".final-answer-outline-toggle { position: relative; display: inline-flex; flex: none; width: 34px; height: 52px; align-items: center; justify-content: center;",
    );
    expect(styles).toContain(
      ".final-answer-outline-card { position: relative; width: 172px; min-height: 0; max-height: 65vh; display: flex; flex-direction: column; overflow: hidden;",
    );
    expect(styles).not.toContain(
      ".final-answer-outline-card { position: relative; width: 172px; min-height: 0; max-height: 65vh; display: flex; flex-direction: column; overflow: hidden; border: 0; border-radius: 0 14px 14px 0; padding: 14px 9px 13px 11px; background: linear-gradient",
    );
    expect(styles).toContain("background: #222825f2;");
    expect(styles).toContain(
      "transform-origin: left center; animation: final-outline-open",
    );
    expect(styles).toContain(
      "@keyframes final-outline-open { from { opacity: 0; transform: translateX(-12px) scale(.985); } }",
    );

    expect(styles).toContain(
      ".final-answer-reading { --final-outline-width: 165px; position: relative; min-width: 0; }",
    );
    expect(styles).toContain(
      ".final-answer-outline { position: absolute; z-index: 5; left: 0; width: var(--final-outline-width);",
    );
    expect(styles).not.toContain(".final-answer-outline::before {");
    expect(styles).toContain(
      ".final-answer-outline-toggle { position: relative; display: inline-flex; flex: none; width: 34px; height: 52px;",
    );
    expect(styles).not.toContain(".final-answer-outline-toggle span {");
    expect(styles).toContain(
      ".final-answer-outline-anchor { position: sticky; top: var(--final-outline-sticky-top, 15vh); width: max-content; min-height: 86px; max-height: 65vh; display: flex; align-items: flex-start; pointer-events: auto; }",
    );
    expect(styles).toContain(
      ".final-answer-outline.collapsed { width: 36px; }",
    );
    expect(styles).toContain("border-radius: 0 10px 10px 0;");
    expect(styles).not.toContain("inset 2px 0 #7bb7a547");
    expect(styles).not.toContain("inset 1px 0 #ffffff0a");
    expect(styles).not.toContain(".final-answer-outline.expanded { width:");
    expect(styles).toContain(
      ".user-message-navigation { position: absolute; z-index: 6; top: calc(50% + 38px); left: max(10px, calc((100% - 876px) / 2));",
    );
    expect(styles).not.toContain(
      ".final-answer-outline-anchor { position: sticky; top: calc(50% + 38px);",
    );
    expect(source).toContain(
      '{outlineCollapsed && <button type="button" className="final-answer-outline-toggle"',
    );
    expect(source).toContain(
      '<header><button type="button" className="final-answer-outline-close"',
    );
    expect(source).toContain(
      '<ChevronLeft size={16} strokeWidth={2.4} aria-hidden="true"/></button><span><Bookmark size={12} aria-hidden="true"/>Contents</span></header>',
    );
    expect(styles).toContain(
      ".final-answer-outline-close { width: 28px; height: 28px;",
    );
    expect(styles).toContain(
      ".final-answer-outline-card header { flex: 0 0 auto; min-height: 22px; display: flex; align-items: center; justify-content: flex-start; gap: 3px; margin: 0 2px 9px 0; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .final-answer-outline-card { background: #f5f9f7f7;',
    );
    expect(styles).toContain(
      ".final-answer-outline.overlay.expanded .final-answer-outline-card { background: linear-gradient(90deg,",
    );
    expect(styles).toContain(
      "mask-image: linear-gradient(90deg, #000 0%, #000 70%, #000c 84%, transparent 100%);",
    );
    expect(styles).toContain(
      "font-size: 11px; font-weight: 530; line-height: 1.42;",
    );
    expect(styles).toContain(
      ".final-answer-outline-card nav > button.level-3 { padding-left: 25px; color: #747e79; font-size: 10px; }",
    );
    expect(styles).toContain(
      ".final-answer-outline-card nav > button { position: relative; flex: 0 0 auto; min-width: 0; overflow: hidden; border: 0; border-radius: 7px; padding: 6px 7px 6px 11px;",
    );
    expect(styles).not.toContain(".final-answer-outline-card::after {");
    expect(source).not.toContain("sidebar-layout-change");
    expect(source).toContain("observer?.observe(chatShell);");
    expect(source).toContain("data-final-answer-reading-id={readingId}");
    expect(source).toContain(
      "const FINAL_ANSWER_OUTLINE_VIEWPORT_RATIO = 0.15;",
    );
    expect(source).toContain("const FINAL_ANSWER_OUTLINE_TOGGLE_HEIGHT = 52;");
    expect(source).toContain(
      "const targetWindowTop = window.innerHeight * FINAL_ANSWER_OUTLINE_VIEWPORT_RATIO;",
    );
    expect(source).toContain(
      "const targetViewportTop = targetWindowTop - viewportRect.top;",
    );
    expect(source).toContain(
      "const nextStickyTop = Math.max(0, targetViewportTop - (outlineCollapsed ? FINAL_ANSWER_OUTLINE_TOGGLE_HEIGHT / 2 : 0));",
    );
    expect(source).toContain(
      "}, [outline, outlineCollapsed, text, timelineActive]);",
    );
    expect(source).toContain("current.stickyTop === nextStickyTop");
    expect(source).toContain(
      '"--final-outline-sticky-top": `${outlineGeometry.stickyTop}px`',
    );
    expect(source).toContain("</aside>, outlineHost) : null;");
    expect(styles).toContain(
      ".chat-scroll { position: relative; overflow-x: hidden; }",
    );
    expect(source).not.toContain("--final-outline-boundary-offset");
    expect(styles).not.toContain(
      ".final-answer-outline-anchor { position: sticky; top: 33.333%;",
    );
    expect(styles).not.toContain(".final-answer-outline { display: none; }");
    expect(styles).toContain(
      "@media (max-width: 720px) { .final-answer-outline-card",
    );
    expect(source).toContain(
      '{hasInterim && <div className="turn-answer-tabs" role="tablist" aria-label="Turn answers">',
    );
    expect(source).toContain("const showFinalTab = hasFinal || hasInterim;");
    expect(source).toContain(
      '{showFinalTab && <button type="button" role="tab"',
    );
    expect(source).toContain(">Final Answer</button>");
    expect(source).toContain(">Interim</button>");
    expect(source).toContain(
      'className="turn-final-placeholder" role="status" aria-live="polite"',
    );
    expect(source).toContain(
      'turn.state === "interrupted" ? "Interrupted by runtime restart."',
    );
    expect(styles).toContain(
      ".turn-final-placeholder { padding: 3px 0 15px; color: #7d8b93; font-size: 13px; font-style: italic; }",
    );
    expect(source).toContain('className="turn-interim-list"');
    expect(source).toContain('className="turn-interim-item"');
    expect(source).toContain(
      'className={`turn-answer-view ${selected === "final" ? "selected" : "inactive"}`}',
    );
    expect(source).toContain(
      'className={`turn-answer-view ${selected === "interim" ? "selected" : "inactive"}`}',
    );
    expect(source).toContain('aria-hidden={selected !== "final"}');
    expect(styles).toContain(".turn-answer-panel { display: grid;");
    expect(styles).toContain(
      ".turn-answer-view { min-width: 0; grid-area: 1 / 1; }",
    );
    expect(styles).toContain(
      ".turn-answer-view.inactive { visibility: hidden; pointer-events: none; user-select: none; }",
    );
    expect(source).toContain("<h3><span>{index + 1}.</span> {item.task}</h3>");
    expect(styles).toContain(".turn-interim-item + .turn-interim-item");
    expect(source).not.toContain("label: `Answer ${item.ordinal}`");
    expect(source).not.toContain('className="turn-final-toolbar"');
    expect(source).toContain('className="final-answer-actions"');
    expect(source).toContain(
      "const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(text, {",
    );
    expect(source).toContain('copied: "Answer copied"');
    expect(source).toContain('failed: "Copy answer failed"');
    expect(clipboardSource).toContain(
      'const copyClass = copyState === "copied" ? "copy-success" : copyState === "failed" ? "copy-failed" : "";',
    );
    expect(source).toContain("className={`final-copy ${copyClass}`}");
    expect(markdownSource).toContain("aria-label={copyLabel}");
    expect(source).toContain("title={copyLabel}");
    expect(source).toContain(
      'aria-label={copyLabel} onClick={() => void copy()}>{copyState === "copied"',
    );
    expect(source).not.toContain('<span aria-live="polite">{copyLabel}</span>');
    expect(source).toContain("answerActions={answerActions}");
    expect(source).toContain("{answerActions}");
    expect(source).toContain(
      'className="chat-message-delete assistant-message-delete"',
    );
    expect(markdownSource).toContain(
      "<figcaption><span title={language}>{language}</span>",
    );
    expect(clipboardSource).toContain("navigator.clipboard.writeText(text)");
    expect(clipboardSource).toContain(
      "async function copyTextToClipboard(text: string)",
    );
    expect(clipboardSource).toContain('document.createElement("textarea")');
    expect(clipboardSource).toContain(
      'textarea.setAttribute("readonly", "true")',
    );
    expect(clipboardSource).toContain('document.execCommand("copy")');
    expect(clipboardSource).toContain("document.body.removeChild(textarea)");
    expect(clipboardSource).toContain(
      "window.getSelection()?.removeAllRanges()",
    );
    expect(clipboardSource).toContain(
      "window.clearTimeout(resetTimerRef.current)",
    );
    expect(clipboardSource).toContain('setCopyState("idle");\n  }, [text]);');
    expect(source).toContain("<CompletionCard completion={completion}");
    expect(styles).toContain(".completion-card");
    expect(styles).toContain(".final-answer-actions");
    expect(styles).not.toContain(".turn-final-toolbar");
    expect(styles).toContain(".final-copy");
    expect(styles).toContain(
      ".final-copy.copy-success, .code-block figcaption button.copy-success",
    );
    expect(styles).toContain(
      ".final-copy.copy-failed, .code-block figcaption button.copy-failed",
    );
    expect(styles).toContain(':root[data-theme="light"] .final-copy');
    expect(styles).toContain(
      ':root[data-theme="light"] .final-copy.copy-success',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .final-copy.copy-failed',
    );
    expect(styles).not.toContain("::root");
    expect(styles).toContain(".completion-card { gap: 0 7px;");
    expect(styles).toContain("font-size: 10px; overflow-wrap: anywhere;");
    expect(styles).toContain(
      ".completion-card span { min-width: 0; padding: 0; border: 0; white-space: normal; }",
    );
    expect(styles).toContain(
      ".completion-card .completion-status { white-space: normal; overflow-wrap: anywhere; }",
    );
    expect(styles).toContain(".turn-final-delivery");
    expect(source).toContain("function completionFactTitle");
    expect(source).toContain(
      "title={completionFactTitle(label, completion, stats) ?? `${label}: ${value}`}",
    );
    expect(source).toContain("`${stats.prompt_tokens} input tokens`");
    expect(source).toContain("`${stats.completion_tokens} output tokens`");
    expect(source).toContain("`${stats.cached_tokens} cached input tokens`");
    expect(source).toContain(
      '["Compact", formatOptionalTokens(stats.shrunk_tokens)]',
    );
    expect(source).not.toContain(
      '["Shrunk", formatTokens(stats.shrunk_tokens)]',
    );
  });

  it("binds assistant-ui running state to the authoritative session lifecycle", () => {
    expect(source).toContain('isRunning: activeSession?.state === "working"');
    expect(source).toContain('cancelled ? "Cancelled" : "Completed"');
    expect(viewModelSource).toContain('worker.state === "working"');
  });

  it("deduplicates rapid cancel clicks and clears the guard when a turn finishes", () => {
    expect(source).toContain(
      "const cancellingSessionIds = useRef<Set<string>>(new Set());",
    );
    expect(source).toContain("const [cancellingSessionIdSet");
    expect(source).toContain(
      "if (cancellingSessionIds.current.has(activeSession.session_id)) return;",
    );
    expect(source).toContain("cancellingSessionIds.current.add(sessionId);");
    expect(source).toContain(
      "cancellingSessionIds.current.delete(event.session_id);",
    );
    expect(source).toContain(
      "const cancellingSessionCommandIds = useRef<Map<string, string>>(new Map());",
    );
    expect(source).toContain(
      "const cancellingSessionTimeouts = useRef<Map<string, number>>(new Map());",
    );
    expect(source).toContain("TURN_CANCEL_UI_TIMEOUT_MS = 15_000");
    expect(source).toContain('const commandId = clientId("turn-cancel");');
    expect(source).toContain(
      "cancellingSessionCommandIds.current.get(sessionId) === event.command_id",
    );
    expect(source).toContain(
      'sendCommand({ type: "turn_cancel", session_id: sessionId }, commandId)',
    );
    expect(source).toContain(
      '"Background cleanup is taking longer than expected"',
    );
    expect(source).toContain("This timer is diagnostic only");
    expect(source).not.toContain('"Stopping…"');
    expect(source).toContain("const cancelActiveSessionTurn = async () =>");
    expect(source).toContain(
      "queuedAutoContinueSessionIdsRef.current.delete(activeSessionId)",
    );
    expect(source).not.toContain(
      'pauseQueuedMessages(activeSessionId, "用户停止了当前任务", "user")',
    );
    expect(source).not.toContain(
      "clearSessionQueuedMessages(previous, activeSessionId)",
    );
    expect(source).toContain(
      "releaseSessionQueuedMessageClaims(queuedMessageClaimsRef.current, sessionId)",
    );
    expect(source).toContain("onClick={() => void cancelActiveSessionTurn()}");
  });

  it("removes the transient working presentation immediately after Stop", () => {
    expect(source).toContain(
      "isCancelling={session.session_id === activeSessionId && isCancelling}",
    );
    expect(source).toContain(
      'isCancelling={isCancelling && turn.state === "working"}',
    );
    expect(source).toContain(
      'const isWorking = turn.state === "working" && !isCancelling;',
    );
    expect(source).toContain(
      '{isWorking && !hasVisibleProcess && <div className="turn-starting-status"',
    );
    expect(source).toContain(
      "{isWorking && <WorkingElapsed createdAtMs={turn.created_at_ms}/>}",
    );
    expect(source).toContain("{isWorking && <LiveTurnUsage turn={turn}/>}");
    expect(source).toContain("previous.isCancelling !== next.isCancelling");
  });

  it("queues send invisibly while cancellation finishes in the background", () => {
    const start = source.indexOf("const sendTextForSession = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(sendText).toContain(
      "cancellingSessionIds.current.has(targetSession.session_id)",
    );
    expect(sendText).toContain("sendCommand(command, commandId)");
    expect(source).not.toContain("postCancelCommands");
  });

  it("keeps sending enabled during a working turn by bypassing assistant-ui Send", () => {
    const start = source.indexOf("const sendTextForSession = useCallback");
    const end = source.indexOf("const uploadFile = useCallback", start);
    const sendText = source.slice(start, end);
    expect(source).toContain("composerSendDecision");
    expect(viewModelSource).toContain(
      "command: forceSupplement && !forceNewTurn",
    );
    expect(sendText).toContain("composerSendDecision(");
    expect(source).toContain("value={draft}");
    expect(source).toContain(
      "onSubmit={(event) => { event.preventDefault(); submitDraft(); }}",
    );
    expect(source).toContain('type="submit" title={effectiveSendLabel}');
    expect(source).not.toContain("ComposerPrimitive.Send");
  });

  it("uses synchronous pending guards for rapid repeated browser clicks", () => {
    expect(source).toContain("creatingSessionRef.current");
    expect(source).toContain("const [draftsBySession, setDraftsBySession]");
    expect(source).toContain(
      "const submittingDraftSessionIdsRef = useRef<Set<string>>(new Set());",
    );
    expect(source).toContain(
      "const directSubmissionsRef = useRef<Map<string, {",
    );
    expect(source).toContain(
      "reserveSessionDraftSubmission(submittingDraftSessionIdsRef, activeSessionId, draftsBySession)",
    );
    expect(source).toContain(
      "finishSessionDraftSubmission(submittingDraftSessionIdsRef, draftsBySession, reserved.sessionId, reserved.text, sent)",
    );
    expect(source).toContain(
      "directSubmissionsRef.current.set(reserved.sessionId, {",
    );
    expect(source).toContain("const pendingDirectSubmission =");
    expect(source).toContain(
      "directSubmissionsRef.current.has(activeSessionId)",
    );
    expect(source).toContain("await onCancel(pendingDirectSubmission)");
    expect(source).toContain(
      'activeSession.state !== "working" && !allowPendingDirectSubmission',
    );
    expect(source).toContain(
      'event.command_id.startsWith("submit-") && event.status === "rejected"',
    );
    expect(source).toContain(
      "rejectedDirectDrafts.set(sessionId, submission.text)",
    );
    expect(source).toContain(
      "onRolesConsumed(session.session_id, submission.roleIds)",
    );
    expect(source).toContain(
      "sent = !!reliableStorageScope\n        && saveQueuedMessages(window.localStorage, reliableStorageScope, nextQueues, queuedMessagesBySessionRef.current);",
    );
    expect(source).toMatch(
      /if \(sent\) \{[\s\S]*?updateQueuedMessages\(\(\) => nextQueues\);[\s\S]*?\}/,
    );
    expect(source).toContain(
      "shouldDirectManualMessage(activeSession.state, existingQueue.length, !!queuedMessagesPause, isCancelling || !!activeSession.cancelling_turn_id)",
    );
    const submitDraftStart = source.indexOf("const submitDraft = () =>");
    const submitDraftEnd = source.indexOf(
      "const submitDraftAsSupplement = () =>",
      submitDraftStart,
    );
    const submitDraftSource = source.slice(submitDraftStart, submitDraftEnd);
    expect(submitDraftSource).not.toContain("resumeQueuedMessages()");
    expect(source).toContain(
      "sessionIds={sessions.map((session) => session.session_id)}",
    );
    expect(source).toContain("pruneSessionDrafts(current, sessionIds)");
    expect(source).toContain(
      "pruneSessionSubmissionLocks(submittingDraftSessionIdsRef, sessionIds)",
    );
    expect(source).toContain(
      "disabled={!activeSession || !hasDraftText || submittingDraft || uploadingAttachment || sessionInteractionLocked}",
    );
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
    expect(source).toContain(
      'className={`load-history ${loadingHistory ? "loading" : ""}`}',
    );
    expect(source).toContain(
      'aria-label={historyButtonLabel} aria-live="polite" aria-busy={loadingHistory || undefined}',
    );
    expect(source).toContain(
      "disabled={loadingHistory || sessionInteractionLocked}",
    );
    expect(source).toContain(
      'loadingHistory && <LoaderCircle size={13} aria-hidden="true"/>',
    );
  });

  it("locks old-session interactions while a mem switch snapshot is pending", () => {
    expect(source).toContain("sessionInteractionLocked={runtimeLocked}");
    expect(source).toContain("disabled={runtimeLocked}");
    expect(source).toContain("if (pendingMemSwitch) return;");
    expect(source).toContain('reason === "mem_switching"');
    expect(source).toContain(
      "disabled={!activeSession || sessionInteractionLocked}",
    );
    expect(source).toContain(
      "disabled={!activeSession || !hasDraftText || submittingDraft || uploadingAttachment || sessionInteractionLocked}",
    );
    expect(source).toContain(
      "disabled={loadingHistory || sessionInteractionLocked}",
    );
    expect(source).toContain("disabled={removing || sessionInteractionLocked}");
    expect(source).toContain("const disabled = pending || locked;");
    expect(source).toContain("disabled={disabled}");
    expect(source).toContain(
      "const runtimeReady = connected && snapshotReady;",
    );
    expect(source).toContain(
      "const runtimeLocked = pendingMemSwitch || !runtimeReady;",
    );
    expect(source).toContain(
      'const newSessionLabel = runtimeLocked ? "Session controls are temporarily locked" : "New session";',
    );
    expect(source).toContain(
      'ref={newSessionButtonRef} className="new-session" title={newSessionLabel} aria-label={newSessionLabel} disabled={runtimeLocked || sessionDeleteMode}',
    );
    expect(source.indexOf('className="new-session-group"')).toBeLessThan(
      source.indexOf('ref={newSessionButtonRef} className="new-session"'),
    );
    expect(source).toContain(
      'title={runtimeLocked ? "Session controls are temporarily locked" : session.workers.length === 0 ? "No workers in this session" : `${expandedSessionIds.has(session.session_id) ? "Hide" : "Show"} workers`}',
    );
    expect(source).toContain(
      "aria-label={runtimeLocked ? `Workers locked while the runtime synchronizes for ${session.display_name}`",
    );
    expect(source).toContain(
      "disabled={runtimeLocked || sessionDeleteMode || renamingSessionId === session.session_id || session.workers.length === 0}",
    );
    expect(source).toContain(
      "aria-label={runtimeLocked ? `${session.display_name} locked while the runtime synchronizes` : renamingSession ? `${session.display_name} rename is being saved` : undefined}",
    );
    expect(source).toContain(
      'disabled={runtimeLocked} onClick={() => { if (sessionDeleteMode) { setSelectedDeleteSessionId((current) => current === session.session_id ? "" : session.session_id); return; } performanceTraceRef.current.beginSessionSelection(session.session_id);',
    );
    expect(source).toContain(
      "setActiveSessionId(session.session_id); closeMobileSidebar();",
    );
    expect(source).toContain(
      "onDoubleClick={() => { if (!runtimeLocked && !sessionDeleteMode && renamingSessionId !== session.session_id) beginRename(session); }}",
    );
    expect(source).toContain("sessionRenameDecision(");
    expect(styles).toContain(".session:disabled, .session-expand:disabled");
    expect(styles).toContain(
      ".session:disabled:hover, .session-expand:disabled:hover",
    );
    expect(viewModelSource).toContain('"mem_switching"');
    expect(viewModelSource).toContain('"already_pending"');
  });

  it("clears stale pending browser guards when a reconnect snapshot arrives", () => {
    const helloStart = source.indexOf('if (event.type === "hello")');
    const helloEnd = source.indexOf(
      'if (event.type === "session_created")',
      helloStart,
    );
    const helloBranch = source.slice(helloStart, helloEnd);
    expect(helloBranch).toContain("clearAllPendingCommands();");
    expect(helloBranch).toContain(
      "setDecisions(decisionsFromSessions(event.snapshot.sessions));",
    );
    expect(helloBranch).toContain("applySnapshot(event.snapshot);");
    expect(helloBranch).toContain("setSnapshotReady(true);");
    expect(source).toContain(
      "if (socket.current?.readyState !== WebSocket.OPEN || !snapshotReady) return reliable;",
    );
    expect(source).toContain("hasConnectedOnce = true;");
    expect(source).toContain("disconnectNoticeShown = false;");
    expect(source).toContain("retryAttempt = 0;");
    expect(source).toContain("setConnected(true);");
    expect(source).toContain("setRuntimeEverConnected(true);");
    expect(source).toContain("setSnapshotReady(false);");
    expect(source).toContain(
      "setConnected(false);\n        setSnapshotReady(false);",
    );
  });

  it("moves active selection to a live session when a reconnect or mem snapshot swaps sessions", () => {
    expect(viewModelSource).toContain("resolveActiveSessionId");
    expect(source).toContain(
      "resolveActiveSessionId(current, snapshot.sessions)",
    );
    expect(source).not.toContain("current || snapshot.sessions[0]?.session_id");
  });

  it("switches assistant-ui sessions without remounting the heavy thread tree", () => {
    expect(source).toContain('<ThreadPrimitive.Root className="aui-thread">');
    expect(source).not.toContain(
      '<ThreadPrimitive.Root key={activeSessionId ?? "no-session"}',
    );
    expect(source).toContain(
      'const runtimeMessageSessionId = activeSession?.session_id ?? "";',
    );
    expect(source).toContain(
      "auiMessageState.sessionId === runtimeMessageSessionId",
    );
    expect(source).toContain(": runtimeMessages;");
    expect(source).toContain(
      "setAuiMessageState({ sessionId: runtimeMessageSessionId, messages });",
    );
    expect(source).toContain("const sessionDecisions = useMemo(");
    expect(source).toContain("const activePendingToolGenTurnIds = useMemo(");
    expect(source).toContain("const activeToolGenBusy = useMemo(");
    expect(source).toContain("const replyToDecision = useCallback(");
    expect(source).toContain("const requestActiveToolGen = useCallback(");
    expect(source).toContain(
      "pendingToolGenTurnIds={activePendingToolGenTurnIds}",
    );
    expect(source).toContain("onDecisionReply={replyToDecision}");
    expect(source).toContain(
      'key={activeSession?.session_id ?? "no-session"}\n          panelRef={mcpPanelRef}',
    );
    expect(source).toContain(
      'key={activeSession?.session_id ?? "no-session"}\n        session={activeSession}',
    );
    expect(source).toContain(
      'key={activeSession?.session_id ?? "no-session"}\n        panelRef={toolRepoPanelRef}',
    );
    expect(source).toContain(
      "{deleteMessageCandidate && deleteMessageCandidate.sessionId === activeSessionKey && <ChatMessageDeleteDialog",
    );
    expect(source).toContain(
      "{toolGenEnabled && toolgenDialog && toolgenDialog.sessionId === activeSessionKey && <ToolGenDialog",
    );
    expect(source).toContain(
      "if (userMessageNavigationAnimationRef.current !== null) {",
    );
    expect(source).toContain(
      "userMessageNavigationHoverLockedRef.current = false;",
    );
    expect(source).toContain("setComposerExpanded(false);");
    expect(source).toContain("setDraggedQueueMessageId(undefined);");
    expect(source).toContain("setQueuedMessageOverId(undefined);");
    expect(source).toContain("setEditingQueuedMessage(undefined);");
    expect(source).toContain("}, [activeSessionId]);");
  });

  it("renders live task usage and session context without replacing final telemetry", () => {
    expect(source).toContain("<HeaderContextUsage session={activeSession}");
    expect(source).toContain("<LiveTurnUsage turn={turn}");
    expect(source).toContain('aria-label="Current task token usage"');
    expect(styles).not.toContain("animation: live-turn-usage-breathe");
    expect(styles).not.toContain("@keyframes live-turn-usage-breathe");
    expect(styles).toContain(
      ".live-turn-usage, .pulse, .turn-starting-mark::after, .connection.offline",
    );
    expect(source).toContain(
      'const level = ratio >= 90 ? "critical" : ratio >= 75 ? "warning" : "normal";',
    );
    expect(source).toContain("className={`header-context ${level}`}");
    expect(source).toContain(
      "const cacheHitPercent = session ? sessionCacheHitPercent(session) : undefined;",
    );
    expect(source).toContain(
      'const cacheLabel = cacheHitPercent === undefined ? "cache: —" : `cache: ${cacheHitPercent.toFixed(1)}%`;',
    );
    expect(source).toContain('className="header-context-main"');
    expect(source).toContain(
      'className="header-cache-rate"><span aria-hidden="true">· </span>{cacheLabel}',
    );
    expect(source).toContain(
      "const ratio = limit ? Math.min(100, Math.ceil((usage?.prompt_tokens ?? 0) * 100 / limit)) : 0;",
    );
    expect(source).toContain("const contextUsageLabel = limit");
    expect(source).toContain(
      "`Context usage ${ratio}% · ${formatTokens(usage?.prompt_tokens ?? 0)} / ${formatTokens(limit)} input tokens · ${cacheLabel}`",
    );
    expect(source).toContain(
      "title={contextUsageLabel} aria-label={contextUsageLabel}",
    );
    expect(source).toContain('className="header-context-meter"');
    expect(source).toContain("style={{ width: `${ratio}%` }}");
    expect(source).toContain("`${ratio}%/${formatTokens(limit)}`");
    expect(source).toContain(
      '{limit ? `${ratio}%/${formatTokens(limit)}` : "—"}',
    );
    expect(source).toContain('role="status" aria-live="polite"');
    expect(source).toContain(
      'className={`turn-work-scroll has-content${pendingUpdates > 0 ? " has-pending-updates" : ""}`} role="region" aria-label={isToolGenTurn ? "ToolGen work stream" : "Task work stream"}',
    );
    expect(source).toContain(
      "const persistentToolGenItems = useMemo(() => visibleItems.filter",
    );
    expect(source).toContain('activity.toolgen_phase === "published"');
    expect(source).toContain(
      "const scrollItems = useMemo(() => visibleItems.filter",
    );
    expect(source).toContain(
      'className="turn-persistent-toolgen" aria-label="ToolGen result"',
    );
    expect(source).toContain("scrollItems.map((item, index)");
    expect(source).toContain("const visibleItems = timelineItems;");
    expect(source).not.toContain("MAX_RENDERED_TURN_EVENTS");
    expect(source).not.toContain(
      "earlier work updates are retained by the host but not rendered",
    );
    expect(styles).not.toContain(".turn-events-omitted");
    expect(styles).toContain(".turn-persistent-toolgen");
    expect(source).toContain('title="Scroll to latest work update"');
    expect(source).toContain(
      'aria-label={`${pendingUpdates} new work update${pendingUpdates === 1 ? "" : "s"}; scroll to latest`}',
    );
    expect(source).toContain(
      'scroll.scrollTo({ top: scroll.scrollHeight, behavior: prefersReducedMotion() ? "auto" : "smooth" });',
    );
    expect(source).toContain("function prefersReducedMotion()");
    expect(source).toContain(
      'window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false',
    );
    expect(source).toContain('<ArrowDown size={13} aria-hidden="true"/>');
    expect(styles).toContain(".turn-new-updates:focus-visible");
    expect(source).toContain(
      "!turn.final_answer && turn.sub_answers.length === 0 && turn.completion",
    );
    expect(viewModelSource).toContain("turnLiveUsage");
    expect(viewModelSource).toContain("sessionContextUsage");
    expect(viewModelSource).toContain("sessionCacheHitPercent");
    expect(viewModelSource).toContain("sessionRuntimeUsage");
    expect(viewModelSource).toContain('message.kind === "runtime_restart"');
    expect(viewModelSource).toContain(
      "turnLiveUsageSince(turn, runtimeRestartAtMs)",
    );
    expect(styles).toContain(".header-context-meter");
    expect(styles).toContain(".header-context-main");
    expect(styles).toContain(".header-cache-rate");
    expect(styles).toContain(
      ".header-context.warning .header-context-meter > span",
    );
    expect(styles).toContain(
      ".header-context.critical .header-context-meter > span",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .header-context.warning .header-context-meter > span',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .header-context.critical .header-context-meter > span',
    );
    expect(styles).not.toContain(".context-usage-bar");
    expect(styles).toContain(".turn-work-scroll.has-pending-updates");
    expect(styles).toContain(".live-turn-usage");
  });

  it("supports agent rename and a distinct animated working state", () => {
    expect(viewModelSource).toContain('type: "session_rename"');
    expect(viewModelSource).toContain("sessionRenameDecision");
    expect(source).toContain('event.type === "session_renamed"');
    expect(source).toContain("Rename session failed");
    expect(source).toContain(
      "Reconnect to Timem Web before renaming this session.",
    );
    expect(source).toContain("session-working-icon");
    expect(source).toContain("const visuallyWorking = sessionVisuallyWorking(");
    expect(viewModelSource).toContain("!session.cancelling_turn_id");
    expect(viewModelSource).toContain(
      "!locallyCancellingSessionIds.has(session.session_id)",
    );
    expect(source).toContain('aria-label="Session working"');
    expect(source).toContain('aria-hidden="true"');
    expect(source).toContain(
      'className="sr-only">Session state: {session.state}</span>',
    );
    expect(source).not.toContain("Agent working");
    expect(source).toContain("session-rename-input");
    expect(source).toContain(
      'if (event.key === "Enter" && !event.nativeEvent.isComposing) { event.preventDefault(); finishRename(session.session_id); }',
    );
    expect(source).toContain(
      'if (event.key === "Escape") { event.preventDefault(); setRenamingSessionId(""); setRenameDraft(""); }',
    );
    expect(source).toContain(
      "const renamingSession = pendingRenameSessionIds.has(session.session_id);",
    );
    expect(source).toContain('renamingSession ? "renaming-session" : ""');
    expect(source).toContain(
      "aria-busy={renamingSession || deletingSession || undefined}",
    );
    expect(source).toContain("Saving name...");
    expect(source).toContain(
      "onDoubleClick={() => { if (!runtimeLocked && !sessionDeleteMode && renamingSessionId !== session.session_id) beginRename(session); }}",
    );
    expect(styles).toContain("@keyframes session-working-glow");
    expect(styles).toContain(".session-row.renaming-session");
    expect(styles).toContain(".session-pending");
    expect(styles).toContain(
      ':root[data-theme="light"] .session-row.renaming-session',
    );
    expect(styles).toContain(
      ".sr-only { position: absolute; width: 1px; height: 1px;",
    );
  });

  it("uses a toolbar selection mode before permanently deleting a session", () => {
    expect(protocolSource).toContain(
      '{ type: "session_delete"; session_id: string }',
    );
    expect(protocolSource).toContain(
      '{ type: "session_deleted"; session_id: string }',
    );
    expect(source).toContain("SessionDeleteDialog");
    expect(source).toContain(
      "const [sessionDeleteMode, setSessionDeleteMode] = useState(false);",
    );
    expect(source).toContain(
      'const [selectedDeleteSessionId, setSelectedDeleteSessionId] = useState("");',
    );
    expect(source).toContain(
      'className={`session-management-actions ${sessionDeleteMode ? "deleting" : ""}`}',
    );
    expect(source).toContain(
      'className={`session-delete-manage ${sessionDeleteMode ? "confirm" : ""}`}',
    );
    expect(source).toContain(
      'className={`session-delete-select ${selectedDeleteSessionId === session.session_id ? "selected" : ""}`}',
    );
    expect(source).not.toContain(
      'className={`session-delete ${deletingSession ? "deleting" : ""}`}',
    );
    expect(source).toContain(
      'setSessionGroupEditor(null); setRenamingSessionId(""); setRenameDraft(""); setSessionDeleteMode(true);',
    );
    expect(source).toContain(
      "disabled={runtimeLocked || sessions.length === 0 || (sessionDeleteMode && !selectedDeleteSessionId)}",
    );
    expect(source).toContain(
      'sendCommand({ type: "session_delete", session_id: sessionId })',
    );
    expect(source).toContain(
      "This permanently deletes the session, its stored task history, settings, and session tools.",
    );
    expect(source).toContain("This cannot be undone.");
    expect(source).toContain('event.type === "session_deleted"');
    expect(styles).toContain(".session-delete-dialog");
    expect(styles).toContain(".decision-actions .danger");
  });

  it("expands each session into its scoped worker status list", () => {
    expect(source).toContain("expandedSessionIds");
    expect(protocolSource).toContain("debug_mode: boolean;");
    expect(source).toContain(
      '{server?.debug_mode && <button type="button" className={`session-expand',
    );
    expect(source).toContain(
      "server?.debug_mode && session.workers.length > 0 && expandedSessionIds.has(session.session_id)",
    );
    expect(source).toContain(
      "if (!snapshot.server.debug_mode) setExpandedSessionIds(new Set());",
    );
    expect(source).toContain("session-expand");
    expect(source).toContain("worker-list");
    expect(source).toContain(
      'aria-label={`Workers for ${session.display_name}: ${session.workers.length} worker${session.workers.length === 1 ? "" : "s"}`}',
    );
    expect(source).toContain("sessionWorkerTreeRows(session.workers)");
    expect(source).toContain('role="treeitem" aria-level={depth + 1}');
    expect(source).toContain('className="worker-relation"');
    expect(source).toContain('className="worker-working-icon"');
    expect(source).toContain("worker.display_name || `ID${worker.ordinal}`");
    expect(styles).toContain(".worker-row");
    expect(styles).toContain(
      ".worker-row.child-worker .worker-relation::before",
    );
    expect(styles).toContain(".worker-working-icon {");
    expect(styles).not.toContain(".worker-state-dot.working");
  });

  it("keeps cwd in the composer footer instead of repeating it in session navigation", () => {
    expect(source).toContain(
      'className={`session ${session.session_id === activeSession?.session_id ? "active" : ""}`}',
    );
    expect(source).toContain(
      'className="session-name" title={session.display_name}',
    );
    expect(styles).toContain(
      ".session-row .session-identity { margin-left: 3px; }",
    );
    expect(source).not.toContain('className="session-detail session-cwd"');
    expect(source).not.toContain("workspacePathLabel(session.current_dir)");
    expect(source).toContain(
      'title={runtimeLocked ? "Session controls are temporarily locked" : session.display_name}',
    );
    expect(source).toContain(
      "const sessionEndpointName = endpointNameForProfile(server?.model_endpoints ?? [], session.runtime_profile) ?? UNCONFIGURED_MODEL_LABEL;",
    );
    expect(source).toContain(
      'className={`session-endpoint-reveal ${renamingSession ? "pending" : ""}`} title={renamingSession ? "Saving name" : sessionEndpointName}',
    );
    expect(source).not.toContain('className="session-detail session-profile"');
    expect(source).toContain(
      'className="session-working-icon" size={15} aria-label="Session working"',
    );
    expect(source).toContain(
      'visuallyWorking ? <LoaderCircle className="session-working-icon"',
    );
    expect(source).toContain(
      'session.state === "interrupted" ? <CircleStop className="session-interrupted-icon" size={15} aria-label="Session interrupted by runtime restart"',
    );
    expect(styles).toContain(".session-interrupted-icon {");
    expect(source).toContain(
      'className="session-unread-dot" aria-label="Session has new completed work"',
    );
    expect(source).not.toContain("className={`session-dot ${session.state}`}");
    expect(styles).not.toContain(".session-dot {");
    expect(source).not.toContain('className="session-state">busy</span>');
    expect(styles).not.toContain(".session-state");
    expect(source).toContain('className="composer-cwd-inline"');
    expect(source).toContain(
      '<span className="composer-cwd-inline" title={activeSession.current_dir}><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></span>',
    );
    expect(source.indexOf('className="queued-message-list"')).toBeLessThan(
      source.indexOf('<form className="composer"'),
    );
    expect(source.indexOf('aria-label="Message Timem"')).toBeLessThan(
      source.lastIndexOf('className="composer-cwd-inline"'),
    );
    expect(styles).toContain(
      ".path-tail { direction: rtl; text-align: left; unicode-bidi: plaintext; }",
    );
    expect(viewModelSource).toContain("context_state");
    expect(styles).toContain(".composer-cwd-inline");
    expect(styles).toContain(
      ".composer-actions { min-height: 26px; margin-top: 6px; padding-top: 7px; border-top: 1px solid #ffffff12; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .composer-actions { border-top-color: #17324d14; }',
    );
    expect(source).toContain(
      'activeSession?.debug_dir && <span className="composer-cwd-inline composer-debug-inline" title={activeSession.debug_dir}><b>DEBUG:</b><span>{activeSession.debug_dir}</span></span>',
    );
    expect(source).not.toContain("tailPath(activeSession.debug_dir, 64)");
    expect(styles).toContain(
      ".composer-paths { min-width: 0; flex: 1 1 auto; display: grid; gap: 2px; overflow: hidden; }",
    );
    expect(styles).toContain(
      ".composer-debug-inline { align-items: flex-start; overflow: visible; }",
    );
    expect(styles).toContain(
      ".composer-debug-inline span { overflow: visible; text-overflow: clip; white-space: normal; overflow-wrap: anywhere; user-select: text; }",
    );
    expect(styles).toContain(
      'font-family: "SFMono-Regular", Consolas, monospace;',
    );
  });

  it("keeps Search, Favorite, and Settings in a stable sidebar hierarchy", () => {
    const searchEntry = source.indexOf(
      "{!leftSidebarCollapsed && <span>Search</span>}",
    );
    const favoriteEntry = source.indexOf(
      "{!leftSidebarCollapsed && <span>Favorite</span>}",
    );
    const settingsEntry = source.indexOf(
      "{!leftSidebarCollapsed && <span>Settings</span>}",
    );
    expect(searchEntry).toBeGreaterThan(-1);
    expect(favoriteEntry).toBeGreaterThan(searchEntry);
    expect(settingsEntry).toBeGreaterThan(favoriteEntry);
    expect(source).toContain(
      'const [settingsSection, setSettingsSection] = useState<SettingsSection>("appearance");',
    );
    expect(source).not.toContain("const [showMemSettings, setShowMemSettings]");
    expect(source).not.toContain("const [showMemSwitch, setShowMemSwitch]");
    expect(source).toContain(
      'ref={settingsButtonRef} className={`sidebar-settings-button ${showAppearance ? "active" : ""}`}',
    );
    expect(source).toContain('<Settings size={17} aria-hidden="true"/>');
    expect(source).toContain(
      "{!leftSidebarCollapsed && <span>Settings</span>}",
    );
    expect(source).toContain('aria-controls="settings-center"');
    expect(source).toContain(
      'onClick={() => { setChatLibraryMode(null); setSettingsSection("appearance"); setShowAppearance(true); }}',
    );
    expect(source).not.toContain('className="mem-card-row"');
    expect(source).not.toContain('className="mem-card"');
    expect(source).not.toContain(
      'className="mem-switch-action settings-action"',
    );
    expect(source).toContain("function SettingsCenter(");
    expect(source).toContain('className="settings-center-layout"');
    expect(source).toContain(
      'className="settings-center-nav" aria-label="Settings categories"',
    );
    expect(source).toContain(
      "<Palette size={16}/><span><strong>Appearance</strong></span>",
    );
    expect(source).toContain(
      "<Sparkles size={16}/><span><strong>Model Endpoints</strong></span>",
    );
    expect(source).toContain(
      "<Database size={16}/><span><strong>Memory</strong></span>",
    );
    expect(source).toContain(
      "<Wrench size={16}/><span><strong>ToolGen</strong></span>",
    );
    expect(source).not.toContain("<small>Theme, fonts, and text size</small>");
    expect(source).not.toContain(
      "<small>Add, edit, and delete endpoints</small>",
    );
    expect(source).not.toContain(
      "<small>Retention, temporary data, workspace</small>",
    );
    expect(source).not.toContain("<strong>ToolGen <em>Beta</em></strong>");
    expect(source).toContain("function EndpointSettingsPane(");
    expect(source).toContain(
      'className="memory-identity-card" aria-label="Current MEM"',
    );
    expect(source).toContain('className="memory-switch-entry"');
    expect(source).toContain('memoryPage === "switch"');
    expect(source).toContain('className="settings-back-link"');
    expect(source).toContain(
      'className="memory-switch-route" aria-label="Memory switch route"',
    );
    expect(source).toContain('className="memory-switch-impact"');
    expect(source).not.toContain('className="settings-memory-switch"');
    expect(source).toContain(
      'className="settings-temporary-list" role="list" aria-label="Largest temporary files"',
    );
    expect(source).toContain(
      'const memTemporaryItemsLoadedForRef = useRef("");',
    );
    expect(source).toContain(
      "if (memTemporaryItemsLoadedForRef.current === memPath) return;",
    );
    expect(source).toContain(
      "memTemporaryItemsLoadedForRef.current = memPath;",
    );
    expect(source).toContain("if (!memTemporaryItemsLoading) return;");
    expect(source).toContain(
      "Temporary files took too long to load. Reconnecting to try again…",
    );
    expect(source).toContain("socket.current?.close();");
    expect(source).toContain(
      "Limited tiers include a 4 MiB safe-write reserve.",
    );
    expect(source).toContain(
      'sendCommand({ type: "mem_temporary_items_list" })',
    );
    expect(source).toContain(
      'sendCommand({ type: "mem_temporary_items_delete", ids })',
    );
    expect(source).toContain(
      'sendCommand({ type: "mem_temporary_retention_update", days, max_bytes: maxBytes })',
    );
    expect(source).toContain(
      'sendCommand({ type: "mem_switch", path, stop_running: false })',
    );
    expect(source).toContain(
      'sendCommand({ type: "mem_switch", path: memSwitchCandidate.path, stop_running: true })',
    );
    expect(source).toContain(
      "function memSwitchRunningSessionCount(sessions: Session[])",
    );
    expect(source).toContain(
      'event.error === "mem_switch_active_sessions_confirmation_required"',
    );
    expect(source).toContain(
      "runningSessionCount: Math.max(1, memSwitchRunningSessionCount(sessionsRef.current))",
    );
    expect(source).toContain("function MemSwitchConfirmDialog(");
    expect(source).toContain(
      "will be marked interrupted and will not continue in the background",
    );
    expect(source).toContain(
      "function shellQuoteCommandArgument(value: string)",
    );
    expect(source).toContain(
      "timem-web --space {shellQuoteCommandArgument(candidate.path)}",
    );
    expect(source).toContain("Stop work and switch");
    expect(styles).toContain(".sidebar-settings-button {");
    expect(styles).toContain(".sidebar.collapsed .sidebar-settings-button {");
    expect(styles).toContain(
      ".settings-center-layout { flex: 1; min-height: 0; display: grid; grid-template-columns: 220px minmax(0, 1fr); }",
    );
    expect(styles).toContain(
      "/* Settings visual system: quiet solid surfaces, soft depth, and concise labels. */",
    );
    expect(styles).toContain(`.settings-center {
  border: 0;
  background: #141817;`);
    expect(styles).toContain(`.settings-center-nav button {
  grid-template-columns: 20px minmax(0, 1fr);
  align-items: center;
  border: 0;`);
    expect(styles).toContain(
      "/* Settings category navigation keeps every first-level item readable before hover. */",
    );
    expect(styles).toContain(
      ".settings-center-nav button {\n  color: #b9c7c1;\n}",
    );
    expect(styles).toContain(
      ".settings-center-nav button > svg {\n  color: #8fb0a6;\n}",
    );
    expect(styles).toContain(
      ".settings-center-nav button.active > svg {\n  color: #8fd0bd;\n}",
    );
    expect(styles).toContain(
      ".settings-center-nav button:disabled {\n  opacity: .68;\n  color: #9eaaa5;\n}",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .settings-center-nav button {\n  color: #40574f;\n}',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .settings-center-nav button > svg {\n  color: #537f72;\n}',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .settings-center-nav button:disabled {\n  opacity: .72;\n  color: #596b65;\n}',
    );
    expect(styles)
      .toContain(`.settings-center .segmented-control button.active {
  border: 0;
  background: #21342d;`);
    expect(styles).toContain(`.appearance-font-selects select,
.settings-field input,
.settings-field select,
.endpoint-editor-grid input,
.endpoint-editor-grid select,
.endpoint-editor-grid .structured-field-row input {
  border: 0;`);
    expect(styles)
      .toContain(`.settings-center :is(.primary, .secondary, .danger).compact,
.endpoint-editor-buttons :is(.primary, .secondary) {
  min-height: 34px;`);
    expect(styles).toContain(`.settings-center .danger.compact.confirm {
  background: #8f3f3a;`);
    expect(styles).toContain(`.endpoint-settings-edit {
  border: 0;
  background: #111614;`);
  });

  it("announces runtime connection state and explains settings availability", () => {
    expect(source).toContain(
      "const [runtimeEverConnected, setRuntimeEverConnected] = useState(false);",
    );
    expect(source).toContain(
      "const [reconnectAttempt, setReconnectAttempt] = useState(0);",
    );
    expect(source).toContain(
      "const connectionLabel = runtimeConnectionLabel(connected, snapshotReady, runtimeEverConnected, reconnectAttempt);",
    );
    expect(viewModelSource).toContain("export function runtimeConnectionLabel");
    expect(source).toContain(
      'const settingsTitle = !runtimeReady ? "Wait for the runtime snapshot before opening settings" : pendingMemSwitch ? "Memory switch is in progress" : "Open settings";',
    );
    expect(source).toContain(
      "const settingsButtonRef = useRef<HTMLButtonElement | null>(null);",
    );
    expect(source).toContain(
      "if (restoreFocus) settingsButtonRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      'className="settings-runtime-status" role="status" aria-live="polite" title={connectionLabel}',
    );
    expect(source).toContain(
      "const runtimeDisconnected = runtimeEverConnected && !connected;",
    );
    expect(source).toContain(
      "showRuntimeUnavailableDialog && <RuntimeUnavailableDialog detail={runtimeDisconnectedDetail} onClose={() => setRuntimeUnavailableDialogDismissed(true)}/>",
    );
    expect(source).toContain(
      "sessionInteractionLockReasonForState(pendingMemSwitch, connected, runtimeEverConnected, reconnectAttempt)",
    );
    expect(source).toContain(
      'className="runtime-disconnect-banner" role="alert"',
    );
    expect(source).toContain(
      'ref={settingsButtonRef} className={`sidebar-settings-button ${showAppearance ? "active" : ""}`}',
    );
    expect(source).toContain('<code title={memPath}>{memPath || "…"}</code>');
    expect(source).toContain("setPendingMemSwitch(false);");
    expect(styles).toContain(".runtime-disconnect-banner");
    expect(styles).toContain("@keyframes connection-retry");
  });

  it("uses session terminology consistently for the creation workflow", () => {
    expect(source).toContain("New session");
    expect(source).toContain(
      'const welcomeTitle = activeSession ? "Ready when you are." : "Create a session to start.";',
    );
    expect(source).toContain(
      'const welcomeText = activeSession ? "Ask Timem to investigate, write, or work with you." : "Use New session to choose a workspace and runtime profile.";',
    );
    expect(source).toContain("<h2>{welcomeTitle}</h2><p>{welcomeText}</p>");
    expect(source).toContain('aria-label="Create session"');
    expect(source).toContain('creating ? "Creating…" : "Create session"');
    expect(source).toContain("disabled={creating}");
    expect(source).toContain("activeModelRetryStatus, activityFromTopic");
    expect(viewModelSource).toContain('label: "响应超时"');
    expect(viewModelSource).toContain('label: "服务限流"');
    expect(viewModelSource).toContain('label: "上游异常"');
    expect(viewModelSource).toContain('label: "网络异常"');
    expect(viewModelSource).toContain(
      'progress ? `重试进度：${progress}` : ""',
    );
    expect(viewModelSource).not.toContain(
      "模型服务连接暂时失败，系统正在自动重连。",
    );
    expect(source).toContain("sessionCreateDecision");
    expect(source).toContain(
      'const canCreateSession = createDecision.kind === "send";',
    );
    expect(source).toContain(
      'value={workspaceDir} disabled={creating} placeholder="/absolute/path/to/workspace"',
    );
    expect(source).toContain(
      "workspaces.map((workspace) => <option value={workspace} key={workspace}>{tailPath(workspace, 64)}</option>)",
    );
    expect(source).toContain('list="new-session-workspaces"');
    expect(source).toContain('placeholder="/absolute/path/to/workspace"');
    expect(source).toContain(
      "Choose a suggested workspace or type an absolute directory path that exists on the Timem host.",
    );
    expect(source).toContain("disabled={!canCreateSession}");
    expect(source).not.toContain("New agent");
  });

  it("creates sessions with independent runtime environment overrides", () => {
    expect(source).toContain("SESSION_RUNTIME_FIELDS");
    expect(source).toContain("TIMEM_MODEL");
    expect(source).toContain("TIMEM_API_KEY");
    expect(source).toContain("TIMEM_ENABLE_THINKING");
    expect(source).toContain("TIMEM_REASONING_EFFORT");
    expect(source).toContain("TIMEM_STREAM");
    expect(source).toContain('kind === "boolean"');
    expect(source).toContain("type={kind}");
    expect(source).toContain("const resetEnv = (key: string)");
    expect(source).toContain('className="session-runtime-control"');
    expect(source).toContain('className="session-runtime-reset"');
    expect(source).toContain("title={`Reset ${label} to inherited value`}");
    expect(source).toContain(
      "aria-label={`Reset ${label} to inherited value`}",
    );
    expect(source).toContain("onClick={() => resetEnv(key)}>Reset</button>");
    expect(source).toContain("onCreate={(command) => {");
    expect(source).toContain(
      "endpointNameForProfile(server?.model_endpoints ?? [], session.runtime_profile)",
    );
    expect(styles).toContain(".session-runtime-grid");
    expect(styles).toContain(".session-runtime-control");
    expect(styles).toContain(".session-runtime-reset");
    expect(styles).toContain(
      ':root[data-theme="light"] .session-runtime-reset',
    );
    expect(styles).toContain(".session-profile");
  });

  it("keeps model failures in the task stream without a duplicate workspace banner", () => {
    expect(source).toContain("commandSessionId(completed?.command)");
    expect(source).toContain("isModelSubmissionCommand(completed?.command)");
    expect(source).toContain(
      "reportUiError(issue.title, issue.detail, sessionId)",
    );
    expect(source).toContain('kind === "model_error"');
    expect(source).not.toContain(
      'className="model-config-banner" role="alert"',
    );
    expect(source).not.toContain("activeModelServiceIssue");
    expect(source).not.toContain("modelServiceIssues");
    expect(styles).not.toContain(".model-config-banner");
  });

  it("coordinates endpoint, role, and session management colors across themes", () => {
    expect(source).toContain(
      'className="primary compact" disabled={deleteMode} onClick={() => onEdit("new")}><Plus size={14}/> Add endpoint',
    );
    expect(styles).toContain(
      "/* Coordinated management palette: teal is constructive/selected, amber is unavailable, red is destructive. */",
    );
    expect(styles).toContain("--management-accent: #68b8a7;");
    expect(styles).toContain("--management-panel: #171d1c;");
    expect(source).toContain(
      'className={`mcp-server ${connectionState} ${active && !deleteMode ? "selected" : ""}',
    );
    expect(styles).toContain(
      "/* Unified normal selection language across Session, endpoint, MCP, and Role. */",
    );
    expect(styles).toContain(`.session-row.active,
.endpoint-row.active,
.mcp-server.selected,
.worker-role-item.selected {
  background: #20352f;
  box-shadow: inset 0 0 0 1px #4b6d64, 0 2px 8px #05080740;
}`);
    expect(styles).toContain(`:root[data-theme="light"] .session-row.active,
:root[data-theme="light"] .endpoint-row.active,
:root[data-theme="light"] .mcp-server.selected,
:root[data-theme="light"] .worker-role-item.selected {
  background: #e5f2ee;
  box-shadow: inset 0 0 0 1px #a5c9bf, 0 2px 8px #31564b20;
}`);
    expect(styles).toContain(
      ".new-session {\n  border: 1px solid var(--management-accent-border);",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] {\n  --management-accent: #3f9886;',
    );
    expect(styles).toContain(
      '.mcp-session-toggle.failed[aria-checked="true"] { border-color: #d2a23b; background: #a97818;',
    );
    expect(styles).toContain(
      ".session-delete-manage.confirm { border-color: #c94f49; background: #c94f49;",
    );
    expect(styles).toContain(
      ".endpoint-add-action {\n  border-color: #3e5d55;\n  background: #20332e;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-add-action { border-color: #b8d2cb; background: #e8f2ef;',
    );
  });

  it("uses edge shadows instead of visible borders for non-form management surfaces", () => {
    expect(styles).toContain(
      "/* Border-light management surfaces: reserve visible borders for form controls and stateful toggles. */",
    );
    expect(styles).toContain(`.sidebar {
  border-right-color: transparent;
  box-shadow: none;
}`);
    expect(styles).toContain(`.worker-role-panel {
  border-left-color: transparent;
  box-shadow: none;
}`);
    expect(styles).toContain(`.worker-role-group,
.worker-role-item,
.endpoint-row {
  border-color: transparent;
  box-shadow: inset 0 0 0 1px #ffffff0b;
}`);
    expect(styles).toContain(`.endpoint-menu,
.mcp-panel {
  border-color: transparent;`);
    expect(styles).toContain(`.session-row.active,
.endpoint-row.active,
.mcp-server.selected,
.worker-role-item.selected {
  border-color: transparent;`);
    expect(styles).toContain(`:root[data-theme="light"] .endpoint-menu {
  border-color: transparent;`);
    expect(styles).toContain(
      ".endpoint-editor-grid input, .endpoint-editor-grid select { border-color: #3a4b45;",
    );
    expect(styles).toContain(`.worker-role-group-editor input,
.worker-role-editor input,
.worker-role-editor textarea { border-color: #394742;`);
    expect(styles).toContain(
      ".mcp-session-toggle { position: relative; width: 32px; height: 18px; min-height: 18px; flex: none; margin-right: 7px; border: 1px solid",
    );
    expect(styles).toContain(
      "/* Endpoint selection is shown only by the blue check badge, not by the row surface. */",
    );
    expect(styles).toContain(`.endpoint-row.active:not(.delete-selecting) {
  border-color: transparent;
  background: var(--management-card);
  box-shadow: inset 0 0 0 1px #ffffff0b;
}`);
    expect(styles)
      .toContain(`:root[data-theme="light"] .endpoint-row.active:not(.delete-selecting) {
  border-color: transparent;
  background: #fff;
  box-shadow: inset 0 0 0 1px #47605714;
}`);
    expect(styles).not.toContain("background: #1c302b;");
    expect(styles).not.toContain("background: #d8e8e3;");
    expect(styles).toContain(
      "/* Role cards stay borderless; depth comes from background and soft ambient shadow. */",
    );
    expect(styles).toContain(`.worker-role-item {
  border-color: transparent;
  box-shadow: 0 2px 7px #0508071f;
}`);
    expect(styles).toContain(`.worker-role-item.selected {
  border-color: transparent;
  box-shadow: 0 3px 10px #0508073d, 0 0 8px #68b8a71a;
}`);
  });

  it("uses softer placeholders across endpoint, role, session, MCP, and composer forms", () => {
    expect(styles).toContain(
      "/* Softer placeholder hierarchy across creation and editing forms. */",
    );
    expect(styles).toContain(":root { --form-placeholder: #62706c; }");
    expect(styles).toContain(
      ':root[data-theme="light"] { --form-placeholder: #9aa5a1; }',
    );
    expect(styles).toContain(`.endpoint-editor-grid,
  .worker-role-editor,
  .worker-role-group-editor,`);
    expect(styles).toContain(`) :is(input, textarea)::placeholder,
.composer textarea::placeholder,
.expanded-text-editor > textarea::placeholder {
  color: var(--form-placeholder);
  opacity: .78;
}`);
    expect(styles).toContain(`.composer textarea:disabled::placeholder {
  opacity: .52;
}`);
  });

  it("keeps session creation controls on one compact row without reserving hidden group-action space", () => {
    expect(source).toContain(
      'className={`session-management-actions ${sessionDeleteMode ? "deleting" : ""}`}><div className="session-create-actions"',
    );
    expect(source).toContain(
      'className="new-session" title={newSessionLabel} aria-label={newSessionLabel}',
    );
    expect(source).toContain(
      '<FolderPlus size={16}/></button><button type="button" ref={newSessionButtonRef} className="new-session"',
    );
    expect(source).toContain(
      'className="session-group-toggle" title={bucket?.name ?? "Unsorted"}',
    );
    expect(styles).toContain(
      "/* Compact Session toolbar and full-width group labels. */",
    );
    expect(styles).toContain(`.session-create-actions,
.session-delete-actions { display: flex; align-items: center; gap: 5px; }`);
    expect(styles).toContain(`.session-group-actions {
  position: absolute;`);
    expect(styles).toContain(`visibility: hidden;
  opacity: 0;`);
    expect(styles).not.toContain(
      ".session-group-heading:hover .session-group-toggle,",
    );
  });

  it("keeps endpoint management in Settings and the header menu apply-only", () => {
    expect(source).toContain("function EndpointSettingsPane(");
    expect(source).toContain(
      "const [deleteMode, setDeleteMode] = useState(false);",
    );
    expect(source).toContain(
      'const [selectedEndpointId, setSelectedEndpointId] = useState("");',
    );
    expect(source).toContain('className="endpoint-settings-toolbar"');
    expect(source).toContain(
      'className={`danger compact ${deleteMode ? "confirm" : ""}`}',
    );
    expect(source).toContain(
      'className={`endpoint-settings-row ${deleteMode ? "delete-selecting" : ""} ${selectedForDelete ? "delete-selected" : ""}`}',
    );
    expect(source).toContain(
      '<small className="endpoint-model-summary"><Sparkles size={10} className="session-model-icon" aria-hidden="true"/><span>{endpoint.model}',
    );
    expect(styles).toContain(
      ".endpoint-model-summary { min-width: 0; display: flex; align-items: center; gap: 5px; }",
    );
    expect(styles).toContain(
      ".endpoint-model-summary > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }",
    );
    expect(source).toContain("endpoint-choice-box");
    expect(source).toContain("{active && <Check size={11} strokeWidth={3}/>}");
    expect(source).toContain('className="endpoint-menu-edit" onClick={onEdit}');
    expect(source).toContain("<Pencil size={13}/><span>编辑</span>");
    expect(source).not.toContain('className="endpoint-add-action"');
    expect(source).not.toContain('className="endpoint-delete-manage');
    expect(source).toContain(
      "function ModelEndpointPanel({ panelRef, server, session, onEdit, onApply }",
    );
    expect(source).not.toContain(
      "function ModelEndpointPanel({ panelRef, server, session, endpointEditor",
    );
    expect(source).not.toContain("endpoint-selected-badge");
    expect(source).toContain('className="endpoint-copy"');
    expect(source.indexOf("className={`endpoint-choice-box")).toBeGreaterThan(
      source.indexOf("<small title={endpoint.base_url}>"),
    );
    expect(styles).toContain(
      ".endpoint-select { min-width: 0; display: grid; grid-template-columns: minmax(0, 1fr) 18px; align-items: center; gap: 12px;",
    );
    expect(styles).toContain(
      ".endpoint-copy { min-width: 0; display: grid; gap: 3px; }",
    );
    expect(styles).toContain(
      ".endpoint-choice-box { box-sizing: border-box; width: 17px; height: 17px; place-self: center; display: grid; place-items: center; overflow: hidden;",
    );
    expect(styles).toContain(
      ".endpoint-choice-box svg { display: block; width: 11px; height: 11px; color: currentColor; }",
    );
    expect(styles).toContain(
      ".endpoint-choice-box.selected { border-color: #5aa08d; background: #367b69; color: #effff9;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-choice-box.selected { border-color: #4f9381; background: #3f8b78; color: #fff;',
    );
    expect(styles).toContain(
      ".endpoint-name-line { min-width: 0; display: flex; align-items: center; }",
    );
    expect(styles).toContain(
      "/* Endpoint list text uses normal reading contrast instead of placeholder-like gray. */",
    );
    expect(styles).toContain(
      ".endpoint-select:disabled { opacity: 1; cursor: default; }",
    );
    expect(styles).toContain(
      ".endpoint-select strong { color: #edf4f1; font-weight: 650; }",
    );
    expect(styles).toContain(".endpoint-select small { color: #a9b7b2; }");
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-select strong { color: #1f2d29; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-select small { color: #5f706a; }',
    );
    expect(styles).toContain(
      "/* Active Session uses a deeper pine surface with borderless, graduated edges. */",
    );
    expect(styles).toContain(
      "background: linear-gradient(90deg, #17372f 0%, #1d453b 52%, #183b33 100%);",
    );
    expect(styles).toContain(
      "box-shadow: 0 3px 10px #050a0838, 0 0 12px #285e501f;",
    );
    expect(styles).toContain(
      "background: linear-gradient(90deg, #1b4037 0%, #234f44 52%, #1d443a 100%);",
    );
    expect(styles).toContain(
      "background: linear-gradient(90deg, #b6d2ca 0%, #c2dad3 52%, #b9d3cc 100%);",
    );
    expect(styles).toContain(
      "box-shadow: 0 3px 10px #31564b18, 0 0 12px #4f887827;",
    );
    expect(styles).toContain(
      "background: linear-gradient(90deg, #abcac1 0%, #b8d3cb 52%, #aecdc4 100%);",
    );
    expect(styles).not.toContain("inset 0 0 0 1px #4f756a80");
    expect(styles).not.toContain("inset 0 0 0 1px #9ebfb680");
    expect(styles).toContain(
      "/* High-contrast interaction states across Session, endpoint, MCP, Role, and Memory. */",
    );
    expect(styles).toContain("--delete-check-bg: #a83f39;");
    expect(styles).toContain(`.session-delete-select.selected,
.endpoint-delete-select.selected,
.mcp-delete-select.selected {`);
    expect(styles).toContain("stroke-width: 3.25;");
    expect(styles).toContain(`.session-row.delete-selected,
.endpoint-row.delete-selected,
.mcp-server.delete-selected,
.worker-role-item.delete-selected {`);
    expect(styles).toContain('.worker-role-item input[type="checkbox"] {');
    expect(styles).toContain(
      '.worker-role-item.delete-selecting input[type="checkbox"] { accent-color: var(--delete-check-bg); }',
    );
    expect(styles).toContain(`.mem-card:disabled,
.mem-switch-action:disabled {
  opacity: .72;`);
    expect(styles).toContain(".mem-card:focus-visible,");
    expect(styles).toContain(
      ':root[data-theme="light"] .mcp-delete-select.selected {',
    );
    expect(source).toContain(
      'className="endpoint-delete-select">{selectedForDelete && <Check size={13}/>}',
    );
    expect(source).toContain("<Plus size={14}/> Add endpoint");
    expect(source).not.toContain("title={`Delete ${endpoint.name}`}");
    expect(styles).toContain(
      "/* Shared compact create/delete management controls for model endpoints. */",
    );
    expect(styles).toContain(
      ".endpoint-delete-manage.confirm { border-color: #c94f49; background: #c94f49;",
    );
    expect(styles).toContain(
      ".endpoint-row.delete-selected { border-color: #a45e59; background: #332625;",
    );
    expect(styles).toContain(
      ".endpoint-menu-heading > .endpoint-management-actions {",
    );
    expect(styles).toContain(
      "display: flex;\n  align-items: center;\n  justify-content: flex-end;",
    );
    expect(styles).toContain(
      ".endpoint-menu-heading > .endpoint-management-actions > button {\n  display: grid;\n  place-items: center;",
    );
    expect(styles).toContain(
      ".endpoint-management-actions > button > svg { display: block; margin: 0; }",
    );
    expect(styles).toContain(
      "/* Refined endpoint delete selection affordance. */",
    );
    expect(styles).toContain(
      ".endpoint-delete-select {\n  width: 20px;\n  height: 20px;",
    );
    expect(styles).toContain(
      ".endpoint-delete-select.selected svg {\n  display: block;\n  color: #fff8f7;",
    );
    expect(styles).toContain(
      ".endpoint-row.delete-selecting .endpoint-select:focus-visible {",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-delete-select.selected {',
    );
  });

  it("manages shared model endpoints without leaking API keys into snapshots", () => {
    expect(source).toContain("MODEL ENDPOINTS");
    expect(source).toContain("新增接入点");
    expect(source).toContain("model_endpoint_upsert");
    expect(source).toContain("model_endpoint_delete");
    expect(styles).toContain(
      "/* Endpoint deletion confirmation follows the Settings control language. */",
    );
    expect(styles)
      .toContain(`.endpoint-delete-backdrop .decision-actions button {
  min-height: 36px;
  border: 0;
  border-radius: 8px;`);
    expect(styles)
      .toContain(`.endpoint-delete-backdrop .decision-actions .danger {
  background: #8f3f3a;
  color: #fff4f2;`);
    expect(source).toContain("model_endpoint_apply");
    expect(source).toContain("model_endpoint_secret_reveal");
    expect(source).toContain("最大上下文窗口");
    expect(source).toContain("最大输出");
    expect(source).toContain("MODEL_CONTEXT_WINDOW_OPTIONS");
    expect(source).toContain("MODEL_OUTPUT_TOKEN_OPTIONS");
    expect(source).toContain('event.type === "model_endpoint_secret_revealed"');
    expect(source).toContain("api_key_configured");
    expect(source).toContain('className="endpoint-api-key"');
    expect(source).toContain(
      "const [showApiKey, setShowApiKey] = useState(false);",
    );
    expect(source).toContain(
      '<input type={showApiKey ? "text" : "password"} autoComplete="new-password" spellCheck={false} value={apiKey}',
    );
    expect(source).toContain(
      'const apiKeyVisibilityLabel = showApiKey ? "隐藏 API Key" : "显示 API Key";',
    );
    expect(source).toContain('className="endpoint-api-key-actions"');
    expect(source).toContain('idle: "复制 API Key"');
    expect(source).toContain(
      '{copyState === "copied" ? <CheckCheck size={12}/> : <Copy size={12}/>}',
    );
    expect(source).toContain(
      "{showApiKey ? <EyeOff size={13}/> : <Eye size={13}/>}",
    );
    expect(source).toContain(
      "setShowApiKey(false); setShowHeaders(!endpoint);",
    );
    expect(source).toContain('className="wide endpoint-structured-headers"');
    expect(styles).toContain(
      ".endpoint-editor-grid > .wide { grid-column: 1 / -1; }",
    );
    expect(source).toContain('StructuredKeyValueEditor label="Headers"');
    expect(source).toContain("无需输入 JSON 或多行格式文本");
    expect(source).toContain("添加 Header");
    expect(source).toContain("aria-label={`删除 ${label}`}");
    expect(styles).toContain(
      ".endpoint-editor-grid .endpoint-api-key input { padding-right: 56px; }",
    );
    expect(styles).toContain(".endpoint-api-key-actions { position: absolute;");
    expect(styles).toContain(
      ".endpoint-api-key-actions button { width: 22px; height: 22px;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-api-key-actions button { background: transparent;',
    );
    expect(source).toContain("endpointMatchesProfile");
    expect(styles).toContain(".endpoint-menu");
    expect(source).toContain('className="runtime-card endpoint-menu"');
    expect(source).toContain(
      'if (endpointEditor) return <section className="settings-pane endpoint-settings-pane editing"',
    );
    expect(source).toContain('setSettingsSection("endpoints");');
    expect(source).toContain('setEndpointEditor("new");');
    expect(styles).toContain(
      ".endpoint-menu { position: absolute; z-index: 8; top: 62px; left: 24px; width: min(430px, calc(100vw - 32px)); max-height: min(480px, calc(100vh - 90px));",
    );
    expect(styles).toContain(".endpoint-actions");
    expect(source).toContain("尚未配置模型接入点");
    expect(source).toContain('className="endpoint-guide-bubble"');
    expect(source).toContain('className="endpoint-guide-icon"');
    expect(source).toContain('className="endpoint-guide-copy"');
    expect(source).toContain("添加一个接入点，即可开始使用当前 Session。");
    expect(source).toContain("<span>立即配置</span>");
    expect(styles).toContain(
      "grid-template-columns: 34px minmax(0, 1fr) auto;",
    );
    expect(styles).toContain(".endpoint-guide-icon {");
    expect(styles).toContain(".endpoint-guide-copy {");
    expect(styles).toContain(
      "background: linear-gradient(145deg, #17242b 0%, #121d24 100%);",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .endpoint-guide-bubble',
    );
    expect(styles).toContain("@media (max-width: 420px)");
    expect(source).toContain('setEndpointEditor("new")');
    expect(source).toContain("...NO_MODEL_ENDPOINTS_ISSUE");
    expect(styles).toContain(".endpoint-guide-bubble");
  });

  it("dismisses the runtime configuration card on outside click or Escape", () => {
    expect(source).toContain(
      "runtimePanelRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      "const closeRuntimePanel = useCallback((restoreFocus = true) => {",
    );
    expect(source).toContain(
      "if (restoreFocus) runtimeButtonRef.current?.focus({ preventScroll: true });",
    );
    expect(source).toContain(
      'document.addEventListener("pointerdown", dismissOnOutsidePointer)',
    );
    expect(source).toContain("runtimeButtonRef.current?.contains(target)");
    expect(source).toContain("runtimePanelRef.current?.contains(target)");
    expect(source).toContain("closeRuntimePanel(false);");
    expect(source).toContain('if (event.key === "Escape") closeRuntimePanel()');
    expect(source).toContain(
      'const runtimeLabel = showRuntime ? "Close runtime information" : "Open runtime information";',
    );
    expect(source).toContain(
      "aria-label={`${runtimeLabel}: ${headerModelLabel}`}",
    );
    expect(source).toContain("aria-expanded={showRuntime}");
    expect(source).toContain(
      "if (showRuntime) closeRuntimePanel(); else setShowRuntime(true);",
    );
    expect(source).toContain(
      'id="runtime-panel" ref={panelRef} className="runtime-card" tabIndex={-1}',
    );
    expect(source).toContain(
      'id="runtime-panel" ref={panelRef} className="runtime-card runtime-settings" tabIndex={-1}',
    );
  });

  it("reconciles only applied runtime fields and preserves unrelated drafts", () => {
    expect(source).toContain(
      "setDrafts((current) => reconcileRuntimeDrafts(current, runtimeOptions))",
    );
    expect(source).toContain(
      "sessionRuntimeOptions(session?.runtime_profile, server?.runtime_options ?? [])",
    );
    expect(source).toContain(
      "useEffect(() => setDrafts({}), [session?.session_id]);",
    );
    expect(source).toContain(
      'const pendingRuntimeLabel = pendingKeys.size ? `Applying runtime setting${pendingKeys.size === 1 ? "" : "s"}: ${Array.from(pendingKeys).map(runtimeOptionLabel).join(", ")}` : "";',
    );
    expect(source).toContain("const dirty = value !== option.value;");
    expect(source).toContain(
      "const optionLabel = runtimeOptionLabel(option.key);",
    );
    expect(source).toContain("<span>{optionLabel}</span>");
    expect(source).toContain('className="secondary compact runtime-reset"');
    expect(source).toContain("title={`Reset ${optionLabel} to current value`}");
    expect(source).toContain(
      "aria-label={`Reset ${optionLabel} to current value`}",
    );
    expect(source).toContain(
      "const resetDraft = () => setDrafts((current) => { const { [option.key]: _removed, ...rest } = current; return rest; });",
    );
    expect(source).toContain(
      'if (event.key === "Enter" && !event.nativeEvent.isComposing && dirty && !pending) { event.preventDefault(); onUpdate(option.key, value); }',
    );
    expect(source).toContain(
      'if (event.key === "Escape" && dirty) { event.preventDefault(); resetDraft(); }',
    );
    expect(source).toContain("onClick={resetDraft}");
    expect(source).toContain("disabled={pending || !dirty}");
    expect(source).toContain(
      '(pendingRuntimeLabel || credentialPending) && <p className="runtime-pending-status" role="status" aria-live="polite">',
    );
    expect(styles).toContain(
      ".runtime-options label > div input, .runtime-options label > div select { flex: 1 1 auto; }",
    );
    expect(styles).toContain(".runtime-reset { flex: none; }");
    expect(styles).toContain(
      ".runtime-settings { display: block; overflow: visible; padding: 14px; }",
    );
    expect(styles).not.toContain("max-height: 360px; overflow: auto;");
    expect(styles).toContain(".runtime-pending-status");
  });

  it("defaults OpenAI-compatible endpoint streaming on while preserving saved endpoint choices", () => {
    expect(source).toContain("stream: endpoint?.stream ?? true");
    expect(source).toContain(
      'setDraft({ ...draft, api_protocol, stream: api_protocol === "openai-compatible" });',
    );
    expect(source).toContain(
      'disabled={draft.api_protocol !== "openai-compatible"}',
    );
  });

  it("uses select controls for runtime settings with predefined values", () => {
    expect(source).toContain(
      "function runtimeSelectOptions(key: string): readonly string[] | null",
    );
    expect(source).toContain('case "TIMEM_BASH_APPROVAL":');
    expect(source).toContain('return ["approve", "ask"];');
    expect(source).toContain('case "TIMEM_WORK_INSTRUCTIONS":');
    expect(source).toContain('return ["silent", "ask", "off"];');
    expect(source).toContain('case "TIMEM_API_PROTOCOL":');
    expect(source).toContain(
      'return ["openai-compatible", "openai-responses", "anthropic"];',
    );
    expect(source).toContain('case "TIMEM_RESPONSE_PROTOCOL":');
    expect(source).toContain('return ["xml", "json"];');
    expect(source).not.toContain('<option value="markdown">markdown</option>');
    expect(source).toContain('case "TIMEM_MAX_ROUNDS":');
    expect(source).toContain('return ["50", "200", "500", "unlimited"];');
    expect(source).toContain("options ? <select value={value}");
    expect(source).toContain(
      'options.map((choice) => <option value={choice} key={choice}>{choice === "unlimited" ? "Unlimited" : choice}</option>)',
    );
    expect(styles).toContain(
      ".runtime-options input, .runtime-options select, .session-modal input, .session-modal select",
    );
  });

  it("renders context compaction as a compact status pill with a reduced-motion fallback", () => {
    expect(source).toContain("<ContextCompactNotice");
    expect(source).toContain("<Gauge size={13}/>");
    expect(source).toContain("<span>Dynamic context</span>");
    expect(source).toContain(
      "Text ${formatTokens(activity.text_before_tokens)",
    );
    expect(source).toContain(
      "Tool ${formatTokens(activity.native_before_tokens)",
    );
    expect(source).toContain("aria-label={label} title={breakdown}");
    expect(styles).toContain(".context-compact-notice");
    expect(styles).toContain("width: fit-content");
    expect(styles).toContain(
      "grid-template-columns: 22px minmax(0, auto) 72px",
    );
    expect(styles).toContain(".compact-copy small");
    expect(styles).toContain("border-radius: 999px");
    expect(styles).toContain(
      ".compact-meter { position: relative; width: 72px; height: 3px;",
    );
    expect(styles).toContain("prefers-reduced-motion: reduce");
  });

  it("keeps routing identifiers out of the task work stream", () => {
    expect(source).toContain(
      '["kind", "session_id", "context_id", "worker_id"].includes(key)',
    );
  });

  it("persists appearance preferences inside the unified settings center without changing core state", () => {
    expect(appearanceSource).toContain(
      'APPEARANCE_STORAGE_KEY = "timem-web-appearance-v1"',
    );
    expect(appearanceSource).toContain("root.dataset.theme = appearance.theme");
    expect(appearanceSource).toContain(
      "root.dataset.userFont = appearance.userFont",
    );
    expect(appearanceSource).toContain(
      "root.dataset.userChineseFont = appearance.userChineseFont",
    );
    expect(appearanceSource).toContain(
      "root.dataset.userBold = String(appearance.userBold)",
    );
    expect(appearanceSource).toContain(
      "root.dataset.agentFont = appearance.agentFont",
    );
    expect(appearanceSource).toContain(
      "root.dataset.agentChineseFont = appearance.agentChineseFont",
    );
    expect(appearanceSource).toContain(
      "root.dataset.agentBold = String(appearance.agentBold)",
    );
    expect(appearanceSource).toContain(
      "root.dataset.textSize = appearance.textSize",
    );
    expect(source).toContain(
      "const appearancePanelRef = useRef<HTMLElement | null>(null);",
    );
    expect(source).not.toContain("const appearanceButtonRef");
    expect(source).not.toContain('aria-controls="appearance-panel"');
    expect(source).not.toContain("<AppearancePanel");
    expect(source).toContain("<SettingsCenter");
    expect(source).toContain("panelRef={appearancePanelRef}");
    expect(source).toContain("appearance={appearance}");
    expect(source).toContain("onAppearanceChange={setAppearance}");
    expect(source).toContain("aria-pressed={appearance.theme === theme}");
    expect(source).toContain(
      'value={appearance.userChineseFont} aria-label="User Chinese font"',
    );
    expect(source).toContain(
      'value={appearance.userFont} aria-label="User other language font"',
    );
    expect(source).toContain("checked={appearance.userBold}");
    expect(source).toContain(
      'value={appearance.agentChineseFont} aria-label="Agent Chinese font"',
    );
    expect(source).toContain(
      'value={appearance.agentFont} aria-label="Agent other language font"',
    );
    expect(source).toContain("checked={appearance.agentBold}");
    expect(source).toContain("aria-pressed={appearance.textSize === size}");
    expect(source).toContain(
      'role="dialog" aria-modal="true" aria-labelledby="settings-center-title"',
    );
    expect(source).toContain(
      'className="settings-pane appearance-settings-pane"',
    );
    expect(styles).toContain(':root[data-user-font="serif"]');
    expect(styles).toContain(':root[data-agent-font="serif"]');
    expect(styles).toContain(':root[data-user-chinese-font="heiti"]');
    expect(styles).toContain(':root[data-agent-chinese-font="kaiti"]');
    expect(styles).toContain(':root[data-user-bold="true"]');
    expect(styles).toContain(':root[data-agent-bold="true"]');
    expect(styles).toContain(
      ':root[data-text-size="small"] { --content-size: 12.6px; }',
    );
    expect(styles).toContain(
      ':root[data-text-size="large"] { --content-size: 14.4px; }',
    );
  });

  it("keeps the active session label readable in light theme after style overrides", () => {
    expect(styles).toContain(
      ':root[data-theme="light"] .session-row.active { background: #e8e8e8; box-shadow: none; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .session-row.active .session.active { background: transparent; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .session-row.active .session { color: #202020; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .session-row.active .session-profile { color: #747474; }',
    );
  });

  it("renders GFM, mathematical notation, and highlighted code with a copy affordance", () => {
    expect(markdownSource).toContain('import remarkGfm from "remark-gfm"');
    expect(markdownSource).toContain('import remarkMath from "remark-math"');
    expect(markdownSource).toContain('import rehypeKatex from "rehype-katex"');
    expect(source).toContain('import "katex/dist/katex.min.css"');
    expect(markdownSource).toContain("remarkPlugins={[remarkGfm, remarkMath]}");
    expect(markdownSource).toContain(
      "rehypePlugins={[rehypeHighlight, rehypeKatex]}",
    );
    expect(markdownSource).toContain(
      ">{normalizeMarkdownMath(text)}</ReactMarkdown>",
    );
    expect(markdownSource).toContain("pre: CodeBlock");
    expect(markdownSource).toContain(
      'className="table-scroll" role="region" tabIndex={0} aria-label="Scrollable table. Use horizontal scroll to inspect all columns."',
    );
    expect(markdownSource).toContain(
      "const codeCopySubject = `${language} code`;",
    );
    expect(markdownSource).toContain(
      "const { copyState, copy, copyLabel, copyClass } = useTimedClipboardCopy(code, {",
    );
    expect(markdownSource).toContain("idle: `Copy ${codeCopySubject}`");
    expect(markdownSource).toContain("copied: `${codeCopySubject} copied`");
    expect(markdownSource).toContain(
      "failed: `Copy ${codeCopySubject} failed`",
    );
    expect(markdownSource).toContain("className={copyClass}");
    expect(markdownSource).toContain("aria-label={copyLabel}");
    expect(styles).toContain(".markdown-body blockquote");
    expect(styles).toContain(
      ".markdown-body .katex-display {\n  max-width: 100%;",
    );
    expect(styles).toContain("margin: 1.05em 0 1.15em;");
    expect(styles).toContain("overflow-x: auto;\n  overflow-y: visible;");
    expect(styles).toContain("padding: .55em 0 .7em;\n  line-height: 1.35;");
    expect(styles).toContain(".table-scroll");
    expect(styles).toContain("scrollbar-gutter: auto;");
    expect(styles).toContain("scrollbar-gutter: stable;");
    expect(styles).toContain(".table-scroll:focus-visible");
    expect(styles).toContain(':root[data-theme="light"] .table-scroll');
    expect(styles).toContain(".code-block figcaption");
    expect(styles).toContain(
      ".markdown-body h1, .markdown-body h2, .markdown-body h3, .markdown-body h4 { margin: 1.1em 0 .4em;",
    );
    expect(styles).toContain(
      ".markdown-body p, .markdown-body ul, .markdown-body ol { margin: .48em 0; }",
    );
    expect(styles).toContain(".markdown-body li + li { margin-top: .2em; }");
    expect(styles).toContain(
      ".markdown-body blockquote { margin: .68em 0; padding: .15em .75em;",
    );
    expect(styles).toContain(".table-scroll { width: 100%; margin: 11px 0;");
    expect(styles).toContain(
      ".code-block { overflow: hidden; margin: .68em 0;",
    );
    expect(styles).toContain(
      ".code-block figcaption { min-width: 0; height: 26px;",
    );
    expect(styles).toContain(
      "border-radius: 0; padding: 10px 11px; background: #0d141b;",
    );
    expect(styles).toContain(
      ".code-block figcaption > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }",
    );
    expect(styles).toContain(".code-block figcaption button { flex: none;");
    expect(styles).toContain(
      ':root[data-theme="light"] .code-block { border-color: #cfddda; background: #f3f7f6; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .code-block figcaption { border-color: #d5e1de; background: #eaf1ef; color: #657b79; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .turn-final-delivery > .message-content .code-block pre,',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .code-block .hljs-comment,',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .code-block .hljs-keyword,',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .code-block .hljs-punctuation,',
    );
  });

  it("moves submitted files from the composer into a compact user attachment list", () => {
    expect(source).toContain("consumedAttachmentIds");
    expect(source).toContain('className="turn-entry-attachments"');
    expect(source).toContain("entry.attachments.map");
    expect(styles).toContain(".turn-entry-attachments > span");
  });

  it("lets users remove pending attachments without losing access to long file names", () => {
    expect(source).toContain('type: "attachment_remove"');
    expect(source).toContain(
      "const attachedFileCount = activeSession?.attachments.length ?? 0;",
    );
    expect(source).toContain(
      'const attachmentSummary = attachedFileCount === 1 ? "1 file attached" : `${attachedFileCount} files attached`;',
    );
    expect(source).toContain(
      "const attachmentStripLabel = uploadingAttachment",
    );
    expect(source).toContain(
      "? `${attachmentSummary}; ${uploadingAttachmentText}`",
    );
    expect(source).toContain(
      ": `Files attached to the next message; ${attachmentSummary}`;",
    );
    expect(source).toContain(
      'className="attachment-summary" title={attachmentSummary}',
    );
    expect(source).toContain('className="pending-attachment-name"');
    expect(source).toContain("title={attachment.name}");
    expect(source).toContain("pendingAttachmentRemoveIds.has");
    expect(source).toContain("disabled={removing || sessionInteractionLocked}");
    expect(source).toContain(
      "const removeLabel = removing ? `Removing ${attachment.name}` : sessionInteractionLocked ? `${sessionInteractionLockReason} · cannot remove ${attachment.name}` : `Remove ${attachment.name}`;",
    );
    expect(source).toContain("title={removeLabel} aria-label={removeLabel}");
    expect(source).toContain("aria-busy={removing || undefined}");
    expect(styles).toContain(".attachment-summary");
    expect(styles).toContain(".pending-attachment-name");
    expect(styles).toContain("text-overflow: ellipsis");
  });

  it("guards file uploads with visible pending feedback and no-session disabled state", () => {
    expect(source).toContain("pendingUploadSessionIdsRef");
    expect(source).toContain("setPendingUploadSessionIds");
    expect(source).toContain(
      "const [pendingUploadFiles, setPendingUploadFiles]",
    );
    expect(source).toContain(
      "setPendingUploadFiles((current) => ({ ...current, [activeSession.session_id]: { name: file.name, bytes: file.size } }));",
    );
    expect(source).toContain("Upload already in progress");
    expect(source).toContain(
      "removePendingKey(pendingUploadSessionIdsRef, setPendingUploadSessionIds, activeSession.session_id);",
    );
    expect(source).toContain(
      "uploadingAttachment={!!activeSession && pendingUploadSessionIds.has(activeSession.session_id)}",
    );
    expect(source).toContain(
      "uploadingAttachmentFile={activeSession ? pendingUploadFiles[activeSession.session_id] : undefined}",
    );
    expect(source).toContain(
      'const lockedControlHint = sessionInteractionLocked ? sessionInteractionLockReason : "";',
    );
    expect(source).toContain(
      'const uploadingAttachmentText = uploadingAttachmentFile ? `Uploading ${uploadingAttachmentFile.name}` : "Uploading file…";',
    );
    expect(source).toContain(
      'const attachTitle = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");',
    );
    expect(source).toContain(
      'const attachLabel = missingSessionHint || lockedControlHint || (uploadingAttachment ? uploadingAttachmentText : "Attach a file");',
    );
    expect(source).toContain(
      'const effectiveSendLabel = missingSessionHint || lockedControlHint || (submittingDraft ? "Sending…" : uploadingAttachment ? "Wait for file upload" : sendLabel);',
    );
    expect(source).toContain(
      'className={`attach-button ${uploadingAttachment ? "uploading" : ""}`}',
    );
    expect(source).toContain(
      "{uploadingAttachment ? <LoaderCircle size={17}/> : <Paperclip size={17}/>}",
    );
    expect(source).toContain("title={attachTitle}");
    expect(source).toContain("aria-label={attachLabel}");
    expect(source).toContain(
      "disabled={!activeSession || uploadingAttachment || sessionInteractionLocked}",
    );
    expect(source).toContain(
      "disabled={!activeSession || !hasDraftText || submittingDraft || uploadingAttachment || sessionInteractionLocked}",
    );
    expect(source).toContain(
      'aria-label={attachmentStripLabel} aria-live="polite" aria-busy={uploadingAttachment || undefined}',
    );
    expect(source).toContain(
      'uploadingAttachment && <div className="pending-attachment uploading" role="status"',
    );
    expect(source).toContain(
      "aria-label={uploadingAttachmentFile ? `${uploadingAttachmentText}, ${formatBytes(uploadingAttachmentFile.bytes)}` : uploadingAttachmentText}",
    );
    expect(source).toContain(
      "title={uploadingAttachmentFile?.name ?? uploadingAttachmentText}",
    );
    expect(source).toContain('className="upload-dot" aria-hidden="true"');
    expect(source).toContain(
      'uploadingAttachmentFile?.name ?? "Uploading file…"',
    );
    expect(source).toContain("formatBytes(uploadingAttachmentFile.bytes)");
    expect(styles).toContain(".attach-button.uploading:disabled");
    expect(styles).toContain(".attach-button.uploading svg");
    expect(styles).toContain(".pending-attachment.uploading");
    expect(styles).toContain(".upload-dot");
    expect(styles).toContain("@keyframes upload-button-pulse");
    expect(styles).toContain("@keyframes upload-dot-pulse");
    expect(styles).toContain("@media (prefers-reduced-motion: reduce)");
    expect(styles).toContain(
      ".toolrepo-header-button.count-pulse .toolrepo-header-count, .attach-button.uploading:disabled, .attach-button.uploading svg, .toolrepo-search-pending, .toolrepo-empty.searching svg, .upload-dot",
    );
    expect(styles).toContain(".send-button.sending svg");
    expect(styles).toContain(".completion-toolgen.sending svg");
    expect(styles).toContain("animation: none;");
  });

  it("keeps working-turn input visually consistent with a normal send", () => {
    expect(source).toContain(
      'placeholder={!activeSession ? "Create a session to start…" : sessionInteractionLocked ? sessionInteractionLockReason : activeSession.state === "working" ? "继续输入…"',
    );
    expect(source).toContain(
      '"Ask Timem to investigate, write, or work with you."',
    );
    expect(source).not.toContain("Ask Timem anything about this workspace");
    expect(source).toContain(
      'activeSession?.state === "working" ? "Queue message" : "Send message"',
    );
    expect(source).toContain(
      'className={`queued-message-list ${queueExpanded ? "expanded" : "collapsed"} ${queuePanelCollapsed ? "summary-only" : ""} ${queuedMessagesPause ? "paused" : ""}`}',
    );
    expect(source).toContain("自动发送已停止");
    expect(source).toContain("上一条正常完成后自动发送");
    expect(source).toContain('role="switch"');
    expect(source).toContain('className="queued-auto-send-switch"');
    expect(source).toContain("aria-checked={!queuedMessagesPause}");
    expect(source).toContain(
      'aria-label={queuedMessagesPause ? "开启自动发送" : "停止自动发送"}',
    );
    expect(source).toContain(
      'if (queuedMessagesPause) resumeQueuedMessages(activeSessionId); else pauseQueuedMessages(activeSessionId, "用户关闭了自动发送", "user");',
    );
    expect(source).toContain(
      "const queuedMessagesPauseBySessionRef = useRef<Record<string, QueuedMessagesPauseState>>({});",
    );
    expect(source).not.toContain('source: "error"');
    expect(source).not.toContain("runtime-disconnected:");
    expect(source).toContain("queuedAutoContinueSessionIdsRef");

    expect(source).toContain(
      "const pause = stopQueuedAutoSend(current, reason, source, Date.now());",
    );
    expect(source).toContain(
      "const queuedMessagesPause = activeSessionId ? queuedMessagesPauseBySession[activeSessionId] ?? null : null;",
    );
    expect(source).toContain(
      "new Set(Object.keys(queuedMessagesPauseBySessionRef.current))",
    );
    expect(source).toContain(
      "queuedMessagesPauseSessionId(reliableStorageScope, event.key)",
    );
    expect(source).toContain("liveSessionIds.has(pauseSessionId)");
    expect(source).toContain("delete next[sessionId];");
    expect(source).not.toContain("手动发送仍可用");
    expect(source).toContain(
      'const sendAsNewTurn = activeSession.state !== "working";',
    );
    expect(source).toContain(
      'sendAsNewTurn ? "作为新消息开始任务" : "立即发送为当前任务的补充"',
    );
    expect(source).toContain("!sendAsNewTurn, messageRoleIds, sendAsNewTurn)");
    expect(source).toContain("forceNewTurn = false");
    expect(viewModelSource).toContain(
      "command: forceSupplement && !forceNewTurn",
    );
    expect(source).not.toContain(
      "onClick={resumeQueuedMessages}>继续发送</button>",
    );
    expect(styles).toContain('.queued-auto-send-switch[aria-checked="true"]');
    expect(styles).toContain("transform: translateX(14px)");
    expect(styles).toContain(".queued-auto-send-switch:focus-visible");
    expect(styles).toContain(".queued-auto-send-thumb");
    expect(source).toContain('className="queued-message-supplement"');
    expect(source).toContain(
      'claimed ? "发送中…" : message.deliveryError ? "重试" : "立即"',
    );
    expect(source).toContain(
      "title={effectiveSendLabel} aria-label={effectiveSendLabel}",
    );
    expect(source).not.toContain(">Supplement</span>");
  });

  it("bounds, expands, collapses, and reorders the queued message list", () => {
    expect(source).toContain(
      "displayQueuedMessages.slice(0, COLLAPSED_QUEUE_LIMIT)",
    );
    expect(source).toContain('queueExpanded ? "expanded" : "collapsed"');
    expect(source).toContain("aria-expanded={queueExpanded}");
    expect(source).toContain(
      'queueExpanded ? "收起" : `展开 ${hiddenQueuedMessageCount} 条`',
    );
    expect(source).toContain(
      "const [collapsedQueuePanelSessionIds, setCollapsedQueuePanelSessionIds]",
    );
    expect(source).toContain(
      "const firstQueuedMessage = displayQueuedMessages[0];",
    );
    expect(source).toContain(
      'className={`queued-message-summary ${firstQueuedMessage?.deliveryError ? "delivery-error" : ""}`}',
    );
    expect(source).toContain("<p>{firstQueuedMessage?.text}</p>");
    expect(source).toContain('className="queued-message-summary-attachments"');
    expect(source).toContain(
      'className="queued-message-summary-count">{displayQueuedMessages.length} 条</small>',
    );
    expect(source).toContain('className="queued-message-panel-toggle"');
    expect(source).toContain(
      'title={queuePanelCollapsed ? "展开待发送队列" : "折叠待发送队列为一行"}',
    );
    expect(source).toContain("{!queuePanelCollapsed && <DndContext");
    expect(source).toContain(
      'className="queued-message-drag" disabled={dragDisabled}',
    );
    expect(source).toContain(
      "const finishQueuedMessageDrag = ({ active, over }: DragEndEvent)",
    );
    expect(source).toContain("reorderQueuedMessages(");
    expect(styles).toContain(
      ".queued-message-list.collapsed .queued-message-items { max-height: 224px; overflow: hidden; }",
    );
    expect(styles).toContain(
      ".queued-message-list.expanded .queued-message-items { max-height: min(50vh, 420px); overflow-y: auto;",
    );
    expect(styles).toContain(
      ".queued-message-list.summary-only { gap: 0; padding-block: 7px; }",
    );
    expect(styles).toContain(
      ".queued-message-header-actions { flex: 0 0 auto;",
    );
    expect(styles).toContain(
      ".queued-message-toggle, .queued-message-panel-toggle",
    );
    expect(styles).toContain(
      ".queued-message-list.summary-only > header { min-height: 26px; padding-bottom: 0; }",
    );
    expect(styles).toContain(
      ".queued-message-list > header small { min-width: 0; overflow: hidden;",
    );
    expect(styles).toContain(
      ".queued-message-summary { min-width: 0; flex: 1 1 auto;",
    );
    expect(styles).toContain(
      ".queued-message-summary p { min-width: 0; flex: 1 1 auto;",
    );
    expect(styles).toContain(
      ".queued-message-summary-count { padding-left: 6px; border-left: 1px solid",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .queued-message-summary p',
    );
    expect(styles).toContain(
      "@media (max-width: 520px) {\n  .queued-message-list > header { gap: 5px; }",
    );
  });

  it("keeps queued worker roles in normal flow above long message previews", () => {
    expect(source).toContain('className="queued-message-preview"');
    expect(source).toContain(
      'className="queued-message-roles" title={messageRoleNames.join(" | ")}',
    );
    expect(source).toContain('className="queued-message-actions"');
    expect(styles).toContain(
      ".queued-message-preview { min-width: 0; display: grid; justify-items: start; gap: 3px; overflow: hidden; }",
    );
    expect(styles).toContain(".queued-message p { width: 100%; min-width: 0;");
    expect(styles).toContain(
      ".queued-message-roles { max-width: 100%; display: inline-flex;",
    );
    expect(source).toContain(
      'className="queued-message-role-separator" aria-hidden="true">|</i>',
    );
    expect(source).not.toContain('messageRoleNames.join("、")');
    expect(styles).toContain(
      ".queued-message-role-names { min-width: 0; display: inline-flex;",
    );
    expect(styles).toContain(
      ".queued-message-role-separator { flex: 0 0 auto; margin: 0 4px; color: #6f9187;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .queued-message-role-separator { color: #91aaa2; }',
    );
    expect(styles).not.toContain(".queued-message-preview.has-roles");
    expect(styles).not.toContain(".queued-message-roles { position: absolute;");
    expect(source).not.toContain(
      'messageRoleNames.length > 0 ? "has-roles" : ""',
    );
  });

  it("claims each queued message before immediate or automatic dispatch", () => {
    expect(source).toContain("queuedMessageClaimsRef");
    expect(source).toContain(
      "claimQueuedMessage(queuedMessageClaimsRef.current",
    );
    expect(source).toContain(
      "releaseQueuedMessageClaim(queuedMessageClaimsRef.current",
    );
    expect(source).toContain("queuedMessagesBySessionRef.current");
    expect(source).toContain(
      "unclaimedQueuedMessages(queuedMessages, queuedMessageClaims, activeSessionId)",
    );
    expect(source).toContain("displayQueuedMessages.length > 0");
    expect(source).toContain(
      "removeQueuedMessage(current[activeSession.session_id] ?? [], message.id, queuedMessageClaimsRef.current",
    );
    expect(source).toContain(
      "disabled={claimed || sessionInteractionLocked || isCancelling}",
    );
    expect(source).toContain("aria-busy={claimed || undefined}");
    expect(source).toContain(
      'claimed ? "发送中…" : message.deliveryError ? "重试" : "立即"',
    );
    expect(styles).toContain(".queued-message.sending");
  });

  it("lets queued messages be re-edited without changing their queue position", () => {
    expect(source).toContain('className="queued-message-edit"');
    expect(source).toContain('className="queued-message-editor" autoFocus');
    expect(source).toContain(
      "message.id === edit.id ? { ...message, text, deliveryError: undefined } : message",
    );
    expect(source).toContain("queuedAutoContinueSessionIdsRef.current,");
    expect(source).toContain("processedCompletedTurnKeysRef");
    expect(source).toContain(
      'event.outcome.completion.stop_reason === "CancelledByUser"',
    );
    expect(source).toContain(
      "for (const [sessionId, completion] of Object.entries(completedTurnsBySession))",
    );
    expect(source).toContain(
      "processedCompletedTurnKeysRef.current.get(sessionId) === completion.key",
    );
    expect(source).toContain(
      "if (draftLocksChanged) setSubmittingDraftSessionIds(new Set(submittingDraftSessionIdsRef.current))",
    );

    expect(source).toContain(
      "queuedAutoContinueSessionIdsRef.current.delete(sessionId);",
    );
    expect(source).toContain(">保存</button>");
    expect(source).toContain('className="queued-message-edit-cancel"');
    expect(source).toContain(">取消</button>");
    expect(styles).toContain(
      ".queued-message.editing { grid-template-columns: 19px minmax(0, 1fr);",
    );
    expect(styles).toContain(
      ".queued-message.editing .queued-message-drag { display: none; }",
    );
    expect(styles).toContain(
      ".queued-message.editing .queued-message-preview { grid-column: 2; grid-row: 1; width: 100%;",
    );
    expect(styles).toContain(
      ".queued-message-editor { box-sizing: border-box; width: 100%;",
    );
    expect(styles).toContain(
      "min-height: 112px; max-height: min(42vh, 320px);",
    );
    expect(styles).toContain(
      ".queued-message.editing .queued-message-actions { grid-column: 2; grid-row: 2; justify-self: end;",
    );
    expect(styles).toContain(
      ".queued-message.editing .queued-message-edit-save",
    );
    expect(styles).toContain(
      "@media (max-width: 720px) {\n  .queued-message.editing",
    );
  });

  it("keeps composer typing away from expensive turn history recomputation", () => {
    expect(source).toContain("memo(function VisibleTurnList");
    expect(source).toContain("memo(function TurnInteraction");
    expect(source).toContain("const decisionsByTurn = useMemo");
    expect(source).toContain(
      "decisions={decisionsByTurn.get(sessionTurnKey(sessionId, turn.turn_id)) ?? EMPTY_DECISIONS}",
    );
    expect(source).toContain("<VisibleTurnList");
    expect(source).not.toContain("decisions={decisions.filter");
    expect(source).toContain(
      "const lifecycleEvents = useMemo(() => coalesceActionLifecycle(turn.events), [turn.events]);",
    );
  });

  it("releases a stuck send affordance only from the authoritative turn completion", () => {
    expect(source).toContain("const completedKey = event.turn_id");
    expect(source).toContain("`${event.session_id}:${event.turn_id}`");
    expect(source).toContain("clientId(`turn-finished-${event.session_id}`)");
    expect(source).toContain("setCompletedTurnsBySession((current) => ({");
    expect(source).toContain(
      'event.outcome.completion.stop_reason === "CancelledByUser"',
    );
    expect(source).not.toContain("shouldPauseQueuedMessages");
    expect(source).not.toContain("setQueuePauseRequest");
    expect(source).not.toContain(
      'if (event.turn.state !== "working") setCompletedTurn',
    );
    expect(source).toContain(
      "releaseSessionDraftSubmission(submittingDraftSessionIdsRef, sessionId)",
    );
    expect(source).toContain(
      "submittingDraftStartedAtRef.current.delete(sessionId)",
    );
    expect(source).not.toContain("completedTurnsBySession[activeSessionId]");
    expect(source).toContain(
      'applyQueuedMessagesAck(nextQueues, ack.command_id, ack.status, ack.error, clientId("queued"))',
    );
    expect(source).toContain(
      "releaseQueuedMessageClaim(queuedMessageClaimsRef.current, sessionId, commandId);",
    );
  });

  it("shows long current directories by their tail only in the composer", () => {
    expect(source).not.toContain("workspacePathLabel(session.current_dir)");
    expect(source).not.toContain('className="session-detail session-cwd"');
    expect(styles).toContain(".session-detail::before");
    expect(styles).toContain("border-bottom-left-radius: 5px");
    expect(styles).toContain(
      ".session-profile { display: inline-flex; align-items: center; gap: 6px;",
    );
    expect(source).toContain('className="composer-cwd-inline"');
    expect(source).toContain(
      '<span className="composer-cwd-inline" title={activeSession.current_dir}><b>CWD:</b><span className="path-tail">{tailPath(activeSession.current_dir, 64)}</span></span>',
    );
  });

  it("removes the access token from the visible URL while retaining the session credential", () => {
    expect(source).toContain(
      'const TOKEN_STORAGE_KEY = "timem-web-access-token";',
    );
    expect(source).toContain(
      "window.sessionStorage.setItem(TOKEN_STORAGE_KEY, query)",
    );
    expect(source).toContain("window.history.replaceState");
    expect(source).toContain('if (token) query.set("token", token);');
    expect(source).not.toContain('query.set("last_event_seq"');
    expect(source).not.toContain("saveEventCursor(window.sessionStorage");
    expect(source).not.toContain("loadEventCursor(window.sessionStorage");
    expect(source).toContain(
      "eventCursorRef.current = Number.isSafeInteger(event.event_cursor)",
    );
    expect(source).toContain(
      "new WebSocket(`${scheme}://${window.location.host}/ws${queryString}`)",
    );
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
    expect(source).toContain(
      "const nextDrafts = finishSessionDraftSubmission(submittingDraftSessionIdsRef, draftsBySession, reserved.sessionId, reserved.text, sent);",
    );
    expect(source).not.toContain('setDraft("");');
  });

  it("surfaces failed user operations instead of silently restoring local pending state", () => {
    expect(source).toContain("const pushActivity = useCallback");
    expect(source).toContain('activity.sessionId === "system"');
    expect(source).toContain(
      "appendActivityToCurrentTurn(session, { ...activity, sessionId: requestedSessionId })",
    );
    expect(source).toContain("const reportUiError = useCallback");
    expect(source).toContain(
      'pushActivity({ id: clientId(), sessionId, tone: "error", title, detail, createdAt: Date.now() });',
    );
    expect(
      [...source.matchAll(/pushActivity\(activity\);/g)].length,
    ).toBeGreaterThanOrEqual(10);
    expect(source).toContain("Load history failed");
    expect(source).toContain(
      "Reconnect to Timem Web before loading earlier history.",
    );
    expect(source).toContain("Runtime unavailable");
    expect(source).toContain(
      "Timem Web runtime is not connected. Restart timem-web and reopen the authenticated URL before sending another message.",
    );
    expect(source).toContain("Cancel failed");
    expect(source).toContain(
      "Reconnect to Timem Web before cancelling this turn.",
    );
    expect(source).toContain("Remove attachment failed");
    expect(source).toContain(
      "Reconnect to Timem Web before removing this attachment.",
    );
    expect(source).toContain("File upload failed");
    expect(source).toContain(
      "const params = new URLSearchParams({ session_id: activeSession.session_id });",
    );
    expect(source).toContain('if (token) params.set("token", token);');
    expect(source).toContain("model_endpoint_apply");
    expect(source).toContain("model_endpoint_delete");
    expect(source).toContain("Decision reply failed");
    expect(source).toContain(
      "Reconnect to Timem Web before replying to this runtime request.",
    );
    expect(source).toContain("Create session failed");
    expect(source).toContain(
      "Reconnect to Timem Web before creating a new session.",
    );
    expect(source).toContain("Mem switch failed");
    expect(source).toContain(
      "Reconnect to Timem Web before switching the mem directory.",
    );
    expect(source).toContain("ToolGen start failed");
    expect(source).toContain(
      "Reconnect to Timem Web before generating a reusable tool.",
    );
    expect(source).not.toContain("setActivities((current)");
    expect(source).toContain('event.source === "ui_activity"');
  });

  it("groups each task into user input, bounded process, and separate final delivery", () => {
    expect(source).toContain('className="turn-user-frame"');
    expect(source).toContain(
      'className={`turn-assistant-frame ${turn.state} ${workStreamVisible ? "" : "collapsed-work"}`}',
    );
    expect(source).toContain("sessionId={renderedSession.session_id}");
    expect(source).toContain(
      "function TurnInteraction({ sessionId, turn, decisions",
    );
    expect(source).toContain(
      "const hasVisibleProcess = scrollItems.some((item) => item.activity !== null) || decisions.length > 0;",
    );
    expect(source).toContain(
      "{hasVisibleProcess && <section className={`turn-assistant-frame",
    );
    expect(source).toContain(
      "<ActivityView key={item.key} activity={activity}/>",
    );
    expect(source).not.toContain("function TurnEventView(");
    expect(source).toContain(
      'className={`turn-work-scroll has-content${pendingUpdates > 0 ? " has-pending-updates" : ""}`}',
    );
    expect(source).toContain('className="turn-final-delivery"');
    expect(source).toContain(
      "const supplementItems = useMemo(() => turn.user_entries",
    );
    expect(source).toContain('entry.kind === "supplement"');
    expect(source).toContain('kind: "user_supplement" as const');
    expect(source).toContain('title: "[用户补充]"');
    expect(source).toContain(
      "const timelineItems = useMemo(() => [...lifecycleItems, ...supplementItems]",
    );
    expect(source).toContain("left.createdAt - right.createdAt");
    expect(source).toContain(
      "turn.events.length + supplementItems.length + decisions.length",
    );
    expect(source).toContain('activity.kind === "user_supplement"');
    expect(source).toContain(
      '<span className="activity-mark" aria-hidden="true">💡</span>',
    );
    expect(source).toContain(
      '<div className="user-supplement-line"><strong>{activity.title}</strong>',
    );
    expect(styles).toContain(".turn-work-item.user-supplement");
    expect(styles).toContain(".user-supplement-line strong");
    expect(source).toContain(
      "const lifecycleItems = useMemo(() => lifecycleEvents.map((event) => ({",
    );
    expect(source).toContain(
      "const processActivities = useMemo(() => timelineItems",
    );
    expect(source).toContain("id: event.event_id,");
    expect(source).toContain("createdAt: event.created_at_ms,");
    expect(source).not.toContain("scrollEventActivities");
    expect(source).not.toContain(
      "const hasOnlyFreeTalk = hasOnlyFreeTalkActivity(processActivities, decisions.length);",
    );
    expect(source).toContain(
      'const interrupted = turn.state === "interrupted"',
    );
    expect(source).toContain(
      'const [showWorkStream, setShowWorkStream] = useState(() => turn.state === "working");',
    );
    expect(source).toContain(
      'if (!wasWorking && turn.state === "working") setShowWorkStream(true);',
    );
    expect(source).toContain(
      'if (wasWorking && turn.state !== "working") setShowWorkStream(false);',
    );
    expect(source).toContain(
      'const canCollapseCompletedWork = turn.state !== "working" && (!!turn.final_answer || interrupted);',
    );
    expect(source).toContain(
      'const canToggleWorkStream = turn.state === "working" || canCollapseCompletedWork;',
    );
    expect(source).toContain(
      "const workStreamVisible = !canToggleWorkStream || showWorkStream;",
    );
    expect(source).toContain(
      "className={`working-chip work-title-chip work-collapse-toggle",
    );
    expect(source).toContain(
      'turn.state === "working" ? " active-work-title" : " completed-work-title"',
    );
    expect(source).toContain('className="work-collapse-arrow"');
    expect(source).toContain("aria-expanded={showWorkStream}");
    expect(source).toContain(
      "onClick={() => setShowWorkStream((visible) => !visible)}",
    );
    expect(source).toContain('turn.state === "interrupted"');
    expect(source).toContain(
      '<span className="work-title-status">(Interrupted)</span>',
    );
    expect(styles).toContain(".working-chip.interrupted-work-title");
    expect(source).toContain(
      '{workStreamVisible && <div className="turn-work-panel">',
    );
    expect(source).toContain(
      '{workStreamVisible && <div className="turn-work-panel">',
    );
    expect(source).toContain("<div className={`turn-work-scroll");
    expect(source).toContain(
      '{pendingUpdates > 0 && <button type="button" className="turn-new-updates"',
    );
    expect(styles).toContain(".turn-work-scroll { max-height:");
    expect(styles).not.toContain(".turn-work-scroll.empty");
    expect(styles).toContain(
      ".turn-starting-status { width: fit-content; min-height: 44px;",
    );
    expect(styles).toContain("margin: 12px 0 8px 10px;");
    expect(styles).toContain("@keyframes turn-starting-ripple");
    expect(styles).toContain(".turn-work-scroll.has-pending-updates");
    expect(styles).toContain(".work-collapse-toggle");
    expect(styles).toContain(
      '.work-collapse-toggle[aria-expanded="true"] .work-collapse-arrow { transform: rotate(90deg); }',
    );
    expect(styles).toContain(".turn-assistant-frame.collapsed-work");
    expect(styles).toContain("overflow-y: auto;");
    expect(source).toContain("followLatest.current = isNearScrollBottom({");
    expect(source).toContain("const observer = new ResizeObserver(() => {");
    expect(source).toContain(
      "if (!followLatest.current || scrollFrame !== undefined) return;",
    );
    expect(source).toContain(
      "scrollFrame = window.requestAnimationFrame(() => {",
    );
    expect(source).toContain(
      "if (scrollFrame !== undefined) window.cancelAnimationFrame(scrollFrame);",
    );
    expect(source).toContain("observer.observe(content);");
    expect(source).toContain('className="turn-new-updates"');
  });

  it("uses frame styling without repeating user or session identity labels", () => {
    expect(source).not.toContain('<div className="message-label">You</div>');
    expect(source).not.toContain('className="message-label">{assistantName}');
    expect(source).not.toContain("assistantName={activeSession?.display_name");
    expect(source).not.toContain('<span className="eyebrow">SESSION');
    expect(source).not.toContain(
      'activeSession?.display_name ?? "Starting Timem…"',
    );
    expect(source).toContain(
      "const headerModelLabel = endpointNameForProfile(server?.model_endpoints ?? [], activeSession?.runtime_profile) ?? UNCONFIGURED_MODEL_LABEL;",
    );
    expect(source).not.toContain("?.name ?? modelDisplayName(activeSession)");
    expect(source).toContain(
      'className={`header-model ${showRuntime ? "selected" : ""}`}',
    );
    expect(source).toContain(
      '<Sparkles size={10} aria-hidden="true"/><span title={headerModelLabel}>{headerModelLabel}</span><ChevronDown',
    );
    expect(source).not.toContain("<Settings size={17}/>");
    expect(styles).toContain(".chat-header { flex: none; min-width: 0;");
    expect(styles).toContain(
      ".header-model { min-width: 0; max-width: min(42vw, 260px); flex: 0 1 auto;",
    );
    expect(styles).toContain(".header-model { font-size: 14px; }");
    expect(styles).toContain("text-overflow: ellipsis; white-space: nowrap;");
    expect(styles).toContain(".header-actions { flex: none;");
  });

  it("coalesces tool lifecycles and renders tools as compact subordinate rows", () => {
    expect(source).toContain("coalesceActionLifecycle(turn.events)");
    expect(source).toContain(
      "<ToolActivityGroup key={`tool-activity-group-${item.key}`} summary={summary}/>",
    );
    expect(source).toContain("summarizeConsecutiveToolActivities(");
    expect(source).toContain(
      "className={`tool-activity-group ${summary.status}`}",
    );
    expect(source).toContain('className="tool-activity-group-counts"');
    expect(source).toContain("<strong>{count}</strong>");
    expect(source).toContain("{index > 0 && <i>|</i>}");
    expect(source).toContain("tool-activity-status");
    expect(styles).toContain(
      ".tool-activity-group { margin: 3px 0; color: #aaa; font-size: 10px; }",
    );
    expect(styles).toContain(
      ".tool-activity-group-count > span { overflow: hidden; font-weight: 350; text-overflow: ellipsis; }",
    );
    expect(styles).toContain(
      ".tool-activity-group-count > strong { color: #c7c7c7; font-weight: 750; }",
    );
    expect(styles).toContain(".tool-activity");
  });

  it("uses an explicit session-created event and session-scoped inline decisions", () => {
    expect(source).toContain('event.type === "session_created"');
    expect(source).toContain("enqueueDecision(current, pendingDecision)");
    expect(source).toContain("decision.event.session_id === activeSessionKey");
    expect(source).toContain("<InlineDecision");
    expect(source).not.toContain("<DecisionDialog");
    expect(styles).toContain(".inline-decision");
  });

  it("shows inline decision submission state instead of silently disabling controls", () => {
    expect(source).toContain(
      'const status = pending ? "Sending decision…" : locked ? "Session interaction is temporarily locked." : "";',
    );
    expect(source).toContain("aria-busy={pending}");
    expect(source).toContain(
      'const canAlwaysAllow = decision.event.topic.name === "core.user.approval.request";',
    );
    expect(source).toContain(
      'className="inline-decision-status" role="status" aria-live="polite"',
    );
    expect(source).toContain(
      "title={denyLabel} aria-label={denyLabel} disabled={disabled}",
    );
    expect(source).toContain(
      "title={allowLabel} aria-label={allowLabel} disabled={disabled}",
    );
    expect(source).toContain(
      "title={alwaysAllowLabel} aria-label={alwaysAllowLabel} disabled={disabled}",
    );
    expect(source).toContain(
      '{canAlwaysAllow && <button type="button" className="primary always-allow"',
    );
    expect(source).toContain(
      'onClick={() => onReply("always_allow")}>Always Allow</button>',
    );
    expect(styles).toContain(".inline-decision-status");
    expect(styles).toContain(
      ".inline-decision pre { max-height: min(240px, 34vh); overflow: auto;",
    );
    expect(styles).toContain(".decision-actions .primary.sending svg");
    expect(styles).toContain(
      ':root[data-theme="light"] .inline-decision-status',
    );
  });

  it("keeps blocking requests in the session flow when their reply cannot be sent", () => {
    expect(source).toContain('if (sendCommand({ type: "topic_reply"');
    expect(source).toContain("worker_id: event.worker_id ?? undefined");
    expect(source).toContain(
      "current.filter((candidate) => candidate !== decision)",
    );
    expect(source).toContain("onCreate={(command) => {");
    expect(source).toContain("if (sendCommand(command))");
  });

  it("keeps long-session switching and scrolling away from repeated full mounts", () => {
    expect(source).toContain(
      "const VisibleTurnList = memo(function VisibleTurnList",
    );
    expect(source).toContain(
      "const SessionTimelinePane = memo(function SessionTimelinePane",
    );
    expect(source).toContain("const MAX_MOUNTED_SESSION_TIMELINES = 2;");
    expect(source).toContain("reconcileSessionTimelineCache(");
    expect(source).toContain(
      "mountedTimelineSessions.map((session) => <SessionTimelinePane",
    );
    expect(source).toContain(
      "sessions.filter((session) => mountedTimelineSessionIdSet.has(session.session_id))",
    );
    expect(source).toContain("LRU order controls eviction only");
    expect(source).toContain("const renderedSessionRef = useRef(session);");
    expect(source).toContain(
      "if (active) renderedSessionRef.current = session;",
    );
    expect(source).toContain("turns={renderedSession.turns}");
    expect(source).toContain(
      "the pane catches up synchronously when it becomes active again",
    );
    expect(source).toContain("hidden={!active}");
    expect(source).toContain(
      'data-session-timeline-active={active ? "true" : "false"}',
    );
    expect(source).toContain("<VisibleTurnList");
    expect(source).not.toContain(
      "mountedTimelineSessions.map((session) => <ThreadPrimitive.Root",
    );
    expect(styles).toContain(
      ".session-timeline-pane[hidden] { display: none; }",
    );
    expect(styles).toContain(
      ".turn-interaction.completed { contain: layout style; }",
    );
    expect(styles).not.toContain(
      ".turn-interaction.completed { contain: layout style; content-visibility:",
    );
    expect(styles).not.toContain("contain-intrinsic-size: auto 320px;");
    expect(styles).toContain("scroll-behavior: auto;");
  });

  it("keeps long-session scroll frames free of full DOM geometry scans", () => {
    expect(source).toContain(
      "createFrameTask({ run: updateUserMessageNavigation })",
    );
    expect(source).toContain(
      "createFrameTask({ run: refreshUserMessageGeometry })",
    );
    expect(source).toContain(
      "createFrameTask({ run: updateUserMessageNavigationLayout })",
    );
    expect(source).toContain(
      "const mutationObserver = new MutationObserver(update);",
    );
    expect(source).toContain(
      'const resizeObserver = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(update);',
    );
    expect(source).toContain(
      "const navigationTop = viewport.scrollTop + userMessageNavigationOffset;",
    );
    expect(source).toContain(
      "const anchorOffsets = userMessageAnchorOffsetsRef.current;",
    );
    expect(source).toContain('requestTimelineNavigationWork("content"');
    expect(source).toContain('requestTimelineNavigationWork("layout"');
    expect(source).toContain('requestTimelineNavigationWork("scroll"');
    expect(source).toContain(
      '[data-session-timeline-active="true"] [data-user-message-anchor]',
    );
    expect(source).toContain(
      '[data-session-timeline-active="true"] .final-answer-outline.expanded',
    );
    expect(source).toContain("new IntersectionObserver(");
    expect(source).toContain('rootMargin: "100% 0px"');
    expect(source).toContain(
      'window.addEventListener("session-timeline-activation-change", syncActivePane);',
    );
    expect(source).toContain(
      'window.dispatchEvent(new Event("session-timeline-activation-change"));',
    );
    expect(source).toContain(
      "if (activePane) resizeObserver?.observe(activePane);",
    );
    const scrollHandlerStart = source.indexOf(
      "onScroll={(event) => {",
      source.indexOf('className="chat-scroll aui-thread-viewport"'),
    );
    const scrollHandlerEnd = source.indexOf("}}", scrollHandlerStart);
    const scrollHandler = source.slice(scrollHandlerStart, scrollHandlerEnd);
    expect(scrollHandler).toContain('requestTimelineNavigationWork("scroll"');
    expect(scrollHandler).not.toContain(
      'requestTimelineNavigationWork("layout"',
    );
    expect(scrollHandler).not.toContain(
      'requestTimelineNavigationWork("content"',
    );
    expect(scrollHandler).not.toContain("querySelectorAll");
    expect(scrollHandler).not.toContain("getBoundingClientRect");
    expect(source).toContain(
      "const outlineHeadingOffsetsRef = useRef(new Map<string, number>());",
    );
    expect(source).toContain(
      "const threshold = viewport.scrollTop + FINAL_ANSWER_OUTLINE_SCROLL_OFFSET;",
    );
    expect(source).toContain('pane?.dataset.sessionTimelineActive !== "true"');
    const outlineScrollStart = source.indexOf(
      "const updateScrollState = () => {",
      source.indexOf("function FinalAnswerContent"),
    );
    const outlineScrollEnd = source.indexOf("    };", outlineScrollStart);
    const outlineScrollHandler = source.slice(
      outlineScrollStart,
      outlineScrollEnd,
    );
    expect(outlineScrollHandler).toContain(
      "outlineActiveTaskRef.current?.request();",
    );
    expect(outlineScrollHandler).not.toContain("outlinePositionTaskRef");
    expect(outlineScrollHandler).not.toContain("getBoundingClientRect");
    expect(outlineScrollHandler).not.toContain("querySelectorAll");
  });

  it("keeps compact mobile controls and long content inside the viewport", () => {
    expect(styles).toContain(
      ".chat-scroll { position: relative; overflow-x: hidden; }",
    );
    expect(styles).toContain(".turn-work-scroll { overflow-x: hidden; }");
    expect(styles).toContain("overscroll-behavior-y: auto;");
    expect(styles).toContain(
      ".markdown-body .table-scroll { max-width: 100%; overflow-x: auto;",
    );
    expect(styles).toContain(
      ".header-context-main > span:first-child { display: none; }",
    );
    expect(styles).toContain(
      ".composer-buttons { flex: none; width: auto; flex-wrap: nowrap; }",
    );
    expect(styles).toContain(".stop-button { width: 34px; height: 32px;");
    expect(styles).toContain(
      ".session-name { font-size: 12px; font-weight: 400; line-height: 1.3; }",
    );
    expect(styles).toContain(".session-profile { font-size: 11px; }");
  });

  it("uses the shared worker-aware decision key for inline request pending state", () => {
    expect(source).toContain(
      "decisionKey, decisionsFromSessions, draftForSession",
    );
    expect(source).toContain("pendingDecisionKeys.has(decisionKey(decision))");
    expect(source).not.toContain("function decisionKey(decision: Decision)");
    expect(viewModelSource).toContain('decision.event.context_id ?? ""');
    expect(viewModelSource).toContain('decision.event.worker_id ?? ""');
  });

  it("backs off and reconnects the WebSocket instead of only changing the label", () => {
    expect(source).toContain("const connect = () =>");
    expect(source).toContain(
      "Math.min(10_000, 500 * 2 ** Math.min(nextAttempt - 1, 5))",
    );
    expect(source).toContain("window.setTimeout(connect, delay)");
    expect(source).toContain("window.clearTimeout(retryTimer)");
    expect(source).toContain("let hasConnectedOnce = false;");
    expect(source).toContain("let disconnectNoticeShown = false;");
    expect(source).toContain("hasConnectedOnce = true;");
    expect(source).toContain("Runtime disconnected");
    expect(source).toContain(
      "Timem Web lost its runtime connection. If timem-web has exited, restart it and reopen the authenticated URL.",
    );
  });

  it("manages session-scoped MCP servers with accessible and responsive controls", () => {
    expect(source).toContain(
      'const mcpLabel = `Manage MCP servers · ${connectedMcpCount} connected${failedMcpCount ? ` · ${failedMcpCount} failed` : ""}`;',
    );
    expect(source).toContain('aria-label="MCP servers" tabIndex={-1}');
    expect(source).toContain(
      '<h2><strong className="mcp-session-name">{session?.display_name ?? "Current session"}</strong> \'s Capabilities</h2>',
    );
    expect(source).not.toContain("Capabilities of current session");
    expect(source).not.toContain("are injected into its model and executor");
    expect(source).toContain(
      "mcpButtonRef.current?.contains(target) || mcpPanelRef.current?.contains(target)",
    );
    expect(source).toContain('if (event.key === "Escape") closeMcpPanel();');
    expect(source).toContain('type: "mcp_session_toggle"');
    expect(source).toContain('type: "mcp_server_reconnect"');
    expect(source).toContain('type: "mcp_server_upsert"');
    expect(source).toContain("window.confirm(`Delete MCP server");
    expect(source).toContain(
      "const [deleteMode, setDeleteMode] = useState(false);",
    );
    expect(source).toContain(
      'const [selectedDeleteServerId, setSelectedDeleteServerId] = useState("");',
    );
    expect(source).toContain(
      'className={`mcp-delete-manage ${deleteMode ? "confirm" : ""}`}',
    );
    expect(source).toContain(
      'className={`mcp-server ${connectionState} ${active && !deleteMode ? "selected" : ""} ${deleteMode ? "delete-selecting" : ""}',
    );
    expect(source).toContain(
      'className={`mcp-delete-select ${selectedDeleteServerId === server.config.id ? "selected" : ""}`}',
    );
    expect(source).toContain(
      'type: "mcp_server_delete", server_id: server.config.id',
    );
    expect(source).toContain("disabled={!session || pending || deleteMode}");
    expect(source).toContain(
      '{!deleteMode && <button type="button" className="mcp-add"',
    );
    expect(source).not.toContain('className="danger" title="Delete server"');
    expect(source).toContain('(["stdio", "streamable_http", "sse"] as const)');
    expect(source).toContain(
      "const [transportDrafts, setTransportDrafts] = useState(() => createMcpTransportDrafts(config.transport));",
    );
    expect(source).toContain("onClick={() => setTransportType(type)}");
    expect(source).toContain(
      "One MCP endpoint may return JSON or an SSE stream.",
    );
    expect(source).toContain('role="switch" aria-checked={active}');
    expect(source).toContain(
      'const connectionState = !active ? "disabled" : server.state === "connected" ? "connected" : server.state === "error" || !!server.error ? "failed" : "connecting";',
    );
    expect(source).toContain(
      "className={`mcp-session-toggle ${connectionState}`}",
    );
    expect(source).toContain(
      'connectionState === "connected" ? `${server.tools.length} tools` : connectionState === "failed" ? "⚠️无法连接"',
    );
    expect(source).toContain(
      "{active && <div className={`mcp-server-meta ${connectionState}`}",
    );
    expect(source).not.toContain("<small>{mcpEndpoint(server.config)}</small>");
    expect(source).not.toContain("function mcpEndpoint(");
    expect(source).toContain(
      "className={`mcp-server-meta ${connectionState}`}",
    );
    expect(source).not.toContain("Enabled, connection failed");
    expect(source).not.toContain(
      "<span>{mcpTransportLabel(server.config.transport)}</span>",
    );
    expect(source).not.toContain('className="mcp-error"');
    expect(source).toContain('className="mcp-session-toggle-thumb"');
    expect(source).not.toContain('className="mcp-port-glyph"');
    expect(source).not.toContain('className="mcp-toggle-label"');
    expect(source).toContain(
      "const pendingMcpKeysRef = useRef<Set<string>>(new Set());",
    );
    expect(source).toContain(
      "!addPendingKey(pendingMcpKeysRef, setPendingMcpKeys, key)",
    );
    expect(source).toContain(
      "removePendingKey(pendingMcpKeysRef, setPendingMcpKeys, key)",
    );
    expect(source).toContain("pendingMcpKeysRef.current.clear();");
    expect(source).toContain('StructuredListEditor label="Arguments"');
    expect(source).toContain('StructuredKeyValueEditor label="Environment"');
    expect(source).toContain('StructuredKeyValueEditor label="Headers"');
    expect(source).not.toContain("Arguments<textarea");
    expect(source).not.toContain('aria-label="Headers" rows={3}');
    expect(styles).toContain(
      ".structured-field-row { min-width: 0; display: grid; grid-template-columns: minmax(0, .8fr) minmax(0, 1.4fr) 28px;",
    );
    expect(styles).toContain("@media (max-width: 520px)");
    expect(protocolSource).toContain('type: "mcp_updated"');
    expect(protocolSource).toContain(
      '| { type: "sse"; url: string; headers: Record<string, string> };',
    );
    expect(protocolSource).toContain("mcp_server_ids: string[]");
    expect(styles).toContain(':root[data-theme="light"] .mcp-panel');
    expect(styles).toContain(
      ".mcp-panel { position: fixed; inset: 58px 8px 8px;",
    );
    expect(styles).toContain(".mcp-button > svg { transform: rotate(90deg); }");
    expect(styles).toContain(
      ".header-context-actions { min-width: 0; display: flex; align-items: center; gap: 5px; grid-column: 2; grid-row: 1; justify-self: start; }",
    );
    expect(styles).toContain(".mcp-count { position: absolute; z-index: 2;");
    expect(source).toContain(
      'const connectedMcpCount = activeMcpServers.filter((item) => item.state === "connected").length;',
    );
    expect(source).toContain(
      'const failedMcpCount = activeMcpServers.filter((item) => item.state !== "connected" && (item.state === "error" || !!item.error)).length;',
    );
    expect(source).toContain('connectedMcpCount > 0 ? "enabled" : ""');
    expect(source).toContain('className="mcp-count mcp-count-connected"');
    expect(source).toContain('className="mcp-failure-indicator"');
    expect(source).toContain("<TriangleAlert size={9}/>");
    expect(source).not.toContain('className="mcp-count mcp-count-failed"');
    expect(source.indexOf("ref={mcpButtonRef}")).toBeLessThan(
      source.indexOf('<div className="header-actions">'),
    );
    expect(source).not.toContain('className="mcp-enabled-dot"');
    expect(styles).toContain(
      ".mcp-button.enabled { border-color: transparent; background: #174b78; color: #e8f5ff;",
    );
    expect(styles).toContain(
      ".mcp-button.enabled.selected { border-color: transparent; background: #23689f;",
    );
    expect(styles).toContain(
      ".mcp-count-connected { top: -6px; right: -7px; background: #3487e8; }",
    );
    expect(styles).toContain(
      ".mcp-failure-indicator { position: absolute; z-index: 3; right: 2px; bottom: 2px; width: 10px; height: 10px;",
    );
    expect(styles).toContain(
      ".mcp-failure-indicator svg { width: 9px; height: 9px; stroke-width: 3; }",
    );
    expect(styles).toContain(
      ".mcp-panel { position: absolute; z-index: 10; top: 62px; left: 24px;",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .mcp-button.enabled { border-color: transparent; background: #dceffc; color: #165b8d;',
    );
    expect(styles).toContain(
      ".mcp-button.enabled { border-color: transparent; background: #174b78;",
    );
    expect(styles).toContain(
      ".mcp-button.enabled:hover { border-color: transparent; background: #1c5a8d;",
    );
    expect(styles).toContain(
      ".mcp-button.enabled.selected { border-color: transparent; background: #23689f;",
    );
    expect(styles).toContain(
      ".mcp-panel-header-actions { display: flex; align-items: center; gap: 5px; }",
    );
    expect(styles).toContain(
      ".mcp-panel-header-actions .mcp-delete-manage.confirm",
    );
    expect(styles).toContain(
      ".mcp-panel-header-actions .mcp-delete-manage.confirm { border-color: #c94f49; background: #c94f49; color: #fff;",
    );
    expect(styles).toContain(
      ".mcp-panel-header-actions .mcp-delete-cancel { border-color: #d5dadd; background: #f7f8f8; color: #111820;",
    );
    expect(styles).toContain(
      ".mcp-panel-header-actions > .icon-button { width: 30px; height: 30px; margin-left: 5px; }",
    );
    expect(styles).toContain(".mcp-server.delete-selected");
    expect(styles).toContain(".mcp-delete-select.selected");
    expect(styles).toContain(
      ':root[data-theme="light"] .mcp-server.delete-selected',
    );
    expect(styles).toContain(
      ".mcp-session-toggle { position: relative; width: 32px; height: 18px;",
    );
    expect(styles).toContain(
      '.mcp-session-toggle[aria-checked="true"] { border-color: #2563eb; background: #3b82f6; }',
    );
    expect(styles).toContain(
      ".mcp-session-toggle-thumb { position: absolute; top: 2px; left: 2px; width: 12px; height: 12px;",
    );
    expect(styles).toContain(
      '.mcp-session-toggle[aria-checked="true"] .mcp-session-toggle-thumb { transform: translateX(14px); background: #fff; }',
    );
    expect(styles).toContain(
      '.mcp-session-toggle.failed[aria-checked="true"] { border-color: #d2a23b; background: #a97818; box-shadow: 0 0 0 2px #d8a52d24; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .mcp-session-toggle { border-color: #aebcc2; background: #cad2d5; }',
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .mcp-session-toggle.failed[aria-checked="true"] { border-color: #c99420; background: #e9b63f; box-shadow: 0 0 0 2px #d6a32324; }',
    );
    expect(source).toContain(
      '<div className="mcp-editor-actions"><button type="button" className="secondary"',
    );
    expect(source).toContain(
      'className={`primary ${pending ? "pending" : ""}`} disabled={!valid || pending}',
    );
    expect(styles).toContain(
      ".mcp-editor-actions .primary.pending svg { animation: spin 1.2s linear infinite; }",
    );
    expect(source).toContain("Save and connect</button>");
    expect(styles).toContain(
      ".mcp-editor-actions button { min-height: 36px; display: inline-flex; align-items: center; justify-content: center; gap: 7px; border: 0; border-radius: 8px;",
    );
    expect(styles).toContain(
      ".mcp-editor-actions .secondary { background: #151c1a; color: #9caaa5; box-shadow: 0 5px 14px #0003; }",
    );
    expect(styles).toContain(
      ".mcp-editor-actions .primary { background: #245e50; color: #effaf6; box-shadow: 0 7px 18px #0005; }",
    );
    expect(styles).toContain(
      ':root[data-theme="light"] .mcp-editor-actions .primary { background: #2d7060; color: #fff; box-shadow: 0 7px 18px #29443b35; }',
    );
    expect(styles).toContain(
      "grid-template-columns: repeat(3, minmax(0, 1fr))",
    );
  });

  it("keeps only global runtime availability in a banner and puts other errors in the task work stream", () => {
    expect(source).toContain(
      'className="runtime-disconnect-banner" role="alert"',
    );
    expect(source).toContain(
      'const runtimeDisconnectedTitle = runtimeUnavailable ? "Runtime unavailable" : "Connection lost";',
    );
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

describe("session activity navigation polish", () => {
  it("renders workers as a parent-linked tree without idle status dots", () => {
    expect(source).toContain("sessionWorkerTreeRows(session.workers)");
    expect(source).toContain('role="tree"');
    expect(source).toContain('role="treeitem" aria-level={depth + 1}');
    expect(source).toContain('className="worker-relation"');
    expect(source).toContain('className="worker-working-icon"');
    expect(source).toContain('className="worker-idle-spacer"');
    expect(source).not.toContain(
      "className={`worker-state-dot ${worker.state}`}",
    );
    expect(styles).toContain(".worker-list::after");
    expect(styles).toContain(
      "grid-template-columns: 16px 14px minmax(0, 1fr);",
    );
    expect(source).not.toContain(
      "<span className={`worker-state ${worker.state}`}>{worker.state}</span>",
    );
    expect(styles).toContain("margin-left: 0;");
    expect(source).toContain(
      'style={{ "--worker-depth": depth } as CSSProperties}',
    );
    expect(styles).toContain(".worker-working-icon {");
  });

  it("marks an unseen background Session completion until the Session is opened", () => {
    expect(source).toContain("previousSessionStatesRef");
    expect(source).toContain(
      'previous.get(session.session_id) === "working" && session.state !== "working"',
    );
    expect(source).toContain(
      "session.session_id !== activeSessionIdRef.current",
    );
    expect(source).toContain("setUnreadCompletedSessionIds");
    expect(source).toContain(
      'className="session-unread-dot" aria-label="Session has new completed work"',
    );
    expect(source).toContain("previousSessionStatesRef.current = null;");
    expect(styles).toContain(".session-unread-dot {");
    expect(styles).toContain(
      ".session-row.has-unread-completion:not(.active) .session-name",
    );
  });

  it("keeps a compact thread-edge control visible and smoothly transitions working to idle", () => {
    expect(source).toContain(
      "const [threadAwayFromBottom, setThreadAwayFromBottom] = useState(false);",
    );
    expect(source).toContain("setThreadAwayFromBottom(!nearBottom);");
    expect(source).not.toContain("workingDots");
    expect(source).toContain('activeSession && <button type="button"');
    expect(source).not.toContain(
      'activeSession?.state === "working" && <button',
    );
    expect(source).toContain(
      'activeSession.state === "working" ? "is-working" : "is-idle"',
    );
    expect(source).toContain(
      'threadAwayFromBottom ? " away-from-bottom" : " at-live-edge"',
    );
    expect(source).toContain(
      'threadAwayFromBottom ? "跳转到聊天最下方" : "当前已是聊天最下方"',
    );
    expect(source).toContain(
      '<span className="thread-edge-symbol" aria-hidden="true">{activeSession.state === "working" ? <span className="thread-working-orbit"><span className="thread-working-core"/></span> : <ArrowDownToLine className="thread-idle-bottom-icon" size={17} strokeWidth={2.35}/>',
    );
    expect(source).toContain(
      'activeSession.state === "working" ? "Working" : "Jump to bottom"',
    );
    expect(source).toContain("onClick={navigateWorkingToThreadBottom}");
    expect(source).toContain(
      "const navigateWorkingToThreadBottom = useCallback(() => {",
    );
    expect(source).toContain("const durationMs = 90;");
    expect(source.indexOf("className={`thread-working-away")).toBeGreaterThan(
      source.indexOf('<nav className="user-message-navigation"'),
    );
    expect(styles).toContain(".thread-working-away {");
    expect(styles).toContain("width: 30px !important;");
    expect(styles).toContain("height: 30px !important;");
    expect(styles).toContain("border-radius: 50% !important;");
    expect(styles).toContain("display: grid;");
    expect(styles).toContain("place-items: center;");
    expect(styles).toContain(
      "radial-gradient(circle at center, #33383cdd, #22272bdd 72%)",
    );
    expect(styles).toContain(".thread-edge-symbol {");
    expect(styles).toContain("position: absolute;");
    expect(styles).toContain("inset: 0;");
    expect(styles).not.toContain(
      ".thread-working-away > * { position: relative;",
    );
    expect(styles).toContain(".thread-idle-bottom-icon {");
    expect(styles).toContain("overflow: visible;");
    expect(styles).toContain(".thread-working-away::before {");
    expect(styles).toContain("transition: opacity .5s ease;");
    expect(styles).toContain(
      ".thread-working-away.is-working::before { opacity: 1; }",
    );
    expect(styles).toContain(
      "animation: thread-working-breathe 1.65s ease-in-out infinite;",
    );
    expect(styles).toContain("@keyframes thread-working-breathe");
    expect(styles).toContain(
      ".thread-working-away.is-working .thread-working-orbit::before {",
    );
    expect(styles).toContain(
      "animation: thread-working-orbit 0.95s linear infinite;",
    );
    expect(styles).toContain(
      ".thread-working-away.is-working .thread-working-core {",
    );
    expect(styles).toContain("contain: paint;");
    expect(styles).toContain(
      ':root[data-theme="light"] .thread-working-away::before',
    );
  });
});

describe("friendly favorites capacity handling", () => {
  it("offers persisted capacity tiers with plain-language near-full and full states", () => {
    expect(protocolSource).toContain('type: "favorite_capacity_update"');
    expect(protocolSource).toContain('type: "favorite_capacity_reached"');
    expect(source).toContain('"收藏夹空间快满了"');
    expect(source).toContain('"收藏夹已满"');
    expect(source).toContain("这条回复已收藏");
    expect(source).toContain("这条回复还没有收藏");
    expect(source).toContain('{ label: "256 MB", bytes: 256 * 1024 * 1024 }');
    expect(source).toContain('{ label: "1 GB", bytes: 1024 * 1024 * 1024 }');
    expect(source).toContain('{ label: "不限", bytes: null }');
    expect(source).toContain('if (bytes <= 0) return "0 MB";');
    expect(styles).toContain(".favorite-capacity-dialog");
    expect(styles).toContain(
      ':root[data-theme="light"] .favorite-capacity-dialog',
    );
    expect(styles).toContain(".favorite-capacity-options");
  });
});

describe("chat library modal and favorite management", () => {
  it("uses a settings-style full-screen modal instead of the right rail", () => {
    expect(source).toContain('className="chat-library-center-backdrop"');
    expect(source).toContain('id="chat-library-center"');
    expect(source).toContain('role="dialog" aria-modal="true"');
    expect(source).not.toContain(
      'className="chat-library-panel session-side-panel"',
    );
    expect(source).not.toContain("chat-library-open");
    expect(styles).toContain(
      ".chat-library-center-backdrop { position: fixed; z-index: 45; inset: 0;",
    );
    expect(styles).toContain(
      ".chat-library-center { width: min(960px, 100%); height: min(720px, calc(100vh - 48px));",
    );
    expect(styles).toContain(".chat-library-header { min-height: 78px;");
    expect(styles).toContain(".chat-library-search > label { height: 42px;");
    expect(styles).toContain(".chat-library-summary { min-height: 34px;");
    expect(source).not.toContain('className="chat-library-tabs"');
    expect(source).toContain('<select aria-label="Search scope" value={scope}');
    expect(source).toContain('<option value="all">All Sessions</option>');
    expect(source).toContain(
      '<option value="session" disabled={!activeSession}>Current Session</option>',
    );
    expect(source).toContain('<option value="favorites">Favorites</option>');
    expect(source).toContain('const showingFavorites = scope === "favorites";');
    expect(source).toContain(
      "const filteredFavoriteItems = normalizedFavoriteQuery",
    );
    expect(styles).not.toContain(".chat-library-tabs");
    expect(styles).toContain(".chat-library-scope select { height: 36px;");
    expect(styles).not.toContain(".app-shell.chat-library-open");
  });

  it("renders Search as read-only full-width rows with no favorite control", () => {
    expect(source).toContain('className="chat-library-list search-results"');
    expect(source).toContain('className="chat-library-search-row"');
    expect(source).toContain('className="chat-library-search-main"');
    expect(source).not.toContain("className={`chat-library-star");
    expect(styles).toContain(
      ".chat-library-list.search-results { display: flex; flex-direction: column; gap: 7px; overflow-y: auto; }",
    );
    expect(styles).toContain(
      ".chat-library-list.search-results > * { flex: 0 0 auto; }",
    );
    expect(styles).toContain(
      ".chat-library-search-main p { display: -webkit-box;",
    );
    expect(styles).toContain("-webkit-line-clamp: 3;");
  });

  it("renders Favorite as sortable full-width rows with shared confirmed batch removal", () => {
    expect(source).toContain("const [favoriteSort, setFavoriteSort]");
    expect(source).toContain('"size-desc"');
    expect(source).toContain(
      "bytes: utf8ByteLength(favorite.content_snapshot)",
    );
    expect(source).toContain("className={`chat-library-list favorites");
    expect(source).toContain('className="chat-library-favorite-main"');
    expect(source).toContain(
      "window.confirm(`Remove ${selected.length} selected favorite",
    );
    expect(source).toContain("for (const item of selected)");
    expect(source).toContain(
      "onToggleFavorite(item.sessionId, item.turnId, item.favoriteId, item.sourceKey)",
    );
    expect(source).not.toContain("selected.every((item) => onToggleFavorite");
    expect(source).toContain(
      "const sourceKey = sourceKeyOverride ?? `legacy:${sessionId}:${turnId}:assistant:0`;",
    );
    expect(source).toContain("Delete {selectedFavoriteIds.size}");
    expect(source).toContain(">Cancel</button>");
    expect(styles).toContain(
      ".chat-library-list.favorites { grid-template-columns: minmax(0, 1fr);",
    );
    expect(styles).toContain(".chat-library-favorite-row.selected");
    expect(styles).toContain(
      ".chat-library-favorite-copy p { display: -webkit-box;",
    );
    expect(source).toContain("const CHAT_LIBRARY_INITIAL_ROWS = 40;");
    expect(source).toContain("visibleSearchItems.map");
    expect(source).toContain("visibleFavoriteItems.map");
    expect(source).toContain("const ChatLibrarySearchRow = memo");
    expect(source).toContain("const ChatLibraryFavoriteRow = memo");
    expect(styles).toContain(
      ".chat-library-center-backdrop {\n  backdrop-filter: blur(9px) saturate(.78);\n  -webkit-backdrop-filter: blur(9px) saturate(.78);",
    );
    expect(styles).toContain("content-visibility: auto;");
  });
});
