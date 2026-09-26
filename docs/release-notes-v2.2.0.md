# TimemAi 2.2.0

TimemAi 2.2.0 focuses on a calmer streaming Web experience, reliable Session
model-endpoint binding, sharper model protocol guidance, and a correctness fix
that keeps never-sent input out of the model's resumable history.

## Highlights

### Calmer streaming Web experience

- Stream responses reveal incrementally with a retained live thought, and
  completed tools settle into the transcript without layout jumps.
- Retired stream tool rows fold away under the collapsed Tools control, keep a
  static running dot without pulsing, and show a compact elapsed label.
- Tool results render in a beta display mode with a compact live rhythm.
- Answer outlines refresh after tools archive; tool collapse waits for the
  next AI reply so panels stay stable.
- Stream tool handoff is stabilized and SSE audit failures are surfaced.
- Browser acceptance now pins `prefers-reduced-motion` explicitly, so CI animation checks stay deterministic regardless of the host OS accessibility settings.

### Web usability

- The interface is localized in Chinese and English.
- Working Sessions show a chat-synced breathing dot in the sidebar.
- Paste screenshots into the composer for visual question answering.

### Reliable Session model-endpoint binding

- Sessions bind to stable model endpoint IDs so restarts and configuration
  edits keep the intended model service.

### Sharper and more honest model protocol

- Structured `user resume directly` prompt entries distinguish an explicit
  user resume from other turn input.
- Response trailers and tool prompt guidance are refined for better
  instruction following.
- Response-header send failures are classified as retryable network errors.
- The Shell STILL RUNNING table includes a bounded original command.

### Queued input is never a phantom task

- Input that was still waiting in the Web send queue when the runtime
  restarted, or when a confirmed MEM switch landed, is now materialized as
  `queued_interrupted` interrupted history. The model-visible chat history no
  longer presents never-dispatched input as a resumable task, so a later
  "continue" cannot resurrect a message that was never sent.
