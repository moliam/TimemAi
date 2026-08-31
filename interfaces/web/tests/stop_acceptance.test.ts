import { describe, expect, it } from "vitest";
import type { Session, WebTurn } from "../src/protocol";
import {
  finishTurn,
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

const cancelledTurn = (): WebTurn => ({
  ...workingTurn(),
  state: "finished",
  completion: { stop_reason: "CancelledByUser" },
});

function runningSession(): Session {
  return updateSessionWorkerState(
    upsertTurn(session(), workingTurn()),
    "worker-1",
    "working",
  );
}

function hostCancelledSession(): Session {
  const running = runningSession();
  return {
    ...running,
    state: "ready",
    workers: running.workers.map((item) => ({ ...item, state: "ready" })),
    cancelling_turn_id: "turn-1",
    turns: [cancelledTurn()],
  };
}

describe("Stop feature acceptance", () => {
  it("keeps rendering the last Host state until the Stop response arrives", () => {
    const running = runningSession();

    // A click is only a transport action. Before Host responds, business UI
    // remains a rendering of the last authoritative Session.
    expect(sessionVisuallyWorking(running)).toBe(true);
    expect(running.turns[0]).toMatchObject({ state: "working", completion: null });
  });

  it("renders the complete authoritative Session returned by Host", () => {
    const acknowledged = hostCancelledSession();

    expect(acknowledged.active_turn_id).toBe("turn-1");
    expect(acknowledged.cancelling_turn_id).toBe("turn-1");
    expect(acknowledged.turns[0]).toMatchObject({
      state: "finished",
      completion: { stop_reason: "CancelledByUser" },
    });
    expect(acknowledged.state).toBe("ready");
    expect(acknowledged.workers.every((worker) => worker.state !== "working")).toBe(true);
    expect(sessionCancellationApplies(acknowledged)).toBe(false);
    expect(sessionVisuallyWorking(acknowledged)).toBe(false);
  });

  it("does not revive the Host-confirmed terminal state with late Core events", () => {
    const acknowledged = hostCancelledSession();
    const afterLateTurn = upsertTurn(acknowledged, workingTurn());
    const afterLateWorker = updateSessionWorkerState(
      afterLateTurn,
      "worker-1",
      "working",
    );

    expect(afterLateWorker.active_turn_id).toBe("turn-1");
    expect(afterLateWorker.cancelling_turn_id).toBe("turn-1");
    expect(afterLateWorker.turns[0]).toMatchObject({
      state: "finished",
      completion: { stop_reason: "CancelledByUser" },
    });
    expect(afterLateWorker.state).toBe("ready");
    expect(afterLateWorker.workers[0].state).toBe("ready");
  });

  it("does not let live command correlation override a stale Host snapshot", () => {
    const staleSnapshot = runningSession();
    expect(sessionCancellationApplies(staleSnapshot)).toBe(false);
    expect(sessionVisuallyWorking(staleSnapshot)).toBe(true);
  });

  it("does not revive a cancelled Session after terminal completion", () => {
    const finished = finishTurn(runningSession(), "turn-1", {
      stop_reason: "CancelledByUser",
    });
    const afterLateTurn = upsertTurn(finished, workingTurn());
    const afterLateWorker = updateSessionWorkerState(
      afterLateTurn,
      "worker-1",
      "working",
      "turn-1",
    );

    expect(afterLateWorker.turns[0]).toMatchObject({
      turn_id: "turn-1",
      state: "finished",
      completion: { stop_reason: "CancelledByUser" },
    });
    expect(afterLateWorker.state).toBe("ready");
    expect(afterLateWorker.workers[0].state).toBe("ready");
  });

  it("submits new input directly after Host confirms the cancelled UI state", () => {
    const acknowledged = hostCancelledSession();

    expect(
      shouldDirectManualMessage(
        acknowledged.state,
        0,
        false,
        !!acknowledged.cancelling_turn_id,
      ),
    ).toBe(true);

    // Browser queue dispatch is not the cancellation ordering mechanism. A
    // direct turn_submit goes to Host, whose private FIFO waits for Core.
    expect(
      selectQueuedDispatches(
        [acknowledged],
        {},
        new Set(),
        undefined,
        new Set(),
        new Set(["session-1"]),
      ),
    ).toEqual([]);
  });
});
