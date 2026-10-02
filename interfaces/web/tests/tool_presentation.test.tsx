import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { isMemorySearch, memorySearchPresentation, readFilePresentation, runBashEditPresentation, selfToolPresentation } from "../src/tool_presentation";
import { MemoryIcon, MemorySearchIcon, MemorySearchInvocation, ReadFileIcon, ReadFileInvocation, readFileRangeLabel, RunBashEditIcon, RunBashEditInvocation, SelfToolIcon, SelfToolInvocation } from "../src/tool_invocation";
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
  ] as const)("renders self_tool %j as a readable Info action", (input, expected, copyKey) => {
    const operation = selfToolPresentation("self_tool", input)!;
    expect(operation).toEqual(expected);
    const html = renderToStaticMarkup(<><SelfToolIcon /><SelfToolInvocation operation={operation} /></>);
    expect(html).toContain("lucide-info");
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
