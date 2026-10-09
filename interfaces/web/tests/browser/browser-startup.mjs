import { open, opendir } from "node:fs/promises";
import { join } from "node:path";

// Test-harness diagnostics only. Never collect command lines, environments or
// arbitrary process trees. /proc counters describe the Chrome PID we spawned.
async function boundedText(path) {
  let file;
  try {
    file = await open(path, "r");
    const buffer = Buffer.alloc(4096);
    const { bytesRead } = await file.read(buffer, 0, buffer.length, 0);
    return buffer.subarray(0, bytesRead).toString("utf8");
  } catch (error) {
    return { unavailable: error.code ?? error.message };
  } finally {
    await file?.close();
  }
}

export async function linuxStartupSnapshot(pid, procRoot = "/proc") {
  if (!Number.isSafeInteger(pid) || pid <= 0) return { unavailable: "no_child_pid" };
  const root = join(procRoot, String(pid));
  const entries = await Promise.all(["stat", "status", "io", "schedstat", "wchan"].map(
    async name => [name, await boundedText(join(root, name))],
  ));
  const snapshot = Object.fromEntries(entries);
  snapshot.pressure = Object.fromEntries(await Promise.all(["cpu", "io", "memory"].map(
    async name => [name, await boundedText(join(procRoot, "pressure", name))],
  )));
  snapshot.threads = [];
  try {
    const directory = await opendir(join(root, "task"));
    for await (const entry of directory) {
      if (!/^\d+$/.test(entry.name)) continue;
      snapshot.threads.push({ tid: Number(entry.name),
        wchan: await boundedText(join(root, "task", entry.name, "wchan")) });
      if (snapshot.threads.length === 16) break;
    }
  } catch (error) {
    snapshot.threadsUnavailable = error.code ?? error.message;
  }
  return snapshot;
}

export function createStartupDiagnostics(child, executable, {
  now = () => performance.now(),
  platform = process.platform,
  snapshot = linuxStartupSnapshot,
} = {}) {
  const started = now();
  const elapsed = () => Math.round(now() - started);
  let stderr = "";
  let spawnError = null;
  let firstStderrMs = null;
  let port = null;
  let portReadyMs = null;
  const probes = [];
  child.on("error", error => { spawnError = error.message; });
  child.stderr.on("data", chunk => {
    firstStderrMs ??= elapsed();
    stderr = (stderr + String(chunk)).slice(-8192);
  });
  return {
    stderr() { return stderr; },
    stopped() { return spawnError !== null || child.exitCode !== null || child.signalCode !== null; },
    port(value) {
      port = value;
      if (value !== null) portReadyMs ??= elapsed();
    },
    async probe(url) {
      const startMs = elapsed();
      let result;
      try {
        const response = await fetch(url, { signal: AbortSignal.timeout(1000) });
        result = { status: response.status };
        // Preserve the original gate: readiness depends on headers, not body completion.
        return response.ok;
      } catch (error) {
        result = { error: error.message };
        return false;
      } finally {
        probes.push({ startMs, endMs: elapsed(), ...result });
        if (probes.length > 16) probes.shift();
      }
    },
    async failure() {
      // Capture facts at the deadline, before cleanup sends any signal.
      const result = { executable, pid: child.pid, exitCode: child.exitCode,
        signalCode: child.signalCode, spawnError, elapsedMs: elapsed(),
        firstStderrMs, portReadyMs, port, probes: [...probes], stderr };
      if (platform === "linux") {
        let timer;
        try {
          result.linux = await Promise.race([
            Promise.resolve().then(() => snapshot(child.pid)),
            new Promise(resolve => {
              timer = setTimeout(() => resolve({ unavailable: "snapshot_timeout" }), 250);
            }),
          ]);
        } catch (error) {
          result.linux = { unavailable: error.code ?? error.message };
        } finally {
          clearTimeout(timer);
        }
      }
      return `Chrome DevTools did not start: ${JSON.stringify(result)}`;
    },
  };
}
