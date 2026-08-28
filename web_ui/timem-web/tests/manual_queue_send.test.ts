import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(
  new URL("../src/main.tsx", import.meta.url),
  "utf8",
);

function normalizedSource(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

function functionBody(startMarker: string, endMarker: string): string {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  expect(start).toBeGreaterThanOrEqual(0);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
}

describe("manual sending while the durable queue is paused", () => {
  it("keeps a normal manual send in the durable queue without implicitly resuming it", () => {
    const body = functionBody(
      "const submitDraft = () => {",
      "const submitDraftAsSupplement = () => {",
    );

    expect(normalizedSource(body)).toContain(
      "shouldDirectManualMessage( activeSession.state, existingQueue.length, !!queuedMessagesPause, isCancelling || !!activeSession.cancelling_turn_id, )",
    );
    expect(body).toContain("onSendForSession(");
    expect(body).toContain('directCommandId = clientId("submit")');
    expect(normalizedSource(body)).toContain(
      "directSubmissionsRef.current.set(reserved.sessionId, {",
    );
    expect(body).toContain(
      "submittingDraftSessionIdsRef.current.add(reserved.sessionId)",
    );
    expect(body).toContain("saveQueuedMessages(");
    expect(body).toContain("updateQueuedMessages(() => nextQueues)");
    expect(body).not.toContain("resumeQueuedMessages()");
  });

  it("uses Ctrl/Command+Enter as an explicit immediate-send escape hatch", () => {
    const supplementBody = functionBody(
      "const submitDraftAsSupplement = () => {",
      "const toggleQueuedMessages = () => {",
    );
    const composerStart = source.indexOf('className="composer"');
    const keyboardStart = source.indexOf(
      "onKeyDown={(event) => {",
      composerStart,
    );
    const keyboardEnd = source.indexOf("/>", keyboardStart);
    expect(composerStart).toBeGreaterThanOrEqual(0);
    expect(keyboardStart).toBeGreaterThan(composerStart);
    expect(keyboardEnd).toBeGreaterThan(keyboardStart);
    const keyboardBody = source.slice(keyboardStart, keyboardEnd);

    expect(supplementBody).toContain("onSendForSession(");
    expect(supplementBody).toContain('clientId("supplement")');
    expect(supplementBody).toMatch(/true,\s*selectedRoleIds,\s*\);/);
    expect(keyboardBody).toContain("event.metaKey || event.ctrlKey");
    expect(keyboardBody).toContain("submitDraftAsSupplement()");
    expect(keyboardBody).toContain("submitDraft()");
  });
});
