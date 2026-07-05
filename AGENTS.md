# Timem development guardrails

This file records project-level principles that future agents must read before
making changes. Keep it short, concrete, and enforceable.

## Delivery discipline

- Develop in the source project first. For split/public packages, modify
  `timem_ios` or the canonical source first, verify, then sync outward.
- Do not push remote changes until local implementation, tests, docs, and
  release-quality checks are complete.
- Every small feature needs an architecture/requirement consistency check:
  confirm it matches the user's actual requirement, has reasonable UX, and
  covers normal, edge, and awkward user flows.
- New functionality must be tested end to end, not only by isolated helper
  tests. Include multi-turn flows when the feature participates in agent
  interaction.
- Tests should challenge behavior with malformed model output, boundary sizes,
  cancellation, retries, and realistic terminal interaction where relevant.
- Do not use real user private facts, paths, keys, internal URLs, or personal
  conversations as test fixtures or public docs.

## Architecture boundaries

- The intended runtime chain is:
  `host UI -> agent_core -> provider -> LLM`.
- `timem_shell` is a terminal host/UI. It owns terminal input, menus, rendering,
  shell-only slash commands, and local process startup.
- `timem_shell` must not implement provider HTTP, model transport, provider
  response interpretation, cache-control protocol, or model response protocol
  parsing.
- `timem_shell` must not execute model-requested tools such as `run_bash`.
  Tool execution, action evidence, capability validation, and tool audit belong
  to `agent_core`/executor. Shell only renders action topics, shows approval UI,
  collects user decisions, and signals cancellation.
- `agent_core` owns reusable agent state, turn execution, prompt/context
  management, model response protocol parsing, capability execution,
  provider/cache planning, provider transport, audit semantics, and structured
  topic/request output.
- Provider-specific HTTP/payload logic may live inside `agent_core` for now. It
  can later move to a provider crate, but it must not move into shell UI code.
- Core/UI communication is structured. Core provides semantic structures,
  reports, topic events, request topics, and outcomes. UI renders those
  structures in its own style.
- UI is allowed to understand public protocol fields. The goal is not a dumb UI;
  the goal is one shared core/UI contract so Rust shell, Swift, web, and IPC
  hosts do not each invent different meanings.
- Do not collapse semantically different strings into generic `text`. Preserve
  meanings such as final answer, job progress, action intent, command evidence,
  diagnostic reason, and status metadata.
- Core may return strings when the string itself is data, such as model
  `final_answer`, provider message, path, id, or diagnostic reason. Core should
  not embed terminal-specific localized copy, ANSI styling, or UI layout.
- Topic events are owned by core while callbacks run. If a host wants to render
  later, cross a thread/process boundary, or store events for tests/logs, it
  must copy/clone the event or the needed fields before the callback returns.

## Prompt and protocol rules

- Runtime must not try to understand natural-language user semantics with
  hard-coded case logic. If behavior depends on meaning, expose evidence/tools
  and let the model reason.
- Prompt is a protocol between runtime and model. Be precise about fields that
  runtime parses, and concise about natural-language guidance.
- The model is not a traditional backward-compatible client. When protocol is
  intentionally changed before public release, update prompt, parser, tests, and
  examples together instead of keeping old compatibility paths.
- Capability descriptions should be useful to the model in natural language,
  while executor validation remains structured and authoritative.
- Built-in tool capability packages are paired files under
  `resources/capabilities/tools/`: `{tool}.yaml` is the model/executor
  interface and manifest-derived validation source, and `{tool}.rs` is the
  concrete tool callback implementation. `agent_core` may know builtin action
  names for dispatch, but concrete tool parameter parsing and execution belong
  to the tool implementation, not the top-level turn loop.
- Do not leak runtime internals unless the user asks for implementation details.
  In normal answers, avoid exposing internal memory structures, tool caps, or
  protocol mechanics.

## Shell UX rules

- `Ctrl+C` and `Esc` mean cancel the current user activity. During model work,
  `Ctrl+C` also cancels the current thinking/model turn. A single accidental
  `Ctrl+C` should not abruptly exit the whole process.
- Terminal rendering must be tested with multi-line input, paste, CJK text,
  malformed paste labels, retries, long status text, and cancellation paths.
- Shell-only UI conveniences may live in `timem_shell`, but reusable behavior
  that web/iOS would need belongs in `agent_core`.
