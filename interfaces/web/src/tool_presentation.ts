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
  const edit = input.edit;
  const paths = typeof edit === "string"
    ? [edit]
    : Array.isArray(edit) && edit.every(path => typeof path === "string")
      ? edit
      : undefined;
  if (!paths || paths.length === 0 || paths.some(path => !path.trim())) return;
  return { paths: paths.map(path => path.trim()) };
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
