# Stream UI Mode

Settings → Beta → **Stream UI Mode** is disabled by default and stored only in the browser.

Core emits a provisional `core.model.preview` snapshot scoped to the current response attempt. The Host validates the active Core Turn and primary Worker, maps it to the existing Web turn, and retains only the latest preview. Preview chunks do not become chat history events. Browser reload restores the Host snapshot; a Host restart is not a resumable model request.

Public content is routed separately from interim Chat (`sub_answer.task` and `sub_answer.answer`). Other tool arguments and reasoning are not display content. Provisional display never permits tool execution. Complete protocol validation remains the execution gate. Invalid responses retract their provisional windows; network interruption and Stop retain partial text with an interruption notice. Confirmed Chat is delivered by `core.sub_answer`, with preview correlation metadata used to replace provisional content.

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
