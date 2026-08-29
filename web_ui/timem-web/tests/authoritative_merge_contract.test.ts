import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const source = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const styles = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");

function functionSlice(name: string, nextName: string) {
  const start = source.indexOf(name);
  const end = source.indexOf(nextName, start + name.length);
  expect(start).toBeGreaterThanOrEqual(0);
  expect(end).toBeGreaterThan(start);
  return source.slice(start, end);
}

describe("authoritative Core/Host/UI merge boundary", () => {
  it("keeps semantic turn projection and explicit finish handling authoritative", () => {
    expect(source).toContain("applyTurnProjection(session, event.projection)");
    expect(source).toContain('if (event.type === "turn_updated")');
    expect(source).toContain('if (event.type === "turn_finished")');
    expect(source).not.toContain('event.turn.state !== "working"');
  });

  it("keeps answer delivery presentation-only", () => {
    const delivery = functionSlice(
      "function TurnAnswerDelivery(",
      "function FinalAnswerDelivery(",
    );
    expect(delivery).toContain("turn.final_answer");
    expect(delivery).toContain("turn.sub_answers");
    expect(delivery).toContain("newestInterimAnswersFirst(turn.sub_answers)");
    expect(delivery).toContain("if (finalArrived) setChatExpanded(false)");
    expect(delivery).not.toContain("setSessions(");
    expect(delivery).not.toContain("applyTurnProjection(");
  });

  it("collapses work only on an authoritative final answer or interruption", () => {
    const interaction = functionSlice(
      "const TurnInteraction = memo(function TurnInteraction(",
      "function areTurnInteractionPropsEqual(",
    );
    expect(interaction).toContain(
      'finalArrived || (wasWorking && turn.state === "interrupted")',
    );
    expect(interaction).not.toContain(
      "if (wasWorking && !isWorking) setShowWorkStream(false)",
    );
  });
});

describe("ported UI isolation and rendering optimizations", () => {
  it("keeps settings callbacks stable and reads live sessions through the ref", () => {
    expect(source).toContain("const closeSettingsCenter = useCallback(");
    expect(source).toContain("memSwitchRunningSessionCount(sessionsRef.current)");
    expect(source).toContain(
      "const SettingsCenter = memo(function SettingsCenter(",
    );
    expect(source).toContain("const deletableTemporaryItems = useMemo(");
  });

  it("isolates center modals from the workspace", () => {
    expect(source).toContain("inert={workspaceModalOpen}");
    expect(source).toContain("aria-hidden={workspaceModalOpen || undefined}");
    expect(source).toContain("return createPortal(");
    expect(styles).toContain("body.workspace-modal-open");
    expect(styles).toContain("overscroll-behavior: none");
  });

  it("updates work edge fades without changing turn ownership", () => {
    expect(source).toContain("const next = scrollEdgeFades({");
    expect(source).toContain("updateWorkEdgeFades(event.currentTarget)");
    expect(styles).toContain(".turn-work-scroll.fade-top");
    expect(styles).toContain(".turn-work-scroll.fade-bottom");
  });
});
