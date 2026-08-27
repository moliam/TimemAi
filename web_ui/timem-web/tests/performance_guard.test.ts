import { describe, expect, it } from "vitest";
import { createFrameEventQueue } from "../src/frame_event_queue";
import { coalesceActionLifecycle } from "../src/view_model";
import type { WebTurnEvent } from "../src/protocol";

const guardEnabled = process.env.TIMEM_PERF_GUARD === "1";
function assertUnder(label: string, elapsedMs: number, budgetMs: number) {
  console.log(`performance_guard: ${label} elapsed_ms=${elapsedMs.toFixed(2)} budget_ms=${budgetMs}`);
  if (guardEnabled) expect(elapsedMs, `${label} exceeded ${budgetMs}ms`).toBeLessThanOrEqual(budgetMs);
}
function actionEvent(index: number, lifecycle: "start" | "finish"): WebTurnEvent {
  const actionId = `action-${index}`;
  return {
    event_id: `${lifecycle}-${index}`, source: "core_topic",
    created_at_ms: index * 2 + (lifecycle === "finish" ? 1 : 0),
    payload: {
      session_id: "session-perf",
      topic: { name: "core.action", attributes: { event: lifecycle, action_id: actionId } },
      state: { name: "running" },
      payload: { action: "run_bash", action_id: actionId, event: lifecycle, status: lifecycle === "start" ? "running" : "completed", input: { cmd: "true" } },
    },
  };
}
describe("web performance guard", () => {
  it("coalesces a long action lifecycle without a scale cliff", () => {
    const events: WebTurnEvent[] = [];
    for (let index = 0; index < 10_000; index += 1) events.push(actionEvent(index, "start"), actionEvent(index, "finish"));
    const started = performance.now();
    const visible = coalesceActionLifecycle(events);
    const elapsedMs = performance.now() - started;
    expect(visible).toHaveLength(10_000);
    assertUnder("web_action_lifecycle_20000_events", elapsedMs, 1_500);
  });
  it("drains a large browser event burst in bounded batches", () => {
    const scheduled: Array<() => void> = [];
    let consumed = 0;
    const queue = createFrameEventQueue<number>({
      consume: (items) => { consumed += items.length; },
      schedule: (callback) => { scheduled.push(callback); return scheduled.length; },
      cancel: () => undefined, now: () => 0, maxBatch: 24,
    });
    const started = performance.now();
    for (let index = 0; index < 50_000; index += 1) queue.enqueue(index);
    while (scheduled.length > 0) scheduled.shift()?.();
    const elapsedMs = performance.now() - started;
    expect(consumed).toBe(50_000);
    expect(queue.pending()).toBe(0);
    assertUnder("web_frame_queue_50000_events", elapsedMs, 1_500);
  });
});
