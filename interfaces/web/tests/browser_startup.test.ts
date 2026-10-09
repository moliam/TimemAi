import { EventEmitter } from "node:events";
import { createServer } from "node:http";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import { createStartupDiagnostics, linuxStartupSnapshot } from "./browser/browser-startup.mjs";

function child() {
  return Object.assign(new EventEmitter(), {
    pid: 123, exitCode: null as number | null,
    signalCode: null as string | null, stderr: new EventEmitter(),
  });
}
const decode = async (diagnostics: ReturnType<typeof createStartupDiagnostics>) =>
  JSON.parse((await diagnostics.failure()).replace("Chrome DevTools did not start: ", ""));

afterEach(() => { vi.unstubAllGlobals(); vi.useRealTimers(); });

describe("browser startup evidence", () => {
  it("captures late bounded stderr, port timing and exit at the failure boundary", async () => {
    const process = child();
    let time = 100;
    const diagnostics = createStartupDiagnostics(process, "chrome", { now: () => time, platform: "darwin" });
    expect(diagnostics.stopped()).toBe(false);
    time = 130;
    process.stderr.emit("data", "x".repeat(9000));
    time = 180;
    process.stderr.emit("data", "late marker");
    diagnostics.port(4321);
    time = 190;
    diagnostics.port(4321);
    process.exitCode = 23;
    expect(diagnostics.stopped()).toBe(true);
    const result = await decode(diagnostics);
    expect(result).toMatchObject({ exitCode: 23, firstStderrMs: 30, portReadyMs: 80, elapsedMs: 90, port: 4321 });
    expect(result.stderr.length).toBe(8192);
    expect(result.stderr.endsWith("late marker")).toBe(true);
    expect(diagnostics.stderr()).toBe(result.stderr);
    expect(result).not.toHaveProperty("linux");
  });

  it("distinguishes signal termination and spawn failure from a live child", async () => {
    const process = child();
    const diagnostics = createStartupDiagnostics(process, "chrome", { platform: "darwin" });
    process.signalCode = "SIGTERM";
    expect(diagnostics.stopped()).toBe(true);
    expect(await decode(diagnostics)).toMatchObject({ exitCode: null, signalCode: "SIGTERM" });
    const missing = child();
    const failed = createStartupDiagnostics(missing, "missing", { platform: "darwin" });
    missing.emit("error", new Error("ENOENT"));
    expect(failed.stopped()).toBe(true);
    expect(await decode(failed)).toMatchObject({ spawnError: "ENOENT", portReadyMs: null });
  });

  it("retains bounded timed HTTP status/error evidence without weakening success", async () => {
    let time = 0;
    const cancel = vi.fn().mockResolvedValue(undefined);
    const fetch = vi.fn().mockImplementation(async (_url, options) => {
      expect(options.signal).toBeInstanceOf(AbortSignal);
      time += 7;
      return { ok: false, status: 503, body: { cancel } };
    });
    vi.stubGlobal("fetch", fetch);
    const diagnostics = createStartupDiagnostics(child(), "chrome", { now: () => time, platform: "darwin" });
    diagnostics.port(1234);
    for (let index = 0; index < 20; index++) expect(await diagnostics.probe("http://127.0.0.1:1234/json/version")).toBe(false);
    let result = await decode(diagnostics);
    expect(result.probes).toHaveLength(16);
    expect(result.probes[0]).toEqual({ startMs: 28, endMs: 35, status: 503 });
    expect(cancel).not.toHaveBeenCalled();
    fetch.mockRejectedValueOnce(new Error("probe timed out"));
    expect(await diagnostics.probe("http://127.0.0.1:1234/json/version")).toBe(false);
    result = await decode(diagnostics);
    expect(result.probes.at(-1)).toMatchObject({ error: "probe timed out" });
    fetch.mockResolvedValueOnce({ ok: true, status: 200, body: { cancel } });
    expect(await diagnostics.probe("http://127.0.0.1:1234/json/version")).toBe(true);
  });

  it("keeps successful response headers independent of body cancellation", async () => {
    const cancel = vi.fn().mockRejectedValue(new Error("body cancellation failed"));
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue({ ok: true, status: 200, body: { cancel } }));
    const diagnostics = createStartupDiagnostics(child(), "chrome", { platform: "darwin" });
    expect(await diagnostics.probe("http://127.0.0.1:1234/json/version")).toBe(true);
    expect(cancel).not.toHaveBeenCalled();
    expect((await decode(diagnostics)).probes.at(-1)).toMatchObject({ status: 200 });
  });

  it("times out a real listening HTTP server that never sends headers", async () => {
    const server = createServer(() => {});
    await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
    try {
      const address = server.address();
      if (!address || typeof address === "string") throw new Error("missing probe port");
      const diagnostics = createStartupDiagnostics(child(), "chrome", { platform: "darwin" });
      diagnostics.port(address.port);
      expect(await diagnostics.probe(`http://127.0.0.1:${address.port}/json/version`)).toBe(false);
      const result = await decode(diagnostics);
      expect(result.probes).toHaveLength(1);
      expect(result.probes[0].error).toMatch(/timeout/i);
      expect(result.probes[0].endMs - result.probes[0].startMs).toBeGreaterThanOrEqual(900);
      expect(result.probes[0].endMs - result.probes[0].startMs).toBeLessThan(2500);
    } finally {
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve()));
    }
  });

  it("reads Linux facts only at failure, before any caller cleanup", async () => {
    const process = child();
    const snapshot = vi.fn().mockImplementation(async (pid) => {
      expect(pid).toBe(123);
      process.signalCode = "SIGTERM";
      return { stat: "kernel counters" };
    });
    const diagnostics = createStartupDiagnostics(process, "chrome", { platform: "linux", snapshot });
    diagnostics.port(4321);
    expect(snapshot).not.toHaveBeenCalled();
    expect(await decode(diagnostics)).toMatchObject({ signalCode: null, linux: { stat: "kernel counters" } });
    expect(snapshot).toHaveBeenCalledOnce();
  });

  it("preserves the startup error when process evidence cannot be read", async () => {
    const diagnostics = createStartupDiagnostics(child(), "chrome", {
      platform: "linux", snapshot: async () => { throw new Error("permission denied"); },
    });
    expect(await decode(diagnostics)).toMatchObject({ linux: { unavailable: "permission denied" } });
  });

  it("bounds failure evidence wait and tolerates a late snapshot rejection", async () => {
    vi.useFakeTimers();
    let rejectSnapshot!: (error: Error) => void;
    const diagnostics = createStartupDiagnostics(child(), "chrome", {
      platform: "linux", snapshot: () => new Promise((_resolve, reject) => { rejectSnapshot = reject; }),
    });
    let completed = false;
    const result = decode(diagnostics).then(value => { completed = true; return value; });
    await vi.advanceTimersByTimeAsync(249);
    expect(completed).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    expect(await result).toMatchObject({ exitCode: null, signalCode: null, linux: { unavailable: "snapshot_timeout" } });
    rejectSnapshot(new Error("late diagnostic failure"));
    await vi.advanceTimersByTimeAsync(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("bounds proc reads and thread count, and reports inaccessible counters", async () => {
    const root = await mkdtemp(join(tmpdir(), "timem-startup-proc-"));
    try {
      await mkdir(join(root, "123", "task"), { recursive: true });
      await writeFile(join(root, "123", "stat"), "s".repeat(6000));
      for (let tid = 1; tid <= 20; tid++) {
        await mkdir(join(root, "123", "task", String(tid)));
        await writeFile(join(root, "123", "task", String(tid), "wchan"), "futex_wait");
      }
      const result = await linuxStartupSnapshot(123, root);
      expect(result.stat.length).toBe(4096);
      expect(result.io).toEqual({ unavailable: "ENOENT" });
      expect(result.threads).toHaveLength(16);
      expect(result.threads.every(thread => thread.wchan === "futex_wait")).toBe(true);
      expect(await linuxStartupSnapshot(undefined, root)).toEqual({ unavailable: "no_child_pid" });
      expect(await linuxStartupSnapshot(456, root)).toMatchObject({ threadsUnavailable: "ENOENT" });
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });
});
