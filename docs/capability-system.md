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

A tool is executable. The model can request it through `next_actions`, and the
runtime must have a matching executor binding.

Current first-step implementation:

```text
builtin resources/capabilities/tools/{tool}.yaml + {tool}.rs
optional TIMEM_CAPABILITIES_DIR overlay
        ↓ load at runtime
CapabilityRegistry
        ↓ render
{{TOOL_CATALOG}} / {{SKILL_HEADERS}} in the Markdown static prompt
        ↓ generic parse
parse model next_actions action/intent/args
        ↓ resolve binding
ExecutorTarget
        ↓ dispatch
paired builtin tool callback or overlay command
```

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

Foreground/background execution is part of the capability interface:

- Built-in tools can own specialized lifecycle semantics when needed. `run_bash`
  keeps a dedicated path because it includes approval policy and local shell
  safety checks.
- Command-bound registered tools run in the foreground by default. If their
  YAML declares `background` or `mode=background` in `input_schema`, core may
  start the command as a background `tool_job`, persist its status under the
  runtime memory directory, and return a `job_id`.
- Background command-bound tools are checked or cancelled through
  `tool_job_status`; background `run_bash` jobs use `shell_job_status` for the
  same status/cancel lifecycle. The shell UI does not manage those jobs. Core
  owns job ids, output/status files, process termination, polling, bounded
  readback, and action evidence.
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
Built-in tools must document both input and output schema; runtime overlay
tools may omit output schema while they are experimental.
The IDL is intentionally data, not Rust code: the same `input_schema` and
`output_schema` blocks drive generic runtime validation and are exposed by
`capmgr op=load kind=tool`. The static prompt receives a shorter Markdown
capability guide derived from the manifests, not a full schema dump.

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
  `run_bash`, `memmgr`, `shell_job_status`, or `capmgr`.
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
  `{"action": "...", "intent": "...", "args": {"key": "value"}}`.
- Script stdout/stderr is captured as the action result and truncated to a
  bounded size.
- Execution timeout follows the action's `timeout_ms`, clamped to 1-15 seconds.
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
  "action": "capmgr",
  "intent": "Load the needed skill body before using it.",
  "args": {
    "op": "load",
    "kind": "skill",
    "id": "skill_id"
  }
}
```

Current operations:

- `list`: list capability headers
- `load`: load a skill body or tool details into prompt context

`inspect` currently aliases `load` for implemented kinds.

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

`self_tool` exposes Timem runtime self-information to the model through the same
manifest and executor path as other builtin tools. It is intentionally narrow:

- `type=env, op=read|write`: current process env only; API key/token variables
  and secret/password-like variables are denied. Memory path env variables such
  as `TIMEM_DATA_DIR` and `TIMEM_SPACE` are startup-only and writes are denied.
- `type=mem_path, op=read`: current memory/audit paths.
- `type=about_me, op=read`: software name, version, author/contact, project/star
  info, summary, current process id, working directory, and executable path.

Do not use `self_tool` for user memory, shell commands, project file edits, or
provider model calls. Those remain owned by `memmgr`, `run_bash`, and the
session runtime respectively. Future additions should stay within Timem runtime
self-state, such as config inspection, workspace references, capability overlay
status, or recent diagnostics.

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
