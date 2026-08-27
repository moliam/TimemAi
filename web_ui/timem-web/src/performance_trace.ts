import type { ClientCommand, CommandWithId, WebTurn } from "./protocol";

export type BrowserPerformanceStage =
  | "browser_send"
  | "browser_turn_updated"
  | "browser_painted"
  | "browser_session_selected"
  | "browser_session_painted";

type TracePayload = {
  stage: BrowserPerformanceStage;
  session_id: string;
  command_id: string;
  turn_id?: string;
  elapsed_ms?: number;
  event_count?: number;
};

type PendingTrace = { sessionId: string; startedAt: number; updateObserved?: boolean };

export class BrowserPerformanceTrace {
  private enabled = false;
  private readonly commandStarts = new Map<string, PendingTrace>();
  private pendingSessionSelection?: PendingTrace & { commandId: string };

  setEnabled(enabled: boolean) {
    this.enabled = enabled;
    if (!enabled) {
      this.commandStarts.clear();
      this.pendingSessionSelection = undefined;
    }
  }

  instrumentCommand<T extends ClientCommand | CommandWithId>(command: T): T & { performance_sent_at_ms?: number } {
    if (!this.enabled || (command.type !== "turn_submit" && command.type !== "turn_supplement")) return command;
    const commandId = "command_id" in command ? command.command_id : undefined;
    if (!commandId) return command;
    const sentAt = Date.now();
    this.commandStarts.set(commandId, { sessionId: command.session_id, startedAt: performance.now() });
    this.report({ stage: "browser_send", session_id: command.session_id, command_id: commandId });
    return { ...command, performance_sent_at_ms: sentAt };
  }

  observeTurnUpdated(sessionId: string, turn: WebTurn) {
    if (!this.enabled) return;
    const commandId = [...turn.user_entries].reverse().find((entry) => entry.command_id)?.command_id;
    if (!commandId) return;
    const pending = this.commandStarts.get(commandId);
    if (!pending || pending.sessionId !== sessionId || pending.updateObserved) return;
    pending.updateObserved = true;
    this.report({
      stage: "browser_turn_updated",
      session_id: sessionId,
      command_id: commandId,
      turn_id: turn.turn_id,
      elapsed_ms: performance.now() - pending.startedAt,
      event_count: turn.events.length,
    });
    requestAnimationFrame(() => {
      this.report({
        stage: "browser_painted",
        session_id: sessionId,
        command_id: commandId,
        turn_id: turn.turn_id,
        elapsed_ms: performance.now() - pending.startedAt,
        event_count: turn.events.length,
      });
      this.commandStarts.delete(commandId);
    });
  }

  beginSessionSelection(sessionId: string) {
    if (!this.enabled) return;
    const commandId = `session-switch-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    this.pendingSessionSelection = { sessionId, commandId, startedAt: performance.now() };
    this.report({ stage: "browser_session_selected", session_id: sessionId, command_id: commandId });
  }

  observeSessionPainted(sessionId: string) {
    const pending = this.pendingSessionSelection;
    if (!this.enabled || !pending || pending.sessionId !== sessionId) return;
    requestAnimationFrame(() => {
      this.report({
        stage: "browser_session_painted",
        session_id: sessionId,
        command_id: pending.commandId,
        elapsed_ms: performance.now() - pending.startedAt,
      });
      if (this.pendingSessionSelection?.commandId === pending.commandId) this.pendingSessionSelection = undefined;
    });
  }

  private report(payload: TracePayload) {
    const token = queryToken();
    const query = token ? `?token=${encodeURIComponent(token)}` : "";
    void fetch(`/api/performance-trace${query}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
      keepalive: true,
    }).catch(() => undefined);
  }
}

function queryToken() {
  try { return window.sessionStorage.getItem("timem-web-access-token") ?? ""; } catch { return ""; }
}
