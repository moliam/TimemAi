import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const mainSource = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const stylesSource = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const monitorSource = readFileSync(
  new URL(
    "../../../applications/timem/src/server/session_work_file_monitor.rs",
    import.meta.url,
  ),
  "utf8",
);
const serverSource = readFileSync(
  new URL("../../../applications/timem/src/server.rs", import.meta.url),
  "utf8",
);

describe("work file update notice", () => {
  it("renders on the system-notice work-stream lane, not as a user bubble", () => {
    // The notice is an activity on the turn event chain, exactly like memo and
    // context-compression notices.
    expect(mainSource).toContain('if (activity.kind === "work_file_notice")');
    expect(mainSource).toContain("<WorkFileNotice");
    expect(mainSource).toContain("NotebookPen");
    expect(mainSource).toContain(
      "turn-work-item notice system-notice work-file-notice",
    );
    expect(mainSource).toContain('t("turn.workNotesUpdated")');
    // No user-entry / user-bubble rendering path for the notice may remain.
    expect(mainSource).not.toContain('entry.kind === "work_file_update"');
    expect(mainSource).not.toContain('work-file-entry-badge');
    expect(stylesSource).toContain(".work-file-notice");
    expect(stylesSource).not.toContain(".work-file-entry-badge");
    expect(stylesSource).not.toContain(".turn-user-entry.work_file_update");
  });

  it("carries matching zh/en copy", () => {
    expect(zh.turn.workNotesUpdated).toBe("更新当前对话工作笔记");
    expect(en.turn.workNotesUpdated).toBe(
      "Updated the current conversation's work notes",
    );
  });

  it("publishes the notice as a UI-only turn event and persists it as a runtime notice", () => {
    expect(monitorSource).toContain(
      'WORK_FILE_NOTICE_ACTIVITY_KIND: &str = "work_file_notice"',
    );
    expect(monitorSource).toContain('WORK_FILE_NOTICE_SOURCE: &str = "ui_activity"');
    expect(monitorSource).toContain("ChatHistoryEventKind::RuntimeNotice");
    expect(monitorSource).toContain("WireEvent::TurnUpdated");
    expect(monitorSource).toContain("POLL_INTERVAL");
    expect(monitorSource).toContain("NOTICE_COOLDOWN");
    expect(monitorSource).toContain("session_work_file_path");
    // The notice is a timeline annotation, never a Core supplement.
    expect(monitorSource).not.toContain("try_add_system_supplement");
    expect(monitorSource).not.toContain("submit_turn");
  });

  it("migrates legacy user-entry records during restore", () => {
    expect(serverSource).toContain("spawn_session_work_file_monitor");
    expect(serverSource).toContain("LEGACY_WORK_FILE_UPDATE_ENTRY_KIND");
    // The legacy kind is no longer accepted as a user-entry kind.
    expect(serverSource).not.toContain('| "work_file_update"');
  });
});
