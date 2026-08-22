# Capability System

Timem's model-facing prompt and executor-facing action parser must share one
capability contract. The goal is to avoid three drifting copies of the same
protocol: static prompt prose, action validation code, and executor dispatch.

## Concepts

```text
Capability Registry
├─ Tools      executable actions the runtime can dispatch
├─ Skills     self-contained natural-language capability packages
└─ Resources  loadable files, notes, or skill sub-documents
```

### Tool

A tool is executable. The model can request it through the active response
protocol's action block, and the runtime must have a matching executor binding.

Current first-step implementation:

```text
builtin resources/capabilities/tools/{tool}.yaml + {tool}.rs
optional TIMEM_CAPABILITIES_DIR overlay
        ↓ load at runtime
CapabilityRegistry
        ↓ render by interaction mode
inline: builtin/overlay catalog in Static + persistent MCP update deltas
native: builtin/overlay/current MCP definitions in API tools only
        ↓ generic parse
parse model next_actions action/intent/args
        ↓ resolve binding
ExecutorTarget
        ↓ dispatch
paired builtin tool callback or overlay command
```

MCP tools are deliberately excluded from the inline Static catalog: a Session
can enable, disable, or reconnect an MCP server between requests. Initial
enablement and definition/instruction changes append canonical JSON catalogs to
ordinary persistent prompt deltas for inline rendering. In native mode those
inline-only slices are filtered from messages; current definitions form the MCP
portion of the provider API tool list, with server instructions attached to the
corresponding tool descriptions. Disabling MCP removes those definitions from
the next API tools field without injecting an enable/disable RUNTIME notice.
Historical inline deltas remain immutable and become visible again if the
session switches back to inline mode.
Prompt updates are keyed to the model-visible definitions rather than the raw
server configuration. Runtime-only changes such as transport, timeout, endpoint,
headers, credentials, or display metadata do not consume prompt context when the
callable names, descriptions, input schemas, and server instructions are unchanged.

The manifest is the human-maintained source for:

- action id
- builtin binding
- model-facing prompt description
- JSON Schema style input IDL
- JSON Schema style output IDL
- required input fields derived from the input IDL for registry/contract tests
- any-of required groups from the `x-required-any` IDL extension
- conditional required fields from the `x-required-when` IDL extension
- conditional any-of required groups from the `x-required-any-when` IDL
  extension
- enum field constraints derived from property `enum` values
- examples

Normal/background execution is part of the capability interface:

- Built-in tools can own specialized lifecycle semantics when needed. `run_bash`
  keeps a dedicated path because it includes approval policy and local shell
  safety checks.
- Command-bound registered tools run in normal mode by default. If their
  YAML declares `background` in `input_schema`, core may
  start the command as a background `tool_job`, persist its status under the
  runtime memory directory, and return a `job_id`.
- Background command-bound tools are checked or cancelled through
  `capmgr op=job_status|job_cancel`. Background and timed-out `run_bash` jobs
  are tracked by pid in the session running-job set. Core emits natural-language
  action evidence when a job starts or times out, one-time `RUNNING_JOB_UPDATE`
  prompt components when a tracked job exits, and a `RUNNING JOB LIST` snapshot
  after large context compaction. The model can inspect or stop those jobs with
  ordinary `run_bash` commands such as `ps -p <pid>` or `kill <pid>`. The shell
  UI does not manage those jobs.
- External or remote status waiting should use `run_bash` polling mode, not a
  normal `sleep && check` command. When `interval_ms` is present,
  `run_bash` repeatedly runs the command until it exits with code 0, the total
  `loop_timeout_ms` expires, or the active turn is cancelled. `once_timeout_ms`
  bounds each individual check command. The model owns the check command; core
  owns the fixed success condition, interval/timeout bounds, cancellation
  checks, bounded output, approval, audit, and the structured action result.
- Normal `run_bash` uses a positive model-provided `timeout_ms`. Core still owns
  the process and emits a structured host decision request after the
  long-command threshold, so a UI can render elapsed/remaining time and let the
  user keep waiting or stop waiting. Stopping waiting becomes a
  `cancelled_by_user` action result plus a `user_supplement` for the next model
  response.
- A model cannot opt a registered command tool into background execution unless
  that field is declared in the tool manifest. Manifest validation rejects the
  undeclared field before execution.

The Rust executor still owns side-effect behavior, storage access, permissions,
and complex cross-field validation. The top-level parser must not know concrete
tool options such as `command`, `query`, or `expected_version`; it only accepts
the model's `args` JSON object, ensures the action is registered, and applies
manifest-derived generic validation such as required fields, any-of groups,
conditional required fields, and enum values. Those manifest-level argument
errors become protocol repair before execution. Tool executors return natural
language action results for runtime semantics such as storage conflicts, SQL
safety failures, shell approval, missing files, timeouts, or invalid prompt
references. A manifest can expose only capabilities with an existing binding.
The `input_schema` is intentionally data, not Rust code: it drives generic
model-action validation and is exposed by `capmgr op=load kind=tool`. Tool
results remain executor-owned evidence described by `prompt_result`; manifests
do not declare an unused output contract. The static prompt receives a shorter
Markdown capability guide derived from the manifests, not a full schema dump.

Built-in tools live as capability packages under
`resources/capabilities/tools/`. Each package has a `{tool}.yaml` manifest and,
for compiled built-ins, a paired `{tool}.rs` callback implementation. The YAML
defines the action id, model-facing manual, executor binding, and manifest-level
input validation. The Rust callback owns concrete argument extraction,
execution, evidence shaping, and tool-specific runtime safety checks. The
`resources/capabilities/tools/registry.rs` file is the compiled builtin
callback registry. The top-level `AgentCore` turn loop should only resolve the
action through the manifest registry, call the builtin callback registry by
binding name, and handle shared audit/approval plumbing. It should not duplicate
concrete tool option parsing such as `command`, `query`, `expected_version`, or
`delta_ids`.

Executor binding resolution is centralized in `agent_core::executor`:

- manifest-backed `binding_type: builtin` becomes `ExecutorTarget::Builtin`
- manifest-backed `binding_type: command` becomes `ExecutorTarget::Command`
- actions outside the manifest are rejected as unsupported actions

Command-bound executor invocation also lives in `agent_core::executor`: it sends
the model action envelope to the overlay command as JSON stdin, applies the same
bounded timeout policy as other local command execution, and normalizes stdout,
stderr, exit status, and timeout into an action result.

There is no hidden compatibility action path. If the model asks for an action
that is not present in the manifest registry, the runtime emits a protocol repair
slice instead of executing it.

Supported tool bindings:

- `binding_type: builtin`: dispatches to a compiled executor binding such as
  `run_bash`, `memmgr`, `self_tool`, or `capmgr`.
- `binding_type: command`: dispatches to a command script inside the runtime
  overlay directory. `binding_name` must be a relative path such as
  `scripts/my_tool.sh`.

Runtime overlay directory layout:

```text
capabilities/
  tools/*.yaml
  skills/<skill_id>/skill.yaml
  skills/<skill_id>/<entry file>
```

Overlay manifests are loaded at process startup. They can update prompt/IDL
metadata without recompiling, but a restart is still required for the running
process to read changed files. Unknown builtin executor bindings fail startup.

Command binding protocol:

- Runtime starts `/bin/sh <binding_name>`.
- Runtime writes one JSON object to stdin:
  `{"tool_name": {"key": "value"}}`.
- Script stdout/stderr is captured as the action result and truncated to a
  bounded size.
- Execution timeout follows the action's positive `timeout_ms` without an upper
  clamp.
  Long-running work should still use a builtin/background executor.

### Skill

A skill is not directly executable. It is a self-contained method package that
teaches the model how to perform a class of work. Small skills may be one file;
large skills should be folders with a manifest and resources:

```text
skills/<skill_id>/
  skill.yaml
  instructions.md
  checklist.md
  templates/
  scripts/
```

Startup should load only skill headers. Full skill content should be loaded on
demand through `capmgr`.

### Resource

A resource is loadable context, such as a skill sub-document, scratch record,
workspace summary, or offloaded prompt context. Loading a resource does not imply
execution.

## `capmgr`

`capmgr` is the capability manager action. `load` is only one operation; do
not introduce a separate action just for loading.

Expected shape:

```json
{
  "capmgr": {
    "op": "load",
    "kind": "skill",
    "id": "skill_id"
  }
}
```

Current operations:

- `list`: list capability headers
- `load`: load a skill body or tool details into prompt context

Planned operations:

- `resource`: load skill sub-documents, scratch records, workspace summaries,
  and offloaded prompt context as first-class resources
- `unload`: remove loaded resource slices when supported
- `search`: search capability metadata

`capmgr` must not expose a capability id unless the backing executor or
loadable resource exists.

Concrete skills are loaded through overlays or examples, not compiled into
`agent_core` by default. For example, `examples/capabilities/skills` contains a
release-quality skill that can be loaded by pointing `TIMEM_CAPABILITIES_DIR` at
the example capability root.

## `self_tool`

`self_tool` exposes Timem runtime self-information and prompt-context cwd
control through three classes:

- `type=path`: use for questions about where Timem runtime resources are. The
  result returns the relevant known file and directory locations; the model may
  then use normal `run_bash` policy if file contents are actually needed.
- `type=cwd`: omit `new_path` to read the current prompt-context directory as
  `CWD: ...` without changing state. Set `new_path` to an absolute path or a
  path relative to the current prompt-context cwd to change it; on success the
  result contains `CWD changed to: ...`. Later `run_bash` and `readfile` actions
  use the canonical directory. Only a successful change adds
  `context_state.cwd` to the action finish event so hosts can synchronize their
  Session display.
- `type=params`: use for questions about how the current Timem runtime is
  configured. The result returns the relevant effective runtime values.

API keys, tokens, passwords, secrets, credentials, and similarly named env
values are excluded. `params` is an explicit allowlist and never dumps arbitrary
Session environment entries; Base URL userinfo, query, and fragment data are
redacted. `cwd` mutates prompt-context state only when `new_path` is supplied;
`cwd` without it, `path`, and `params` remain read-only.

Successful runtime configuration changes set one Core-side pending notice.
Regardless of how many fields the user changes before the next interaction,
the next model prompt receives exactly one SYSTEM component telling it to
retrieve runtime parameters again when needed.

Do not use `self_tool` for user memory, shell commands, project file edits, or
model service model calls. Those remain owned by `memmgr`, `run_bash`, and the
session runtime respectively. Future additions should stay within Timem runtime
self-state. New inspection information belongs under `path` or `params`; cwd
changes remain under `cwd` with `new_path`.

## Iteration Rule

Move one capability family at a time:

1. Add or update manifest.
2. Generate prompt text from the manifest.
3. Validate model actions through the registry for IDL constraints such as
   required fields, any-of required groups, conditional required fields, and
   enum fields.
4. Resolve the manifest binding through `agent_core::executor`.
5. Dispatch only to an implemented binding.
6. Add unit tests for manifest loading, prompt generation, and executor target
   resolution.
7. Add integration tests for model output, action execution, and UI rendering
   when user-visible behavior changes.
