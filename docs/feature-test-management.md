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
  protocol change, storage change, terminal interaction change, model service change,
  or high-risk bug fix.
- Classify coverage under the two quality axes: Agent Core interaction
  correctness and UI display correctness. If a feature crosses both, it needs
  tests on both sides before it is release-ready.
- A feature is not release-ready if it only has helper-function tests while the
  real user path crosses runtime state, model service IO, storage, shell, TTY, or
  model action parsing.
- Tests should cover normal use, malformed/unexpected model output, boundary
  values, cancellation/error paths, persistence, and repeated multi-turn use
  when those dimensions are relevant.

Current XML protocol coverage includes native `<actions>` tool elements,
explicit `<parallel>` groups, schema-typed scalar/nullable/union/array/tuple/object
arguments, additional-property conversion, namespaced MCP tool ids, literal
leaf CDATA, standard/numeric XML entities, atomic invalid-batch rejection,
bounded action-tree depth/size, and Exact/Cause/Correction repair feedback.
Model-facing XML no longer embeds action JSON in CDATA; the runtime keeps the
retired form only as a rolling-upgrade compatibility path. Release certification
also includes a real-model round where the model independently emits parallel,
escaped, nested, and dependent multi-round actions, plus a rejected typed value
that is corrected from the runtime's next-round feedback.
- Synchronous single-thread tests are useful for deterministic core logic, but
  worker/session concurrency must also have true multi-thread interaction tests
  that exercise ordering, cancellation, and cross-session isolation.
- Every release-ready feature row should have roughly four independent test
  protections across the following dimensions, or explicitly document why a
  dimension is not applicable:
  - Normal path: the user-visible happy path works end to end.
  - Boundary path: limits, empty values, long values, wrapping, id ranges, or
    threshold transitions behave correctly.
  - Error path: malformed model output, model service errors, cancellation,
    permission denial, missing fields, or invalid input fails safely.
  - Stress/repetition path: multi-turn, repeated edge regression, concurrent
    state, pseudo-TTY smoke, or race-prone paths stay stable under repetition.
- For terminal features, include real pseudo-TTY smoke in
  `scripts/real_tty_smoke.expect` when the behavior depends on actual terminal
  control sequences or interactive key handling.
- Shell UI or shell-only command changes must be tested from real user input in
  a pseudo terminal: type the command/text, wait for the rendered UI, wait for
  the prompt to return, then type another command/input. Function-entry tests
  alone are not sufficient for release readiness.
- Model-turn UI changes must include at least one pseudo-terminal test that
  drives a fake-model-server turn from user input through thinking/final rendering,
  returns to the prompt, and then accepts another user command/input.
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
| Agent Core interaction correctness | `core/agent/tests/core_tests.rs`, `core/agent/src/session_runtime.rs` tests, `scripts/edge_regression.sh` | Prove the model/runtime loop, protocol parsing, actions, memory, scratch, discard, model service errors, audit, cancellation, and multi-round state transitions work. | Must pass for every feature touching runtime behavior. |
| UI display correctness | `timem_shell` render tests, observation/status/input tests, `scripts/real_tty_smoke.expect` | Prove shell output is accurate, readable, stable, and renders model free_talk/progress/action semantics in the intended UI surfaces without leaking raw protocol names. Real input-path tests must drive the compiled shell from user keystrokes through rendered output back to a second user input. | Must pass for every feature touching terminal or user-visible display. |
| Script syntax and install logic | `scripts/ci.sh`, `scripts/install_logic_test.sh` | Keep install/update/uninstall scripts syntactically valid and OS logic testable. | Must pass. |
| Contract check | `scripts/test_contract_check.sh` | Ensure repository invariants, CI gates, protocol examples, and this feature ledger remain present; executable tests—not name searches—prove behavior. | Must pass. |
| Rust workspace tests | `cargo test --workspace` | Unit, integration, parser, protocol, storage, UI-render, and runtime tests. | Must pass; ignored live-network tests are not release blockers. |
| Repeated edge regression | `scripts/edge_regression.sh` | Re-run high-risk state machines: shrink, session runtime, memory concurrency, shell jobs, realistic story. | Must pass at default iteration; increase `TIMEM_EDGE_ITERATIONS` for risky releases. |
| Release build | `cargo build --locked --release --bin timem` | Prove the unified distributable executable compiles with the embedded Web bundle and Shell mode. | Must pass. |
| Real TTY smoke | `scripts/real_tty_smoke.expect` | Drive the release binary through a pseudo terminal for prompt, paste, config, workspace, Ctrl+C, and multiline behaviors. | Must pass. |
| Sensitive scan | `scripts/sensitive_scan.sh --current` | Prevent secrets and private/internal endpoints in public source. | Must pass before push/release. |
| Whitespace/diff check | `git diff --check` | Prevent whitespace errors. | Must pass. |
| GitHub Actions CI | `.github/workflows/ci.yml` | Run the same production CI gate on pushes and pull requests for Linux and macOS. | Must exist and call `scripts/ci.sh`. |
| Web UI and host | `applications/timem/tests/unit/web_host_tests.rs`, `interfaces/web/tests/*.test.ts`, production Vite build, `scripts/web_license_check.sh`, real-browser smoke | Prove tokenless loopback-local hosting, authenticated public hosting, session/request isolation, assistant-ui rendering, responsive layout, theme persistence, bounded history, and embedded asset reproducibility. | Must pass for every Web host or browser UI change. |

## Per-Feature Coverage Floor

Each feature row is managed against these four coverage dimensions. The feature
matrix keeps the exact test names close to the feature; this checklist defines
what reviewers must look for before a feature can be considered production
quality.

| Dimension | What counts | Examples in this repo |
|---|---|---|
| Normal | The expected user path works with realistic runtime state. | `session_turn_*`, model request-building tests, memory query/write tests. |
| Boundary | Limits and edge values are exercised. | token thresholds, long wrapped lines, empty query listing, CRLF paste, narrow terminal width. |
| Error | Bad input or external failure is safe and user-readable. | protocol repair, model HTTP errors, invalid SQL/action fields, denied approval, Ctrl+C/Esc cancel. |
| Stress / repetition | Repeated, concurrent, or real-terminal-like paths remain stable. | `scripts/edge_regression.sh`, memory guard multi-process tests, real TTY smoke, repeated shrink/session loops. |
| Performance guard | Hot paths stay bounded as prompt context, topics, terminal rows, Web action lifecycles, and browser event bursts grow. | `scripts/performance_guard.sh`, `performance_guard_large_context_prompt_render_is_bounded`, `performance_guard_topic_generation_for_many_actions_is_bounded`, `performance_guard_many_observation_events_render_bounded`, `performance_guard.test.ts`. |

If a feature cannot reasonably cover all four dimensions, the row's
`Status / supplement needed` column must state the residual risk and the next
test to add when that area changes.

Web UI requirements have an expanded row-by-row checklist in
`docs/web-ui-feature-test-matrix.md`. Update that matrix together with F32 when
adding or changing browser-visible behavior.

## Feature Coverage Matrix

Model API adapter coverage includes optional OpenAI-compatible thinking/SSE
extensions through `openai_compatible_request_supports_official_thinking_stream_options`,
`openai_compatible_sse_collects_content_and_usage_without_exposing_reasoning`,
`malformed_openai_compatible_sse_is_a_transport_error_not_model_content`, and
`openai_compatible_thinking_options_are_loaded_from_env`. Host coverage verifies
the same Session options survive Shell/Web configuration and restore without
persisting API keys. Release certification additionally uses a real GLM request
and checks the audit contains the configured request fields and normalized
content/usage but no `reasoning_content`.

| ID | Feature | User value | Primary tests | Boundary / complexity covered | Status / supplement needed |
|---|---|---|---|---|---|
| F01 | Model service configuration and startup banner | User can choose protocol, model, URL, API key, token limits, data dir, and see effective values. | `parse_cli_args_reads_model_service_and_limits`, `model_service_config_from_sources`, `config_menu_renders_effective_values_and_can_apply_updates`, `config_protocol_update_keeps_endpoint_defaults_consistent`, `config_protocol_update_preserves_explicit_endpoint`, banner wrapping tests, `run_config_protocol_switch_smoke`, real TTY `/config` smoke. | CLI option over env precedence, protocol defaults, custom endpoints, long base URL wrapping, runtime config updates, protocol switching refreshes only default endpoints, explicit endpoints are preserved, missing/non-ASCII API keys. | Covered. Keep adding config fields to help, banner, `/config`, and tests together. |
| F02 | Model API protocol adapters | Same runtime can use OpenAI-compatible, OpenAI Responses, and Anthropic wire formats. | `agent_core::model_api` request/response tests, `agent_core::model_transport` transport tests, `two_megabyte_model_body_reaches_http_server_intact`, `native_http_sends_custom_headers_and_json_body`, `build_request_uses_official_openai_responses_shape`, `anthropic_endpoint_avoids_double_v1_when_base_already_ends_with_v1`, usage parsing tests. | Endpoint joining, max output fields, cached-token response variants, truncated responses, multi-megabyte request bodies sent directly by the native Rust HTTP client without process argv or temporary curl files, custom header preservation, core-owned wire-format packing/parsing, model transport, and model request/response audit. Shell does not execute model HTTP. | Covered. New protocol requires adapter tests plus structured-output/cache tests. |
| F03 | Structured output hints | Model requests can ask for JSON output when supported without assuming every model API supports it. | `structured_output_strategy_is_response_and_api_protocol_specific`, request-building tests. | OpenAI-compatible/OpenAI Responses JSON object support, Anthropic/no-hint path, prompt contains JSON contract in static prompt. | Conditionally covered. Request planning is tested; live model-service acceptance is not proven by default CI. Add opt-in live smoke when credentials are intentionally available. |
| F04 | Prompt cache strategy | Incremental prompt growth can maximize KV-cache reuse without leaking prompt text into audit. | `prompt_cache_strategy_marks_incremental_prefixes`, `prompt_cache_strategy_marks_static_and_recent_tail_blocks`, `prompt_cache_strategy_improves_hits_against_simulated_cloud_cache`, `prompt_cache_strategy_would_miss_with_growing_old_delta_block`, `prompt_cache_strategy_can_mark_recent_multi_slice_delta_blocks`, `formatted_response_trailer_is_not_cached_or_merged_into_delta`, `anthropic_request_maps_cache_strategy_blocks_to_content_blocks`, `anthropic_request_sends_the_current_response_trailer_as_an_uncached_tail`, `openai_compatible_request_sends_the_current_response_trailer_as_an_uncached_tail`, `prompt_cache_audit_summary_has_hashes_without_text`, `session_turn_preserves_incremental_prompt_cache_plan_across_rounds`, `session_turn_preserves_cache_plan_with_json_response_protocol`, `session_turn_preserves_cache_plan_with_xml_response_protocol`, `scripts/kvc_replay_test.sh`, `scripts/kvc_replay.py --data-dir data`, Anthropic usage parsing tests. | Static prompt cache, latest three dynamic prompt slices, newest delta may be cacheable, no ever-growing old-deltas cache block, and the single protocol-neutral continuation trailer is sent as a final non-cache block and never merged into the latest delta; prefix-cache simulation uses 20-block lookback, local audit replay covers stable checkpoints, latest tail, typed tails, and static/no-static variants, the replay fixture covers JSON and XML protocol-shaped prompt history without reading real `data/`, session turn prompt growth preserves stable cache-control marking under all response protocols, audit hashes instead of raw prompt text, and cache read (`⌁`) and cache creation (`✚`) are tracked separately. | Strong local simulation coverage. Payload/cache-control planning, session-level prompt growth, replay CLI behavior, and local audit replay are tested; actual model service KV-cache hit behavior is still not proven by default CI. Supplement with opt-in model service smoke if cache hit reliability becomes release-critical. |
| F05 | Model service error and truncation resilience | User sees actionable model service failure reasons instead of generic protocol failure, and transient model service instability does not immediately interrupt work. | `model_http_error_includes_sanitized_service_reason`, `model_http_error_is_resilient_to_unusual_bodies`, `two_megabyte_model_body_reaches_http_server_intact`, `cancellation_interrupts_waiting_for_response_headers`, `progressing_response_may_outlive_configured_timeout`, `stalled_response_hits_configured_inactivity_timeout`, `malformed_endpoint_is_a_model_network_error`, `declared_oversized_response_is_rejected_before_body_read`, `streaming_response_is_rejected_when_accumulated_body_crosses_limit`, `truncated_repair_failure_explains_model_max_token_reason`, `truncated_native_sse_recovery_guides_small_tool_iteration_to_correct_answer`, `session_turn_truncated_output_replays_partial_response_and_requests_small_iteration`, `session_turn_retries_transient_model_api_errors_and_reports_status`, `session_turn_does_not_retry_non_transient_model_api_errors`, `session_turn_accepts_a_protocol_compliant_repair`, `thinking_status_line_shows_retry_notice`. | HTTP 400/401/404/500 bodies, unusual and large body shapes, secret redaction, multi-megabyte request bodies without OS process argv limits, connect/inactivity timeout semantics that allow progressing streams to outlive the configured interval while stalled responses still fail, timely cancellation during response-header and body waits, native network-error mapping including local address exhaustion (`EADDRNOTAVAIL`), a 16 MiB response-body hard limit for both declared and streaming bodies, OpenAI/Anthropic max-token truncation, truncated-response replay with bounded small-step regeneration, transient HTTP/network retry with user-visible status and audit events, protocol repair issue/count audit events, non-transient 400-style failures do not waste retries. | Strong fixture coverage. Keep adding real model response samples when new failures occur; default CI does not prove every model service's live error shape. |
| F05a | Model service input-overflow recovery | A model service or OS input-size rejection does not immediately lose an otherwise recoverable task or enter an infinite retry loop. | `input_too_large_errors_are_detected_without_matching_unrelated_failures`, `model_input_overflow_recovery_removes_only_latest_action_results`, `session_turn_recovers_from_model_input_overflow_variants`, `repeated_model_input_overflow_stops_after_single_delta_recovery`. | Local E2BIG/os-error-7, HTTP 413, model service context-length and too-many-input-token errors; newest action-result Delta rollback; user question retained; bounded RUNTIME guidance; structured audit; only one recovery when no further action-result Delta exists. | Covered with deterministic model service fixtures. Add vendor-specific error strings when observed. |
| F06 | Static prompt and action contract | Model and runtime share a concise response/action protocol without over-prescribing model reasoning. | `static_prompt_keeps_contracts_concise`, `assistant_name_placeholder_is_replaced_in_static_prompt_and_action_results`, `assistant_prompt_heading_uses_current_worker_speaker_name`, `raw_assistant_replay_is_included_before_action_results_for_working_turns`, `extracted_assistant_replay_mode_keeps_legacy_free_talk_and_final_answer_shape`, `session_turn_defaults_to_raw_assistant_output_replay`, `rendered_prompt_response_schema_is_injected_from_resource`, `response_protocol_kind_controls_rendered_protocol_section`, `session_turn_uses_model_service_config_response_protocol_over_core_state`, `session_worker_lifecycle_uses_model_service_config_response_protocol_over_core_state`, `prompt_spec::tests::*`, `prompt_render::tests::*`, `xml_action_result_preserves_name_escapes_xml_and_wraps_output_with_stable_id`, `oversized_xml_action_result_is_truncated_inside_output_id_envelope`, `xml_bash_result_uses_single_dynamic_output_block_for_one_stream`, `xml_bash_result_handles_empty_and_stderr_only_streams`, `xml_bash_result_preserves_unicode_and_trims_only_trailing_stream_whitespace`, `xml_bash_result_uses_shared_dynamic_id_for_stdout_and_stderr`, `xml_bash_result_boundary_changes_for_time_content_task_and_marker_collision`, `xml_bash_result_avoids_output_out_and_err_marker_collisions_in_either_stream`, `xml_bash_result_single_stream_budget_boundary_stays_complete`, `oversized_xml_bash_result_keeps_stream_tags_and_markers_complete`, `xml_bash_result_renders_running_timeout_cancelled_and_signal_metadata`, `xml_readfile_result_renders_structured_metadata_and_opaque_unicode_content`, `xml_specialized_results_use_lifecycle_status_and_structured_error_markers`, `xml_memmgr_and_self_tool_results_escape_attributes_and_preserve_body`, `xml_specialized_result_boundary_avoids_collisions_and_keeps_large_wrapper_complete`, `xml_action_results_preserve_names_for_sequential_and_parallel_actions`, `xml_parallel_run_bash_results_use_action_names_without_repeating_commands`, `response_protocol::*::tests::*`, `documented_json_response_examples_parse_with_runtime_parser`, `documented_xml_response_examples_parse_with_runtime_parser`, `xml_protocol_rejects_json_markdown_and_plain_text_roots`, `rejects_old_group_object_from_action_json`, `actions_section_rejects_old_group_object`, `final_answer_raw_xml_code_block_is_opaque_text`, `final_answer_raw_unbalanced_xml_is_opaque_text`, `final_answer_raw_text_can_contain_other_string_tags_without_rescanning`, `final_answer_nested_xml_preserves_attributes_and_escaped_text`, `free_talk_nested_xml_is_opaque_and_real_action_still_parses`, `free_talk_raw_xml_text_does_not_break_real_action`, `context_compact_summary_raw_xml_is_opaque_text`, `string_field_protection_does_not_hide_malformed_action_json`, `session_turn_xml_raw_string_tags_do_not_repair_or_execute`, `session_turn_xml_malformed_action_json_still_repairs`, `action_args_can_contain_xml_like_text`, `parses_response_wrapped_in_xml_markdown_fence`, `xml_state_branch_must_choose_one`, `scripts/update_static_prompt_snapshot.sh --check`, schema/action repair tests, `invalid_action_shape_requests_protocol_repair`, `memmgr_missing_op_requests_protocol_repair_from_manifest_idl`, `unsupported_action_is_not_executed_silently`, `protocol_repair_slice_focuses_previous_response_around_error`. | JSON and XML response protocol sections render from `resources/protocol/*`, and every concrete response example in those documents is parsed by its matching runtime suite. valid complex responses may contain protocol-looking XML/JSON/Markdown snippets inside string fields and action args without being re-parsed as control protocol, runtime session turns and worker lifecycle topics use `ModelServiceConfig.response_protocol` as the active protocol source of truth, XML protocol uses a small tag scanner for the `<ASSISTANT>` root and treats `free_talk`, `final_answer`, and compact `summary` as raw text fields, accepts one `<ASSISTANT>` root and XML-native tool elements inside `<actions>` while retaining legacy `<action_json>` only for rolling upgrades, tolerates a whole response wrapped in a documentation-style ```xml fence, rejects top-level JSON/Markdown/plain-text output under the XML protocol instead of silently routing it to another suite, enforces mutually exclusive XML state branches, treats XML-looking examples inside `final_answer`/`free_talk`/compact `summary` as opaque text and avoids rescanning fake tags, and repairs malformed native XML or legacy action JSON with exact error, cause, and correction feedback; action args may contain XML-like protocol text as data; ordinary Markdown headings remain plain answers, malformed action blocks trigger repair instead of being shown as final text, malformed JSON/XML, response schema summary injected from resource, prompt renderer injects response schema/catalog/current protocol language and worker identity, current-assistant placeholders are replaced in static prompt examples and action-result wording; XML action results wrap ordinary tool output in a matching `<output_id_HASH>...</output_id_HASH>` envelope whose six-digit lowercase hexadecimal hash derives from the original return content and generation time; `run_bash` instead uses `<bash_result>` with lifecycle-only `finished|timeout|running` status and optional exit-code/signal/pid/error-type attributes, a four-digit dynamic boundary, a single `OUTPUT` block for one stream, or independently captured stdout/stderr blocks sharing one `OUT`/`ERR` ID; `readfile`, `memmgr`, and `self_tool` use dedicated XML result roots with execution-layer metadata, lifecycle-only status, optional structured error type, and opaque collision-safe `CONTENT`/`ERROR` boundaries; marker collisions are avoided, sequential/parallel result order is preserved, and bounded truncation keeps every stream marker and result tag complete; successful assistant output is replayed raw by default while extracted free_talk/final-answer replay remains selectable, hidden slices stay hidden, per-protocol expanded prompt output can be regenerated by script without being checked into git, invalid actions, optional free_talk field, manifest-level required args become protocol repair, old `action`/`args` and `order`/`actions` objects are rejected for repair, response repair slices include the relevant malformed-output window instead of blindly copying a huge response head. | Covered. Review prompt size, parser tolerance boundary, and JSON/XML semantic parity whenever action catalog or response protocol changes. |
| F06b | Capability manifest and tool registry | Model-facing tool catalog, skill headers, and executor-facing action bindings share a manifest-backed IDL registry. | `capability::tests::*`, `executor::tests::*`, `tool_registry::tests::builtin_registry_lists_all_compiled_tool_callbacks`, `capmgr::tests::*`, `memmgr::tests::*`, `self_tool::tests::*`, `static_prompt_does_not_handwrite_tool_catalog`, `registry_loads_runtime_overlay_tools_and_skills_from_files`, `registry_stress_loads_many_overlay_tools_and_skills_and_filters_removals`, `no_local_command_profile_filters_run_bash_builtin_alias_even_without_requires_host`, `registry_rejects_overlay_tool_without_executor_binding`, `runtime_overlay_add_remove_keeps_prompt_executor_and_repair_consistent`, `runtime_overlay_command_tool_executes_with_json_input`, `overlay_command_background_requires_manifest_declared_fields`, `overlay_command_background_job_uses_capmgr_job_status`, `overlay_command_background_job_can_be_cancelled_through_capmgr`, `performance_guard_many_overlay_capabilities_render_is_bounded`, `rendered_prompt_tool_catalog_is_generated_from_capability_manifests`, `memmgr_tool_catalog_does_not_expose_legacy_query_surface`, `registry_derives_validation_rules_from_json_schema_idl`, `registry_validates_required_input_fields_from_manifest`, `canonical_tool_action_is_validated_through_capability_registry`, `legacy_actions_are_not_visible_or_executable`, `capmgr_load_skill_adds_skill_body_as_action_result`, `capmgr_invalid_values_request_protocol_repair_from_manifest_idl`, `self_tool_public_surface_groups_self_information_into_path_and_params`, `capmgr_action_maps_to_user_readable_observation_events`, `self_tool_action_maps_to_user_readable_observation_events`, `capabilities_dir_option_overrides_env`. | Builtin tool manifests load, runtime overlay manifests load without recompilation, bulk-added overlay tools/skills render and validate under stress, host/profile removal hides local-command capabilities from both prompt and executor, static prompt does not hand-maintain executable tool specs, builtin prompt catalog is generated as a concise Markdown guide from manifests, paired builtin tool callbacks live beside their YAML manifests under `resources/capabilities/tools/`, `capmgr load tool` exposes detailed schemas on demand, command-bound overlay tools execute with JSON stdin and bounded result output, registered command tools can opt into core-owned background tool jobs through manifest-declared fields and use capmgr job_status/job_cancel for status, command stdout/stderr/timeout are normalized by `agent_core::executor`, executor target resolution covers builtin and command actions, unknown actions are rejected instead of silently bridged, parser only parses the generic tool-name action object while manifest IDL owns required fields, any-of groups, conditional required fields, conditional any-of groups, and enum validation; canonical `memmgr` action remains executable, removed/legacy memory query surfaces and context discard/offload operations are not exposed in the catalog, canonical public `self_tool` exposes read-only `path`/`params` inspection plus `cwd` with conditionally required `new_path`, allowlisted output, secret redaction, and structured cwd synchronization, unsupported executor bindings fail startup, `capmgr` can list/load skill content through a dedicated executor module, UI renders `capmgr`/`self_tool` without exposing internal action names. | Covered for the current modular boundary. Runtime side-effect execution remains Rust-owned for builtin tools, but concrete builtin option parsing/execution belongs to each paired tool callback; manifest IDL owns model-facing tool parameter protocol and generic argument validation, including whether a registered command tool may use background execution. Do not expose a manifest action unless the executor binding or loadable resource exists. |
| F06c | Runtime self inspection and config-change awareness | Model can retrieve paths and non-sensitive effective runtime parameters without receiving secrets or a noisy notice per changed field. | `self_tool_public_surface_groups_self_information_into_path_and_params`, `multiple_runtime_config_changes_emit_one_system_notice_on_the_next_interaction`, `runtime_config_notice_is_session_isolated_and_rearms_only_after_a_new_change`, `update_runtime_config_changes_worker_model_service_config`, manifest validation and sensitive-env tests. | Public `self_tool` input is `type=path|cwd|params`; `cwd` without `new_path` reads the current directory as `CWD: ...`, while `cwd` with `new_path` changes it, returns `CWD changed to: ...`, and emits structured cwd state. `path` and `params` return relevant known values. Params uses a known-field allowlist, redacts Base URL userinfo/query/fragment, reports only API-key presence, and excludes arbitrary env plus secrets; multiple successful changes coalesce into one next-interaction RUNTIME notice independently per Session. | Covered. Add inspection fields beneath `path` or `params`; keep cwd mutation beneath `cwd` with `new_path`. |
| F07 | Prompt delta and slice model | Runtime can identify, render, and discard prompt history by stable delta/slice ids. | `one_runtime_increment_can_contain_multiple_slices_in_one_delta`, `one_prompt_delta_can_render_to_multiple_slices`, `user_supplement_appends_to_latest_delta_as_slice`, `session_turn_user_supplement_during_model_wait_continues_after_stale_final`, `scripts/real_tty_supplement_smoke.expect`, `memmgr_context_discard_removes_whole_delta_by_delta_id`, `session_turn_forced_shrink_runs_to_final_without_repeated_shrink`, legacy discard tests. | Multi-slice logical delta, slice ids, delta ids, start-only `[BEGIN DELTA delta_id: ...]` boundaries whose scope continues through following provider-native assistant/tool messages until the next marker or input end, static prompt untouched, hidden slice not rendered, Core hosts may append a `user_supplement` to the current turn; the Shell pseudo-TTY queue smoke separately proves that a second question typed during active work waits for Q1’s final answer, renders as a new user turn, and then returns `SUPPLEMENT_OK`, session-level context reduction continuation. | Covered. Add tests for any future slice search/filter API. |
| F08 | Forced context reduction and long-context compaction | Long sessions avoid unbounded context growth and do not loop endlessly at threshold. | `long_context_forces_shrink_at_ninety_percent_window_with_compaction_instruction`, `response_context_compact_hides_refs_and_appends_summary_slice`, `response_context_compact_reinjects_current_applied_mcp_capabilities`, `multiple_successful_compacts_reinject_active_mcp_only_once`, `active_mcp_compact_note_is_deterministic_and_bounded`, `context_compact_success_hides_ref_details_and_preserves_surviving_exchanges`, `successful_compact_does_not_reinject_large_discard_id_lists`, `successful_prompt_shrink_invalidates_stale_observed_prompt_tokens`, `forced_shrink_is_not_reissued_when_dynamic_context_cannot_reduce_enough`, `session_turn_forced_shrink_runs_to_final_without_repeated_shrink`, `session_turn_scratch_context_offload_records_id_and_continues`, `dynamic_context_estimate_and_shrink_stats_include_native_exchanges`, edge regression. | 90% input threshold, observed model input tokens plus new delta estimate; successful compaction results return only completion status plus actionable offload `scratch_id` when present, while full discarded/offloaded id lists, shrink counters, and live-ref diagnostics remain in host telemetry instead of being re-injected into the model prompt; failed invalid-ref results still include missing ids and current live refs for repair; model-visible forced/manual compaction trailers stay minimal and only require following the `context_compact` capability description with `context_compact` first—token ratios, summary sections, delta-selection tactics, and other compaction policy must not be duplicated in Core trailers because the capability description is the single authority; response-level `context_compact` with `discard` and/or `offload`; one unified dynamic estimate covering visible text slices plus provider-native exchanges, combined before/after compact telemetry with text/native breakdowns, `shrunk_tokens` derived from the same estimator, offloaded deltas written to scratch with scratch id returned in the next RUNTIME delta, exactly one replacement MCP catalog delta when compaction targets the active persistent catalog, MCP catalog tokens included in after-size accounting, static-dominant guard, repeated context reduction loop prevention. | Covered. Add stress with lower `TIMEM_MAX_LLM_INPUT` when changing context accounting. |
| F09 | Scratch memory notes and context offload storage | Model can checkpoint notes through `memmgr`; runtime can offload large prompt deltas to scratch through `context_compact`. | `memmgr::tests::*`, `memmgr_scratch_write_and_read_notes`, `session_turn_scratch_context_offload_records_id_and_continues`, response protocol context compact parser tests. | Missing required fields, search_text empty lists recent, delete miss non-destructive, context compact validates refs before offload/discard, session-level offload writes scratch id into the next prompt delta, scratch kind aliases normalize consistently. | Covered. Supplement if scratch becomes shareable across UI sessions. |
| F10 | Durable memory and SQL read surface | User facts can be stored, updated, deleted, and inspected safely through `memmgr`; durable reads use SQL, while writes use guarded semantic operations. | `memmgr::tests::*`, `memmgr_durable_sql_returns_action_result_delta`, `memmgr_legacy_query_op_is_not_executed_after_sql_search_split`, `session_turn_round_limit_continue_recharges_and_finishes_same_task`, `memory_update_insert_update_and_delete_are_wrapped`, SQL read/write rejection tests, `memory_schema_action_returns_native_schema_contract`. | Expected version fields, SQL read-only, params matching placeholders, table allowlist, legacy row normalization, removed durable `op=query` does not read records, no semantic alias expansion, session-level memory lookup. | Covered. Any new table needs SQL allowlist and rejection tests. |
| F11 | Multi-CLI memory conflict management | Multiple CLI sessions sharing one mem space do not corrupt files or silently overwrite facts. | `mem_guard_serializes_writes_across_processes`, `mem_guard_blocks_second_writer_until_first_writer_releases_lock`, `mem_guard_keeps_concurrent_memory_updates_from_losing_records`, `memory_update_concurrent_same_version_conflicts_allow_only_one_winner`, edge regression. | Lock directory serialization, same-version conflict, stale expected version, no lost records, child process lock helper. | Covered. Future daemon/IPC guard must reuse these semantic tests plus daemon lifecycle tests. |
| F12 | Chat history search, delete, and SQL access | Model can answer questions about visible prior chat records separately from durable memory through `memmgr type=raw_chat`. | `memmgr_raw_chat_search_reads_persisted_chat_records`, `chat_history_search_empty_text_lists_recent_records`, `memory_sql_query_reads_chat_messages_with_time_window`, `chat_history_delete_removes_matching_turn_from_audit_log`. | Empty search_text lists recent, current prompt fallback, time-window SQL, delete safety, chat table read-only. | Covered. Add multi-session chat-history tests if session management expands. |
| F13 | Bash actions and approval | Agent can do local work through Bash while respecting runtime approval policy and evidence rules. | `shell_exec::tests::*`, `foreground_bash_preserves_stdout_and_stderr_independently`, `foreground_bash_preserves_stderr_only_and_empty_streams`, `polling_bash_preserves_last_stdout_and_stderr_evidence`, `historical_shell_job_record_without_stderr_file_is_treated_as_stdout_only`, `shell_job_record_deserializes_without_legacy_stderr_file_field`, `run_bash_executes_shell_syntax_after_user_approval`, `run_bash_requires_approval_for_mutating_commands`, `run_bash_allows_compound_local_write_commands`, `session_turn_bash_approval_executes_action_then_finishes_with_audit`, `session_turn_stop_cancels_parallel_bash_after_approval`, `bash_approval_mode_accepts_only_current_documented_values`. | Ask/approve policy, compound commands, low-risk identity commands, mutating commands, missing command repair, whitespace/case normalization for `approve`, stale aliases such as `approval`/`never` fall back to `ask`, shell executor module validates and bounds normal execution, captures stdout and stderr independently for normal, polling, timeout, and background paths while retaining compatible readable text output, run_bash normal/polling/background/approval/parallel paths use the active prompt context cwd, and approval does not detach actions from later Session cancellation. | Covered. `AgentCore` keeps user approval and turn-loop routing; `agent_core::shell_exec` owns Bash validation/execution. Add real project-edit E2E when introducing write helpers beyond Bash. |
| F13b | User scenario replay | Common user workflows keep working across action protocol, executor results, and final response generation. | `scenario_coding_inspects_project_and_reports_from_shell_evidence`, `scenario_memory_qa_retrieves_durable_and_raw_chat_before_answering`, `scenario_self_qa_and_runtime_env_update_stays_bounded`, `scenario_file_writing_outputs_artifact_and_verifies_content`. | Coding inspection through Bash evidence, durable/raw-chat memory QA, self identity/env/path QA, and file-writing output workflows. | Covered at core replay level. Add UI scenario replay if these workflows gain dedicated UI states beyond existing observation and TTY smoke tests. |
| F14 | Background local jobs | Long local commands and registered command tools can run in background, be tracked, or be stopped without retry loops. | `shell_exec::tests::*`, `tool_jobs::tests::*`, `run_bash_background_job_enters_running_list_and_later_emits_exit_update`, `model_prompt_job_started_between_scans_is_reported_as_still_running`, `running_job_list_is_injected_when_discard_references_running_job_delta`, `running_job_list_is_injected_when_offload_references_running_job_delta`, `running_job_list_is_injected_when_compact_references_running_job_delta`, `running_job_list_is_not_injected_when_discard_refs_unrelated_delta`, `timeout_job_is_reported_running_and_model_can_kill_by_pid`, `xml_timeout_still_running_uses_orthogonal_lifecycle_evidence`, `action_topic_pid_requires_managed_running_bash_evidence`, `session_cancel_and_running_list_ignore_foreign_runtime_records`, `managed_shell_job_pid_is_a_distinct_runtime_child_process_group`, `overlay_command_background_job_uses_capmgr_job_status`, `overlay_command_background_job_can_be_cancelled_through_capmgr`, `timeout_job_supports_heredoc_with_backticks`, `background_job_supports_heredoc_with_backticks`, `tracked_job_preserves_complex_shell_syntax_without_runtime_wrapper`, `shell_lifecycle_validation_rejects_unmanaged_background_without_wait`, `run_bash_unmanaged_background_is_rejected_and_reported_to_the_model`, `supervisor_waits_for_managed_process_group_after_launcher_exits`, `run_bash_background_job_enters_running_list_and_later_emits_exit_update` (asserts Exit status and Final output), `timeout_job_is_reported_running_and_model_can_kill_by_pid` (asserts Final output), edge regression. | `run_bash` background start returns pid and action evidence, normal timeout transitions to `status=running` plus `timed_out=true` for a tracked Runtime-owned child process-group PID instead of overloading the lifecycle status, historical/foreign-owner PIDs are neither exposed nor cancelled, exit update emits once and carries exit status code and bounded final output, every model request rebuilds the authoritative still-running snapshot (including jobs registered between the initial and final request scans), the RUNTIME_INFO table reports each job's cumulative elapsed runtime, a concise platform-native observation location in `notes` when available, and a progress-check reminder when any job exceeds three minutes, compaction cannot hide a still-running job, model can stop a timed-out managed process group with normal Bash, command-bound registered tools keep capmgr job ids/status/cancel semantics. | Covered. Background job lifecycle lives in core (`shell_exec` for Bash, `tool_jobs` for registered command tools); shell UI only renders evidence/status. Each Bash job has one supervisor that owns and reaps its Child, joins bounded stdout/stderr drains, and publishes one terminal result. The manager only indexes handles and removes terminal jobs after direct-result or background-update delivery. Tracked jobs now run under `/bin/bash` and preserve heredoc/complex-syntax commands without runtime wrapping. Normal/polling calls reject unmanaged `&` backgrounding. Detach keywords are allowed because Linux cgroup ownership survives session/process-group changes; completion waits for the Job cgroup to become empty after the launcher exits (or the process group in explicit degraded mode). |
| F14b | External status polling and model-timeout waits | Model can wait for remote/external state without embedding long `sleep && check` commands in normal Bash, and long positive-timeout commands remain cancellable by the host/UI. | `run_bash_poll_mode_*`, `session_turn_run_bash_poll_mode_waits_until_check_succeeds`, `normal_run_bash_rejects_long_sleep_commands`, `normal_run_bash_allows_short_sleep_commands`, `normal_bash_rejects_non_positive_timeout`, `normal_bash_positive_timeout_reports_long_running_status_to_runtime`, `session_turn_long_positive_timeout_command_decline_becomes_user_supplement`, `sequential_group_with_long_timeout_command_uses_host_decision_path`, `long_running_command_prompt_is_keyboard_driven_and_defaults_to_wait`, `model_response_maps_polling_run_bash_to_user_facing_poll`, `action_topic_kind_wire_payload_is_explicit_and_round_trips`, Web `view_model` and `activity_groups` behavior tests, capability registry catalog tests. | `run_bash` polling mode through `loop_cmd`, `interval_ms`, `loop_timeout_ms`, and `once_timeout_ms`; short check command with exit-code-0 completion, the standard lifecycle `status` plus last-`loop_cmd` `exit_code` evidence, with documentation that this is not automatically the waited task's own exit code, total wait/per-check timeout bounds without an upper clamp, timeout result, cancellation during wait, session-level action result replay, normal long-sleep rejection with guidance to use polling mode, positive-timeout normal command status callback after the long-command threshold, host/user stop decision returning `cancelled_by_user` and injecting `user_supplement`, same `core.action` topic with `kind=bash, mode=poll`, shell renders `Poll:`, and Web renders a dedicated clock-marked `Poll` row with live elapsed time and a second-line command without adding a fine-grained topic. | Covered. If future UI adds a "check now" button, add topic/request tests for that host interaction without changing model-facing action shape. |
| F14c | Multi action groups | Model can request grouped actions where workflow entries run one after another and inner arrays run in parallel. | `parses_action_groups_and_flattens_actions_for_notifications`, `actions_section_json_fence_still_parses_action`, `actions_section_rejects_old_group_object`, `rejects_old_group_object_from_action_json`, `parses_bare_action_array_as_parallel_group`, `parses_nested_action_arrays_as_ordered_parallel_groups`, `session_turn_executes_parallel_action_group_before_next_group`, `session_turn_cancels_parallel_long_running_bash_actions`, `session_turn_stop_after_one_parallel_action_completed_cancels_the_running_action`, `session_turn_stop_cancels_parallel_bash_after_approval`, response protocol parity tests, action topic tests. | JSON action workflow arrays plus XML-native sequential tool elements and explicit `<parallel>` groups, strict exact tool ids, old `{ "order": ..., "actions": ... }` group objects rejected for repair, flattened action notifications for UI, sequential workflow fallback, parallel run_bash execution when safe, next workflow entry waits for previous group completion, audit/result replay, Session cancellation shared by all parallel actions, completed siblings preserved while active siblings stop, approved actions remain cancellable, and descendant processes are terminated through their kernel-owned Job, with process-group fallback only in explicit degraded mode. | Covered for Bash groups. Non-Bash actions intentionally execute through the sequential safe path unless future tool executors declare safe parallel semantics. |
| F15 | Session runtime turn loop | UI-neutral runtime can drive model/action rounds, decisions, audit, profiler, cancellation, configurable turn reminders, and bounded supplement dispatch waits during local actions. | `session_turn_*` tests, `ordinary_requests_disable_reasoning_by_default`, `openai_responses_request_carries_reasoning_effort_only_for_critical_requests`, `session_turn_user_supplement_during_model_wait_continues_after_current_response`, `user_supplement_model_dispatch_timeout_interrupts_wait_and_builds_stateful_prompt (action-phase)`, `user_supplement_dispatch_timeout_prompt_includes_still_running_work`, `worker_forces_dispatch_when_a_live_supplement_times_out_during_local_action`, `session_turn_user_supplement_after_model_response_continues_same_turn`, `session_turn_user_supplement_at_final_boundary_continues_same_turn`, `session_turn_terminal_protocol_failure_does_not_consume_or_revive_late_supplement`, `session_worker_does_not_revive_terminal_repair_failure_with_late_supplement`, `turn_focus_reminder_schedule_respects_periods_and_skips_backlog`, `turn_reasoning_reminder_schedule_injects_every_eight_completed_rounds`, `none_reminder_consumes_the_due_period_without_prompt_injection`, `session_turn_injects_due_focus_reminder_before_the_next_model_request`, `session_turn_injects_reasoning_reminder_before_the_ninth_model_request`, `session_replay_story_covers_repair_memory_scratch_shrink_and_observation_rendering`, `session_worker_emits_lifecycle_runs_turn_and_accepts_mid_turn_supplement`, `session_worker_rename_emits_updated_identity_topic`, `session_worker_manager_allocates_id0_default_and_tracks_lifecycle`, `session_worker_manager_allocates_multiple_workers_from_id0`, `session_worker_shutdown_skips_queued_turns`, `session_workers_run_concurrently_without_cross_talk`, `session_workers_stress_ui_threads_supplements_and_renames`, `cancel_stops_all_session_workers_and_next_turn_runs_only_primary`, `noop_turn_ui_defaults_to_noninteractive_denials`, repeated edge session group. | Fake model client, scripted multi-turn model replay, normal reply, malformed response repair, durable memory write/retrieve, scratch context offload, forced context discard with compaction guidance, observation rendering, real core/actions/audit, approval decisions, round limit continue, truncation expansion, cancellation, supplements, and successful Final commit; a supplement accepted while a local action holds the turn waits at most 20 seconds by default, then interrupts that action through its existing cancel checks and constructs a state-aware prompt without inventing the interrupted action result, including still-running shell jobs; independent active-minute/completed-round schedules loaded from global config; bounded validation; random selection including `NONE`; default 10-minute and 8-round schedules; missed-period collapse; host-decision wait exclusion; per-session worker isolation and stress coverage. | Covered. Successful in-flight responses are never discarded because of a supplement; supplement timeout does not override user cancellation or terminal errors. New UI adapters should either use `TurnUi` synchronously or `CoreSessionWorkerManager`/`CoreSessionWorker` per session, then add adapter-specific E2E. |
| F16 | Round limit continuation | User can optionally bound a long task and continue it without resetting model-visible task context. | `default_max_rounds_is_unlimited`, `round_limit_can_be_continued_without_model_visible_task_reset`, `session_turn_round_limit_continue_recharges_and_finishes_same_task`, Web runtime-setting tests. | Product default Unlimited; Web choices 50/200/500/Unlimited; per-Session runtime update and persistence; finite-limit stop/continue path; continuing removes the current finite cap while preserving context. | Covered. Add terminal smoke if the prompt UI changes. |
| F17 | Stale context prompt | After long idle with large context, user can choose whether to continue old task context. | `stale_context_prompt_needed`, `render_stale_context_prompt`, stale context choice tests. | 3-hour idle threshold, 10K context threshold, keyboard-driven choice, no prompt below threshold. | Covered. Add session-runtime E2E if stale context policy moves out of CLI. |
| F18 | Terminal input editor | User can type, edit, cancel, Shift+Enter newline, paste multi-line/CJK text, and add instructions while the model is thinking without corrupt display or triggering accidental model calls. | `reedline_*`, `queued_paste_*`, `raw_multiline_paste_*`, `paste_marker_*`, `thinking_supplement_*`, `submitted_input_rows_counts_real_newlines_independently_of_wrapping`, `submitted_user_line_rewrite_clears_actual_multiline_input_rows`, `chinese_backspace_removes_one_character`, `run_edited_paste_recovery_ctrl_c_smoke`, `run_edited_paste_recovery_esc_smoke`, `run_edited_paste_recovery_return_to_edit_smoke`, `scripts/real_tty_smoke.expect`, `scripts/real_tty_supplement_smoke.expect`, `scripts/real_tty_stress.expect`. | Bracketed paste enable, `[ pasted N lines ]` reverse-video display, edited placeholder recovery with `继续/恢复粘贴/返回编辑`, Ctrl+C/Esc cancel from recovery prompt, return-to-edit restores the edited draft, CRLF boundary, Ctrl+C drains residual input, CJK width, wrapped input, real newline row counting, submitted-line rewrite clears status plus true multiline rows, Shift+Enter, noncanonical thinking-time next-question input with Ctrl+C still delivered as turn cancel, real PTY stress with repeated Thought/Action redraws, long progress/action rows, local Bash action, and a queued next question. | Conditionally covered. Pseudo-TTY proves bracketed paste mode, next-question queue input, stress redraw, and core behavior, but real iTerm2/Terminal/tmux/SSH differences remain. Manual iTerm2 smoke is required before release when changing input code. |
| F19 | Observation panel | User sees current progress/actions without internal protocol names or stale transients. | `observation::tests::*`, `thinking_view_renders_observation_panel_and_status_line`, `multi_worker_thinking_view_keeps_identity_and_bounded_layout`, visual contract tests. | Active/transient/persistent events, scroll window, command wrapping, tree child rows under intent, user-facing Bash label, memory/context labels, malformed model response ignored, protocol repair responses do not publish invalid model-response topics, active color cycling, multiple worker identities rendered at the same time, global working-worker count keeps thinking visible until all active workers finish, direct turns publish 1/0 worker counts, only working-response free talk enters Thought/Action while finished responses stay on the separate final-delivery surface, all model actions are exposed to observation topics, long progress/action rows bounded by visible-width assertions. | Covered. Add richer dashboard tests if the shell starts rendering several workers in one continuously updating screen instead of composing per-worker thinking views. |
| F20 | Token/status rendering | User sees context size, current request token deltas, cache hits, shrink markers, model service, elapsed time, and wasted repair rounds clearly. | `agent_core::status_summary::tests::*`, `token_status_*`, `final_token_status_does_not_show_latest_output_delta`, `final_response_visual_contract`, `final_status_shows_repair_call_count_when_present`, `thinking_status_line_shows_repair_call_count_when_present`, status bar tests. | Core owns structured status data: meaningful latest usage detection, context percent, bar-fill count, model rounds, and repair counts. Shell owns symbols, compact K formatting, and terminal layout. Pending request deltas, final status without latest output delta, `[ctx N]` label, zero totals, cache marker `⌁`, repair count shown as `⇌N (⚠M)` only when nonzero. | Covered. Add screenshot/golden TTY smoke if status layout changes often. |
| F21 | Runtime profiling `/prof` | User can inspect token totals, cache hit rate, model wait time, local time, and storage sizes. | `agent_core::profiler::tests::*`, `timem_shell::profiler::tests::*`, `session_turn_records_cached_tokens_in_profiler_and_latest_usage`, `/prof` real TTY smoke. | Core owns UI-neutral per-model aggregation, timing, storage snapshots, and raw `RuntimeProfileReport`; shell owns terminal rendering, units, percentages, and compact formatting; session runtime records model usage including cached tokens; latest usage is retained for final status; no model calls for `/prof`; large memory/scratch JSONL files are counted by streaming lines instead of reading the whole file into memory. | Covered. Add file IO counters when profiler starts tracking file reads/writes. |
| F22 | API and action audit | Supportability data is stored locally with secret redaction and grouped by user turn. | `append_audit_writes_json_document`, `audit_redacts_secret_fields`, `action_audit_groups_actions_by_user_turn_and_round`, `model_repair_audit_is_core_owned_when_applying_response`, `action_related_audit_event_builders_are_structured`, `audit_retention_keeps_only_the_last_day_across_json_and_jsonl`, `jsonl_tail_compaction_keeps_only_complete_latest_lines`, `oversized_audit_event_is_replaced_with_bounded_summary`, `audit_budget_cleans_legacy_retention_temps`, `oversized_base_document_keeps_latest_events_and_schema`, `legacy_root_sidecar_is_read_and_counted_by_budget`, `audit_budget_reserves_one_atomic_write_unit_below_hard_limit`, `action_audit_capacity_removes_oldest_turns_without_changing_schema`, `action_audit_capacity_summarizes_one_oversized_turn_without_changing_schema`, `legacy_multi_turn_action_audit_migrates_to_slices_before_new_turn`, `action_audit_upgrade_merges_overlapping_legacy_and_segmented_sources_idempotently`, `action_audit_upgrade_removes_only_confirmed_legacy_turn_files`, `action_audit_upgrade_deduplicates_stale_active_checkpoint`, `action_audit_finish_retry_does_not_duplicate_an_already_committed_turn`, `session_turn_retries_transient_model_api_errors_and_reports_status`, `session_turn_accepts_a_protocol_compliant_repair`, session audit assertions, sensitive scan. | Payload audit JSON document with `events`, time retention across base JSON and JSONL, invalid/legacy timestamp migration, normal 64 MiB / debug 512 MiB hard bounds with one reserved 16 MiB allocation slice, newest-complete-record in-place JSONL compaction without a second large temporary file, 16 MiB single-event summary fallback, oldest-event/turn eviction without schema changes, stale retention-temp cleanup, compatibility with both current audit-directory and legacy MEM-root JSONL sidecars, action audit paths, grouped actions, complete-Turn rolling slices, idempotent upgrade merging across legacy JSON/`.turns`/active checkpoints/existing segments, preservation of unreadable legacy files until recoverable, denial/approval audit, retry audit events, protocol repair audit events, realtime `audit/api_output_repair.json` records containing malformed assistant response, RUNTIME repair message, session/turn ids, issue, and rendered human-readable sections, secret-looking strings, memory outside audit dir. | Covered. |
| F23 | Install, uninstall, update, and README run path | New users can install, configure env, run `timem`, and update safely on macOS/Linux. | `scripts/install_logic_test.sh`, script syntax CI, README/help env tests, GitHub Actions macOS/Linux CI invoking `scripts/ci.sh`. | OS detection, Rust version logic, env template, uninstall path, cargo-run latest dev path, install output recommends `source env` then plain `timem` without duplicating `--space/--model`, remote Linux/macOS CI runner coverage. | Covered for repo CI and install logic. Actual destructive install/uninstall on a clean personal machine remains a manual release smoke before major public releases. |
| F24 | Sensitive information control | Public repo must not contain private gateway URLs, real keys, or internal config. | `scripts/sensitive_scan.sh --current`, `scripts/sensitive_scan.sh --history`, `public_repo_sources_do_not_contain_private_service_markers`, release manual scan. | Secret-looking token strings, private gateway markers, redaction tests, audit summary hashes, history marker/secret scan. | Covered for current tree in default CI; history scan is available but not default CI. Run `--history` before force-push, public release, or after any history rewrite. |
| F25 | Documentation and quality gates | Users and maintainers can understand architecture, tests, release risk, feature coverage, and warning regressions. | `docs/architecture.md`, `docs/test-strategy.md`, this document, `scripts/test_contract_check.sh`, `scripts/clippy_check.sh`, `scripts/ci.sh`. | CI gate list, feature matrix, release audit, maintenance rules, managed feature-row presence, four-dimension coverage floor, workspace clippy over all targets with `-D warnings`; behavioral coverage is enforced by executable tests rather than test-name searches. Broad architecture-shape lints (`too_many_arguments`, `type_complexity`, `large_enum_variant`, `result_large_err`) are explicitly allowed so the gate focuses on actionable warning regressions instead of mechanical API churn. | Covered by this change. Future feature work must update this document and its contract checks. |
| F26 | Changelog and release notes | Users and maintainers can see what changed before installing, updating, or tagging a release. | `CHANGELOG.md`, `scripts/test_contract_check.sh` changelog existence/content checks, release checklist, sensitive scan. | Unreleased section, tagged release sections, current public-source scan, release checklist requires release note review, no secrets/private endpoints in notes. | Covered. Future release tags must move relevant Unreleased entries into the tagged section. |
| F27 | GitHub Actions production CI | Pushes and pull requests automatically run the same quality gate used locally. | `.github/workflows/ci.yml`, `scripts/test_contract_check.sh` workflow existence/content checks, `scripts/ci.sh`, GitHub matrix for `ubuntu-latest` and `macos-latest`. | Linux/macOS runners, expect dependency install, stable Rust install, local CI script reuse, push/PR triggers, concurrency cancellation, no separate weaker remote test path. | Covered structurally and by local contract checks. Actual remote green status is verified after push on GitHub. |
| F28 | Bridge modularity and Interface readiness | Agent behavior remains reusable outside the terminal Interface, so future Rust-native, cross-language, or remote Interfaces can reuse the same Core semantics instead of forking agent logic. | `agent_core_stays_terminal_ui_free_for_host_adapters`, `agent_core_dispatches_owned_structured_topic_events_to_host_sink`, `core_init_lifecycle_topic_is_structured_and_ui_neutral`, `core_lifecycle_topic_round_trips_worker_identity_workspace_and_context`, `shell_renders_core_lifecycle_topic_as_startup_status`, `session_worker_emits_lifecycle_runs_turn_and_accepts_mid_turn_supplement`, `session_worker_rename_emits_updated_identity_topic`, `shared_worker_runtime_publishes_global_working_count_on_model_response_topics`, `turn_ui_decision_requests_are_structured_and_ui_neutral`, `stale_context_decision_request_is_structured_and_ui_neutral`, `config_apply_report_is_ui_neutral_command_data`, `session_turn_truncated_output_replays_partial_response_and_requests_small_iteration`, `session_turn_round_limit_continue_recharges_and_finishes_same_task`, `protocol_repair_does_not_publish_invalid_model_response_topic`, `docs/architecture.md` Bridge/Interface Boundary section, `NoopTurnUi` defaults, model transport tests. | `agent_core` has no terminal UI dependencies, keeps JSON-in/JSON-out C ABI entry points, and owns model transport, protocol parsing, tool execution, structured Agent state, requests, lifecycle topics, UI-neutral worker projections, and `CoreTopicEvent` dispatch; `timem_session` owns per-Context worker threads, multi-worker management, and shared worker runtime status; `bridges/in_process` is the zero-transport typed boundary for all same-process Rust Interfaces; `timem_shell` owns terminal UI, command parsing, env loading, host audit-path selection, and `TurnUi` interaction/rendering without direct Agent/Session dependencies. Topic callbacks are synchronous; asynchronous Bridges clone the event fields they retain. | Covered for the current Shell and Web architecture. A new same-process Rust Interface must add an in-process Bridge contract test; a cross-language client must add a real FFI binding test before `bridges/native_ffi` exists; a separate-process client must prove its HTTP/WebSocket or IPC transport. |

| F29 | Sudden action-output context guard | A single unexpectedly large action result cannot jump across both the 90% compact trigger and the model's hard input limit. | `sudden_large_action_output_is_replaced_before_crossing_safety_limit`, `combined_multi_action_output_is_budgeted_as_one_delta`, `action_output_at_or_below_safety_limit_is_preserved`, `action_output_budget_accepts_exact_95_percent_and_rejects_the_next_token`, `non_ascii_action_burst_uses_conservative_token_estimation`, `same_batch_pending_action_updates_are_removed_with_oversized_delta`, `build_next_prompt_guards_pending_precheck_output_without_losing_user_input`, `session_turn_replaces_a_sudden_large_action_delta_before_next_model_call`. | Separate 95% pre-commit boundary, exact boundary transition, single and combined multi-action bursts, CJK/non-ASCII conservative estimation, real run_bash and pending memory-precheck results through the session loop/build boundary, USER input retained, oversized candidate Delta and same-batch action outputs absent from the next prompt, bounded RUNTIME warning. | Covered. Keep this distinct from the earlier 90% proactive context compact policy. |
| F30 | Action failure isolation | A failing tool or crashing child command cannot terminate Timem or silently masquerade as success. | `builtin_execution_contains_callback_panics`, `normal_bash_contains_child_sigsegv_and_accepts_follow_up_command`, `supervisor_reaps_sigsegv_background_job_and_reports_signal_transition`, `command_action_contains_script_sigsegv_and_executor_remains_usable`, `run_bash_child_sigsegv_isolated_and_turn_can_still_finish`, parallel action panic handling tests. | Builtin callback panic containment, `internal_error` audit semantics, foreground and background `run_bash` SIGSEGV, supervisor-backed core action SIGSEGV, overlay command SIGSEGV, reaping and one-time signal transition evidence, and successful follow-up command/turn after failure. | Covered for Rust unwinding panics and child-process faults. Native faults inside the core process require process-isolated capabilities and are not recoverable through `catch_unwind`. |
| F31 | Response-specific protocol repair guidance | A malformed model response receives a correction that identifies its concrete structural mistake instead of repeating a generic protocol reminder. | `root_repair_moves_free_talk_inside_response_with_matching_action_branch`, `root_repair_selects_the_branch_present_in_the_malformed_response`, `malformed_raw_responses_map_to_distinct_issue_and_guidance`, `malformed_response_corpus_maps_raw_output_to_precise_repair_reason`, `final_answer_can_contain_multiple_adjacent_response_examples_as_text`, `non_root_repair_keeps_issue_specific_static_instruction`, `xml_native_actions_reject_unsafe_xml_constructs_and_resource_exhaustion`, `xml_native_action_batch_is_atomic_when_a_later_action_is_invalid`, `session_turn_xml_root_repair_explains_exact_structure_then_continues_action`. | A 30-case raw malformed-response corpus runs through the real XML parser and asserts the exact issue plus standardized Exact/Cause/Correction guidance: empty output, content before/after the single root, missing/unclosed/self-closing/double roots, unknown text/tags, duplicate/out-of-order/unclosed fields, conflicting state branches, invalid/obsolete/empty action workflows, missing tool names, non-object args, unsupported tools, manifest argument failures, and incomplete context compaction. Negative coverage proves multiple XML examples inside opaque final-answer text remain data. Session coverage verifies malformed response replay, RUNTIME repair text, realtime repair audit parity, successful corrected action execution, and repair count. | Covered for current XML structural/action/compaction issues. Add a raw-response corpus case whenever the parser introduces a new repair issue. |
| F32 | Local Web host and assistant-ui experience | Users can run a loopback-only local Web UI, or an authenticated public Web UI, with multiple isolated sessions while preserving all core-owned agent semantics. | `applications/timem/tests/unit/web_host_tests.rs`, `static_assets_cache_by_path_class_and_negotiate_gzip`, `background_restore_publishes_sessions_newest_first`, `turn_submit_during_an_active_turn_cannot_merge_into_the_current_turn`, `explicit_supplement_during_an_active_turn_stays_in_the_current_turn`, `keeps ordinary working-session text as a separate next turn`, `active_turn_supplement_consumes_pending_attachments_into_the_same_turn`, `failed_active_turn_supplement_does_not_drop_pending_attachments`, `stale_supplement_after_cancel_completion_starts_a_new_turn`, `duplicate_cancel_commands_are_idempotent_for_one_active_turn`, `guards one browser draft submission while preserving text typed during the pending send`, `keeps drafts and pending send guards isolated by session`, `prunes stale drafts and pending send locks when a snapshot swaps out sessions`, `moves the active session to a live session when a snapshot swaps out the old one`, `does not send new tasks or supplements while a mem switch is pending`, `locks old-session interactions while a mem switch snapshot is pending`, `keeps a human click storm bounded and session scoped`, `interfaces/web/tests/view_model.test.ts`, `composerSendDecision` tests, `working_panel_start.test.ts`, `markdown_outline.test.ts`, `appearance.test.ts`, `scroll.test.ts`, real Chrome acceptance, frontend TypeScript/Vite build, `scripts/web_license_check.sh`, fake-model-server browser smoke, real Aliyun browser smoke. | Tokenless loopback-local access, rotating token authentication for `--public`, loopback port range, CSP/no-referrer/nosniff headers, bounded uploads and browser commands, registered workspaces, strict browser command schema, explicit session creation, per-session model service/protocol/token/policy overrides, server-only API keys, inherited defaults refreshed after host config changes, Session-owned profiles shared by child contexts/workers, explicit Session/Context/Worker topic scope, parent-worker linkage, aggregate worker state, primary/subworker completion isolation, child output and decisions projected into the primary turn, worker-targeted decision relay, no child-created sidebar Session/chat, Session-wide cancel plus primary-only continuation, turn-finish clears stale child-worker working states, repeated Stop is idempotent, stale Stop after completion is harmless, ordinary Send while a turn is active is retained in the durable FIFO queue as a separate next turn, Q1 final output remains visible before Q2 starts, explicit immediate supplements retain same-turn semantics, stale explicit supplement after cancellation becomes a new turn, frontend cancel clicks are same-event-loop deduplicated, repeated Send clicks are same-event-loop deduplicated, pending send completion does not erase text typed during the send, draft text and pending send guards are isolated by Session, stale draft text and pending send locks are pruned when snapshot/mem switch removes a Session, active Session selection moves to a live Session after a snapshot swaps out the old one, mem switching freezes old-session send/upload/remove/history/decision/create/cancel/rename actions until the new mem snapshot is loaded; switching away from live work requires an explicit second confirmation, synchronously stops old-MEM workers, persists unfinished work as interrupted, and offers `timem --space <target>` as the non-destructive separate-instance alternative; Send is blocked while cancellation is in flight, working-turn file attachments are consumed into the active turn's supplement and passed to the worker, stale active-turn races do not drop pending attachments, duplicate attachment removal is both locally guarded and server-idempotent, stale topic/decision replies after turn completion are ignored before reaching workers, stale work-instruction replies during a later active turn are consumed by the host and not relayed to workers, create-session/rename/runtime-update/decision buttons use immediate local pending guards and visible pending labels, reconnect `hello` snapshots clear stale browser pending guards and stale inline decisions, independent `SessionN` and `IDN` worker identities, expandable worker-state navigation, rename and state topics, concurrent real worker routing, cross-session/scope mismatch rejection, stable/deduplicated per-turn event ids, action lifecycle coalescing without stale running rows, five-session concurrent 1,500-event pressure, client bounds of 200 turns and 500 events per turn, progressive 24-turn DOM mounting, prepend scroll anchoring, latest-task follow without overriding intentional history reading, bounded 200-row process rendering, queued decisions rendered in the owning turn, 30-second work-instruction safe default, mid-turn supplements grouped with the original task, working input with a normal send affordance and concise placeholder, pending file attachments removable before send with Session-scoped disk cleanup and failure rollback, long filename ellipsis plus full-name hover, submitted attachments consumed into the user entry with compact filename/size rendering and no repeated prompt injection, low-distraction scrollable free-talk/action/repair/compact process stream, borderless expandable tool rows, model free-talk shown verbatim without invented captions, internal model request/response and work-instruction bookkeeping hidden from activity rendering, nonduplicated activity details, separate GFM Markdown final delivery, long-answer left-side section outlines gated by at least two level-1 through level-3 ATX headings and a strict two-chat-viewport rendered-height threshold, current-section tracking plus owning-viewport navigation, narrow-screen outline suppression, quiet nonzero completion telemetry, trailing workspace-path display, safe external links, syntax-highlighted copyable code blocks, tables/task lists/quotes, persistent dark/light/font/text-size choices with malformed-storage fallback, responsive overflow checks, live multi-round task/latest-call usage, per-session context usage and limit isolation, per-session cache hit rate since the current Web runtime restart (summed cached tokens / summed prompt tokens, with no-usage fallback), final token/time telemetry even without a final answer, live per-context cwd synchronization and active-context display, a mandatory per-Session restart-cwd decision gate that replaces the Web composer until resolved when the canonical runtime startup cwd differs from the stored Session cwd (including unvisited Sessions, reconnect snapshots, same-cwd suppression, same-process MEM-restore suppression, and Host-side Send/upload/ToolGen enforcement), embedded production assets with immutable hashed caching, gzip/identity negotiation and `Vary: Accept-Encoding`; restart restoration publishes the newest stored Session before starting bounded parallel restoration of older Sessions; Apache-2.0 project metadata, and production dependency license allow-listing. | Covered by Rust host integration, frontend reducer/contract tests, Web dependency license scan, Linux/macOS CI builds, and local real-browser smoke. The latest fake-model-server browser smoke verified `Session0` with expandable `ID0 · ready`, one completed lifecycle row with no stale running duplicate, borderless tool rendering, nonzero-only final telemetry, cwd-tail display, and zero horizontal overflow at desktop and 390px. A 30-turn fake-model-server run exposed the rotating-DOM scroll defect; the corrected 26-turn regression crossed the 24-task mount boundary, retained the latest task, exposed earlier-history loading, and kept desktop/390px layouts free of horizontal overflow. The isolated real Aliyun smoke verified the formal working frame, a GFM table, blockquote, Rust syntax highlighting and copy control, task list, completion telemetry, desktop layout, and 390px overflow/composer bounds. Before the first broad Web release, manually smoke Safari and Firefox. |
| F33 | Cross-host Session resume, environment cache, and chat-history paging | A user can restart Timem or switch between Web and Shell without re-entering Session runtime configuration, losing the visible chat trail, or losing the model's ability to consult prior work on demand. Web itself remains available when model service credentials are not configured. | `core/agent/tests/session_store_tests.rs`, `session_index_permissions_protect_cached_environment`, `optional_api_key_config_supports_configurable_hosts_without_weakening_strict_startup`, `stored_session_restores_after_web_host_restart_with_fresh_worker`, `restored_session_keeps_cached_runtime_environment_without_exposing_it_to_web`, `web_startup_can_bootstrap_model_service_config_from_latest_session_cache`, `web_draft_model_service_config_allows_startup_without_an_api_key`, `incomplete_session_model_service_config_blocks_send_without_starting_a_turn`, `runtime_update_propagates_to_existing_sessions_and_new_session_defaults`, `restored_web_turns_follow_history_time_not_turn_id_lexical_order`, `restored_web_turns_preserve_user_entry_kinds`, `turn_user_entries_are_persisted_with_raw_text_and_semantic_kind`, `runtime_restart_preserves_never_dispatched_queue_items_in_message_queue`, `sorts restored entries and events within one turn by creation time`, `history_page_command_loads_older_records_by_cursor`, `shell_resume_uses_stored_session_cwd_for_core_prompt_context`, `shell_resume_prefers_non_empty_launch_env_then_cli_over_stored_session_env`, `shell_resume_ignores_empty_launch_env_values_instead_of_clearing_cache`, `shell_resume_selects_the_most_recent_valid_session`, `shell_runtime_config_changes_are_cached_before_another_turn_runs`, `shell_can_resume_web_style_session_history`, `scripts/cross_host_resume_smoke.sh`, `interfaces/web/tests/view_model.test.ts`, `view_model.test.ts`, production build, `cargo test -p timem_shell`, `cargo test -p timem`. | Shared core `StoredSession` and `ChatHistoryRecord` JSONL schema; effective allowlisted TIMEM runtime configuration is cached per Session, runtime changes and restored legacy records persist immediately, the most recently updated valid Session supplies restart defaults, explicit launch CLI options remain highest priority, and non-empty process environment values override restored Shell Session values so a freshly sourced env file takes effect. Web alone may construct a model service draft with an empty API key so history/configuration UI can load; Send and ToolGen validate the selected Session before creating a turn, while strict Shell/model service paths still reject missing keys. The local Unix Session directory/index use owner-only `0700`/`0600` permissions because the index may contain the cached API key; secrets stay out of browser snapshots/topics, prompts, history, and audit output. Optional user-message `kind` preserves task/supplement/approval/queued_interrupted entries inside one restored turn while old records omit it safely; `queued_interrupted` marks Web queue input materialized as interrupted history by a runtime restart or confirmed MEM switch even though it was never dispatched into a Core Turn, so the model-visible history must not present it as a resumable task; Web write-path persistence stores raw user text plus semantic kind; latest-page restore creates a fresh worker/context; Web history pages use 200-record chunks; malformed chat JSONL lines are skipped; malformed, non-UTF-8, truncated, oversized, and duplicate-ID Session-index records are backed up and repaired through the shared Core path while valid Sessions remain restorable in both Shell and Web; duplicate IDs deterministically retain the newest update; storage tests use scope-owned temporary directories and failed atomic writes are checked for temporary-file cleanup; Shell appends to the same store, restores cwd, and can resume a Web-style Session. | Covered for deterministic storage, owner-only cache permissions on Unix, recent-Session selection, immediate Shell/Web cache updates and legacy migration, real keyless Web process startup, Send-before-turn rejection for incomplete Sessions, strict non-Web validation, secret redaction, corrupted-history tolerance, bounded and restart-idempotent Session-index salvage with exact backups, duplicate-ID reconciliation, temporary-file cleanup, Web restore/paging, frontend replay, Shell resume, Web-to-Shell handoff, restored entry/event order, cwd consistency, and per-session env/profile precedence. |
| F34 | Manual ToolGen and reusable ToolRepo | A user can request preservation from an exact completed task, optionally add guidance, and publish one or more verified reusable scripts without replacing the original delivery; later tasks can discover them by semantic folder/README. | `core/agent/tests/unit/tool_repo_tests.rs`, `capability_tool_toolgen_tests.rs`, `manual_toolgen_continues_in_current_context_and_preserves_source_answer`, `failed_manual_toolgen_has_bounded_protocol_repair_and_does_not_replace_source_result`, `toolgen_runs_beyond_ten_model_calls_with_the_normal_round_budget`, XML retrospective parser tests, `manual_toolgen_uses_system_only_without_optional_user_guidance`, `manual_toolgen_adds_optional_guidance_as_user_component`, `manual_toolgen_publishes_tool_and_retains_the_complete_web_event_chain`, `toolrepo_commands_are_session_scoped`, `toolrepo_detail_rename_and_future_prompt_hint_share_the_published_state`, Web ToolGen/ToolRepo contract and view-model tests, deterministic fake-model-server self-test, opt-in real Aliyun browser smoke. | Explicit completed-turn trigger while Session is idle; optional USER guidance and SYSTEM-only empty-guidance path; same primary worker, Context, and Assistant identity; `[TOOL_GEN_TASK]` SYSTEM marker; normal turn round budget and host continuation decisions instead of a ToolGen-specific model-call ceiling; bounded five-attempt protocol repair; capability enabled only during ToolGen; immediate `Generating tools…` heading while normal free talk/action/repair/retry/live-usage events remain visible; semantic kebab-case folders; one or multiple independent tools; short README, manifest and entrypoint; path/symlink/file-count/size validation; bounded self-test with isolated environment/process group and concurrent bounded stdout/stderr draining; structured approval; atomic publish/update and same-Session mutation serialization; concurrent unique draft IDs; failure cannot replace the source final answer and carries a diagnostic reason; scoped lifecycle topics; searchable filename/code content; detail tree, README, sort, rename, terminal-open. | Deterministic core/host/frontend coverage is required in CI. Real model service certification must prove manual create/publish plus later reuse before release; it is not replaced by fake-model tests. |
| F35 | Session-scoped MCP capabilities | Web users can configure MCP servers once per mem, enable different servers per Session, and let the model discover and execute their tools through the normal Core action pipeline. UI edits become pending Session revisions, while unavailable external MCP servers never block Web startup, restore, worker creation, or unrelated agent work. | `core/agent/tests/unit/mcp_tests.rs`, `legacy_sse_client_discovers_and_calls_tool`, `curl_headers_split_before_sse_events_and_skips_interim_headers`, `stalled_mcp_call_does_not_block_an_independent_server`, `mcp_action_runs_through_protocol_registry_and_executor`, `mcp_server_error_becomes_action_evidence_instead_of_protocol_repair`, `unresponsive_mcp_tool_times_out_as_action_evidence_and_agent_continues`, `mcp_capability_update_is_injected_only_when_tool_content_changes`, `mcp_server_instructions_are_persistent_and_model_visible_changes_append_updates`, `disabling_mcp_server_appends_explicit_persistent_runtime_update`, `native_mode_puts_builtin_descriptions_in_static_and_mcp_descriptions_in_api_field`, `model_transparent_mcp_configuration_update_does_not_append_prompt_delta`, `queued_mcp_update_is_applied_before_the_next_user_turn_prompt`, `mcp_definition_is_mem_scoped_and_session_enablement_is_isolated`, `mcp_toggle_is_deferred_until_the_next_new_turn_boundary`, `session_create_and_restore_defer_unavailable_mcp_discovery_until_send`, `deleting_mcp_definition_removes_it_from_every_session`, `mcp_snapshot_redacts_secrets_and_edit_preserves_unmodified_values`, `legacy_sse_snapshot_redacts_sensitive_headers`, `MCP secret presentation` frontend tests, `stored_sessions_are_host_agnostic_and_sorted_by_recent_update`, Web MCP behavior/contract tests, TypeScript/Vite build. | stdio initialize/discovery/call including server `instructions`, Streamable HTTP JSON/SSE decoding including trailing SSE delimiters and interim HTTP headers, legacy SSE endpoint discovery plus POST/event response correlation, absolute request deadlines under notification traffic, independent-server isolation, failed-transport eviction, environment substitution, namespaced dynamic registry, prompt/executor consistency, deferred desired/applied revisions, nonblocking and deduplicated background discovery, stale mem/config result rejection, cached-tool application at new-turn boundaries, inline mode uses persistent model-visible definition/instruction catalogs plus explicit enable/disable notices; native mode filters those slices, keeps stable builtin descriptions in the static system prompt, sends builtin names plus schemas without duplicate API descriptions, and keeps dynamic MCP descriptions, schemas, and server instructions in the current API tools field; silent runtime-only configuration changes, protocol-shaped strings and nested arguments, server-authoritative JSON Schema validation, natural timeout/error action evidence without protocol repair, mem persistence, Session isolation/restore ids, secret masking as `****`, request-scoped reveal without broadcast, edit preservation, reconnect/delete, transport-specific draft preservation, explicit transport labels, high-contrast Session enablement, synchronous duplicate-click suppression, dark/light/mobile/accessibility controls. | Covered for the current tool-capability surface. Server-initiated sampling/roots requests and MCP resources/prompts remain outside this tool-capability scope. |

| F36 | Web Session API key configuration | A user can start Web without credentials, then reveal, replace, or clear the selected idle Session's API key from the settings panel. The next model call uses the new value, while browser snapshots/topics never receive the secret. | `update_runtime_config_changes_worker_model_service_config`, `existing_session_api_key_can_be_updated_without_exposing_the_secret`, `session_api_key_update_is_rejected_during_an_active_turn`, `session_api_key_update_rejects_invalid_or_oversized_values_before_dispatch`, `browser_commands_are_strictly_tagged_and_do_not_accept_unknown_variants`, `runtime setting drafts` frontend tests, `lets an existing session edit, reveal, and clear its API key without snapshot leakage`. | Session-scoped authenticated update and direct reveal commands; reveal is returned only to the requesting socket and never broadcast; browser plaintext is cleared on panel/session/mem/reconnect/save boundaries; deleting then saving clears the key; validation and 8 KiB bound before dispatch; active-turn mutation rejection; all Session workers updated before the next turn; owner-protected Session persistence; snapshots and acknowledgements expose only `api_key_configured`; applying one runtime field preserves unrelated unsaved drafts; responsive dark/light/mobile controls. | Covered by core worker, Web host, frontend contract, full workspace, Clippy, TypeScript, and production build gates. |

| F37 | Reliable Web command and event delivery | Browser commands are written once on an open, snapshot-ready WebSocket; they are not persisted or replayed by the browser. Host live correlation, bounded process-local deduplication, bounded command/FIFO/event channels, MEM epoch barriers, authoritative Session persistence, ordered semantic events, and reconnect Snapshots prevent silent cross-routing or unbounded resource growth. | `reliable_command_wire_is_legacy_compatible_and_ack_is_correlated`, `concurrent_same_command_id_has_one_executor_but_distinct_ids_both_execute`, `disconnect_after_acceptance_does_not_abort_queued_commands`, `command_results_from_different_sockets_may_reorder_but_keep_their_ids`, `command_lanes_serialize_one_session_without_globally_serializing_other_sessions`, `queued_command_from_old_mem_epoch_is_rejected_before_domain_execution`, `command_dedup_is_process_local_and_does_not_write_workspace_state`, `all_accepted_command_cache_is_bounded_instead_of_evicting_ownership`, browser one-shot command tests, ordered semantic delivery concurrency and snapshot-baseline tests. | Stable live `command_id`; ACKs are control rather than business success; no browser outbox, auto-retry, cross-tab command bus, generic disk dedup ledger, or per-command fragment files; process-local dedup has a hard capacity and explicitly rejects when all slots are accepted; terminal records may be evicted; per-Session FIFO and all channels are bounded; browser reconnect recovers only from authoritative Snapshot/events; Host/Core restart interrupts unfinished work without generic command redrive. | Covered by deterministic disconnect/capacity tests, frontend tests, Host integration, local loopback smoke, and Linux/macOS production CI. Command-specific irreversible effects still require their own idempotency/reconciliation contract. |
| F38 | Web `--debug` diagnostics | Operators can correlate complete model requests, native tool calls, outcomes, latency, CPU, and repair telemetry without weakening normal-session isolation. | `parses_basic_web_launch_options`, `debug_store_initializes_private_artifacts_and_rejects_unsafe_session_ids`, `debug_statistics_accounts_for_every_metric_and_terminal_outcome`, `debug_statistics_length_finish_metrics_have_explicit_empty_state`, `debug_statistics_aggregates_length_finish_response_bytes`, `debug_statistics_isolates_length_finish_metrics_by_endpoint`, `output_limit_truncation_requires_protocol_specific_terminal_metadata`, `model_response_event_preserves_truncated_flag`, `concurrent_debug_updates_are_atomic_and_worker_isolated`, `html_groups_multiple_endpoints_and_escapes_dynamic_values`, `store_refreshes_profile_and_request_outcomes_per_worker_endpoint`, `prompt_dump_keeps_only_latest_request_and_statistics_is_private_html`, `large_prompt_reasoning_markers_keep_reverse_response_order`, `native_prompt_dump_renders_tool_results_as_runtime_in_model_input_order`, `inline_prompt_dump_records_mode_without_inventing_native_payload`, `responses_api_input_payload_keeps_item_order_and_drops_transport_details`, `llm_response_dump_keeps_newest_ten_in_reverse_chronological_order`, `debug_worker_event_pipeline_persists_native_dumps_metrics_and_repair_history`. | Opt-in flag/default-off behavior; immediate creation of all three artifacts; owner-only directories/files; unsafe Session-id rejection; empty state; prompt HTML replacement with only the latest complete request; provider-adapter payload capture in Core; ordered OpenAI Chat `messages`, OpenAI Responses `input`, and Anthropic `system`/`messages` rendering; preservation of prompt text and essential role/type/tool-call/tool-result correlation fields without model, token-limit, tool-schema, cache-control, or other transport details; newest-ten response retention and order; response `worker_id` plus `request_sequence` correlation; reasoning-marker mapping is a two-pass linear scan over the complete prompt and is exercised with 2,000 alternating messages; inline/native model switching; native tool definitions; request/success/failure accounting including stray terminal events; explicit usage-field KVC aggregation with hit rate defined as cached input / prompt input plus cache-read/create totals; output-limit finish count plus assistant-content UTF-8 byte min/average/max; latency, CPU available/unavailable, tool density, repair normalization and root-repair help; HTML escaping and stable endpoint tabs; multi-worker concurrent atomic rendering without temporary-file residue; clean shutdown removal. | Covered deterministically from launch parsing through the real Web worker-event handler. Debug artifacts intentionally contain prompts/tool data, remain local owner-only files, and are removed on clean process shutdown. A live gateway smoke is optional because wire-format parsing is covered separately by model adapter tests. |
| F38b | Debug directory browser and runtime-gone auto shutdown | The WebUI DEBUG path opens a Host-served directory browser in a new tab (works over authenticated `--public` remote access), and a page whose runtime stays unreachable counts down 15 seconds and closes itself instead of burning CPU on reconnect loops. | `resolve_debug_browse_target_rejects_escape_attempts`, `html_escape_covers_markup_characters`, `debug_browse_listing_renders_links_and_parent`, `fixed_routes_dispatch_only_the_expected_http_methods`, Web `view_model` behavior tests. | `GET /api/debug-browse` requires the same token/cookie auth as other APIs, resolves only inside one session's debug directory (canonicalize-verified, no path escape), lists directories with parent links and escaped names, previews text files up to 256 KiB, and caps oversized files with an explicit message; the WebUI DEBUG chip links to it with `target="_blank"` and carries the token for public mode; after 3 failed reconnects the runtime-unavailable dialog shows a live countdown and the page closes (`window.close()` with an `about:blank` fallback) at zero. | Covered for auth, path safety, listing, and countdown wiring. Browser auto-close behavior is limited by tab ownership rules; the `about:blank` fallback guarantees the CPU burn stops. |
| F39 | Timem Web lifecycle and unexpected-exit diagnostics | Normal Web operation has fixed, negligible diagnostic overhead while graceful and abnormal exits leave bounded evidence for the next investigation. | `applications/timem/tests/unit/lifecycle_diagnostics_tests.rs`, `real_process_records_sigterm_sighup_and_sigint_exit_reasons`, `injected_rust_panic_writes_redacted_report_and_preserves_running_marker`, `sigkill_residue_is_promoted_by_the_next_start_without_guessing_cause`, `startup_configuration_failure_records_bounded_error_without_secret_values`, `unavailable_diagnostics_degrades_without_blocking_help`, `web_shutdown_signal_names_cover_terminal_and_service_stops`, `web_runtime_shutdown_stops_all_session_workers`, full `cargo test -p timem`, Clippy. | Module-separated storage/lifecycle/server responsibilities; 64-event fixed ring; start/config/listener/exit-only checkpoints; atomic overwrite without append growth; option names without values; owner-only permissions; UTF-8-safe error bounds and common credential redaction; panic location/thread/forced bounded backtrace; panic-hook lock contention cannot deadlock; exact Ctrl+C/SIGTERM/SIGHUP/parent-exit labels selected by the winning signal branch; graceful cleanup completion; corrupt or SIGKILL-stale running markers promoted as unknown cause instead of guessed OOM/crash; diagnostics-storage failure degrades without blocking the host; real child-process signal, panic, SIGKILL, startup-error, and unavailable-storage fault injection. | Covered on Unix by deterministic unit tests plus real process fault injection. The panic injection branch exists only in debug-assertion builds. Non-Unix uses the same storage and panic paths with Ctrl+C shutdown; native fatal faults and OS-level cause attribution still require platform crash reports. |

| F40 | Linux OS and Timem Web platform correctness | Linux process, filesystem, service-host, and networking behavior must be proven by Linux-native tests rather than inferred from macOS or portable unit tests. | `core/agent/tests/unit/os_tests.rs` Linux cases, `core/agent/tests/unit/data_layout_tests.rs`, `applications/timem/tests/lifecycle_process_tests.rs`, `scripts/linux_web_platform_smoke.sh`, `scripts/web_runtime_lifecycle_smoke.sh`, `scripts/web_public_runtime_smoke.sh`, Ubuntu `scripts/ci.sh`. | `/etc/os-release` and XDG policy boundaries; `/proc/<pid>/stat` start-tick identity; `kill(pid, 0)` liveness; `waitpid(WNOHANG)` running/reaped states; signal exit status; ordinary child termination; Runtime-owned process-group and descendant termination; `run_bash` foreground-timeout/background supervision, nested `sh -c` detach rejection, launcher-exit handoff, Session cancellation, PID start-time identity verification, and no broad signalling of foreign/reused PIDs; current-process/current-group safety guards; selected Unix MEM root creation and existing-directory tightening to `0700`; real headless Linux Web startup with stdin from `/dev/null`, no DISPLAY/Wayland/SSH variables, and an `xdg-open` sentinel proving GUI launch is skipped; tokenless loopback listener and health endpoint; owner-only diagnostics directories/files; SIGINT/SIGTERM/SIGHUP graceful cleanup; SIGKILL residue promotion; parent-launcher death handoff; same-MEM exclusion and different-MEM concurrency; public token/cookie/WebSocket authentication and listener/token rotation. | Covered by an explicit Ubuntu-only CI section plus the full production CI gate. The Linux Web platform smoke uses the release binary and kernel-visible permissions/process signals. Distribution-specific desktop terminal emulators and systemd unit packaging remain deployment integration concerns; headless service behavior itself is covered. |

| F41 | MEM retention and bounded Web storage | Users can configure MEM-scoped age and capacity policies while ordinary work avoids repository-wide scans. | `mem_temporary_retention_is_mem_scoped_persisted_and_applies_to_all_temporary_data`, `temporary_retention_rolls_forward_is_idempotent_and_unlimited_skips_cleanup`, `temporary_capacity_evicts_only_complete_oldest_items`, `conversation_capacity_skips_running_session_and_prunes_another_session`, `ordinary_history_append_does_not_require_or_run_temporary_maintenance`, `temporary_maintenance_runtime_accumulates_across_restarts_without_counting_stopped_time`, `temporary_maintenance_trigger_persists_due_runtime_and_accepts_segment_hint`, `busy_temporary_maintenance_does_not_reset_due_runtime_or_hint`, `successful_temporary_maintenance_completion_resets_runtime_and_clears_hint`, `failed_idle_maintenance_preserves_due_state_records_diagnostics_and_recovers`, `idle_temporary_maintenance_*`, `mem_temporary_commands_return_correlated_results_without_background_broadcast`, `mem_temporary_items_ignore_legacy_shell_job_directories`, `rolling_favorite_rewrite_evicts_oldest_and_keeps_newest_record_complete`, memory command/host tests, runtime settings behavior tests, and the full `timem` and Web UI suites. | Audit and favorite slices enforce their own capacity from small manifests at append/rewrite boundaries without scanning the MEM. Ordinary chat-history appends do not start retention, directory scans, or global capacity accounting. Saving a Settings policy applies cleanup immediately. The fallback maintenance pass becomes due after six hours of cumulative Timem runtime across restarts; stopped time does not count, and a tiny per-MEM state is checkpointed only every 15 minutes plus MEM switch or clean shutdown. A 16 MiB audit-segment rollover writes one lightweight hint as a supplemental opportunity; ordinary audit events do not add hint I/O or scan. Due maintenance obtains the global browser-command barrier, rechecks that no Session is active or queued, and only then performs age/capacity reconciliation. Audit retention reconciles physical segments with its manifest before and after edits, preserves monotonic segment numbering, and leaves a tiny dirty marker so an append after an interrupted pass repairs stale manifest state before trusting it. Successful completion alone resets the runtime and clears the hint; failure preserves the due state, records bounded lifecycle diagnostics, and does not print an internal maintenance code to the user Shell. Temporary-item Top 100 scanning occurs only after the user enters Settings → Memory or explicitly selects Refresh; the result is cached once per MEM in the browser. Capacity evicts complete items/Turns and limited tiers reserve one safe-write slice. | Covered for explicit policy application, incremental segmented stores, idle-only reconciliation, active/pending-work exclusion, command-barrier serialization, Settings-triggered Top scans, and complete-item eviction. The generic temporary-file capacity is a reconciled soft limit because not every producer shares one write API; it may temporarily exceed the configured value until explicit Settings maintenance or the next cumulative-runtime idle pass. |

## Current Supplement Decisions

The following items are not release blockers for the current state, but they are
the next tests to add when the corresponding area changes:

| Area | Why current coverage is not absolute | Next supplement when touched |
|---|---|---|
| Terminal paste across emulator variants | Pseudo-TTY smoke proves bracketed paste mode and core behavior, but iTerm2, Terminal.app, tmux, and SSH can differ. | Run the terminal matrix in `docs/manual-release-smoke.md` before broad releases or input/redraw changes. |
| Live model service behavior | Unit tests use model response fixtures; live tests require credentials and network. | Run the live-model service row in `docs/manual-release-smoke.md` with throwaway credentials when model service behavior is release-critical. |
| Clean-machine install | Script logic and macOS/Linux CI runners are covered, but a fully destructive install/uninstall on a personal clean machine depends on host policy and package state. | Run the clean-machine row in `docs/manual-release-smoke.md` before major public releases. |
| Browser engine variants | Automated smoke covers the in-app Chromium browser; Safari and Firefox may differ in WebSocket reconnect, font metrics, and storage policy. | Run the browser matrix in `docs/manual-release-smoke.md` before the first broadly distributed Web release. |
| Heavy Turn concurrency certification | `scripts/turn_concurrency_stress.sh` implements the 300-iteration PR slice for PromptCut/terminal ownership inside a real Core/Worker plus independent producer, including replayable seed, named stages, TurnToken/final/stat/input ownership, deadlines, and bounded command-ID resource assertions. The Host attachment/FIFO half, Stop/Start storm, real WebSocket/FIFO recovery, Chrome latency percentiles, 1,000 release profile, and 10,000 soak profile remain absent. | Keep the remaining work as a release blocker for any claim of complete concurrency certification; do not count the implemented Core/Worker slice as evidence for Host, WebSocket, browser, release, or soak profiles. |


| F42 | Timem runtime disk-I/O guard | Ordinary Timem work remains disk-light instead of hiding expensive scans or fragmented persistence behind passing functional tests. | `scripts/runtime_io_guard.py`, instrumented `scripts/real_tty_stress.expect`, Linux/macOS `scripts/ci.sh`, uploaded `runtime-io-*` reports, `scripts/test_contract_check.sh`. | Measures only the fully started Timem process tree—not Cargo, CI, Expect, or the fake model server—across an idle interval followed by a real model call, local Bash action, audit/history persistence, and final response. Physical reads plus writes are averaged over the complete measured window and must remain at or below 500,000 B/s. PID start identity prevents reuse confusion; Linux uses `/proc/<pid>/io`, macOS uses `proc_pid_rusage`; child processes are included; workload errors and threshold failures both produce JSON reports. | Covered by a real macOS run (about 101.5 KB/s including the idle interval) and the same gate on Ubuntu/macOS CI. This is an average-workload guard, not a per-second burst cap; explicit user maintenance and corruption/migration recovery are tested separately and are not represented as ordinary work. |
| F43 | macOS OS and Timem Web platform correctness | macOS browser launch, filesystem protection, process identity, Web lifecycle, UI rendering, performance, and runtime resource behavior must be proven on Darwin rather than inferred from Linux or portable tests. | `core/platform/tests/unit/platform_tests.rs` macOS cases, `applications/timem/tests/lifecycle_process_tests.rs`, `scripts/macos_web_platform_smoke.sh`, `scripts/web_runtime_lifecycle_smoke.sh`, `scripts/web_public_runtime_smoke.sh`, `interfaces/web/tests/browser/stop-ui-acceptance.mjs`, `scripts/performance_guard.sh`, `scripts/runtime_io_guard.py`, macOS `scripts/ci.sh`. | Application Support path and native `open` argv policy; kernel-derived PID start-time identity; direct browser URL argument without shell interpretation; selected MEM and diagnostics permissions (`0700` directories, `0600` files); loopback health; SIGTERM graceful exit record and running-marker cleanup; restart, parent-launcher handoff, public authentication, cross-host resume, real Chrome authoritative Stop/reconnect behavior, bounded browser hot paths, and measured process-tree disk I/O. | Covered by the full production gate on `macos-latest`, including real Chrome acceptance and the release-binary macOS platform smoke. Safari remains a manual compatibility smoke because CI uses Chrome; Linux-specific headless/XDG and `/proc` behavior remains exclusively covered by F40 on Ubuntu. |
| F44 | Runtime process containment and model-visible health signals | Long-running, detached, killed, or disk-consuming local work remains contained and leaves bounded, decision-relevant facts for the model without adding per-action filesystem scanning or completed-process noise. | `shell_exec::tests::*` containment/timeout/background/setsid/orphan cases; `linux_managed_process_job_contains_and_kills_setsid_descendants`; `aggregate_process_scope_is_one_shot_and_rearmed_after_compaction`; `runtime_info::tests::*`; Linux process-scope key/identity tests; `command_action_timeout_kills_its_setsid_escapee_with_job_ownership`; `timed_out_job_keeps_owned_setsid_descendant_until_explicit_cancellation`; `completed_job_waits_for_its_owned_setsid_descendant`; Linux fallback ownership tests; managed synchronous-command helper tests; disk-pressure tests; exact-name Linux gates in `scripts/ci.sh`; repeated shell-job edge regression; real field tests. | Linux creates `timem.jobs/runtime-<pid>-<start_ticks>/session-<opaque-key>/job-*`; only Job leaves contain user processes, while Runtime/Session parents are aggregate observation points. Child self-placement occurs in `pre_exec`; `setsid` cannot escape ownership. Cancellation targets only the Job leaf, and terminal delivery waits for `cgroup.events populated 0`. The aggregate Runtime and current Session paths are injected once after startup when they first exist and once again after successful context compaction, never appended on every request and without pre-reading cgroup metrics. Previous-Runtime scopes with live members, live/zombie unowned fallback children, running jobs, killed exits, and disk pressure remain visible until resolved; normal adopted/reaped history is not model-visible. Empty stale scopes are cleaned only after bounded, exact-name, kernel-confirmed checks; live stale scopes are neither adopted nor killed. Degraded process-group mode is explicit when cgroup delegation is unavailable. | Linux kernel ownership is covered by real cgroup-v2 and `setsid` tests plus exact-name CI gates. The current Linux host must provide a writable delegated subtree; permission denial is an explicit capability condition, not silently certified containment. Non-Linux aggregate-scope APIs return no path until a native backend exists; direct-child registration remains portable. Per-Job containment certification outside Linux still requires native implementation/evidence. macOS mount enumeration and Windows fixed-drive/runtime-job behavior retain their platform-specific certification requirements. |
| F45 | Terminal attach client | A terminal can attach to an authoritative Web Host Session and remain bounded during long-running, replay-heavy use. | `attach::tests::*`, `attach_turn_views_evict_oldest_state_at_host_turn_limit`, `pending_decisions_dedupe_live_replays_and_evict_oldest_at_limit`, Host attach endpoint tests, and Shell help/launch parsing tests. | Host discovery and authenticated Session listing; authoritative WebSocket event rendering; restart-cwd and Core decision replies; duplicate final-answer suppression; 256 MiB WebSocket frame/message caps; 5-second HTTP timeouts and 256 KiB Session-list response cap; local rendering state capped at 200 Turns; pending decisions deduplicated by request identity and capped at 200 while preserving newest-reply semantics. | Covered by deterministic client-state tests plus Host command/event tests. Full interactive TTY/WebSocket recovery, Stop/Start storms, latency percentiles, and soak profiles remain part of the explicitly incomplete heavy-concurrency certification and must not be inferred from these unit bounds. |
## Adversarial Audit Notes

The current suite is broad, but the following claims should not be overstated:

- Model service features are fixture-strong, not live-model service-complete. CI proves
  request/response shaping and representative error handling, not every vendor
  deployment behavior.
- Terminal input behavior is pseudo-TTY strong, not emulator-complete. Real
  terminals can differ in bracketed paste, keyboard protocol, tmux, SSH, and
  locale behavior.
- Install/update behavior is logic-tested, not clean-machine-proven.
- Default CI scans the current tree for secrets. History scanning is available
  and must be run before history rewrites or public releases where history risk
  matters.
- The feature ledger is guarded for managed feature-row presence, but it still
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
8. Run applicable rows from `docs/manual-release-smoke.md` when the release is
   broad, host-facing, or touches terminal/Web/model service/install behavior.

### Stream UI reading handoff

Stream UI keeps all thought rounds, supplements and interim answers in chronological
order throughout authoritative working state. Model-response boundaries never archive
previous content. Accepted interim answers reuse preview attempt/index identity.
Only leaving working starts a height-dependent 320–700 ms height/opacity handoff into collapsed Thought/Action;
completion and interruption both archive, while ordinary mode keeps its working panel.
Reduced-motion skips animation. Bottom following during handoff stops on wheel/touch.

Completed tools retain their DOM identity: status highlights locally, output folds with
100 ms delay and 360 ms easing, and completed calls in an adjacent tool run merge after
600 ms. Calls separated by thought, answer or supplement are not merged. Groups and
outputs can be reopened. Adjacent failed calls merge with successful calls; the summary uses bold Tools followed by N Succ | M Failed. Expanding the group retains failure details for diagnosis. No typing caret is
rendered; the growing trailer dot alone indicates live work.

Commands use the assistant reading font. Thought/interim text shares final-answer
font size and line height. Coverage: activity grouping, stream reveal, lifecycle identity
unit tests and Chrome continuous-stream acceptance (multi-round retention, adjacency,
reopening, terminal/interruption animation and reduced-motion). Subjective visual quality
still needs user review; automated tests verify behavior rather than aesthetic preference.

### Action 状态局部更新

- Web 同一 action 的生命周期合并保留首次可见的展示 ID 与排序时间；权威事件 ID、执行时间及耗时计算不变。
- 状态变化不得重挂载整行或命令节点；仅状态字段短暂高亮并通过 polite live region 提醒，减少动态效果时改用静态下划线。
- 回归：`view_model.test.ts` 覆盖执行/后台/完成及裁剪历史身份；`stream-preview-acceptance.mjs` 验证真实浏览器 DOM 身份、单行数量和状态局部提示。

### 流式区视觉交互验收

- 新增完成调用不得重新展开已合并历史；失败调用参与相邻工具合并并计入 Failed 数量；展开组后失败详情默认展开且允许手动折叠。
- 用户选中文字或焦点位于工具内容中时保留内容，避免自动折叠中断复制和键盘操作。
- 归档前将内部焦点转移到思考框按钮；滚轮、触摸和滚动键可以打断底部跟随。
- 归档时长随高度变化并限制在 320–700 ms，减少动态效果时直接归档。
- 粗指针控件至少 44px，合并控件有可见键盘焦点，窄屏长标签允许换行。
- Chrome 回归覆盖合并稳定性、文本选择、失败详情开关及 390/768px 横向溢出。
- 尚未完成：Safari/Firefox 真机验证、所有 Markdown 高度变化的阅读锚点验证、屏幕阅读器实测、全站设置/侧栏/会话切换视觉验收。不能以流式区通过代替全站视觉通过。

### 流式渲染开销

- 工具行按实际展示字段 memo；活动列表按输入引用缓存，避免预览更新重复创建历史工具行。
- selectionchange/focusin 使用共享监听，随最后一个订阅卸载清理；订阅量跟随已挂载行，不积累历史记录。
- 已合并组不重复启动合并定时器；非底部跟随不启动归档逐帧循环，用户输入立即取消循环。
- 验证包含源码约束测试、启用阈值的 Web 性能门禁和 Chrome 行为回归。这些结果不是实际会话 CPU 降幅测量；仍需同负载浏览器性能采样判断剩余热点。

### Chrome 流式渲染实测

使用生产 dist、独立 headless Chrome 和确定性模拟 Host，通过 CDP Performance 测量
120 次、间隔 40ms 的增长文本更新，并继续观察 1500ms。运行命令：
`STREAM_CPU_BENCH=1 node interfaces/web/tests/browser/stream-preview-acceptance.mjs`。

文字推进仍按帧累计字符预算，但 Markdown/React 绘制合并为约 32ms 一次，末尾可立即提交。
一次优化前 Task/Script/Layout 耗时分别为 1.56089/1.049251/0.105822 秒，布局 381 次；
优化后两次分别为 1.234403/0.808963/0.058852 秒、192 次，以及
1.253865/0.819110/0.058257 秒、193 次。这是固定采样窗口的主线程开销，
并非整个 Chrome CPU 百分比，也不是用户原高占用标签页的性能追踪。
测试使用持续增长的文本，不代表所有复杂 Markdown、长历史或高并发情形。

### Tool command presentation

Thought/Action tool groups omit the left rail; keyboard focus uses a thin outline rather than a thick left stripe. Stream tool disclosure buttons precede the tool name, both collapsed and expanded. Chrome stream-preview acceptance checks left-side placement and dark/light archived tool styles.

Completed stream tools omit the dot before the tool name, including when reopened; running/background-running calls retain it. Chrome lifecycle acceptance verifies the transition without remounting the row or command.

Retired stream tools fold away completely under the collapsed Tools control as ~32px summary bars (tool name, duration, status) with hover brightening and a native tooltip preview of the captured command, while the running step keeps the breathing dot plus a glow pulse and a live log clamped to 120px with a fade-out mask. The timeline is pure-CSS decoration (::before rail, existing fold/merged transitions), so row identity, leading-slot alignment, the peers contract and scroll stability are unchanged; Chrome acceptance asserts bar height, rail geometry, glow animation and the 120px clamp.

### Stream / ordinary UI regression ownership

`pnpm --dir interfaces/web test:browser` (also called by `scripts/ci.sh`) owns:
- Chrome simulated Host: both UI modes, completion/interruption, reload, manual expansion, dark/light archived styles.
- Real Host + HTTP SSE: XML/JSON/native normal delivery in both UI modes; ordinary interim collapse/reopen and tool visibility; stream invalid response, disconnect, Stop, supplement, long-text clipboard/reading anchor and tool execution.
- Stream completed-dot removal, adjacent success/failure grouping with **Tools** N Succ | M Failed, reopening diagnostics, and dark/light user-colored supplement bubbles.
- Deterministic Chrome stream performance window: 120 updates at 40 ms plus 1500 ms settling; main-thread TaskDuration < 4 seconds and LayoutCount < 500. These generous regression ceilings are not a CPU percentage or a guarantee for all content; failure must be investigated rather than raising limits to pass.

`scripts/performance_guard.sh` additionally checks 20,000 mixed activities through stream retention and ordinary grouping within 1500 ms, with correctness assertions, alongside lifecycle/event-queue/scroll guards.
Long-text browser acceptance retains 180 paragraphs; its 60-second bounded wait accommodates the 240 UTF-16 units/second progressive reveal cap without reducing the fixture or skipping clipboard/scroll checks.
Generated dist changes are rebuilt main JavaScript and CSS plus index.html hashed references; dependency chunks and fonts remain unchanged.

Scope: module tests and applicable architecture/performance/browser guards do not replace the full repository `scripts/ci.sh`, cross-browser manual review, or screen-reader testing.

### Final-answer handoff scroll geometry

The portaled final-answer outline observes its enclosing Turn size as well as answer size.
Collapsing preceding tools changes the answer offset without changing the answer height;
leaving the outline at its old absolute top creates phantom scroll space and can hide the
answer above the viewport. Resize invalidation remains frame-coalesced and observers are
cleaned up; it does not introduce unconditional scrolling or override the reader's position.
Chrome stream acceptance covers 80 tools plus a multi-section final answer in both UI modes
and with/without reduced motion, asserting actual answer/viewport intersection and less than
150px trailing scroll space after archive. The fixture failed before the fix with an invisible
answer and approximately 1567px phantom space. This regression runs through test:browser/CI.

### Web 工具结果文案

- 工具成功/失败标签统一为 `Succ` / `Failed`；不展示原始 `completed`。
- 合并工具计数在失败数为零时隐藏 `0 Failed` 及分隔符，非零时保留失败计数。
- 仅改变 Interface 展示，不修改 Host 状态或运行中、超时等状态语义。
- 回归：`interfaces/web/tests/tool_status.test.ts`、`tool_activity_layout.test.ts`。
- 新增工具并入折叠区时，仅计数播放 360ms 上移/高亮反馈，不重挂载按钮或改变滚动；减少动态效果时禁用动画。回归：`stream_reveal.test.ts`。

### Web 工具计数反馈与紧凑间距回归

- Chrome 验收 `interfaces/web/tests/browser/stream-preview-acceptance.mjs` 校验真实 DOM：初始计数无动画、隐藏零失败、非零失败保留、终态 Succ、连续新增逐次动画、重复快照不重播、按钮和既有工具行不重挂载、动画结束无残留、减少动态效果禁用动画但保留计数更新。
- 连续新增 20 个工具的浏览器预算：主线程 TaskDuration < 4 秒、LayoutCount < 500；检查唯一计数节点和全部历史保持折叠。预算始终启用，随现有 `test:browser` / CI 入口运行，不只检查源码字符串。
- 流式区域内部 gap 与底部 margin 使用 `clamp(.25rem, calc(var(--content-size) * .375), .75rem)`，16px 正文字号下为 6px；计数动画位移使用 `.25em`。保留原有按钮触摸目标尺寸。
- Chrome 响应式回归覆盖 390/768/1440 CSS 像素视口、12/16/24/40px 正文字号、100%/150%/200% CSS zoom 共 36 组，视口分别采用 DPR 3/2/1；检查计算间距、页面横向溢出、计数标签与按钮非零尺寸，另测根字号下限缩放。CSS zoom 不等同于浏览器原生缩放，未宣称覆盖全部 DPR 交叉组合或真实设备。
- 原有 80 工具最终答案交接仍覆盖两种显示模式和两种动态效果设置，防止答案不可见及旧目录位置撑出空白。

- 补充边界回归：空计数、取消/错误/未知状态语义不变；Chrome 根字号变化下的间距上限；单次 Host 快照批量完成成功与失败工具时只反馈一次，重复快照不重播且历史保持折叠。

### Web 单行工具摘要与下一回复交接

- 运行中默认收起详情，摘要单行显示工具名、状态、截断命令，按钮可展开完整命令及输出；完成/失败不改变手动展开状态。
- 完成工具不再按计时器合并；后续 AI 正文/答案到来才渐隐折叠，单工具同样适用。用户补充或工具完成本身不触发交接；选区与详情焦点仍阻止强制折叠。
- 回复边界定位单次反向扫描，避免按工具分组重复扫描后缀；沿用 CSS 高度/透明度短过渡及 reduced-motion，无新增逐帧 JS 或计时器。
- Chrome 回归检查运行时默认关闭且可展开、完成后等待 700ms 高度仍保持（误差 < 1px）、下一回复收起单工具、重新展开保留详情状态。已有批量计数、重复快照、36 组响应式、80 工具最终交接及 CPU 预算继续执行。
- 本节替代前述“完成立即折叠/失败默认展开”的旧展示约定；仅为 Interface 行为，不改变 Host 生命周期。

### Stream Chat disclosure

Stream UI renders each interim answer with a Chat disclosure. A confirmed answer
collapses when later AI content arrives, not on a tool completion or user supplement;
provisional Chat stays visible while streaming. Users can collapse/reopen it manually.
After stream archival or history reload, confirmed answers live in the collapsed
Chat panel rather than Thought/Action. Core delivery and Turn semantics are unchanged.
Chrome `stream-preview-acceptance.mjs` covers latest-answer visibility, next-reply
collapse, manual reopening, completed-history recovery, preview continuity and both UI modes.

### SSE failure audit and execution indicators

- SSE wire event capacity is 4 MiB; preview parser limits remain 1 MiB and the
  whole HTTP response limit remains 16 MiB.
- Stream decode failures produce a bounded `llm_response` audit record with
  `error_kind=stream_decode_error`, HTTP status, normalized SSE content type,
  received bytes, timing, redirect count, safe bounded provider request id,
  and completed-event/current-line/current-event byte counters. No failed body
  is persisted. `audit_request_id` correlates request and response records.
- Request audit write failure fails before sending; stream-failure audit write
  failure preserves the original error with an audit-write failure marker.
  This does not yet add diagnostic snapshots for all transport errors.
- Coverage: `stream_failure_is_audited_without_response_body` (oversized and invalid
  JSON over real local HTTP), `sse_event_between_one_and_four_mib_is_accepted`,
  `unterminated_and_oversized_events_never_emit`, existing preview bound tests.
- Execution dots only represent running/background-running actions. Chrome
  stream acceptance tests bash/readfile across success, failure, timeout,
  cancellation and both running states; ordinary action views already use the
  shared running-state predicate.

Tools absorption feedback tracks newly absorbed completed results, not merely
completion count changes. Deferred next-reply handoff pulses the count even if
completion happened earlier. Initial snapshots and manual reclose do not replay.
Component/CSS comments preserve these visual contracts; Chrome acceptance checks
the deferred absorption case alongside count increments and reduced motion.

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

Stream tool rows use the breathing dot alone for running state, with an accessible
label. Background execution shows only `bg`, never redundant `running` text.
Terminal result labels use the shared success/failure symbols. Chrome
lifecycle/status-matrix acceptance guards this visual contract.

Terminal tool result labels use `✓` for success and `✗` for failure, including
Tools counts (for example `2 ✓ | 1 ✗`). Accessible labels retain full words.
Unit and Chrome count/status acceptance tests guard the exact symbols.

### 用户气泡复制与工作区重绘回归

- 用户气泡选区延伸到相邻布局空白时，复制处理由聊天视口接管；只处理单条用户消息，不改写跨消息选区，也不在输入框粘贴时全局裁剪文本。
- `interfaces/web/tests/browser/stop-ui-acceptance.mjs` 使用真实 Chrome 剪贴板验证气泡内容、整节点、局部文字、延伸至助手区起点的选区及粘贴到 composer；修复前边界选区复现尾部三个换行。
- 工作中回答框保留静态状态边框，移除持续改变大面积边框与阴影的呼吸动画；浏览器验收断言其 animationName 为 none。小型状态指示不变。
- 此项减少已知持续重绘源，不代表已测得用户环境 CPU 或温度下降；Safari/Firefox 和实际用户 Chrome CPU 对照仍需验证。

### 流式工具状态位置连续性

- 运行圆点与完成/失败标记共用工具名称前的固定最小宽度状态槽；后台 `(bg)` 仍位于名称后并以较淡颜色显示，避免圆点消失后结果跳到另一侧。
- `tool_activity_layout.test.ts` 守卫 DOM 顺序和状态槽样式；`stream-preview-acceptance.mjs` 验证状态标记居中且位于工具名称前，状态更新不重建工具行。

### Tools disclosure alignment

Collapsed stream tool summaries use `+ tools N ✓ | M ✗` (omit zero failures).
Tool status/layout unit tests guard labels and inset; stream-preview Chrome
acceptance checks summary/live disclosure alignment at 1440px and 390px, mixed
counts, and reopening results. Individual terminal failure labels remain `✗`.

The `tools` label is lowercase without a colon; the entire summary row,
including success/failure counts, uses normal font weight (400).

### Low-cost stream rendering

- `interfaces/web/tests/stream_reveal.test.ts`: 40ms refresh-independent reveal
  scheduling, preserved character pacing, hidden/reduced-motion immediate text.
- `interfaces/web/tests/browser/stream-preview-acceptance.mjs`: short tool entry,
  static duplicate activity cues, no blurred scroll navigation, plus existing
  multi-round handoff, failure, selection, count and reduced-motion coverage.
- The `STREAM_CPU_BENCH=1 TIMEM_PERF_GUARD=1` variant retains main-thread/layout
  budgets; measurement scope and limitations are in `web-performance-tracing.md`.

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

### Atomic tool absorption

Automatic tool absorption no longer animates grid height or opacity: collapsing
rows and presenting the next tool settle in one layout update. Tool nodes remain
mounted and only the absorbed count pulses. This avoids repeated movement of the
following content during a height transition; it does not freeze global scrolling
or reserve permanent blank space. Content shrinkage may still require a single
viewport adjustment. Manual disclosure and selection protection are unchanged.

Coverage: tool layout guard and Chrome serial-handoff sampling check that the
viewport scroll position and running row position vary by less than 1px across
18 frames after the committed handoff. This checks post-commit stability, not
zero displacement between the pre-handoff and post-handoff layouts.

### Running tool breathing indicator

Running and background-running tool dots breathe from scale .65 to 1 over a
1.2s cycle using only transform and opacity. Their 8px layout size and status
slot stay fixed; terminal tools use result markers instead. Reduced motion
disables breathing. Tool absorption remains free of height animation. Chrome
coverage seeks animation time to verify changing dot size with a stable slot
for bash/readfile and both running states, plus reduced-motion behavior.

## Model endpoint binding regression coverage

`shared_model_endpoints_are_persisted_redacted_editable_and_deletable` covers stable
Session endpoint IDs, full route edits, renames, persistence round trips, active
Turn deferral, next-Turn resolution and deleted-binding rejection.
`endpoint_edits_and_switches_apply_at_the_next_new_request_boundary` uses a
blocking model client to capture the complete configuration of three real model
requests: an in-flight request retains its reasoning level after an endpoint-only
edit, the next new Turn receives that edit, an active endpoint switch updates only
the durable binding, and the following new Turn receives the selected endpoint's
model, URL, protocol, token limits, stream mode, credentials, headers, request
fields, transport policy, reasoning level and requirements. Web
`model_endpoints.test.ts` covers ID-based labels/selection and deleted bindings;
`endpoint_header_button.test.ts` guards basic-field-first editor order and keeps
endpoint selection available while a request is working with next-request copy.

Browser endpoint layout acceptance is included in
`interfaces/web/tests/browser/stop-ui-acceptance.mjs`: real Chrome checks dark/light
themes at 1440px and 390px, basic field order, responsive grid columns, horizontal
control overflow and API key masking/reveal. The same run retains the existing
Stop/reconnect/scroll acceptance checks. Geometry assertions are not screenshot
comparison or a substitute for human aesthetic review.

## Image paste and visual input regression coverage

`attached_images_reach_every_provider_wire_format` proves pasted images reach
OpenAI Chat (`image_url`), OpenAI Responses (`input_image`) and Anthropic
(`image` base64) request bodies, with Anthropic inline mode merging parts into
its single user message instead of emitting consecutive user messages.
`attached_images_append_after_native_history_without_touching_cache_marks`
keeps image delivery after projected tool results and leaves cache-marked
deltas untouched. `multimodal_audit_events_redact_image_payloads` keeps base64
payloads out of audit dumps. Host `turn_image_parts_encodes_images_skips_files_
and_fails_closed` covers base64 encoding, non-image skipping and oversized or
unreadable image rejection. Web pasting reuses `clipboardImageFiles`
(`tests/clipboard_images.test.ts`) and the existing upload pipeline; the
composer `onPaste` wiring itself is not yet covered by an automated browser
scenario.

### System tool-result retention and Beta display preferences

Settings → System owns the MEM-persisted **Maximum retained length per tool** setting. The exact choices are 30K, 20K, 16K, 10K and 8K (KiB); old MEM settings migrate to the 16K default. Host validates the enum, projects it through snapshots and `mem_settings_updated`, updates existing Session workers, and applies it when creating future workers. Core bounds the complete structured action-result JSON envelope to the selected byte budget while preserving valid JSON and each action's head/tail retention policy. The absolute supported maximum is 30K.

The same System page contains a lower-level **Beta features** section. It exposes **Tool Result Status**, a browser-local presentation choice.
With no saved choice, both it and Stream UI Mode default off normally and on when
Host `server.debug_mode` is true (`--debug`). Explicit true/false choices survive
reload and take precedence over defaults on reconnect; defaults are not persisted.
Storage denial falls back to a tab-local choice; storage events synchronize tabs.

When result display is off, terminal tool rows and groups show `Done` (folded
stream counts: `N Done`), including accessible labels, without success/failure
coloring. Foreground/background running indicators remain active. Raw Host tool
statuses, execution results, output details, timeout/process evidence and model
inputs are unchanged. `Done` describes completion of the call, not correctness
of the task or termination of a process that outlives a wait budget.

Coverage: Core action-result tests check the 8K/10K/16K/20K/30K choices, the 16K default, invalid-value rejection, and complete-envelope bounds. Host Web tests check old-file migration, persistence, snapshot/event projection and invalid command rejection. `beta_settings_ui.test.ts` guards the System navigation, exact selector choices and command/protocol wiring. `beta_preferences.test.ts` checks defaults, explicit choices, reload,
reconnect, cross-tab updates and unavailable storage; `tool_activity_layout.test.ts`
guards the group wiring. `stream-preview-acceptance.mjs` retains result-mode
mixed success/failure assertions and checks live neutral-mode switching, folded
counts and accessible row labels. Web dist changes reflect the preference store,
settings UI and display integration (entry chunk/hash and HTML reference).

### Structured direct-resume prompt entry and startup ordering

Core's explicit direct-resume entry submits `user_resume_directly`, a User
component with an empty body. JSON/native Markdown renders
`## USER (user resume directly)`; XML renders
`<USER kind="user resume directly">` with an empty body. A user who actually
writes `user resume directly` still gets an ordinary USER entry: routing must
use structured intent, never string matching. Supplements retain their existing
USER (supplement) header. Approval, round-budget, output expansion and stale-context
decisions remain runtime decision/evidence paths, not fabricated user messages.

Turn-start supporting context (restart/history, cwd instructions and attachment
context) precedes the initial user entry. Later runtime observations retain their
chronological position; no role-wide sorting is introduced. Prior pending assistant
output stays before the new user input. Internal turn IDs and turn-boundary markers
are not injected into model prompts or prompt examples; runtime turn ownership and
audit identities remain unchanged. Empty ordinary user inputs and supplements remain omitted.

Regression coverage: `core/agent/tests/unit/lib_tests.rs` exercises direct resume
in JSON/XML and inline/native modes, empty component preservation, interruption
ordering, literal-text nonclassification and supplements; existing prompt component
ordering and renderer tests cover repeated roles and protocol escaping.

### Compact live text/tool rhythm

Live tool grid spacing is 0.125× reading size (bounded to 2–4px at a 16px
root), tool rows/disclosures have 2px vertical padding, and direct live thought
blocks do not add a bottom margin on top of the grid gap. Text line-height and
archived/non-stream paragraph spacing are unchanged; coarse-pointer controls
retain 44px minimum targets. Layout unit tests and Chrome acceptance check these
bounds along with responsive overflow and stable tool folding. Regenerated Web
assets include CSS plus the entry chunk/HTML references to the new CSS hash.

### Web UI localization (zh/en)

Settings > Appearance adds a Language segmented control backed by the
browser-local locale store (`src/i18n/locale.ts`): localStorage persistence,
`navigator.languages` default, storage-denial fallback, cross-tab sync, and
`<html lang>` updates. `strings.en.ts` is typed against the zh source catalog,
so missing or extra keys fail `tsc`; `tests/i18n.test.ts` checks parity,
non-empty values, interpolation, runtime switching, and unknown-key fallback.
`tests/i18n_source_guard.test.ts` keeps user-visible CJK literals inside the
catalog only (comments stripped first). Directory-copy assertions pin
`setLocale("zh")`; browser acceptance launches Chrome with
`--lang=zh-CN --accept-lang=zh-CN` so default-render assertions stay stable.
CSS pseudo-element labels read localized `data-*` attributes instead of
hardcoded `content` strings.

### Memo recreation and deletion reminders

Creating or updating an active memo cancels a pending reminder about a previously
deleted memo and rearms the one-shot reminder for the next deletion. Regression
`memo_recreation_cancels_stale_delete_notice_and_rearms_next_delete` covers initial
creation, delete/create before prompt rendering, repeated deletion cycles, and
one-shot delivery. This does not change the active-memo finish guard.

### Manual context compaction at terminal handoff

A manual compact marker accepted before `task_finished` must be returned as a
structured unconsumed request, including a marker already drained into the worker
flag. On clean completion the Host starts a direct-resume turn without requiring
another user message; mixed text supplements retain their normal handoff. Cancel
and error boundaries retain requests without automatically restarting work.
The model loop consumes mailbox-only compact markers before dispatch and does not
consume their flags at a terminal step.

Regression coverage: `manual_compact_at_finish_is_handed_off_without_waiting_for_user_input`,
`task_finished_hands_manual_compact_off_to_direct_resume`, and
`mailbox_only_manual_compact_is_consumed_before_model_dispatch`.

## Turn timeline delivery and archived detail recovery

- Live `core_topic` / `worker_activity` carry the same `timeline_seq`, timestamp and event ID as the Host Turn; whole-turn updates must not reorder subsequent live progress.
- Completed-turn snapshots retain a compact 40-event preview. Expanding details requests `turn_history_page`, scoped to Session + Turn, with at most 64 records / 2 MiB of source data per page. Oversized records fail explicitly; raw history is never rewritten.
- Detail replies are connection-local queries, not lifecycle projections. Reconnect clears pending page assembly; completed detail replacement leaves final outcome intact.
- Host regression: `live_worker_wire_preserves_snapshot_timeline_metadata`, `bounded_turn_history_pages_recover_all_events_beyond_snapshot_tail`.
- Chrome regression: `interfaces/web/tests/browser/stream-preview-acceptance.mjs` sends a real incremental topic after a sequenced snapshot, and restores early/late thoughts and tools from an 80-event archive over multiple pages, twice across reload.
- Gates: `cargo test -p timem --lib`; Web `pnpm test`, `pnpm build`, `node tests/browser/stream-preview-acceptance.mjs`; architecture/module/test-contract guards.

## Restart-heavy legacy history pagination

- Structured system messages with `kind=runtime_restart` remain in history but do not consume the conversation-turn page limit. Detection is independent of turn-ID naming and message wording.
- Cached append, cold index rebuild and uncached readers share the same page boundaries; record-offset cursors and JSONL schemas stay unchanged. Reading never migrates or rewrites old logs.
- Regression: `restart_notices_do_not_displace_legacy_chat_turns_from_history_page` covers legacy user records without optional fields, 40 trailing restart markers, cached/cold/uncached paths, older-page continuity and unchanged file bytes.
- Opt-in read-only diagnostic: `TIMEM_DIAGNOSTIC_HISTORY_PATH` with `local_history_page_read_only_diagnostic --ignored --nocapture`; no private fixture is committed.

- Consecutive structured restart markers coalesce to the last record in each history page; intervening chat/events split runs. Original log bytes and record-offset cursors are preserved. `restart_notice_coalescing_preserves_separate_runs_and_old_cursors` covers separate runs and legacy cursors.

- Host projection guard: `restart_heavy_history_restores_chat_and_only_latest_consecutive_notice` verifies user text, final answer and latest restart survive the actual restore projections. `action_events_split_consecutive_restart_notice_runs` ensures tool evidence prevents merging distinct restart runs.


## Prompt-context restart handoff regression

`applications/timem/tests/unit/web_host_tests.rs` covers single-use restoration
(`context_handoff_is_consumed_before_import_and_cannot_replay_after_crash`),
authoritative primary selection (`context_handoff_shutdown_uses_registered_primary_not_first_worker`),
failed export invalidation (`context_handoff_failed_export_invalidates_previous_snapshot`),
and real Core compaction followed by save/restore and next-prompt inspection
(`context_handoff_real_compaction_survives_graceful_restart`). The first three fail
against the previous implementation; the normal compaction path remains compatible.
Snapshots are graceful-shutdown handoffs: after an ungraceful exit dynamic context
may be absent, but a consumed pre-compaction generation cannot reappear. Raw chat
history remains available independently.

## Tool glyph and lighter icon strokes

Bash tool rows (live and archived) render Lucide SquareTerminal at 14px with a retained tooltip and screen-reader name. Lucide default strokes are 1.5 instead of 2; explicit weights and CSS overrides scale by 0.75, including memo/context-compaction icons. Tool layout/unit tests and Chrome stream acceptance guard the terminal glyph and computed stroke. Icon sizes, click targets and counter typography are unchanged.

Reasoning notices use a 13px Lucide Infinity glyph with the same 1.5 stroke, driven exclusively by model-request `reasoning_enabled=true`. Chrome regression covers enabled and disabled cases.

Prompt timestamps: semantic non-empty `user_question` and `user_supplement` slices render their original input timestamp as readable UTC seconds. Delta separators, tool results, runtime notes, supporting context and empty direct resumes have no input timestamp. Internal millisecond timestamps remain unchanged; old delta headers remain parseable. Covered by `input_time_is_semantic_not_a_transport_role_and_uses_slice_time`.

Reasoning scheduling counts actual model-request preparation, not prompt rendering. `reasoning_counts_actual_request_preparation_not_prompt_rebuilds` covers repeated rebuilds, direct-render tool continuations, requests 35/70, and manual compaction. The existing >30 context-message guard remains; compact requests bypass that guard.

### OpenAI Chat Completions 推理级别映射

配置了 reasoning effort 时，普通请求发送 `reasoning_effort=none`；需要推理的请求发送用户选择的级别，用户选择 `disabled` 则始终发送 `none`。不再使用 Claude 风格的 `thinking.type=disabled` 代替该字段。未配置 effort 时保持不发送的兼容行为；其他协议及周期计数逻辑不变。

回归覆盖：`chat_reasoning_wire_effort_respects_user_selection_and_disable` 验证各级别、普通/推理请求、禁用优先及未配置情况；`periodic_and_compact_reasoning_reach_http_payload` 验证周期与压缩触发到 HTTP 请求体的映射。

### 统一推理语义与协议适配

`ReasoningPreference` + `ReasoningDemand` → `EffectiveReasoning`，仅此处决策启停。
显式 `enable_thinking=false`、effort `disabled` 或 `none` 优先关闭；未配置保持不指定；普通轮次关闭，推理轮次保留强度。强度字符串保留供应商扩展值，不自动降档。

- Chat Completions：`reasoning_effort`；兼容已配置的 `enable_thinking` 扩展。
- Responses：`reasoning.effort`；只开启未指定强度时使用空 reasoning 对象，由服务端选择默认值。
- Anthropic：`thinking.type=adaptive/disabled`，开启且有强度时写 `output_config.effort`。只允许 low/medium/high/max；其他值在发送前明确拒绝。不猜预算、不自动退回旧模型模式；模型是否支持 adaptive 由上游明确校验。
- reasoning/thinking/output_config 为保留字段，不允许自定义请求字段绕过统一策略。已有这类自定义字段配置需要迁移到统一推理设置。
- 仅当本次解析出的有效档位确实高于正常 H0 基线时，最终非缓存 prompt trailer 才声明本次使用更强推理，并要求模型增加工作方向、方法和纠偏反思；仅有关键调度标记、thinking 已启用或 H0→H0 均不注入。该提示与 inline/native/上下文压缩各自的原协议尾指令合并，不覆盖 `task_finished` 或 `context_compact` 约束。

测试：`policy_is_protocol_independent_and_disable_wins`、`semantic_reasoning_policy_maps_to_every_protocol`、`legacy_preferences_resolve_before_all_protocol_adapters`、`custom_fields_cannot_override_reasoning_policy`、`anthropic_unsupported_intensity_is_rejected_without_downgrade`、`higher_than_h0_trailer_is_injected_only_for_a_real_reasoning_upgrade`、`higher_than_h0_trailer_preserves_context_compaction_protocol`、`higher_reasoning_intensity_trailer_is_an_uncached_final_block`；保留周期/压缩到请求体的原有回归测试。

### 动态上下文统计包含纯工具 delta

Native 模式下，动态上下文估算按保留的 delta 边界统计所属工具历史，不以是否有可见文字切片作为入选条件；无所属存活 delta 的工具记录不计入。文字/工具 token 仍为本地估算，不等同于 API 完整输入用量。
回归：`dynamic_context_estimate_counts_tool_only_deltas_and_excludes_orphans` 覆盖纯工具 delta、孤立工具记录排除、删除后剩余估算及 shrunk_tokens；原有文字与工具混合统计测试保留。

### 接入点白名单表单
- Core：`cargo test -p agent_core --test model_catalog_tests --locked`，覆盖默认值、固定/禁用协议组合、预算上下界与不支持的字段绑定。
- Host：`cargo test -p timem catalog_endpoint_admission --locked`，覆盖白名单准入、代理地址/预算覆盖和自定义模式。
- Web：`pnpm --dir interfaces/web test`；`TIMEM_CATALOG_TEST_URL=http://127.0.0.1:23451/ node interfaces/web/tests/browser/catalog-endpoint-e2e.mjs`。浏览器用例必须指向独立测试数据目录的宿主，会创建测试接入点；覆盖协议联动、预算拦截、无 Key 保存、编辑回填、自定义切换与窄屏横向溢出检查。不得指向当前开发实例。

### 白名单绑定的向后兼容回归
`cargo test -p timem catalog_legacy_session --locked` 覆盖旧会话缺失字段、空值/空白清除绑定、缓存再次恢复、显式绑定保留、未知绑定按自定义恢复；旧接入点加载用例断言缺失 catalog_id 为 None。恢复旧会话不得从宿主模板继承白名单绑定。`session_runtime_env_rejects` 与 `restored_session` 用例验证原有环境校验和会话恢复不退化。

### 既有持久化数据兼容验证（预置配置接入）
- `catalog_legacy_endpoint_file_round_trip_preserves_configuration_and_log_bytes`：无 catalog_id 的旧 JSON 文件即使模型名称命中白名单仍不绑定；读取不改写源文件，修改名称后保存再读取保留凭据、地址、预算、推理、Headers、扩展字段；旁路日志字节不变（不替代日志解析测试）。
- `catalog_persisted_sessions_restore_missing_and_empty_bindings_with_history`：缺失/空绑定的两个旧会话从真实临时 SessionStore 恢复，历史消息不丢，不继承宿主绑定；再次恢复仍成立。
- `cargo test -p agent_core --lib legacy --locked` 和 `cargo test -p agent_core --lib audit_upgrade --locked`：既有日志读取、升级/合并、幂等迁移回归。
- `cargo test -p timem_session --lib --locked`：Session 模块回归。
- 当前已知未解决：宿主全量中的 `model_endpoint_import_maps_reasoning_and_vendor_request_fields` 因 Claude thinking 字段与请求字段保护规则冲突失败；不得将本专项通过标记为全量兼容性通过。

预置绑定兼容规则更新：未知 ID 在导入、持久化加载和环境恢复入口规范化为自定义，读取不改写旧文件。`unknown_catalog_import_and_persisted_endpoint_become_custom_without_parameter_loss` 覆盖参数保留与保存再加载。请求包测试 `catalog_request_*`、`catalog_final_wire_*` 覆盖全部允许的协议/推理组合、输出预算、扩展字段、禁止组合，以及最终 model/输出字段篡改拦截。输入预算属于本地上下文管理，不虚构为 API 字段。

### Responses 流式
`cargo test -p agent_core --lib responses_ --locked` 覆盖 stream 请求字段、完整结束响应的文本/工具/用量解析、输出截断、畸形 JSON、失败与缺失结束；模拟 HTTP 用例 responses_provisional_content_arrives_before_terminal 使用握手验证增量在结束前到达。`model_transport` 回归覆盖共用取消/超时路径。宿主 model_endpoint_stream_accepts_responses_but_not_anthropic 验证保存开关。当前以结束事件的完整 response 为权威，不执行部分工具参数，不支持省略完整结束响应的非标准网关。

### Responses 真宿主端到端验收
`STREAM_API_PROTOCOL=openai-responses STREAM_PREVIEW_PROTOCOL=native node interfaces/web/tests/browser/stream-preview-product.mjs` 启动独立临时 MEM、真实 debug Timem（18987）、真实 Chrome 和本地模拟模型 HTTP 服务。接入点经宿主命令创建/应用，初始消息从浏览器输入框 Enter 提交，禁止直接提交 turn 绕过 UI。握手控制结束事件，断言结束前出现预览；原生 readfile 真实执行且 function_call_output 回到下一请求，task_finished 完成后检查磁盘 JSON/JSONL 存在会话最终内容并刷新验证。模型服务是模拟的，不代表官方远端验收。
同脚本 `STREAM_PREVIEW_PROTOCOL=xml|json` 验证内联模式；XML 可用 `STREAM_PREVIEW_SCENARIO=stop|network|tools` 验证停止后再发送、断网保留部分输出、工具执行。去掉 STREAM_API_PROTOCOL 做 Chat 回归。测试 finally 清理浏览器、测试宿主与临时目录；不触碰用户开发实例。

## Zhipu templates and directly editable token budgets

- Contract/evidence: `docs/zhipu-model-catalog.md`, `scripts/validate_zhipu_model_catalog.py`.
- Core: `model_api::tests::zhipu_*`, `model_transport::tests::zhipu_native_stream_tool_roundtrip_over_real_http`.
- Host: `zhipu_catalog_endpoints_roundtrip_and_reject_forbidden_efforts`.
- Web: `endpoint_header_button.test.ts` and isolated-host `catalog-endpoint-e2e.mjs` verify no budget steppers, direct entry, template listing and persisted editing.
- Coverage boundary: local HTTP fixtures, not paid remote provider calls; no full million-token or Coding Plan qualification claim.

## 接入点需求与 payload 分层（v1）

- `demand_v1_*`：日常/关键调用、H1 不降级、允许子集、模板与身份独立、协议拒绝、最终 wire 篡改拦截。
- `external_catalog_json_only_extension_reaches_payload_in_fresh_process`：OpenAI/智谱虚构未来模型只加 JSON，在独立进程加载并生成请求；不修改代码/重新编译。负例覆盖未知字段/handler、重复身份。
- `endpoint_requirements_and_cleared_preferences_survive_session_cache_roundtrip`：来源、允许集合、自适应设置的接入点磁盘与 Session 缓存往返；空值清除旧设置。
- `model_endpoints.test.ts`：用户字段优先、模板移除保留、旧接入点迁移、不从 URL 猜供应商。
- `catalog-endpoint-e2e.mjs`：独立真实 Host + Chrome；冲突不静默改值、无 Key 保存恢复、模板/无同一入口、移动端预算控件。
- `stream-preview-acceptance.mjs`：仅真实 `reasoning_upgrade` 显示加强通知；普通 `model_request` 不显示。测试不需要付费远端 API。
- `readfile` 起始行越界回归：错误包含实际总行数，覆盖空文件、LF/CRLF/CR 及73行样例。

本次验证：Core lib 786通过/2忽略；描述加载5通过；Web 532通过；Host全量346通过/1忽略/1已知失败（上文 Claude thinking 导入）；两套 Chrome 验收通过。全量 Host 不标记为通过。


## 紧凑模板入口与协议级 Base URL

- `protocol_urls_override_connection_defaults_and_are_projected`：协议地址覆盖、模型地址回退、序列化投影及非法 URL 拒绝。
- `model_endpoints.test.ts`：模板地址双向跟随协议；手动值（含同值/空值）不覆盖；恢复当前协议地址后重新联动；用户选定协议优先；未知协议、移除模板、旧配置保守处理。
- `catalog-endpoint-e2e.mjs`：桌面模板区域高度≤80px，下拉框≤36px；390px 窄屏区域≤110px 且不溢出。
- 额外设置 `TIMEM_CATALOG_TEST_PROTOCOL_TEMPLATE=fixture/protocol-urls`，隔离宿主加载外部 JSON 测试模型（Responses 地址 `https://responses.example.test/v1`，Chat 地址 `https://chat.example.test/v2`），验证协议切换、手动覆盖、恢复、保存再编辑、移除模板停止联动。测试域名不发起远端调用。
- 验证结果：Web 538通过；目录配置6通过；Core集成238通过；隔离Chrome验收通过；类型检查、描述文件校验、格式、架构/模块/测试契约及性能守卫通过。已重新生成 dist（入口 JS/CSS 与 index.html），不删除其他并行工作产物、不重启开发实例。


## 默认地址提示与协议选项修正

- 默认地址恢复入口及说明按当前模板/协议的默认值与草稿值比较；默认值时隐藏，即使来源为手动；差异值时显示，恢复后立即隐藏。
- `model_endpoints.test.ts` 覆盖默认值/手动同值/自定义值/未知默认、协议切换，以及模型协议筛选、禁用当前不兼容协议、自定义服务与供应商约束。
- `catalog-endpoint-e2e.mjs` 真浏览器验证：OpenAI 无 Anthropic 可选项；智谱仅 Chat；自定义服务仍可选择 Anthropic；显式不兼容协议保留为禁用项且不可保存；智谱默认 URL 不显示恢复提示，修改/手动填回默认/点击恢复均正确。
- 验证：Web 543通过，类型检查通过，隔离 Host + Chrome（含协议 URL 外部描述夹具）通过；架构/模块/测试契约、5项浏览器性能检查及 diff 检查通过。更新 dist 入口 JS/index.html；开发实例未重启。


## 推理设置视觉分组

- 将日常强度、允许档位与自适应加强收纳到独立区域；档位使用保留原生 checkbox 语义的标签，提供选中态及键盘焦点提示；窄屏单列，适配明暗主题。
- 已知模型的范围恢复入口显示“使用模型能力范围”；通用且未声明允许集合时显示范围未知、不自动加强的说明，不暗示存在已知能力。此次不改变 Core 调度、协议字段映射或厂商适配。
- `endpoint_header_button.test.ts` 覆盖分组、原生复选框、范围状态、焦点与响应式样式，以及已知/未知范围文案分支。
- `catalog-endpoint-e2e.mjs` 在隔离 Host + Chrome 验证真实鼠标点击、键盘空格切换及可见焦点、自适应整行点击、范围重置状态、明暗主题几何布局及390px窄屏不溢出；原有模板、预算、保存恢复验收一并通过。
- 验证结果：类型检查、Web 545项测试、5项性能检查、架构/模块/测试契约守卫、diff检查通过；Vite产物已更新入口JS/CSS及index.html，Host编译及隔离浏览器验收通过。未运行完整Rust工作区测试；未调用远端模型服务，未重启开发实例。


## 接入点编辑操作置顶

- 新建/编辑共用顶部操作区，包含标题、取消、保存接入点；移除底部操作区，滚动时顶部操作区保持可见。保存校验与取消语义不变。
- 按用户要求不增加保存提示，移除编辑简介与模板说明，保留模板标签和下拉选择。
- `endpoint_header_button.test.ts` 验证唯一顶部操作区、保存校验、取消事件、sticky样式及精简文案。
- `catalog-endpoint-e2e.mjs` 在隔离Host与Chrome验证：新建桌面/编辑390px窄屏、明暗主题、滚至表单底部后按钮仍可见且可点击，保存恢复正常，取消后重新打开仍为已保存值。
- 最终版本验证：类型检查、Web 547项测试及隔离浏览器验收通过；5项性能检查、架构/模块/测试契约守卫通过；dist入口JS/CSS/index.html已重新生成，Host编译通过。未运行完整Rust工作区测试，未重启开发实例。


## 三行推理设置与模板厂商前缀

- 推理区域按三个操作行排列：点击配置可用推理档位（原生details，默认收起）、正常推理使用档位下拉框、自适应推理加强复选框。“点击”使用高对比加粗字体，明暗主题分别适配。
- 面向用户的文案：自适应说明改为“需要时使用推理加强”；通用范围恢复操作改为“清除手动选择”，已知模型为“使用模型支持的档位”；不添加额外解释段落。此次不改变Core推理策略或请求字段映射。
- 模板选项显示“厂商: 模型名字”，保留原始模板ID和“无”选项。
- `endpoint_header_button.test.ts` 覆盖三行顺序、可展开入口、加粗标签及厂商前缀；`catalog-endpoint-e2e.mjs` 验证明暗主题、390px窄屏、展开/收起布局、鼠标展开、空格键收起、档位选择与清除、模板显示名称及保存/取消回归。CDP仅注入Enter未触发原生summary切换，改用原生支持的Space键完成键盘验收；不声称验证了Enter。
- 验证结果：类型检查、Web 549项测试、5项性能检查、架构/模块/测试契约守卫、diff检查通过；Vite产物与Host编译更新，隔离Host+Chrome验收通过。未执行完整Rust工作区测试，未调用远端模型服务，未重启开发实例。


## 智谱展示名称简化

- 厂商选择和模板前缀统一使用“智谱”/“Zhipu”，去除括号中的z；模型ID与协议适配不变。同步更新浏览器模板名称断言。
- 类型检查、Web 549项测试和Vite构建通过；本次仅文案变化，未重新运行浏览器验收或完整Rust测试。dist入口JS和index.html已更新，CSS无变化。


## 可用推理档位选中标记

- 已选档位右上角显示小勾号，未选时隐藏；预留文字间距，保持原生checkbox语义、鼠标及键盘切换，不新增说明文案。
- `endpoint_header_button.test.ts`覆盖图标和选中态样式；隔离Chrome验收检查明暗主题、390px窄屏、鼠标/键盘切换与清除后的勾号显隐、右上角位置及无文字重叠。
- 类型检查、Web 550项测试、5项性能检查、架构/模块/测试契约守卫与diff检查通过；Vite和Host编译、隔离浏览器验收通过。dist入口JS/CSS及index.html已更新；未执行完整Rust工作区测试，未重启开发实例。


## 接入点分享与导入

- 接入点列表逐项提供“分享”，小面板按基本配置（URL、协议、模型、预算、推理）、高级配置（自定义请求字段、跨源重定向）、个人配置（API Key、请求头、私有证书）勾选。默认仅基本配置，个人配置旁使用 Lucide `TriangleAlert`，不使用警示 emoji。提供分享和复制；关闭面板清空临时内容。
- 顶部“导入分享”与原目录导入并存。Host 解码并调用既有 Core 配置校验，只新增 ID，重名依次追加数字，不覆盖原接入点。未包含基本配置的分享片段不能新增接入点，界面提示补充基本配置。
- 分享格式为 `timem.endpoint` v1 JSON 的标准 Base64，最多256 KiB，允许外层ASCII空白；拒绝未知结构、版本、无效配置及注入的ID。Base64不是加密，URL和自定义字段仍可能包含敏感信息。
- 导出不读取脱敏投影，不广播、不进入语义重放和命令结果缓存；仅显式勾选个人配置时包含其密钥。导入落盘成功后更新内存与脱敏投影；取消/超时清理相关临时命令引用，关闭面板丢弃迟到结果。
- `applications/timem/tests/unit/web_host_tests.rs`：`endpoint_share_*`三项测试覆盖全部七种非空类别组合、Unicode、默认无密钥、私有直接回复、去重缓存排除、原配置不变、连续重名、持久化、无效Base64/JSON/版本/配置/超长/ID注入、写入失败不修改内存及原文件、8个并发导入唯一名称。
- `interfaces/web/tests/wire_delivery.test.ts`：两种分享结果作为直接事件，不被语义事件游标误过滤。
- `interfaces/web/tests/browser/endpoint-share-e2e.mjs`：只针对隔离Host，以 `TIMEM_SHARE_TEST_URL=http://127.0.0.1:<port> node interfaces/web/tests/browser/endpoint-share-e2e.mjs` 执行。真实鼠标操作验证默认选项、三类内容、切换后清空旧输出、Lucide图标、明暗主题/390px窄屏、导出为只读自动换行代码区且仅提供复制、256 KiB 导入使用无控有界输入并保持响应、剪贴板粘贴、第二连接无秘密广播、关闭重开清空秘密、无选择禁用、错误导入不新增、连续名称后缀、刷新后完整配置再次导出。脚本自行清理Chrome与临时目录；Host由调用者隔离启动并清理。
- 最新分享弹窗优化验证：Rust分享专项3项、Web671项、类型检查、5项性能检查、架构/模块/测试契约守卫及diff检查通过；以临时MEM和独立回环端口运行隔离Host+Chrome完整分享验收，覆盖只读换行导出、复制/剪贴板粘贴、256 KiB无控导入、错误/重名/刷新回环和秘密不广播。实测最大输入不重挂弹窗或面板、无React `value` 属性回写，输入处理约43ms。Vite dist及嵌入Host构建已更新。未运行完整Rust工作区测试，未调用远端模型服务，未重启开发实例。


## 内置工具语义摘要与整行展开交互

- `readfile` 调用在普通工作详情和流式工具行统一显示 Lucide `FileText`、可截断目录、文件名和按真实选择器生成的行/字节/匹配范围；示例显示为 `applications/timem/tests/unit/web_host_tests.rs · 15190–15244 行`。`tail_out` 和编码可作为附加摘要；历史事件中的 `max_bytes` 仅做合法性校验并从主摘要省略，完整脱敏参数仍保留在展开详情中。非法、未知字段或第三方同名工具回退到原脱敏参数，不伪造成功语义。
- `memmgr` 的 raw_chat/scratch search 与 durable/raw_chat SQL 查询显示 `DatabaseSearch`；其他 memory 读写、更新和删除显示 Lucide `Database`。当前依赖版本未导出 `DatabaseSearch`，使用 Lucide 官方SVG节点的本地 `createLucideIcon` 后向移植，并在源码保留ISC许可声明。
- 工具行常态顺序统一为“状态 → 工具图标或名称 → 命令/参数摘要”。有详情时不常驻展开箭头，整行在 hover/focus 下呈现可点击反馈；鼠标、触屏、Enter/Space点击同一行切换展开/收起，文本选择不会误触折叠。无详情行不获得虚假按钮语义。
- 状态槽、工具身份和摘要垂直居中并使用固定行高；执行中圆点切换到完成/失败文字不改变已展开行高度。折叠工具组与实时工具状态槽共用水平起点；390px窄屏允许摘要换行但保持分组对齐。
- `tool_presentation.test.tsx`覆盖参数顺序、行/字节/匹配/单端选择器、tail/encoding、非法范围、未知字段、HTML转义、Windows路径、敏感文本脱敏、第三方回退，以及memory操作图标分类。`tool_activity_layout.test.ts`守卫整行披露、状态/工具/摘要顺序、固定行高和水平起点。
- `tool-presentation-e2e.mjs`在真实Chrome验证普通/流式视图、FileText/DatabaseSearch/Database、无常驻箭头、整行hover指针、鼠标与键盘切换、原始参数详情、状态→工具→摘要顺序、明暗主题和390px布局。既有`stream-preview-acceptance.mjs`完整回归验证状态更新不重挂载、不改变展开高度、文本选择保护、折叠组/实时行对齐及连续流式交互。
- 验证：类型检查、Web 588项、5项性能守卫、工具专项Chrome及完整流式Chrome验收、架构/模块/测试契约与diff检查通过；Host嵌入构建和dist已更新。未重启开发实例。
