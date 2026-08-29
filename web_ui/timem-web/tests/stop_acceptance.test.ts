import { describe, expect, it } from "vitest";
import type { Session, WebTurn } from "../src/protocol";
import {
  finishTurn,
  markSessionCancelling,
  sessionCancellationApplies,
  sessionVisuallyWorking,
  updateSessionWorkerState,
  upsertTurn,
} from "../src/view_model";
import {
  selectQueuedDispatches,
  shouldDirectManualMessage,
} from "../src/queued_messages";

const session = (): Session => ({
  session_id: "session-1",
  display_name: "Acceptance agent",
  ordinal: 0,
  state: "ready",
  current_dir: "/work",
  max_llm_input_tokens: 100_000,
  tools: [],
  contexts: [
    {
      context_id: "context-1",
      current_dir: "/work",
      worker_ids: ["worker-1"],
    },
  ],
  workers: [
    {
      worker_id: "worker-1",
      context_id: "context-1",
      display_name: "Acceptance worker",
      ordinal: 0,
      state: "ready",
      parent_worker_id: null,
    },
  ],
  active_context_id: "context-1",
  primary_worker_id: "worker-1",
  attachments: [],
  messages: [],
  turns: [],
  history_before_cursor: null,
  history_has_more: false,
  active_turn_id: null,
});

const workingTurn = (): WebTurn => ({
  turn_id: "turn-1",
  state: "working",
  created_at_ms: 1,
  user_entries: [{ kind: "task", text: "Long task", created_at_ms: 1 }],
  events: [],
  sub_answers: [],
  final_answer: null,
  completion: null,
});

function runningSession(): Session {
  return updateSessionWorkerState(
    upsertTurn(session(), workingTurn()),
    "worker-1",
    "working",
  );
}

describe("Stop feature acceptance", () => {
  it("stops showing work immediately when the user presses Stop", () => {
    // Given a Turn visibly working in the Session list.
    const running = runningSession();
    expect(sessionVisuallyWorking(running)).toBe(true);

    // When the browser records the user's Stop action, before Host round-trip.
    const locallyStopping = new Set([running.session_id]);

    // Then the user no longer sees the Session as working.
    expect(sessionVisuallyWorking(running, locallyStopping)).toBe(false);
  });

  it("keeps the Session non-working after Host acknowledgement and reconnect snapshot", () => {
    // Given Host has accepted Stop for the current Turn.
    const acknowledged = markSessionCancelling(runningSession(), "turn-1");

    // Then observable identity is preserved while all visible working state is gone.
    expect(acknowledged.active_turn_id).toBe("turn-1");
    expect(acknowledged.cancelling_turn_id).toBe("turn-1");
    expect(acknowledged.state).toBe("ready");
    expect(
      acknowledged.workers.every((worker) => worker.state !== "working"),
    ).toBe(true);
    expect(sessionVisuallyWorking(acknowledged)).toBe(false);
  });

  it("does not revive the spinner when late Core working events arrive", () => {
    // Given Stop has already been accepted.
    const acknowledged = markSessionCancelling(runningSession(), "turn-1");

    // When delayed TurnStarted/worker-working projections arrive.
    const afterLateTurn = upsertTurn(acknowledged, workingTurn());
    const afterLateWorker = updateSessionWorkerState(
      afterLateTurn,
      "worker-1",
      "working",
    );

    // Then Turn correlation remains, but the Session stays visibly stopped.
    expect(afterLateWorker.active_turn_id).toBe("turn-1");
    expect(afterLateWorker.cancelling_turn_id).toBe("turn-1");
    expect(afterLateWorker.state).toBe("ready");
    expect(afterLateWorker.workers[0].state).toBe("ready");
    expect(sessionVisuallyWorking(afterLateWorker)).toBe(false);
  });


  it("keeps a stale reconnect snapshot visually stopped while a durable targeted cancel applies", () => {
    // Given a reload has discarded memory-only cancellation state while Host
    // still reports the pre-cancel working snapshot.
    const staleSnapshot = runningSession();

    // Then the durable targeted cancel is the shared cancellation truth for
    // both the chat and the Session row.
    expect(
      sessionCancellationApplies(
        staleSnapshot,
        new Set(),
        "submit-original",
      ),
    ).toBe(true);
    expect(
      sessionVisuallyWorking(staleSnapshot, new Set(), "submit-original"),
    ).toBe(false);
  });

  it("does not revive a cancelled Session after terminal completion", () => {
    // Given the cancelled Turn has already reached its authoritative terminal state.
    const finished = finishTurn(runningSession(), "turn-1", {
      stop_reason: "CancelledByUser",
    });

    // When delayed started and worker-working projections for that same Turn arrive.
    const afterLateTurn = upsertTurn(finished, workingTurn());
    const afterLateWorker = updateSessionWorkerState(
      afterLateTurn,
      "worker-1",
      "working",
      "turn-1",
    );

    // Then terminal state remains monotonic: chat stays Cancelled and the
    // Session row cannot return to working.
    expect(afterLateWorker.turns[0]).toMatchObject({
      turn_id: "turn-1",
      state: "finished",
      completion: { stop_reason: "CancelledByUser" },
    });
    expect(afterLateWorker.state).toBe("ready");
    expect(afterLateWorker.workers[0].state).toBe("ready");
    expect(sessionVisuallyWorking(afterLateWorker)).toBe(false);
  });

  it("durably queues new input and holds it until cancellation is authoritatively finished", () => {
    // Given Stop is acknowledged, while the old Turn is still cleaning up.
    const acknowledged = markSessionCancelling(runningSession(), "turn-1");
    const queued = {
      "session-1": [{ id: "message-2", text: "Next task", createdAtMs: 2 }],
    };

    // Then Enter cannot bypass the queue, even though visual state is ready.
    expect(
      shouldDirectManualMessage(
        acknowledged.state,
        0,
        false,
        !!acknowledged.cancelling_turn_id,
      ),
    ).toBe(false);

    // And even an auto-continue permit cannot dispatch before terminal completion.
    expect(
      selectQueuedDispatches(
        [acknowledged],
        queued,
        new Set(),
        undefined,
        new Set(),
        new Set(["session-1"]),
      ),
    ).toEqual([]);

    // When Core authoritatively reports the cancelled Turn finished.
    const finished = finishTurn(acknowledged, "turn-1", {
      stop_reason: "CancelledByUser",
    });

    // Then exactly the FIFO head becomes eligible for the next Turn.
    expect(finished.cancelling_turn_id).toBeNull();
    expect(
      selectQueuedDispatches(
        [finished],
        queued,
        new Set(),
        undefined,
        new Set(),
        new Set(["session-1"]),
      ),
    ).toEqual([{ sessionId: "session-1", message: queued["session-1"][0] }]);
  });
});
