import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

describe("fixed Session group UI", () => {
  it("offers creation from every group heading without a Session count", () => {
    expect(source).toContain('className="session-group-new"');
    expect(source).toContain('groupId: bucket?.id ?? null');
    expect(source).toContain('groupName: bucket?.name ?? "Unsorted"');
    expect(source).toContain('groupId={newSessionTarget.groupId}');
    expect(source).toContain('groupName={newSessionTarget.groupName}');
    expect(source).not.toContain("<small>{bucketSessions.length}</small>");
    expect(styles).toContain(".session-group-new {");
    expect(source).toContain("bucketSessions.length > 0");
    expect(source).toContain(
      "Delete every session in this group before deleting the group",
    );
    expect(source).toContain("{bucket && sessionGroupEditor?.id !== bucket.id && (");
    expect(source).toMatch(
      /\.\.\.sessionGroups\.map[\s\S]*__ungrouped[\s\S]*group: undefined/,
    );
  });

  it("renames group and Session names from hover edit buttons into inline inputs", () => {
    expect(source).toContain('className="session-group-actions"');
    expect(source).toContain('className="session-group-name-editor"');
    expect(source).toContain('className="session-rename-button"');
    expect(source).toContain("onClick={() => beginRename(session)}");
    expect(source).toContain("onBlur={(event) => {");
    expect(source).toContain('event.currentTarget.dataset.cancelled === "true"');
    expect(source).toContain("finishRename(session.session_id);");
    expect(source).not.toContain('className="session-group-editor inline"');
    expect(styles).toContain(".session-group-heading:hover .session-group-actions");
    expect(styles).toContain(".session-row:hover .session-rename-button");
    expect(styles).toContain(".session-row.has-workers .session-rename-button");
  });

  it("has no Session move or drag affordance", () => {
    expect(source).not.toContain('type: "session_group_move"');
    expect(source).not.toContain('className="session-legacy-group-assign"');
    expect(source).not.toContain('className="session-drag"');
    expect(source).not.toContain("sessionDragSensors");
    expect(source).not.toContain("finishSessionDrag");
    expect(source).not.toContain("moveSessionGroup");
    expect(source).not.toContain('type: "session_groups_reorder"');
    expect(source).not.toContain('title="Move group up"');
    expect(source).not.toContain('title="Move group down"');
    expect(styles).not.toContain(".session-drag");
    expect(styles).not.toContain(".session-group-drop-hint");
    expect(styles).not.toContain(".session-overlay");
  });
});
