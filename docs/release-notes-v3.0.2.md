# TimemAi 3.0.2

TimemAi 3.0.2 lands chain-of-thought context replay with a reworked
menuconfig-style settings surface, prompt-context governance hardening, and
a batch of web session stream fixes. Storage stays compatible; upgrading
from 3.0.1 is a drop-in replacement.

## Protocol

- Chain-of-thought segments carry a distinct heading
  (`## ASSISTANT (thinking)` / `<ASSISTANT kind="thinking">`) separated from
  the final reply, and Markdown assistant section headings are standardized
  to protocol role names. Third-party consumers that parse assistant
  sections must recognize the thinking heading.

## Added

- Chain-of-thought context replay: native tool-call turns uniformly carry
  reasoning back into the prompt; non-streaming responses synthesize a
  reasoning delta from the full JSON body (OpenAI-compatible
  `reasoning_content` and Anthropic thinking blocks); providers that reject
  `reasoning_content` degrade dynamically with the refusal remembered for
  the session. Settings -> System gains a menuconfig-style settings surface
  with a chain-of-thought toggle (enabled by default).
- Queued timer badge with a live countdown; a queued prompt whose timeout
  expires enters the current conversation immediately.
- `run_bash` accepts `command` as an alias of the `cmd` parameter.

## Changed

- Prompt-context governance rework: precise per-delta visibility filtering
  and compression re-injection.
- The Host publishes settled previews immediately (final, intermediate, and
  thinking frames bypass the 150 ms broadcast throttle in order), so the
  working trailer and star attribution reflect real model/local-work state.
- Debug browse renders large-file previews with a header truncation notice
  instead of refusing.

## Fixed

- Long-session compaction retries on every turn while the provider-reported
  usage or the dynamic estimate exceeds the prompt budget.
- Global title tooltips survive scroll bursts and streaming re-renders.
- The web session stream suppresses duplicated provisional previews; mailbox
  detail rows stay visible; markdown tables and plain-text fences wrap; the
  upgrade panel no longer doubles the `v` in version labels.
- `run_bash` heredoc scanning no longer flags `&` in heredoc bodies, and the
  CLI preflight rejects unknown web options before workspace side effects.

## Upgrade

In-app: Settings -> System -> Software upgrade -> Check for updates. Or
download the archive for your platform below and replace the `timem` binary;
sha256 sidecars are provided for verification.
