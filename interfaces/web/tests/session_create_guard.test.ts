import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import {
  SESSION_CREATE_CONFIRM_TIMEOUT_MS,
  sessionCreateFailure,
  sessionCreateUnconfirmed,
} from "../src/session_create_guard";
import { setLocale } from "../src/i18n";
import { zh } from "../src/i18n/strings.zh";
import { en } from "../src/i18n/strings.en";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");

describe("session create failure mapping", () => {
  it("explains a missing workspace instead of leaving the dialog spinning", () => {
    // 用户反馈：输入不存在的目录后创建，对话框永远停在 Creating 且关不掉。
    setLocale("zh");
    for (const code of ["workspace_not_found", "workspace_not_found: /nope"]) {
      const failure = sessionCreateFailure(code);
      expect(failure.title).toBe(zh.sessions.createFailedTitle);
      expect(failure.detail).toBe(zh.sessions.createErrorWorkspaceNotFound);
    }
  });

  it("explains relative paths and non-directories", () => {
    setLocale("zh");
    expect(sessionCreateFailure("workspace_must_be_absolute").detail).toBe(
      zh.sessions.createErrorWorkspaceMustBeAbsolute,
    );
    expect(sessionCreateFailure("workspace_not_directory").detail).toBe(
      zh.sessions.createErrorWorkspaceNotDirectory,
    );
  });

  it("surfaces unknown rejection codes verbatim", () => {
    setLocale("zh");
    expect(sessionCreateFailure("weird_failure").detail).toContain("weird_failure");
    expect(sessionCreateFailure(undefined).detail).toContain("unknown");
    setLocale("en");
    expect(sessionCreateFailure("workspace_not_found").detail).toBe(
      en.sessions.createErrorWorkspaceNotFound,
    );
    setLocale("zh");
  });

  it("reports an unconfirmed create as a definite failure", () => {
    setLocale("zh");
    const failure = sessionCreateUnconfirmed();
    expect(failure.title).toBe(zh.sessions.createUnconfirmedTitle);
    expect(failure.detail).toBe(zh.sessions.createUnconfirmedDetail);
  });
});

describe("session create watchdog wiring", () => {
  it("resets creating on a rejected create and shows the host reason", () => {
    const rejected = source.indexOf("if (completed?.type === \"session_create\") {");
    expect(rejected).toBeGreaterThan(-1);
    const block = source.slice(rejected, rejected + 600);
    expect(block).toContain("creatingSessionRef.current = false;");
    expect(block).toContain("setCreatingSession(false);");
    expect(block).toContain("sessionCreateFailure(event.error)");
    expect(block).toContain("reportUiError(failure.title, failure.detail, \"system\");");
  });

  it("abandons an unconfirmed create after the bounded deadline", () => {
    expect(SESSION_CREATE_CONFIRM_TIMEOUT_MS).toBeGreaterThanOrEqual(5_000);
    expect(SESSION_CREATE_CONFIRM_TIMEOUT_MS).toBeLessThanOrEqual(30_000);
    const armed = source.indexOf("}, SESSION_CREATE_CONFIRM_TIMEOUT_MS);");
    expect(armed).toBeGreaterThan(-1);
    const block = source.slice(Math.max(0, armed - 700), armed);
    expect(block).toContain("if (!creatingSessionRef.current) return;");
    expect(block).toContain("sessionCreateUnconfirmed()");
  });

  it("clears the watchdog when the host confirms the session", () => {
    const created = source.indexOf("if (event.type === \"session_created\") {");
    expect(created).toBeGreaterThan(-1);
    const cleared = source.indexOf(
      "window.clearTimeout(sessionCreateTimerRef.current);",
      created,
    );
    expect(cleared).toBeGreaterThan(created);
  });

  it("keeps both locales in sync for the new messages", () => {
    const keys = [
      "createFailedTitle",
      "createUnconfirmedTitle",
      "createUnconfirmedDetail",
      "createErrorWorkspaceNotFound",
      "createErrorWorkspaceMustBeAbsolute",
      "createErrorWorkspaceNotDirectory",
      "createErrorGeneric",
    ];
    for (const key of keys) {
      expect((zh.sessions as Record<string, string>)[key]).toBeTruthy();
      expect((en.sessions as Record<string, string>)[key]).toBeTruthy();
    }
  });
});
