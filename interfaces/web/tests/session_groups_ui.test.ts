import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("../src/main.tsx", import.meta.url),
  "utf8",
);
const styles = readFileSync(
  new URL("../src/styles.css", import.meta.url),
  "utf8",
);

describe("sortable Session group UI", () => {
  it("offers creation from every group heading without a Session count", () => {
    expect(source).toContain('className="session-group-new"');
    expect(source).toContain("groupId: bucket?.id ?? null");
    expect(source).toContain('groupName: bucket?.name ?? "Unsorted"');
    expect(source).toContain("groupId={newSessionTarget.groupId}");
    expect(source).toContain("groupName={newSessionTarget.groupName}");
    expect(source).not.toContain("<small>{bucketSessions.length}</small>");
    expect(styles).toContain(".session-group-new {");
    expect(source).toContain("bucketSessions.length > 0");
    expect(source).toContain(
      "Delete every session in this group before deleting the group",
    );
    expect(source).toMatch(
      /\{bucket &&\s*sessionGroupEditor\?\.id !== bucket\.id && \(/,
    );
    expect(source).toMatch(
      /\.\.\.sessionGroups\.map[\s\S]*__ungrouped[\s\S]*group: undefined/,
    );
  });

  it("shows a collapsed Group dot only while at least one child Session has unread completed work", () => {
    expect(source).toMatch(
      /const hasUnreadCompletion = bucketSessions\.some\(\s*\(session\) =>/,
    );
    expect(source).toContain(
      "unreadCompletedSessionIds.has(session.session_id)",
    );
    expect(source).toContain(
      'className={`session-group ${collapsed ? "collapsed" : ""} ${hasUnreadCompletion ? "has-unread-completion" : ""}`}',
    );
    expect(source).toContain('className="session-group-chevron"');
    expect(source).toContain("{collapsed && hasUnreadCompletion && (");
    expect(source).toContain('className="session-group-unread-dot"');
    expect(source).toContain("aria-expanded={!collapsed}");
    expect(source).toContain("contains unread completed work");
    expect(source).not.toContain("collapsed && bucketSessions.length > 0");
    expect(source).not.toContain("session-group-content-indicator");
    expect(styles).toMatch(
      /\.session-group-heading \{[^}]*margin: 3px 2px 1px;[^}]*padding: 3px;/,
    );
    expect(styles).toContain(
      ".session-group:first-child .session-group-heading { margin-top: 2px; }",
    );
    expect(styles).toContain(".session-group-unread-dot {");
    expect(styles).not.toContain(".session-group-content-indicator");
    expect(styles).toMatch(
      /\.session-group-list \{[^}]*min-height: 8px;[^}]*padding: 0 0 2px;/,
    );
  });

  it("keeps independent unread Session ids so viewing one does not clear another", () => {
    expect(source).toContain(
      "for (const sessionId of completedAway) updated.add(sessionId);",
    );
    expect(source).toContain(
      "if (!current.has(activeSessionId)) return current;",
    );
    expect(source).toContain("next.delete(activeSessionId);");
    expect(source).not.toMatch(
      /setUnreadCompletedSessionIds\(new Set\(\)\);\s*}\s*, \[activeSessionId\]/,
    );
  });

  it("renames group and Session names from hover edit buttons into inline inputs", () => {
    expect(source).toContain('className="session-group-actions"');
    expect(source).toContain('className="session-group-name-editor"');
    expect(source).toContain('className="session-rename-button"');
    expect(source).toMatch(/onClick=\{\(\) =>\s*beginRename\(session\)\s*\}/);
    expect(source).toContain("onBlur={(event) => {");
    expect(source).toMatch(
      /event\.currentTarget\.dataset\s*\.cancelled === "true"/,
    );
    expect(source).toMatch(/finishRename\(\s*session\.session_id,?\s*\);/);
    expect(source).not.toContain('className="session-group-editor inline"');
    expect(styles).toContain(
      ".session-group-heading:hover .session-group-actions",
    );
    expect(styles).toContain(".session-row:hover .session-rename-button");
    expect(styles).toContain(".session-row.has-workers .session-rename-button");
  });

  it("keeps Search, Favorite, and Settings visible beside a long Session list", () => {
    expect(source).toContain('className="session-list"');
    expect(source).toContain('className="sidebar-footer"');
    expect(source).toContain('title="Search chats"');
    expect(source).toContain('title="Favorite answers"');
    expect(source).toContain("title={settingsTitle}");
    expect(styles).toMatch(
      /\.sidebar \{[^}]*min-height: 0;[^}]*overflow: hidden;/,
    );
    expect(styles).toMatch(
      /\.session-list \{[^}]*flex: 1 1 auto;[^}]*min-height: 0;[^}]*overscroll-behavior: contain;/,
    );
    expect(styles).toMatch(/\.sidebar-footer \{[^}]*flex: none;/);
    expect(styles).toMatch(
      /\.session-row \{[^}]*min-height: 26px;[^}]*border-radius: 4px;/,
    );
    expect(styles).toMatch(
      /\.session-row \.session \{[^}]*min-height: 26px;[^}]*padding: 0 2px 0 34px;/,
    );
    expect(styles).toContain(".session-name { line-height: 1.15; }");
  });

  it("sorts only on drag end, keeps Unsorted fixed, and never sends cross-group Session moves", () => {
    expect(source).toContain(
      "activationConstraint: { delay: 200, tolerance: 5 }",
    );
    expect(source).toContain("onDragEnd={handleSessionNavigationDragEnd}");
    const navigationHandler = source.slice(
      source.indexOf("const handleSessionNavigationDragEnd"),
      source.indexOf(
        "useEffect(() =>",
        source.indexOf("const handleSessionNavigationDragEnd"),
      ),
    );
    expect(navigationHandler).toContain("(event: DragEndEvent)");
    expect(navigationHandler).not.toContain("DragOverEvent");
    expect(source).toContain('type: "session_groups_reorder"');
    expect(source).toContain('type: "session_reorder"');
    expect(source).toContain(
      "if (activeGroupId !== (overSession.group_id ?? null)) return;",
    );
    expect(source).toContain(
      'if (activeGroupId === "__ungrouped" || overGroupId === "__ungrouped")',
    );
    expect(source).toContain("setSessionGroups(reorderedGroups)");
    expect(source).toContain("setSessions((current) =>");
    expect(source).not.toContain('type: "session_group_move"');
    expect(styles).toContain(
      "/* Long-press sorting for Session navigation. */",
    );
    expect(styles).toContain(".session-row.dragging");
  });

  it("uses a compact destructive check inside the 26px Session row", () => {
    expect(styles).toMatch(
      /\.session-delete-select \{[^}]*width: 18px;[^}]*height: 18px;/,
    );
    expect(styles).toContain(
      ".session-delete-select svg { width: 11px; height: 11px;",
    );
  });
});
