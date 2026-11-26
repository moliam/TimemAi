import { describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const main = readFileSync(new URL("../src/main.tsx", import.meta.url), "utf8");
const protocol = readFileSync(new URL("../src/protocol.ts", import.meta.url), "utf8");
const outbox = readFileSync(new URL("../src/command_outbox.ts", import.meta.url), "utf8");
const wire = readFileSync(new URL("../src/wire_delivery.ts", import.meta.url), "utf8");

// 用户反馈：WebUI 每次加载都拉全量快照太慢。Host 按 session LRU 排序 + 8MB
// turns 预算下发 hello；超限 session 的 turns 省略，选中时按需拉取。重开页面
// 保持上次查看的 session（每设备 localStorage）并把选中上报为 view-focus。
describe("session LRU snapshot and selection persistence", () => {
  it("declares the view-focus and on-demand turns wire contract", () => {
    expect(protocol).toContain('type: "session_focus"');
    expect(protocol).toContain('type: "session_turns_load"');
    expect(protocol).toContain('type: "session_turns"');
    expect(protocol).toContain("turns_omitted?: boolean;");
    expect(outbox).toContain('"session_focus",');
    expect(outbox).toContain('"session_turns_load",');
    expect(wire).toContain('"session_turns",');
  });

  it("restores and persists the active session per device", () => {
    expect(main).toContain('const ACTIVE_SESSION_STORAGE_KEY = "timem.active-session.v1";');
    expect(main).toContain("current || readStoredActiveSessionId()");
    expect(main).toContain("storeActiveSessionId(activeSessionId);");
    expect(main).toContain('{ type: "session_focus", session_id: activeSessionId }');
  });

  it("fetches omitted turns on demand when the user selects the session", () => {
    expect(main).toContain("const sessionTurnsLoadInFlightRef = useRef(new Set<string>());");
    expect(main).toContain('{ type: "session_turns_load", session_id: activeSessionId }');
    expect(main).toContain("{ ...session, turns: event.turns, turns_omitted: false }");
    expect(main).toContain("sessionTurnsLoadInFlightRef.current.delete(event.session_id);");
  });
});
