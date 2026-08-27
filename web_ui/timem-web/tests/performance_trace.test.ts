import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BrowserPerformanceTrace } from "../src/performance_trace";
import type { WebTurn } from "../src/protocol";

const requests: Array<{ url: string; body: Record<string, unknown> }> = [];
let frames: FrameRequestCallback[] = [];
let clock = 10;

function turn(commandId: string): WebTurn {
  return {
    turn_id: "turn-1",
    state: "working",
    created_at_ms: 1,
    user_entries: [{ command_id: commandId, kind: "supplement", text: "more", created_at_ms: 2 }],
    events: [{ event_id: "event-1", source: "core_topic", payload: {}, created_at_ms: 3 }],
    sub_answers: [],
    final_answer: null,
    completion: null,
  };
}

function stages() { return requests.map((request) => request.body.stage); }

beforeEach(() => {
  requests.length = 0;
  frames = [];
  clock = 10;
  vi.stubGlobal("window", { sessionStorage: { getItem: () => "token value" } });
  vi.stubGlobal("fetch", vi.fn((url: string, init: RequestInit) => {
    requests.push({ url, body: JSON.parse(String(init.body)) });
    return Promise.resolve(new Response(null, { status: 204 }));
  }));
  vi.stubGlobal("requestAnimationFrame", vi.fn((callback: FrameRequestCallback) => {
    frames.push(callback);
    return frames.length;
  }));
  vi.spyOn(performance, "now").mockImplementation(() => clock);
  vi.spyOn(Date, "now").mockReturnValue(1_700_000_000_000);
  vi.spyOn(Math, "random").mockReturnValue(0.5);
});

afterEach(() => vi.restoreAllMocks());

describe("browser performance trace", () => {
  it("is inert while server tracing is disabled", () => {
    const trace = new BrowserPerformanceTrace();
    const command = { type: "turn_submit" as const, session_id: "session-1", text: "task", command_id: "command-1" };
    expect(trace.instrumentCommand(command)).toBe(command);
    trace.observeTurnUpdated("session-1", turn("command-1"));
    trace.beginSessionSelection("session-2");
    expect(requests).toEqual([]);
  });

  it("correlates send, first update, and first paint exactly once", () => {
    const trace = new BrowserPerformanceTrace();
    trace.setEnabled(true);
    const command = trace.instrumentCommand({ type: "turn_supplement", session_id: "session-1", text: "more", command_id: "command-1" });
    expect(command.performance_sent_at_ms).toBe(1_700_000_000_000);
    expect(requests[0].url).toBe("/api/performance-trace?token=token%20value");
    clock = 25;
    trace.observeTurnUpdated("session-1", turn("command-1"));
    trace.observeTurnUpdated("session-1", turn("command-1"));
    expect(stages()).toEqual(["browser_send", "browser_turn_updated"]);
    expect(requests[1].body).toMatchObject({ command_id: "command-1", elapsed_ms: 15, event_count: 1 });
    expect(frames).toHaveLength(1);
    clock = 31;
    frames.shift()?.(31);
    trace.observeTurnUpdated("session-1", turn("command-1"));
    expect(stages()).toEqual(["browser_send", "browser_turn_updated", "browser_painted"]);
    expect(requests[2].body.elapsed_ms).toBe(21);
  });

  it("ignores updates whose command or session does not match", () => {
    const trace = new BrowserPerformanceTrace();
    trace.setEnabled(true);
    trace.instrumentCommand({ type: "turn_submit", session_id: "session-1", text: "task", command_id: "command-1" });
    trace.observeTurnUpdated("session-2", turn("command-1"));
    trace.observeTurnUpdated("session-1", turn("other-command"));
    expect(stages()).toEqual(["browser_send"]);
  });

  it("correlates session selection to the next matching paint", () => {
    const trace = new BrowserPerformanceTrace();
    trace.setEnabled(true);
    trace.beginSessionSelection("session-2");
    trace.observeSessionPainted("session-1");
    expect(stages()).toEqual(["browser_session_selected"]);
    clock = 42;
    trace.observeSessionPainted("session-2");
    expect(frames).toHaveLength(1);
    frames.shift()?.(42);
    expect(stages()).toEqual(["browser_session_selected", "browser_session_painted"]);
    expect(requests[1].body).toMatchObject({ session_id: "session-2", elapsed_ms: 32 });
  });

  it("drops pending correlations when tracing is disabled", () => {
    const trace = new BrowserPerformanceTrace();
    trace.setEnabled(true);
    trace.instrumentCommand({ type: "turn_submit", session_id: "session-1", text: "task", command_id: "command-1" });
    trace.beginSessionSelection("session-2");
    trace.setEnabled(false);
    trace.setEnabled(true);
    trace.observeTurnUpdated("session-1", turn("command-1"));
    trace.observeSessionPainted("session-2");
    expect(stages()).toEqual(["browser_send", "browser_session_selected"]);
  });
});
