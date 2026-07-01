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
builtin resources/capabilities/tools/*.yaml
optional TIMEM_CAPABILITIES_DIR overlay
        ↓ load at runtime
CapabilityRegistry
        ↓ render
Tool_capability.tool_catalog in prompt_0
        ↓ validate
parse model next_actions
        ↓ resolve binding
ExecutorTarget
        ↓ dispatch
builtin executor implementation
```

The manifest is the human-maintained source for:

- action id
- builtin binding
- model-facing prompt description
- JSON Schema style input IDL
- JSON Schema style output IDL
- required input fields derived from the input IDL
- any-of required groups from the `x-required-any` IDL extension
- conditional required fields from the `x-required-when` IDL extension
- enum field constraints derived from property `enum` values
- examples

The Rust executor still owns side-effect behavior, storage access, permissions,
and complex cross-field validation. A manifest can expose only capabilities with
an existing binding. Built-in tools must document both input and output schema;
runtime overlay tools may omit output schema while they are experimental.
The IDL is intentionally data, not Rust code: the same `input_schema` and
`output_schema` blocks are rendered into `Tool_capability.tool_catalog`, used by
generic runtime validation, and exposed by `capmgr op=load kind=tool`.

Complex built-in protocol rules and small built-in executors should live near
their capability family, not in the top-level turn loop. For example,
`agent_core::memmgr` owns `memmgr` operation validation and scratch-kind
normalization while `AgentCore` still owns the storage side effects during the
migration; `agent_core::capmgr` owns capability-manager operation dispatch over
the registry; `agent_core::shell_exec` owns Bash request validation, foreground
execution, background job persistence, and shell job polling while `AgentCore`
keeps user approval and turn-loop routing.

Executor binding resolution is centralized in `agent_core::executor`:

- manifest-backed `binding_type: builtin` becomes `ExecutorTarget::Builtin`
- manifest-backed `binding_type: command` becomes `ExecutorTarget::Command`
- actions outside the manifest remain `ExecutorTarget::Legacy` fallback while
  the migration is still in progress

Command-bound executor invocation also lives in `agent_core::executor`: it sends
the model action envelope to the overlay command as JSON stdin, applies the same
bounded timeout policy as other local command execution, and normalizes stdout,
stderr, exit status, and timeout into an action result.

Legacy fallback is not rendered into the prompt catalog. It exists only to keep
older transcripts and transitional tests executable until their capability
family is fully migrated.
Legacy memory/chat/scratch/context actions are treated as compatibility entry
points only: execution is bridged through the canonical `memmgr` path where
possible, so new behavior should be added to `memmgr` instead of adding another
legacy branch.

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
  `{"action": "...", "intent": "...", "input": {...}}`.
- Script stdout/stderr is captured as the action result and truncated to a
  bounded size.
- Execution timeout follows the action's `timeout_ms`, clamped to 1-15 seconds.
  Long-running work should still use a builtin/background executor.

### Skill

A skill is not directly executable. It is a self-contained method package that
teaches the model how to perform a class of work. Small skills may be one file;
large skills should be folders with a manifest and resources:

```text
skills/release_quality_gate/
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
  "intent": "Load the release quality skill checklist.",
  "input": {
    "op": "load",
    "kind": "skill",
    "id": "release_quality_gate"
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

## Iteration Rule

Move one capability family at a time:

1. Add or update manifest.
2. Generate prompt text from the manifest.
3. Validate model actions through the registry for generic IDL constraints
   such as required fields, any-of required groups, conditional required
   fields, and enum fields.
4. Resolve the manifest binding through `agent_core::executor`.
5. Dispatch only to an implemented binding.
6. Add unit tests for manifest loading, prompt generation, and executor target
   resolution.
7. Add integration tests for model output, action execution, and UI rendering
   when user-visible behavior changes.
