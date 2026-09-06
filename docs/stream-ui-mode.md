# Stream UI Mode

Settings → Beta → **Stream UI Mode** is disabled by default and stored only in the browser.

Core emits a provisional `core.model.preview` snapshot scoped to the current response attempt. The Host validates the active Core Turn and primary Worker, maps it to the existing Web turn, and retains only the latest preview. Preview chunks do not become chat history events. Browser reload restores the Host snapshot; a Host restart is not a resumable model request.

Public content is routed separately from interim Chat (`sub_answer.task` and `sub_answer.answer`). Other tool arguments and reasoning are not display content. Provisional display never permits tool execution. Complete protocol validation remains the execution gate. Invalid responses retract their provisional windows; network interruption and Stop retain partial text with an interruption notice. Confirmed Chat is delivered by `core.sub_answer`, with preview correlation metadata used to replace provisional content.

## Running tools and typography

While Stream UI Mode is enabled and the turn is active, running tool calls stay out of the Thought/Action frame and render as expanded rows in the stream delivery area (full multi-line command instead of the frame's truncated one-line preview). When a tool reaches a terminal status it persists into the frame like any completed activity, so the frame remains an execution ledger rather than a live spinner. Provisional response and Chat text uses exactly the final-answer font family, size, line height, and color, so confirmation never resizes content.

## Smooth provisional reveal

Provisional response and Chat text is replayed at a frame-independent, backlog-adaptive cadence (default 300 chars/s, catch-up bounded to 6000 chars/s and 160 chars/frame), so bursty SSE pushes and protocol retries never make text jump. Only monotonically growing prefixes animate; retraction, retry resets, restored snapshots, confirmation and `prefers-reduced-motion` users always see the exact delivered text immediately. New top-level markdown blocks fade in softly, an optional streaming caret marks the active tail, and interruption notices stay visible until the turn ends. Pacing is presentation-only and never delays protocol validation or delivery.

## Verification

- `cargo test -p agent_core --test model_stream_tests --locked`: incremental filters, protocol paths, bounds, rollback, retries and preview state.
- `pnpm --dir interfaces/web test:ui:stream`: runs the complete stream browser suite serially. Also included in `test:browser` / CI.
- `node interfaces/web/tests/browser/stream-preview-acceptance.mjs`: browser presentation using a simulated Host, including default-off and midstream toggling.
- `node interfaces/web/tests/browser/stream-preview-product.mjs`: actual product Host, temporary MEM, delayed local HTTP SSE model, and Chrome.
- `scripts/turn_concurrency_stress.sh`: separate Core/Worker PromptCut and terminal ownership stress coverage.

The product test requires `cargo build -p timem --locked` and a current Web build. It uses loopback port 18987 and must not run concurrently with another instance of the same test. No real provider credentials are required.

Normal product scenarios support `STREAM_PREVIEW_PROTOCOL=xml|json|native`. Each withholds the HTTP response tail until Chrome observes public response and provisional Chat, proving rendering occurs before HTTP completion. They also check same-node confirmation, reload recovery and Session-visible isolation.

Additional `STREAM_PREVIEW_SCENARIO=invalid|network|stop|supplement|interaction|tools` scenarios currently use XML:

- Invalid output retracts provisional windows without executing its Chat action; a repaired response completes.
- Broken HTTP retains partial response and Chat with an interruption notice.
- Stop retains partial content and permits the next Send.
- A supplement sent while the first request is held is absent from that sealed request and present in the following request. This observes actual model request bodies; it does not infer ownership merely from UI timing.
- Long-output updates preserve manual scroll position and copied clipboard text.
- A real readfile action remains in the work panel during previews and after final delivery; substantive process details do not automatically collapse with the final answer.

The OpenAI-compatible streaming entry point explicitly requests SSE without requiring TIMEM_STREAM configuration. The browser setting controls presentation only. The product harness leaves TIMEM_STREAM unset and asserts stream=true on actual model request bodies.

## Scope and limits

Preview is bounded and ephemeral, not a replacement for full protocol parsing or durable conversation history. Malformed/oversized preview data fails closed; authoritative response validation still owns execution and final delivery. Host restart does not restore an in-flight stream. The failure/interaction browser matrix covers XML, while JSON/native use unit-level malformed-input coverage and normal product-path acceptance. These tests are not an exhaustive provider compatibility claim.

## Interim Chat folding

In Stream UI, confirmed interim answers collapse into Chat when later AI content
arrives. Tool completion and user supplements alone do not collapse them. Provisional
answers remain visible during generation; each Chat can be manually collapsed or
reopened. Once the stream is archived, including restored completed history, answers
are available in the collapsed Chat panel instead of Thought/Action. The ordinary UI
keeps its existing Chat behavior. This is browser presentation, not a delivery or
Turn lifecycle change.

Regression: Chrome `stream-preview-acceptance.mjs` checks next-reply folding,
manual reopen/collapse, completed-history reload and provisional streaming.

Execution dots are shown only for `running` and `background_running`, never for
completed, failed, timed-out or cancelled tools. This applies to all stream tool
rows, not just bash. SSE wire events allow up to 4 MiB independently of the
1 MiB preview-parser budget; full HTTP responses remain capped at 16 MiB.

### Logical-step tool handoff

Settled stream tools fold when a later tool execution begins (including serial
calls within one model response), or later AI response content arrives. Completion
alone is not a handoff. Host lifecycle event order, preserved through coalescing,
compares execution starts with settlements; presentation timestamps are not used
to invent serial causality. A parallel start preceding settlement does not qualify.
Running/background tools remain visible, and incomplete historical evidence does
not infer an execution step. Eligible settled statuses include failures, timeouts
and cancellations, not only successes. Selection/manual disclosure protections
remain in force. Folding and incoming content are computed in the same render;
only newly absorbed counts pulse, without remounting existing tool rows.

Coverage: logical tool handoff unit tests, Chrome same-round A-finish/B-start/
B-failure and stable-row checks, existing next-AI-response and interaction tests,
and a 20,000-action linear handoff performance guard (1500 ms ceiling).

Stream tool rows use the static dot alone for running state, with an accessible
label. Background execution shows only `bg`, never redundant `running` text.
Terminal result labels use the shared success/failure symbols. Chrome
lifecycle/status-matrix acceptance guards this visual contract.

Terminal tool result labels use `✓` for success and `✗` for failure, while collapsed
tool summaries use `+ tools 2 ✓ | 1 ✗`. Accessible labels retain full words.
Unit and Chrome count/status acceptance tests guard the exact symbols.

The collapsed tools disclosure and individual live tool disclosure arrows
share the same left inset (4px); neither appears nested under the other.
Chrome acceptance checks their horizontal alignment at desktop and narrow widths.

The `tools` label is lowercase without a colon; the entire summary row,
including success/failure counts, uses normal font weight (400).

### Serial tool handoff and disclosure

Core now emits `execution_start` for non-shell builtins, command extensions, MCP
and parallel readfile dispatch as well as the existing approved shell paths.
Proposal `start` remains distinct from execution; approval waiting does not
advance execution. The UI folds a settled predecessor when a later execution
boundary arrives, without waiting for Turn completion. Parallel running tools,
background jobs and active reading/selection remain protected.

Regression: `serial_builtin_actions_emit_execution_boundaries_before_each_finish`
checks two proposals followed by serial execution/finish pairs; the actual-product
Chrome `tools` scenario executes two readfiles and checks predecessor folding
before final delivery. Disclosure uses plus/tools while collapsed and minus/tools
while expanded, with `✓` success and `✗` failure counts; browser tests cover both.
