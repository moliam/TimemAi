# Feature and Test Management

This document is the project management ledger for TimemAi features and their
test protection. It is maintained as product code: every new feature, behavior
change, or high-risk bug fix must update this document in the same change.

The goal is not to list every individual unit test. The goal is to make feature
ownership visible: what user capability exists, which test suites protect it,
what boundary and complexity cases are covered, and what still needs stronger
coverage.

## Maintenance Rules

- Add or update one feature row for every feature, user-visible behavior change,
  protocol change, storage change, terminal interaction change, provider change,
  or high-risk bug fix.
- Classify coverage under the two quality axes: Agent Core interaction
  correctness and UI display correctness. If a feature crosses both, it needs
  tests on both sides before it is release-ready.
- A feature is not release-ready if it only has helper-function tests while the
  real user path crosses runtime state, provider IO, storage, shell, TTY, or
  model action parsing.
- Tests should cover normal use, malformed/unexpected model output, boundary
  values, cancellation/error paths, persistence, and repeated multi-turn use
  when those dimensions are relevant.
- For terminal features, include real pseudo-TTY smoke in
  `scripts/real_tty_smoke.expect` when the behavior depends on actual terminal
  control sequences or interactive key handling.
- For runtime loop features, include at least one `session_runtime` integration
  test with a fake model client.
- For memory and shell state features, include repeated edge coverage in
  `scripts/edge_regression.sh` when races, loops, or cross-process behavior are
  plausible.
- If a feature intentionally keeps residual risk, record the risk and the next
  test that would reduce it.

## Test Suites

| Suite | Command / location | Purpose | Release expectation |
|---|---|---|---|
| Agent Core interaction correctness | `agent_core/tests/core_tests.rs`, `timem_shell/src/session_runtime.rs` tests, `scripts/edge_regression.sh` | Prove the model/runtime loop, protocol parsing, actions, memory, scratch, shrink, provider errors, audit, cancellation, and multi-round state transitions work. | Must pass for every feature touching runtime behavior. |
| UI display correctness | `timem_shell` render tests, observation/status/input tests, `scripts/real_tty_smoke.expect` | Prove shell output is accurate, readable, stable, and does not leak internal action names or model-private thought. | Must pass for every feature touching terminal or user-visible display. |
| Script syntax and install logic | `scripts/ci.sh`, `scripts/install_logic_test.sh` | Keep install/update/uninstall scripts syntactically valid and OS logic testable. | Must pass. |
| Contract check | `scripts/test_contract_check.sh` | Ensure required regression names, CI gates, and this feature ledger remain present. | Must pass. |
| Rust workspace tests | `cargo test --workspace` | Unit, integration, parser, protocol, storage, UI-render, and runtime tests. | Must pass; ignored live-network tests are not release blockers. |
| Repeated edge regression | `scripts/edge_regression.sh` | Re-run high-risk state machines: shrink, session runtime, memory concurrency, shell jobs, realistic story. | Must pass at default iteration; increase `TIMEM_EDGE_ITERATIONS` for risky releases. |
| Release build | `cargo build -p timem_shell --release` | Prove distributable binary compiles. | Must pass. |
| Real TTY smoke | `scripts/real_tty_smoke.expect` | Drive the release binary through a pseudo terminal for prompt, paste, config, workspace, Ctrl+C, and multiline behaviors. | Must pass. |
| Sensitive scan | `scripts/sensitive_scan.sh --current` | Prevent secrets and private/internal endpoints in public source. | Must pass before push/release. |
| Whitespace/diff check | `git diff --check` | Prevent whitespace errors. | Must pass. |

## Feature Coverage Matrix

| ID | Feature | User value | Primary tests | Boundary / complexity covered | Status / supplement needed |
|---|---|---|---|---|---|
| F01 | Provider configuration and startup banner | User can choose provider, protocol, model, URL, API key, token limits, data dir, and see effective values. | `parse_cli_args_reads_provider_model_and_limits`, `provider_config_from_env`, `config_menu_renders_effective_values_and_can_apply_updates`, `config_provider_update_keeps_dependent_defaults_consistent`, `config_provider_update_resets_custom_settings_when_returning_to_known_provider`, banner wrapping tests, `run_config_provider_switch_smoke`, real TTY `/config` smoke. | CLI option over env precedence, provider defaults, custom gateway, long base URL wrapping, runtime config updates, provider switch resets dependent default protocol/base URL, custom provider cannot silently inherit a known provider default URL, provider switching through real TTY menu, missing/non-ASCII API keys. | Covered. Keep adding config fields to help, banner, `/config`, and tests together. |
| F02 | Provider protocol adapters | Same runtime can use OpenAI-compatible, OpenAI Responses, and Anthropic wire formats. | `build_request_uses_official_openai_responses_shape`, `api_protocol_controls_wire_protocol_independent_of_provider_label`, `anthropic_endpoint_avoids_double_v1_when_base_already_ends_with_v1`, usage parsing tests. | Protocol independent from provider label, endpoint joining, max output fields, cached-token response variants, truncated responses. | Covered. New protocol requires adapter tests plus structured-output/cache tests. |
| F03 | Structured output hints | Provider requests can ask for JSON output when supported without assuming every provider supports it. | `structured_output_strategy_is_provider_and_protocol_specific`, request-building tests. | Aliyun/OpenAI JSON object support, Anthropic/no-hint path, prompt contains JSON contract in static prompt. | Conditionally covered. Request planning is tested; real provider acceptance is not proven by default CI. Add opt-in live smoke when credentials are intentionally available. |
| F04 | Prompt cache strategy | Incremental prompt growth can maximize KV-cache reuse without leaking prompt text into audit. | `prompt_cache_strategy_marks_incremental_prefixes`, `prompt_cache_strategy_keeps_multi_slice_delta_together`, `anthropic_request_maps_cache_strategy_blocks_to_content_blocks`, `prompt_cache_audit_summary_has_hashes_without_text`. | Static prompt cache, old deltas cache, new delta uncached, multi-slice delta kept together, audit hashes instead of raw prompt text. | Conditionally covered. Payload/cache-control planning is tested; actual provider KV-cache hit behavior is not proven by CI. Supplement with opt-in provider smoke if cache hit reliability becomes release-critical. |
| F05 | Provider error and truncation resilience | User sees actionable provider failure reasons instead of generic protocol failure. | `provider_http_error_includes_sanitized_provider_reason`, `provider_http_error_is_resilient_to_unusual_bodies`, `truncated_repair_failure_explains_provider_max_token_reason`, `session_turn_truncated_output_expands_limit_and_retries_same_turn`. | HTTP 400/401/404/500 bodies, unusual body shapes, secret redaction, OpenAI/Anthropic max-token truncation, output expansion retry. | Strong fixture coverage. Keep adding real provider response samples when new failures occur; default CI does not prove every provider's live error shape. |
| F06 | Static prompt and action contract | Model and runtime share a concise JSON/action protocol without over-prescribing model reasoning. | `static_prompt_keeps_contracts_concise`, schema/action repair tests, `invalid_action_shape_requests_protocol_repair`, `unsupported_action_is_not_executed_silently`. | Fenced JSON, prose around JSON, malformed JSON, invalid actions, missing intent repair, optional thought field. | Covered. Review prompt size and specificity whenever action catalog changes. |
| F07 | Prompt delta and slice model | Runtime can identify, render, and shrink prompt history by stable delta/slice ids. | `one_runtime_increment_can_contain_multiple_slices_in_one_delta`, `one_prompt_delta_can_render_to_multiple_slices`, `memmgr_context_shrink_removes_whole_delta_by_delta_id`, legacy shrink tests. | Multi-slice logical delta, slice ids, delta ids, static prompt untouched, hidden slice not rendered. | Covered. Add tests for any future slice search/filter API. |
| F08 | Forced shrink and long-context compaction | Long sessions avoid unbounded context growth and do not loop endlessly at threshold. | `long_context_forces_shrink_at_ninety_percent_window_with_compaction_instruction`, `successful_prompt_shrink_invalidates_stale_observed_prompt_tokens`, `forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough`, `session_turn_forced_shrink_runs_to_final_without_repeated_shrink`, edge regression. | 90% input threshold, observed provider tokens plus new delta estimate, static-dominant guard, repeated shrink loop prevention. | Covered. Add stress with lower `TIMEM_MAX_LLM_INPUT` when changing context accounting. |
| F09 | Scratch memory notes and context offload | Model can checkpoint work or offload large prompt deltas by id without rewriting content. | `memmgr_scratch_write_and_read_notes`, `scratch_write_context_offload_stores_runtime_prompt_delta_by_id`, `scratch_context_offload_rejects_invalid_prompt_refs_without_writing`, `session_turn_scratch_context_offload_records_id_and_continues`. | Missing required fields, invalid refs, query empty lists recent, delete miss non-destructive, offload validates delta/slice ids. | Covered. Supplement if scratch becomes shareable across UI sessions. |
| F10 | Durable memory and SQL read surface | User facts can be stored, updated, deleted, queried, and inspected safely through `memmgr`. | `memmgr_durable_query_returns_action_result_delta`, `memory_update_insert_update_and_delete_are_wrapped`, SQL read/write rejection tests, `memory_schema_action_returns_native_schema_contract`. | Expected version fields, SQL read-only, params matching placeholders, table allowlist, legacy row normalization, no semantic alias expansion. | Covered. Any new table needs SQL allowlist and rejection tests. |
| F11 | Multi-CLI memory conflict management | Multiple CLI sessions sharing one mem space do not corrupt files or silently overwrite facts. | `mem_guard_serializes_writes_across_processes`, `mem_guard_blocks_second_writer_until_first_writer_releases_lock`, `mem_guard_keeps_concurrent_memory_updates_from_losing_records`, `memory_update_concurrent_same_version_conflicts_allow_only_one_winner`, edge regression. | Lock directory serialization, same-version conflict, stale expected version, no lost records, child process lock helper. | Covered. Future daemon/IPC guard must reuse these semantic tests plus daemon lifecycle tests. |
| F12 | Chat history search, delete, and SQL access | Model can answer questions about visible prior chat records separately from durable memory through `memmgr type=raw_chat`. | `memmgr_raw_chat_query_reads_persisted_chat_records`, `chat_history_query_empty_query_lists_recent_records`, `memory_sql_query_reads_chat_messages_with_time_window`, `chat_history_delete_removes_matching_turn_from_audit_log`. | Empty query lists recent, current prompt fallback, time-window SQL, delete safety, chat table read-only. | Covered. Add multi-session chat-history tests if session management expands. |
| F13 | Bash actions and approval | Agent can do local work through Bash while respecting runtime approval policy and evidence rules. | `run_bash_executes_shell_syntax_after_user_approval`, `run_bash_requires_approval_for_mutating_commands`, `run_bash_allows_compound_local_write_commands`, `session_turn_bash_approval_executes_action_then_finishes_with_audit`, `bash_approval_mode_accepts_only_current_documented_values`. | Ask/approve policy, read-back approval, compound commands, low-risk identity commands, mutating commands, missing command repair, whitespace/case normalization for `approve`, stale aliases such as `approval`/`never` fall back to `ask`. | Covered. Add real project-edit E2E when introducing write helpers beyond Bash. |
| F14 | Background shell jobs | Long local commands can run in background and be polled without retry loops. | `run_bash_can_start_and_poll_background_job`, `shell_job_status_requires_model_chosen_timeout`, `shell_job_status_waits_for_model_chosen_timeout_before_running_result`, edge regression. | Background start, job id, model-chosen wait timeout, compatibility fields, no immediate busy-loop polling. | Covered. Add cleanup/leak tests if job concurrency increases. |
| F15 | Session runtime turn loop | UI-neutral runtime can drive model/action rounds, decisions, audit, profiler, and cancellation. | `session_turn_*` tests, `noop_turn_ui_defaults_to_noninteractive_denials`, repeated edge session group. | Fake model client, real core/actions/audit, approval decisions, round limit continue, truncation expansion, cancel before provider call. | Covered. New UI adapters must use `TurnUi` and add adapter-specific E2E. |
| F16 | Round limit continuation | User can continue a long task after max rounds without resetting model-visible task context. | `default_max_rounds_is_twenty`, `round_limit_can_be_continued_without_model_visible_task_reset`, `session_turn_round_limit_continue_recharges_and_finishes_same_task`. | Default 20 rounds, continue recharges rounds to 20, stop path, context preserved. | Covered. Add terminal smoke if the prompt UI changes. |
| F17 | Stale context prompt | After long idle with large context, user can choose whether to continue old task context. | `stale_context_prompt_needed`, `render_stale_context_prompt`, stale context choice tests. | 3-hour idle threshold, 10K context threshold, keyboard-driven choice, no prompt below threshold. | Covered. Add session-runtime E2E if stale context policy moves out of CLI. |
| F18 | Terminal input editor | User can type, edit, cancel, Shift+Enter newline, and paste multi-line/CJK text without corrupt display or triggering accidental model calls. | `reedline_*`, `queued_paste_*`, `raw_multiline_paste_*`, `paste_marker_*`, `submitted_input_rows_counts_real_newlines_independently_of_wrapping`, `submitted_user_line_rewrite_clears_actual_multiline_input_rows`, `chinese_backspace_removes_one_character`, `run_edited_paste_recovery_ctrl_c_smoke`, `run_edited_paste_recovery_return_to_edit_smoke`, real TTY smoke. | Bracketed paste enable, `[ pasted N lines ]` reverse-video display, edited placeholder recovery with `继续/恢复粘贴/返回编辑`, Ctrl+C/Esc cancel from recovery prompt, return-to-edit restores the edited draft, CRLF boundary, Ctrl+C drains residual input, CJK width, wrapped input, real newline row counting, submitted-line rewrite clears status plus true multiline rows, Shift+Enter. | Conditionally covered. Pseudo-TTY proves bracketed paste mode and core behavior, but real iTerm2/Terminal/tmux/SSH differences remain. Manual iTerm2 smoke is required before release when changing input code. |
| F19 | Observation panel | User sees current thought/action progress without internal protocol names or stale transients. | `observation::tests::*`, `thinking_view_renders_observation_panel_and_status_line`, visual contract tests. | Active/transient/persistent events, scroll window, command truncation, user-facing Bash label, malformed model response ignored, active color cycling. | Covered. Add multi-agent observation tests when parallel agents exist. |
| F20 | Token/status rendering | User sees context size, current request token deltas, cache hits, shrink markers, provider/model, and elapsed time clearly. | `token_status_*`, `final_token_status_does_not_show_latest_output_delta`, `final_response_visual_contract`, status bar tests. | Pending request deltas, final status without latest output delta, `[ctx N]` label, compact K formatting, zero totals, cache marker `⌁`. | Covered. Add screenshot/golden TTY smoke if status layout changes often. |
| F21 | Runtime profiling `/prof` | User can inspect token totals, cache hit rate, model wait time, local time, and storage sizes. | `profiler::tests::*`, `/prof` real TTY smoke. | Per-model aggregation, avg wait per 1K output, storage entry counts and byte sizes, no model calls for `/prof`. | Covered. Add file IO counters when profiler starts tracking file reads/writes. |
| F22 | API and action audit | Supportability data is stored locally with secret redaction and grouped by user turn. | `append_audit_writes_jsonl`, `audit_redacts_secret_fields`, `action_audit_groups_actions_by_user_turn_and_round`, session audit assertions, sensitive scan. | Payload audit vs action audit paths, grouped actions, denial/approval audit, secret-looking strings, memory outside audit dir. | Covered. Add retention/rotation tests if audit cleanup is introduced. |
| F23 | Install, uninstall, update, and README run path | New users can install, configure env, run `timem`, and update safely on macOS/Linux. | `scripts/install_logic_test.sh`, script syntax CI, README/help env tests. | OS detection, Rust version logic, env template, uninstall path, cargo-run latest dev path, install output recommends `source env` then plain `timem` without duplicating `--space/--model`. | Partially covered. Logic and docs are tested; actual clean-machine install/update/uninstall on macOS/Linux is not automated. Add clean VM/container smoke before major public releases. |
| F24 | Sensitive information control | Public repo must not contain private gateway URLs, real keys, or internal config. | `scripts/sensitive_scan.sh --current`, `scripts/sensitive_scan.sh --history`, `public_repo_sources_do_not_contain_private_gateway_markers`, release manual scan. | Secret-looking token strings, private gateway markers, redaction tests, audit summary hashes, history marker/secret scan. | Covered for current tree in default CI; history scan is available but not default CI. Run `--history` before force-push, public release, or after any history rewrite. |
| F25 | Documentation and quality gates | Users and maintainers can understand architecture, tests, release risk, and feature coverage. | `docs/architecture.md`, `docs/test-strategy.md`, this document, `scripts/test_contract_check.sh`. | CI gate list, feature matrix, release audit, maintenance rules, required regression names, F01-F25 presence check. | Covered by this change. Future feature work must update this document; contract check now verifies all F01-F25 rows exist. |

## Current Supplement Decisions

The following items are not release blockers for the current state, but they are
the next tests to add when the corresponding area changes:

| Area | Why current coverage is not absolute | Next supplement when touched |
|---|---|---|
| Terminal paste across emulator variants | Pseudo-TTY smoke proves bracketed paste mode and core behavior, but iTerm2, Terminal.app, tmux, and SSH can differ. | Add a small manual release checklist or automated tmux smoke if tmux becomes supported. |
| Live provider behavior | Unit tests use provider response fixtures; live tests require credentials and network. | Add opt-in provider smoke scripts keyed off explicit env vars; never run them by default CI. |
| Clean-machine install | Script logic is tested, but full install depends on host OS/package state. | Add macOS and Linux CI runners or a documented pre-release clean VM smoke. |
| Future Web UI | Architecture anticipates a UI adapter, but Web UI is not implemented. | Add UI adapter E2E and mem-guard multi-session tests before shipping Web UI. |

## Adversarial Audit Notes

The current suite is broad, but the following claims should not be overstated:

- Provider features are fixture-strong, not live-provider-complete. CI proves
  request/response shaping and representative error handling, not every vendor
  deployment behavior.
- Terminal input behavior is pseudo-TTY strong, not emulator-complete. Real
  terminals can differ in bracketed paste, keyboard protocol, tmux, SSH, and
  locale behavior.
- Install/update behavior is logic-tested, not clean-machine-proven.
- Default CI scans the current tree for secrets. History scanning is available
  and must be run before history rewrites or public releases where history risk
  matters.
- The feature ledger is now guarded for F01-F25 row presence, but it still
  relies on reviewer discipline to decide whether a new feature deserves a new
  feature id or an update to an existing row.

## Release Checklist

Before tagging a release:

1. Update this document for every new or changed feature.
2. Run `scripts/ci.sh`.
3. Run `scripts/sensitive_scan.sh --current`.
4. Inspect `git diff --check`.
5. For terminal/editor changes, run one real local terminal smoke in addition
   to `scripts/real_tty_smoke.expect`.
6. Confirm README/help examples still match the effective CLI/env behavior.
7. Confirm no internal URLs, local paths, API keys, or private credentials are
   present in tracked source or release notes.
