# TimemAi 3.0.1

TimemAi 3.0.1 fixes a long-session context compaction failure and hardens
browser acceptance tests against slow CI runners. No protocol, storage, or
configuration changes; upgrading from 3.0.0 is a drop-in replacement.

## Fixed

- Long-session context compaction now fails closed. When the
  provider-reported prompt token usage or the dynamic compressible estimate
  reaches the full `max_llm_input_tokens` budget, the compaction requirement
  is re-raised on every turn, bypassing the exhausted-latch suppression.
  Previously a chars/4 estimate drift on non-English content could grow the
  prompt past the budget while one exhausted quality follow-up permanently
  suppressed the automatic compaction, leaving every further request failing
  against the model.
- Browser acceptance tests are deterministic on slow runners: the
  session-insights light-theme hover samples after the cross-fade settles,
  the sticky composer assertion tolerates 1.5px of subpixel scroll rounding,
  the tool self-test entrypoint budget covers nested PowerShell cold starts,
  and the streaming network-outage scenario abortively resets the connection
  on every attempt so the network-error interruption stays observable.

## Upgrade

In-app: Settings -> System -> Software upgrade -> Check for updates. Or
download the archive for your platform below and replace the `timem` binary;
sha256 sidecars are provided for verification.
