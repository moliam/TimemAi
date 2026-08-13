# TimemAi 1.0.3

TimemAi 1.0.3 strengthens the shared Agent Core protocol and substantially
refines the Timem Web session workspace.

## Highlights

- XML-native tool actions with explicit parallel groups, typed arguments,
  bounded parsing, precise repair feedback, and a guarded final-answer boundary.
- Session-scoped MCP configuration for stdio, Streamable HTTP, and legacy SSE
  tools, with secrets kept out of browser state, prompts, topics, and audit.
- Session-owned model settings, API keys, queued messages, deletion, restore,
  cancellation, supplements, and race-safe immediate-send behavior.
- Stable long-conversation scrolling, progressive history loading, clearer
  Thought/Action states, improved Markdown/code contrast, and refined controls.
- Hidden `.timem_data/` storage for new environments, with legacy `data/`
  compatibility only when a Timem-specific storage fingerprint is present.

## Protocol Safety

The runtime may recover outer XML boundaries for working actions and context
compaction. A final answer obtained through such recovery cannot terminate the
task: the model must return one complete, unmodified
`<response><final_answer>...</final_answer></response>` before completion.

## Verification

The release is covered by the repository production CI gate on Linux and macOS,
including Rust formatting, Clippy, workspace tests, Web tests and production
build, sensitive-data checks, performance and repeated-edge regressions,
release builds, cross-host resume, and pseudo-TTY smoke tests.
