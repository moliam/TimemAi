// Interface-only summaries derived from structured, already-redacted inputs.
// Never parse the human-readable argument string or infer execution outcomes.
export type FileSelector = { line_nr: number } | { byte_nr: number } | { match: string };
export type ReadFilePresentation = {
  path: string;
  starter?: FileSelector;
  ender?: FileSelector;
  tail: boolean;
  encoding?: string;
};
export type RunBashEditPresentation = { paths: string[] };
export type MemorySearchPresentation =
  | { kind: "search"; source: "raw_chat" | "scratch"; query: string }
  | { kind: "sql"; source: "durable" | "raw_chat" };
export type SelfToolPresentation =
  | { kind: "path" | "params" | "cwd" }
  | { kind: "change_cwd"; path: string };

const record = (value: unknown): value is Record<string, unknown> =>
  !!value && typeof value === "object" && !Array.isArray(value);
const integer = (value: unknown, min: number) =>
  typeof value === "number" && Number.isSafeInteger(value) && value >= min;
function selector(value: unknown): value is FileSelector {
  if (!record(value) || Object.keys(value).length !== 1) return false;
  return ("line_nr" in value && integer(value.line_nr, 1)) ||
    ("byte_nr" in value && integer(value.byte_nr, 0)) ||
    ("match" in value && typeof value.match === "string" && value.match.length > 0);
}

export function readFilePresentation(action: string, input: unknown): ReadFilePresentation | undefined {
  if (action !== "readfile" || !record(input) || typeof input.path !== "string" || !input.path.trim()) return;
  // max_bytes is retained only for historical activity compatibility. It is an
  // internal read budget, not useful primary-row information.
  const known = new Set(["path", "starter", "ender", "max_bytes", "tail_out", "encoding"]);
  // Unknown or malformed fields stay visible through the generic fallback.
  if (Object.keys(input).some(key => !known.has(key))) return;
  if (input.max_bytes !== undefined && (typeof input.max_bytes !== "number" || !integer(input.max_bytes, 1) || input.max_bytes > 32768)) return;
  if (input.starter !== undefined && !selector(input.starter)) return;
  if (input.ender !== undefined && !selector(input.ender)) return;
  if (input.tail_out !== undefined && typeof input.tail_out !== "boolean") return;
  if (input.encoding !== undefined && (typeof input.encoding !== "string" || !input.encoding.trim())) return;
  const starter = input.starter as FileSelector | undefined;
  const ender = input.ender as FileSelector | undefined;
  // Do not reorder an invalid range into a plausible successful read.
  if (starter && ender && (("line_nr" in starter && "line_nr" in ender && starter.line_nr > ender.line_nr) ||
    ("byte_nr" in starter && "byte_nr" in ender && starter.byte_nr > ender.byte_nr))) return;
  return { path: input.path, starter, ender, tail: input.tail_out === true,
    encoding: input.encoding as string | undefined };
}


export function runBashEditPresentation(
  action: string,
  input: unknown,
): RunBashEditPresentation | undefined {
  if (action !== "run_bash" || !record(input)) return;
  const paths = normalizeEditPaths(input.edit);
  if (!paths || paths.length === 0 || paths.some(path => !path.trim())) return;
  return { paths: paths.map(path => path.trim()) };
}

// The model sometimes stringify-declares the whole array (edit: "[]") or
// passes a single bare path. Parse the same shapes the runtime's
// json_string_list accepts, so a degenerate empty form never renders the call
// as an edit command with a path literally named "[]".
function normalizeEditPaths(edit: unknown): string[] | undefined {
  const raw = typeof edit === "string"
    ? parseStringifiedEditPaths(edit)
    : Array.isArray(edit) && edit.every((item) => typeof item === "string")
      ? edit.map((item) => item.trim())
      : undefined;
  if (!raw) return undefined;
  // 用户反馈：遍历下来没有真实、可定位的文件，就当作空——不渲染 edit、
  // 不把调用当成 edit 命令。展示层无法访问文件系统，用可定位形态近似：
  // 绝对路径、~、./ ../、含分隔符，或带扩展名的相对路径。
  const locatable = raw.filter(isLocatablePathShape);
  return locatable.length > 0 ? locatable : undefined;
}

function parseStringifiedEditPaths(text: string): string[] | undefined {
  const trimmed = text.trim();
  if (!trimmed) return undefined;
  try {
    const parsed: unknown = JSON.parse(trimmed);
    if (Array.isArray(parsed)) {
      if (!parsed.every((item) => typeof item === "string")) return undefined;
      return parsed.map((item) => item.trim());
    }
  } catch {
    // Plain path text, not JSON.
  }
  return [trimmed];
}

function isLocatablePathShape(path: string): boolean {
  if (!path) return false;
  return /^(?:[A-Za-z]:[\\/]|~(?:\/|\b)|\.{1,2}\/)/.test(path)
    || path.includes("/")
    || path.includes("\\")
    || /\.[^./\\]+(?:$|[\\/])/.test(path);
}


export function memorySearchPresentation(
  action: string,
  input: unknown,
): MemorySearchPresentation | undefined {
  if (action !== "memmgr" || !record(input)) return;
  if (input.op === "search" && (input.type === "raw_chat" || input.type === "scratch")
    && typeof input.search_text === "string") {
    const allowed = input.type === "raw_chat"
      ? new Set(["type", "op", "search_text", "scope", "session_id", "after_ms", "before_ms", "limit"])
      : new Set(["type", "op", "search_text", "limit"]);
    if (Object.keys(input).some(key => !allowed.has(key))) return;
    if (input.limit !== undefined && (typeof input.limit !== "number" || !integer(input.limit, 1) || input.limit > 50)) return;
    if (input.type === "raw_chat") {
      if (input.scope !== undefined && input.scope !== "current_session" && input.scope !== "session" && input.scope !== "global") return;
      if (input.session_id !== undefined && (input.scope !== "session" || typeof input.session_id !== "string" || !input.session_id.trim())) return;
      if (input.scope === "session" && (typeof input.session_id !== "string" || !input.session_id.trim())) return;
      if (input.after_ms !== undefined && !integer(input.after_ms, 0)) return;
      if (input.before_ms !== undefined && !integer(input.before_ms, 0)) return;
    }
    return { kind: "search", source: input.type, query: input.search_text };
  }
  if (input.op === "sql" && (input.type === "durable" || input.type === "raw_chat")
    && typeof input.sql === "string" && input.sql.trim()) {
    const allowed = new Set(["type", "op", "sql", "params", "limit"]);
    if (Object.keys(input).some(key => !allowed.has(key))) return;
    if (input.limit !== undefined && (typeof input.limit !== "number" || !integer(input.limit, 1) || input.limit > 200)) return;
    if (input.params !== undefined && (!Array.isArray(input.params) || input.params.some(value =>
      typeof value !== "string" && typeof value !== "number" && typeof value !== "boolean"))) return;
    return { kind: "sql", source: input.type };
  }
}

export function isMemorySearch(action: string, input: unknown): boolean {
  if (action !== "memmgr" || !record(input)) return false;
  return (input.op === "search" && (input.type === "raw_chat" || input.type === "scratch")) ||
    (input.op === "sql" && (input.type === "durable" || input.type === "raw_chat"));
}

export type TimerPresentation =
  | { kind: "new_once"; name?: string; prompt: string; afterMs?: number; at?: string }
  | { kind: "new_repeat"; name?: string; prompt: string; intervalMs: number; afterMs?: number; at?: string }
  | { kind: "new_trigger"; name?: string; prompt: string; triggerEvent: "session_idle" | "after_compression" }
  | { kind: "delete"; name: string };

const TIMER_KNOWN = new Set(["op", "prompt", "name", "mode", "trigger", "after_ms", "at", "interval_ms"]);

export function timerPresentation(
  action: string,
  input: unknown,
): TimerPresentation | undefined {
  if (action !== "timer" || !record(input) || typeof input.op !== "string") return;
  if (Object.keys(input).some(key => !TIMER_KNOWN.has(key))) return;
  const name =
    typeof input.name === "string" && input.name.trim() ? input.name.trim()
    : undefined;
  if (input.name !== undefined && name === undefined) return;
  if (input.prompt !== undefined && (typeof input.prompt !== "string" || !input.prompt.trim())) return;
  const parseMs = (value: unknown, min: number):
    { valid: true; ms?: number } | { valid: false } => {
    if (value === undefined) return { valid: true };
    if (typeof value !== "number" || !Number.isSafeInteger(value)
      || value < min || value > 3.2e12) return { valid: false };
    return { valid: true, ms: value };
  };
  const after = parseMs(input.after_ms, 0);
  if (!after.valid) return;
  const interval = parseMs(input.interval_ms, 1);
  if (!interval.valid) return;
  const afterMs = after.ms;
  const intervalMs = interval.ms;
  const at =
    input.at === undefined ? undefined
    : typeof input.at === "string" && !Number.isNaN(new Date(input.at).getTime()) ? input.at
    : undefined;
  if (input.at !== undefined && at === undefined) return;
  if (input.op === "del") {
    if (!name || input.prompt !== undefined || input.mode !== undefined
      || afterMs !== undefined || at !== undefined || intervalMs !== undefined) return;
    return { kind: "delete", name };
  }
  if (input.op !== "new" || !input.prompt) return;
  const mode = input.mode === undefined ? "once" : input.mode;
  if (mode === "trigger") {
    // Event-driven timers carry no calendar; the event kind is mandatory.
    if (afterMs !== undefined || at !== undefined || intervalMs !== undefined) return;
    const triggerEvent = input.trigger;
    if (triggerEvent !== "session_idle" && triggerEvent !== "after_compression") return;
    return { kind: "new_trigger", name, prompt: input.prompt, triggerEvent };
  }
  if ((afterMs === undefined) === (at === undefined)) return; // exactly one
  if (mode === "once") {
    if (intervalMs !== undefined) return;
    return { kind: "new_once", name, prompt: input.prompt, afterMs, at };
  }
  if (mode !== "repeat" || intervalMs === undefined) return;
  return { kind: "new_repeat", name, prompt: input.prompt, intervalMs, afterMs, at };
}

export function isTimerActivityInput(action: string, input: unknown): boolean {
  return timerPresentation(action, input) !== undefined;
}

/** Split a duration into human units for localized delay/every labels. */
export function timerDelayParts(ms: number):
  | { unit: "seconds" | "minutes"; value: number }
  | { unit: "hours"; hours: number; minutes: number }
  | { unit: "days"; days: number; hours: number } {
  const seconds = Math.max(0, Math.round(ms / 1000));
  if (seconds < 60) return { unit: "seconds", value: seconds };
  const minutes = seconds / 60;
  if (minutes < 60)
    return { unit: "minutes", value: Number.isInteger(minutes) ? minutes : Math.round(minutes * 10) / 10 };
  const totalMinutes = Math.round(minutes);
  const hours = Math.floor(totalMinutes / 60);
  if (hours < 24) return { unit: "hours", hours, minutes: totalMinutes % 60 };
  const days = Math.floor(hours / 24);
  return { unit: "days", days, hours: hours % 24 };
}

/** Local wall-clock rendering for absolute fire instants. */
export function formatTimerAt(iso: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  const p2 = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${p2(date.getMonth() + 1)}-${p2(date.getDate())} `
    + `${p2(date.getHours())}:${p2(date.getMinutes())}:${p2(date.getSeconds())}`;
}

export function selfToolPresentation(
  action: string,
  input: unknown,
): SelfToolPresentation | undefined {
  if (action !== "self_tool" || !record(input)) return;
  if (Object.keys(input).some(key => key !== "type" && key !== "new_path")) return;
  if (input.type === "path" || input.type === "params") {
    if (input.new_path !== undefined) return;
    return { kind: input.type };
  }
  if (input.type !== "cwd") return;
  if (input.new_path === undefined) return { kind: "cwd" };
  if (typeof input.new_path !== "string" || !input.new_path.trim()) return;
  return { kind: "change_cwd", path: input.new_path.trim() };
}

export type MailboxPresentation =
  | { kind: "send"; to: string; msg: string }
  | { kind: "read"; mailId: string }
  | { kind: "search"; query: string }
  | { kind: "list" };

const MAILBOX_KNOWN = new Set([
  "action", "to", "msg", "query", "mail_id", "count",
  "queue_timeout", "resp_timeout", "max_wait",
]);

export function mailboxPresentation(
  action: string,
  input: unknown,
): MailboxPresentation | undefined {
  if (action !== "mailbox" || !record(input)) return;
  if (Object.keys(input).some(key => !MAILBOX_KNOWN.has(key))) return;
  if (input.count !== undefined
    && (typeof input.count !== "number" || !Number.isSafeInteger(input.count)
      || input.count < 1 || input.count > 100)) return;
  const op = input.action === undefined ? "send" : input.action;
  if (op === "send") {
    if (typeof input.to !== "string" || !input.to.trim()) return;
    if (typeof input.msg !== "string" || !input.msg.trim()) return;
    if (input.query !== undefined || input.mail_id !== undefined
      || input.count !== undefined) return;
    return { kind: "send", to: input.to.trim(), msg: input.msg };
  }
  if (op === "read") {
    if (typeof input.mail_id !== "string" || !input.mail_id.trim()) return;
    if (input.to !== undefined || input.msg !== undefined
      || input.query !== undefined || input.count !== undefined) return;
    return { kind: "read", mailId: input.mail_id.trim() };
  }
  if (op === "search") {
    if (typeof input.query !== "string" || !input.query.trim()) return;
    if (input.to !== undefined || input.msg !== undefined
      || input.mail_id !== undefined) return;
    return { kind: "search", query: input.query };
  }
  if (op === "list") {
    if (input.to !== undefined || input.msg !== undefined
      || input.query !== undefined || input.mail_id !== undefined) return;
    return { kind: "list" };
  }
  return;
}

export type FileEditOp = "created" | "modified" | "deleted" | "unchanged" | "skipped";
export type FileEditPresentation = {
  path: string;
  op: FileEditOp;
  diff: string | null;
  added_lines: number;
  removed_lines: number;
  truncated: boolean;
  skip_reason: string | null;
  /** Size at capture time; the only evidence a non-text edit can show. */
  size_bytes?: number | null;
};
export type FileEditsPresentation = FileEditPresentation[];

const FILE_EDIT_OPS = new Set<FileEditOp>([
  "created",
  "modified",
  "deleted",
  "unchanged",
  "skipped",
]);
const FILE_EDIT_SKIP_REASONS = new Set([
  "binary",
  "directory",
  "too_large",
  "missing",
  "unreadable",
  "not_captured",
  "budget_exhausted",
]);
// Cards that show no human-actionable change stay out of the panel: a no-op
// write ("unchanged") and the legacy "skipped" marker, which the runtime no
// longer emits (every real capture now carries created/modified/deleted plus a
// kind tag such as Binary or 文件夹).
const MAX_FILE_EDIT_ENTRIES = 16;
const MAX_FILE_EDIT_DIFF_CHARS = 40_000;

// Parses the runtime-provided `edits` array from the core.action finish
// payload. Strict all-or-nothing: any malformed entry rejects the whole
// payload so the generic fallback never shows half-trusted diff data.
export function fileEditsPresentation(
  value: unknown,
  options?: { maxEntries?: number },
): FileEditsPresentation | undefined {
  const maxEntries = options?.maxEntries ?? MAX_FILE_EDIT_ENTRIES;
  if (!Array.isArray(value) || value.length === 0 || value.length > maxEntries) return;
  const edits: FileEditsPresentation = [];
  for (const item of value) {
    if (!record(item)) return;
    if (typeof item.path !== "string" || !item.path.trim() || item.path.length > 512) return;
    if (typeof item.op !== "string" || !FILE_EDIT_OPS.has(item.op as FileEditOp)) return;
    if (item.diff !== null && typeof item.diff !== "string") return;
    if (typeof item.diff === "string" && item.diff.length > MAX_FILE_EDIT_DIFF_CHARS) return;
    const added = item.added_lines;
    if (typeof added !== "number" || !Number.isSafeInteger(added) || added < 0 || added > 1_000_000) return;
    const removed = item.removed_lines;
    if (typeof removed !== "number" || !Number.isSafeInteger(removed) || removed < 0 || removed > 1_000_000) return;
    if (typeof item.truncated !== "boolean") return;
    if (item.skip_reason !== null
      && (typeof item.skip_reason !== "string" || !FILE_EDIT_SKIP_REASONS.has(item.skip_reason))) return;
    if (item.size_bytes !== undefined
      && item.size_bytes !== null
      && (typeof item.size_bytes !== "number" || !Number.isSafeInteger(item.size_bytes) || item.size_bytes < 0)) return;
    if (Object.keys(item).some(key => !["path", "op", "diff", "added_lines", "removed_lines", "truncated", "skip_reason", "size_bytes"].includes(key))) return;
    if (item.op === "unchanged" || item.op === "skipped") continue;
    edits.push({
      path: item.path,
      op: item.op as FileEditOp,
      diff: item.diff,
      added_lines: added,
      removed_lines: removed,
      truncated: item.truncated,
      skip_reason: item.skip_reason,
      ...(typeof item.size_bytes === "number" ? { size_bytes: item.size_bytes } : {}),
    });
  }
  return edits;
}

export type DiffRowKind = "hunk" | "meta" | "add" | "del" | "ctx";
export type DiffRow = {
  kind: DiffRowKind;
  oldNo?: number;
  newNo?: number;
  text: string;
};

// Parses bounded unified diff text (as produced by the runtime) into table
// rows. Line numbers follow the standard 1-based numbering with 0 starts for
// empty ranges; unknown lines degrade to dimmed meta rows.
export function parseDiffRows(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldNo = 0;
  let newNo = 0;
  for (const line of diff.split("\n")) {
    if (line.startsWith("@@")) {
      const match = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/.exec(line);
      if (match) {
        oldNo = Number(match[1]);
        newNo = Number(match[3]);
        rows.push({ kind: "hunk", text: line });
        continue;
      }
      rows.push({ kind: "meta", text: line });
      continue;
    }
    if (line.startsWith("--- ") || line.startsWith("+++ ") || line.startsWith("\\ ")) {
      rows.push({ kind: "meta", text: line });
      continue;
    }
    if (line.startsWith("+")) {
      rows.push({ kind: "add", newNo: newNo++, text: line.slice(1) });
      continue;
    }
    if (line.startsWith("-")) {
      rows.push({ kind: "del", oldNo: oldNo++, text: line.slice(1) });
      continue;
    }
    if (line.startsWith(" ")) {
      rows.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, text: line.slice(1) });
      continue;
    }
    if (line === "") continue;
    rows.push({ kind: "meta", text: line });
  }
  return rows;
}
