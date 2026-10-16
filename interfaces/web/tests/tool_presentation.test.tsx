import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { isMemorySearch, memorySearchPresentation, readFilePresentation, runBashEditPresentation, selfToolPresentation, timerPresentation, timerDelayParts, formatTimerAt } from "../src/tool_presentation";
import { MemoryIcon, MemorySearchIcon, MemorySearchInvocation, ReadFileIcon, ReadFileInvocation, readFileRangeLabel, RunBashEditIcon, RunBashEditInvocation, SelfToolIcon, SelfToolInvocation, TimerToolIcon, TimerInvocation } from "../src/tool_invocation";
import { activityFromTopic } from "../src/view_model";
import type { CoreTopicEvent } from "../src/protocol";
import { t } from "../src/i18n";

const input = { ender: { line_nr: 15244 }, path: "applications/timem/tests/unit/web_host_tests.rs", starter: { line_nr: 15190 } };
describe("structured built-in tool presentation", () => {
  it("uses structured selectors independently of parameter order", () => {
    const file = readFilePresentation("readfile", input)!;
    expect(file).toMatchObject({ path: input.path, starter: {line_nr:15190}, ender:{line_nr:15244}, tail:false });
    expect(readFileRangeLabel(file)).toContain("15190–15244");
    const html = renderToStaticMarkup(<><ReadFileIcon /><ReadFileInvocation file={file} /></>);
    expect(html).toContain("lucide-square-text");
    expect(html).toContain("file-tool-directory");
    expect(html).toContain("web_host_tests.rs");
    expect(html).not.toContain("line_nr");
  });
  it.each([
    {}, {starter:{line_nr:1}}, {ender:{line_nr:5}},
    {starter:{byte_nr:0},ender:{byte_nr:4095}},
    {starter:{match:"function"},ender:{match:"}"}},
    {starter:{line_nr:2},ender:{byte_nr:40}},
    {tail_out:true,encoding:"utf-16le"},
  ])("represents supported selector and retention modes: %j", extra => {
    const file=readFilePresentation("readfile",{path:"a.txt",...extra});
    expect(file).toBeDefined();
    expect(renderToStaticMarkup(<ReadFileInvocation file={file!} />)).toContain("a.txt");
  });
  it.each([
    null, [], {path:""}, {path:"a",extra:1}, {path:"a",starter:{line_nr:0}},
    {path:"a",starter:{line_nr:1,byte_nr:0}}, {path:"a",ender:{byte_nr:-1}},
    {path:"a",ender:{match:""}}, {path:"a",max_bytes:0}, {path:"a",max_bytes:32769}, {path:"a",max_bytes:"1"},
    {path:"a",tail_out:"true"}, {path:"a",encoding:0},
    {path:"a",starter:{line_nr:20},ender:{line_nr:10}},
  ])("falls back for invalid or unknown inputs: %j", value => {
    expect(readFilePresentation("readfile",value)).toBeUndefined();
  });
  it("keeps legacy internal read budgets out of the primary summary", () => {
    const legacyInput = {
      path: "src/main.tsx",
      starter: { line_nr: 1 },
      ender: { line_nr: 285 },
      max_bytes: 32768,
    };
    const file = readFilePresentation("readfile", legacyInput)!;
    const html = renderToStaticMarkup(<ReadFileInvocation file={file} />);
    expect(file).not.toHaveProperty("maxBytes");
    expect(html).toContain("1–285");
    expect(html).not.toContain("32768");

    const activity = activityFromTopic({
      session_id: "s",
      topic: { name: "core.action" },
      payload: { action: "readfile", status: "completed", input: legacyInput },
    } as CoreTopicEvent)!;
    expect(activity.readfile).toEqual(file);
    expect(activity.detail).toContain("max_bytes=32768");
  });

  it("does not reinterpret third-party tools", () => {
    expect(readFilePresentation("vendor.readfile",input)).toBeUndefined();
  });
  it("keeps byte zero and match semantics distinct from line ranges", () => {
    expect(readFileRangeLabel(readFilePresentation("readfile",{path:"a",starter:{byte_nr:0},ender:{byte_nr:99}})!)).toContain("0–99");
    expect(readFileRangeLabel(readFilePresentation("readfile",{path:"a",starter:{match:"needle"},ender:{line_nr:10}})!)).toContain("needle");
  });
  it("preserves full redacted arguments and authoritative status", () => {
    const event={session_id:"s",topic:{name:"core.action"},payload:{action:"readfile",status:"failed",input:{...input,path:"a?token=private-value",ender:{match:"password=private-match"}}}} as CoreTopicEvent;
    const activity=activityFromTopic(event)!;
    expect(activity.tool_status).toBe("failed");
    expect(activity.detail).toContain("starter=");
    expect(activity.readfile).toBeDefined();
    expect(JSON.stringify(activity)).not.toContain("private-value");
    expect(JSON.stringify(activity)).not.toContain("private-match");
  });
  it("escapes markup-looking paths and supports Windows separators", () => {
    const file=readFilePresentation("readfile",{path:"C:\\work\\<img>.rs"})!;
    const html=renderToStaticMarkup(<ReadFileInvocation file={file} />);
    expect(html).toContain("&lt;img&gt;.rs");
    expect(html).not.toContain("<img>");
    expect(html).toContain("file-tool-directory");
  });
});

describe("structured run_bash edit presentation", () => {
  it.each([
    ["src/main.tsx", ["src/main.tsx"]],
    [["src/main.tsx", "src/styles.css"], ["src/main.tsx", "src/styles.css"]],
  ])("accepts a declared edit string or path list: %j", (edit, paths) => {
    expect(runBashEditPresentation("run_bash", { cmd: "printf ignored", edit }))
      .toEqual({ paths });
  });

  it.each([
    ["run_powershell", { edit: "a.txt" }],
    ["vendor.run_bash", { edit: "a.txt" }],
    ["run_bash", null],
    ["run_bash", {}],
    ["run_bash", { edit: "" }],
    ["run_bash", { edit: [] }],
    ["run_bash", { edit: ["a.txt", ""] }],
    ["run_bash", { edit: ["a.txt", 1] }],
    ["run_bash", { cmd: "touch inferred.txt" }],
  ])("falls back instead of guessing for %s %j", (action, value) => {
    expect(runBashEditPresentation(action as string, value)).toBeUndefined();
  });

  it("renders PenLine followed by the declared comma-separated paths", () => {
    const edit = runBashEditPresentation("run_bash", {
      cmd: "ignored in the compact summary",
      edit: ["src/<main>.tsx", "src/styles.css"],
    })!;
    const html = renderToStaticMarkup(<><RunBashEditIcon /><RunBashEditInvocation edit={edit} /></>);
    expect(html).toContain("lucide-pen-line");
    expect(html).not.toContain("lucide-file-pen");
    expect(html).toContain("src/&lt;main&gt;.tsx, src/styles.css");
    expect(html).not.toContain("ignored in the compact summary");
  });

  it("uses the already-redacted structured edit paths", () => {
    const event = {
      session_id: "s",
      topic: { name: "core.action" },
      payload: {
        action: "run_bash",
        status: "completed",
        input: { cmd: "true", edit: "src/a.ts?token=private-value" },
      },
    } as CoreTopicEvent;
    const activity = activityFromTopic(event)!;
    expect(activity.run_bash_edit).toBeDefined();
    expect(JSON.stringify(activity.run_bash_edit)).not.toContain("private-value");
  });

  it("maps only the structured edit field while preserving command details and status", () => {
    const event = {
      session_id: "s",
      topic: { name: "core.action" },
      payload: {
        action: "run_bash",
        status: "completed",
        input: { cmd: "printf command-detail", edit: ["src/a.ts", "src/b.ts"] },
      },
    } as CoreTopicEvent;
    const activity = activityFromTopic(event)!;
    expect(activity).toMatchObject({
      tool_name: "run_bash",
      tool_status: "completed",
      code: "printf command-detail",
      run_bash_edit: { paths: ["src/a.ts", "src/b.ts"] },
    });
  });
});

describe("readable memory and self-tool summaries", () => {
  it("renders a memory search summary without implementation parameters", () => {
    const input = {
      limit: 5,
      op: "search",
      scope: "current_session",
      search_text: "模型接入点",
      type: "raw_chat",
    };
    const search = memorySearchPresentation("memmgr", input)!;
    expect(search).toEqual({ kind: "search", source: "raw_chat", query: "模型接入点" });
    const html = renderToStaticMarkup(<MemorySearchInvocation search={search} />);
    expect(html).toContain("在对话记录中搜索“模型接入点”");
    expect(html).not.toMatch(/limit|scope|current_session|op=/);

    const activity = activityFromTopic({
      session_id: "s",
      topic: { name: "core.action" },
      payload: { action: "memmgr", status: "completed", input },
    } as CoreTopicEvent)!;
    expect(activity.memory_search).toEqual(search);
    expect(activity.detail).toContain("limit=5");
    expect(activity.detail).toContain('op="search"');
    expect(activity.detail).toContain('scope="current_session"');
    expect(activity.detail).toContain('type="raw_chat"');
  });

  it("falls back for malformed, variant-incompatible, or third-party memory inputs", () => {
    expect(memorySearchPresentation("vendor.memmgr", { type: "raw_chat", op: "search", search_text: "x" })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "raw_chat", op: "search", search_text: "x", unknown: true })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "scratch", op: "search", search_text: "x", scope: "global" })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "raw_chat", op: "search", search_text: "x", scope: "session" })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "raw_chat", op: "search", search_text: "x", limit: 0 })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "scratch", op: "search", search_text: "x", limit: 51 })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "durable", op: "sql", sql: "SELECT 1", limit: 201 })).toBeUndefined();
    expect(memorySearchPresentation("memmgr", { type: "durable", op: "sql", sql: "SELECT 1", params: [{}] })).toBeUndefined();

    const activity = activityFromTopic({
      session_id: "s",
      topic: { name: "core.action" },
      payload: { action: "memmgr", status: "completed", input: { type: "raw_chat", op: "search", search_text: "x", unknown: true } },
    } as CoreTopicEvent)!;
    expect(activity.memory_search).toBeUndefined();
    expect(activity.detail).toContain("unknown=true");
  });

  it.each([
    [{ type: "path" }, { kind: "path" }, "tools.inspectRuntimePaths"],
    [{ type: "params" }, { kind: "params" }, "tools.inspectRuntimeSettings"],
    [{ type: "cwd" }, { kind: "cwd" }, "tools.inspectCurrentDirectory"],
    [{ type: "cwd", new_path: "/work/project" }, { kind: "change_cwd", path: "/work/project" }, "tools.changeDirectoryTo"],
  ] as const)("renders self_tool %j as a readable Eye action", (input, expected, copyKey) => {
    const operation = selfToolPresentation("self_tool", input)!;
    expect(operation).toEqual(expected);
    const html = renderToStaticMarkup(<><SelfToolIcon /><SelfToolInvocation operation={operation} /></>);
    expect(html).toContain("lucide-eye");
    expect(html).toContain(t(copyKey));
    if (operation.kind === "change_cwd") expect(html).toContain("<code>/work/project</code>");
    expect(html).not.toContain('type=&quot;');
  });

  it("falls back instead of guessing malformed self_tool inputs", () => {
    expect(selfToolPresentation("vendor.self_tool", { type: "params" })).toBeUndefined();
    expect(selfToolPresentation("self_tool", { type: "path", new_path: "." })).toBeUndefined();
    expect(selfToolPresentation("self_tool", { type: "cwd", new_path: "" })).toBeUndefined();
    expect(selfToolPresentation("self_tool", { type: "unknown" })).toBeUndefined();
  });
});

describe("generic tool argument projection", () => {
  it("projects memo arguments once as detail without synthesizing code", () => {
    const text = "修复旧接入点并验证保存重读";
    const activity = activityFromTopic({
      session_id: "s",
      topic: { name: "core.action" },
      payload: {
        action: "memo",
        status: "completed",
        input: { op: "create", text },
      },
    } as CoreTopicEvent)!;

    expect(activity.tool_name).toBe("memo");
    expect(activity.code).toBeUndefined();
    expect(activity.detail).toContain('op="create"');
    expect(activity.detail).toContain(`text="${text}"`);
    expect(activity.detail?.match(/text=/g)).toHaveLength(1);
  });
});

describe("memory search icon", () => {
  it.each([["raw_chat","search"],["scratch","search"],["durable","sql"],["raw_chat","sql"]])("recognizes %s/%s", (type,op) => {
    expect(isMemorySearch("memmgr",{type,op})).toBe(true);
  });
  it.each(["write","insert","update","upsert","delete","read","schema"])("does not label %s as search", op => {
    expect(isMemorySearch("memmgr",{type:"durable",op})).toBe(false);
  });
  it("does not infer search from text or tool-name substrings", () => {
    expect(isMemorySearch("vendor.memmgr",{type:"raw_chat",op:"search"})).toBe(false);
    expect(isMemorySearch("memmgr",{type:"scratch",op:"delete",search_text:"search"})).toBe(false);
    expect(isMemorySearch("memmgr",{type:"unknown",op:"search"})).toBe(false);
    expect(renderToStaticMarkup(<MemorySearchIcon />)).toContain("lucide-database-search");
    expect(renderToStaticMarkup(<MemoryIcon />)).toContain("lucide-database");
    expect(renderToStaticMarkup(<MemoryIcon />)).not.toContain("lucide-database-search");
  });
});

describe("readable timer lifecycle presentation", () => {
  it("renders a once timer with a relative delay in user language", () => {
    const timer = timerPresentation("timer", {
      op: "new", name: "build-check", prompt: "检查 CI 构建结果",
      mode: "once", after_ms: 600000,
    })!;
    expect(timer.kind).toBe("new_once");
    const html = renderToStaticMarkup(<><TimerToolIcon /><TimerInvocation timer={timer} /></>);
    expect(html).toContain("lucide-calendar-clock");
    expect(html).toContain("新建定时");
    expect(html).toContain("build-check");
    expect(html).toContain("单次");
    expect(html).toContain("10 分钟后");
    expect(html).toContain("任务");
    expect(html).toContain("检查 CI 构建结果");
    expect(html).not.toContain("after_ms");
    expect(html).not.toContain("600000");
  });

  it("renders a repeat timer with the interval and an absolute once with local time", () => {
    const repeat = timerPresentation("timer", {
      op: "new", prompt: "日报", mode: "repeat", interval_ms: 1800000, after_ms: 60000,
    })!;
    expect(repeat.kind).toBe("new_repeat");
    const repeatHtml = renderToStaticMarkup(<TimerInvocation timer={repeat} />);
    expect(repeatHtml).toContain("循环");
    expect(repeatHtml).toContain("每 30 分钟");
    expect(repeatHtml).not.toContain("interval_ms");

    const at = timerPresentation("timer", {
      op: "new", prompt: "早会", mode: "once", at: "2026-10-16T01:30:00Z",
    })!;
    const atHtml = renderToStaticMarkup(<TimerInvocation timer={at} />);
    expect(atHtml).toContain("于 2026-");
    expect(atHtml).toContain("任务");
  });

  it("renders a delete row naming the timer", () => {
    const del = timerPresentation("timer", { op: "del", name: "build-check" })!;
    expect(del).toEqual({ kind: "delete", name: "build-check" });
    const html = renderToStaticMarkup(<TimerInvocation timer={del} />);
    expect(html).toContain("删除定时");
    expect(html).toContain("build-check");
  });

  it.each([
    { op: "new" },                                   // no prompt
    { op: "new", prompt: "p" },                      // no schedule (exactly one required)
    { op: "new", prompt: "p", after_ms: 1, at: "2026-01-01T00:00:00Z" }, // both schedules
    { op: "new", prompt: "p", mode: "once", after_ms: 1, interval_ms: 2 }, // once+interval
    { op: "new", prompt: "p", mode: "repeat", after_ms: 1 },               // repeat without interval
    { op: "new", prompt: "p", after_ms: -1 },        // negative delay
    { op: "new", prompt: "p", at: "not-a-date" },    // invalid absolute
    { op: "new", prompt: "p", after_ms: 1, extra: 1 }, // unknown field
    { op: "del" },                                   // delete without name
    { op: "del", name: "x", prompt: "y" },           // delete with create fields
    { op: "other", prompt: "p", after_ms: 1 },       // unknown op
  ])("falls back instead of guessing malformed timer inputs: %j", (value) => {
    expect(timerPresentation("timer", value)).toBeUndefined();
  });

  it("splits durations into human units for delay and every labels", () => {
    expect(timerDelayParts(45000)).toEqual({ unit: "seconds", value: 45 });
    expect(timerDelayParts(600000)).toEqual({ unit: "minutes", value: 10 });
    expect(timerDelayParts(750000)).toEqual({ unit: "minutes", value: 12.5 });
    expect(timerDelayParts(3720000)).toEqual({ unit: "hours", hours: 1, minutes: 2 });
    expect(timerDelayParts(90000000)).toEqual({ unit: "days", days: 1, hours: 1 });
  });

  it("formats absolute fire instants as local wall-clock", () => {
    const text = formatTimerAt("2026-10-16T01:02:03Z");
    expect(text).toMatch(/^2026-10-1[56] \d{2}:\d{2}:\d{2}$/);
    expect(formatTimerAt("garbage")).toBe("garbage");
  });
});
