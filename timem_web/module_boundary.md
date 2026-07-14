# timem_web Module Boundary

`timem_web` is a local host adapter. It binds a loopback HTTP/WebSocket server,
owns browser authentication, maps browser commands to `agent_core` session
worker handles, and forwards the canonical core topic wire payload unchanged.

It may contain:

- HTTP/WebSocket lifecycle, localhost port selection, and per-process access tokens.
- Session worker orchestration and browser-facing snapshots.
- Per-session runtime-profile collection and safe projection. The host copies
  global defaults when a Session is created, keeps secrets server-side, and
  gives every context/worker belonging to that Session the same profile. It
  does not reinterpret provider or protocol semantics.
- Static asset serving and browser transport backpressure/reconnect behavior.
- Per-session browser upload storage and attachment metadata. Uploaded bytes
  remain host-local; the host only contributes their paths as session context.
- Host-only settings and UI command validation.

A Web Session is the configuration ownership boundary and contains explicit
`contexts[]` and `workers[]` registries. All workers in the Session inherit its
server-side environment/profile. A Context owns prompt/workspace state such as
cwd and references its workers. A Worker belongs to exactly one Session and one
Context, may reference a parent worker, and owns one core execution loop. The
current Web UI creates one default Context and primary Worker, while the host
creation path can attach child workers to new contexts without moving profile
ownership down to a worker. A different Session may use a different profile.
One Context currently has exactly one worker; subtask concurrency is created as
a new Context plus a new Worker so mutable prompt state cannot fork silently.

Core topic routing is keyed by the cross-language scope tuple
`session_id/context_id/worker_id`. Session-level commands currently target the
primary worker. A child worker finishing must not finish the primary chat turn,
and Session state is derived from all worker states rather than whichever event
arrived last.

Only the primary worker has a user-facing chat channel. Child-worker free talk,
actions, and requests are rendered inside the primary Session turn. For a
request that needs a user decision, `worker_id` is a private routing return
address: the host records the approval in the primary chat flow and relays the
structured `TopicReply` to the requesting child worker. It must not create a
second child-worker chat surface or send every reply to the primary worker.
Creating a child worker is an internal implementation choice for a subtask; it
must not emit `session_created`, add a sidebar Session, or otherwise ask the
user to manage runtime scheduling topology.

Task cancellation is Session-wide: the host forwards the user's Stop action to
every current worker so internal subtasks cannot outlive the cancelled primary
task. A later user turn is submitted only to the primary worker; old child
workers are not resumed or broadcast the new input.

It must not contain:

- Model provider wire formatting, curl calls, prompt assembly, memory semantics,
  tool argument parsing, or tool execution.
- React layout, CSS, browser state reducers, or user-facing visual policy. Those
  belong in `web_ui/timem-web`.
- Natural-language reinterpretation of core topics. UI receives semantic topic
  payloads and decides presentation.
