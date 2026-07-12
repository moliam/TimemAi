use super::*;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

fn env(items: &[(&str, &str)]) -> HashMap<String, String> {
    items
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn strip_ansi_for_test(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code_ch in chars.by_ref() {
                if code_ch.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

fn visible_width_for_test(text: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi_for_test(text).as_str())
}

#[test]
fn generic_api_key_wins_over_vendor_key() {
    let config = provider_config_from_env(
        &CliOptions {
            provider: Some("aliyun".into()),
            ..CliOptions::default()
        },
        &env(&[
            ("TIMEM_API_KEY", "generic"),
            ("DASHSCOPE_API_KEY", "vendor"),
        ]),
    )
    .unwrap();
    assert_eq!(config.api_key, "generic");
}

#[test]
fn default_gateway_provider_is_aliyun() {
    let config =
        provider_config_from_env(&CliOptions::default(), &env(&[("TIMEM_API_KEY", "k")])).unwrap();
    assert_eq!(config.provider, "aliyun");
    assert_eq!(config.model, "qwen-plus");
    assert_eq!(
        config.base_url,
        "https://dashscope.aliyuncs.com/compatible-mode/v1"
    );
    assert_eq!(config.api_protocol, ApiProtocol::OpenAiCompatible);
    assert_eq!(
        config.response_protocol,
        agent_core::ResponseProtocolKind::Xml
    );
}

#[test]
fn empty_generic_api_key_falls_back_to_vendor_key() {
    let config = provider_config_from_env(
        &CliOptions {
            provider: Some("aliyun".into()),
            ..CliOptions::default()
        },
        &env(&[("TIMEM_API_KEY", ""), ("DASHSCOPE_API_KEY", "vendor")]),
    )
    .unwrap();
    assert_eq!(config.api_key, "vendor");
}

#[test]
fn empty_api_key_reports_missing_key() {
    let err = provider_config_from_env(
        &CliOptions {
            provider: Some("openai".into()),
            ..CliOptions::default()
        },
        &env(&[("TIMEM_API_KEY", ""), ("OPENAI_API_KEY", "")]),
    )
    .unwrap_err();
    assert!(err.contains("missing_api_key"));
}

#[test]
fn non_ascii_api_key_reports_clear_error() {
    let err = provider_config_from_env(
        &CliOptions {
            provider: Some("aliyun".into()),
            ..CliOptions::default()
        },
        &env(&[("TIMEM_API_KEY", "你的token")]),
    )
    .unwrap_err();
    assert!(err.contains("invalid_api_key_non_ascii"));
}

#[test]
fn parse_cli_args_reads_provider_model_and_limits() {
    let args = [
        "--space",
        ".x",
        "--gateway-provider",
        "custom-claude-gateway",
        "--api-protocol",
        "openai-compatible",
        "--response-protocol",
        "xml",
        "--api-key",
        "cli-key",
        "--model",
        "gpt-x",
        "--base-url",
        "http://local/v1",
        "--data-dir",
        "/tmp/timem-data",
        "--timeout",
        "33",
        "--max-llm-output",
        "10K",
        "--max-llm-input",
        "100K",
        "--capabilities-dir",
        "/tmp/timem-capabilities",
        "--once-json",
        "你好",
        "--supporting-context",
        "previous transcript",
        "--bash-approval",
        "approve",
        "--work-instructions",
        "ask",
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>();
    let options = parse_cli_args(&args);
    assert_eq!(options.space.as_deref(), Some(".x"));
    assert_eq!(options.provider.as_deref(), Some("custom-claude-gateway"));
    assert_eq!(options.api_protocol.as_deref(), Some("openai-compatible"));
    assert_eq!(options.response_protocol.as_deref(), Some("xml"));
    assert_eq!(options.api_key.as_deref(), Some("cli-key"));
    assert_eq!(options.model.as_deref(), Some("gpt-x"));
    assert_eq!(options.base_url.as_deref(), Some("http://local/v1"));
    assert_eq!(options.data_dir.as_deref(), Some("/tmp/timem-data"));
    assert_eq!(options.timeout_secs, Some(33));
    assert_eq!(options.max_llm_output_tokens, Some(10_000));
    assert_eq!(options.max_llm_input_tokens, Some(100_000));
    assert_eq!(
        options.capabilities_dir.as_deref(),
        Some("/tmp/timem-capabilities")
    );
    assert_eq!(options.once_json_input.as_deref(), Some("你好"));
    assert_eq!(
        options.supporting_context.as_deref(),
        Some("previous transcript")
    );
    assert_eq!(options.bash_approval.as_deref(), Some("approve"));
    assert_eq!(options.work_instructions.as_deref(), Some("ask"));
}

#[test]
fn cli_api_key_overrides_env_api_key() {
    let config = provider_config_from_env(
        &CliOptions {
            api_key: Some("cli-key".into()),
            ..CliOptions::default()
        },
        &env(&[("TIMEM_API_KEY", "env-key")]),
    )
    .unwrap();
    assert_eq!(config.api_key, "cli-key");
}

#[test]
fn default_token_limits_are_input_100k_and_output_10k() {
    let config =
        provider_config_from_env(&CliOptions::default(), &env(&[("TIMEM_API_KEY", "k")])).unwrap();
    assert_eq!(config.max_llm_input_tokens, 100_000);
    assert_eq!(config.max_llm_output_tokens, 10_000);
}

#[test]
fn cli_options_override_env_config_values() {
    let config = provider_config_from_env(
        &CliOptions {
            provider: Some("custom".into()),
            api_protocol: Some("anthropic".into()),
            model: Some("cli-model".into()),
            base_url: Some("https://cli.example/v1".into()),
            timeout_secs: Some(33),
            max_llm_output_tokens: Some(1234),
            max_llm_input_tokens: Some(64_000),
            api_key: Some("cli-key".into()),
            ..CliOptions::default()
        },
        &env(&[
            ("TIMEM_GATEWAY_PROVIDER", "aliyun"),
            ("TIMEM_API_PROTOCOL", "openai-compatible"),
            ("TIMEM_MODEL", "env-model"),
            ("TIMEM_BASE_URL", "https://env.example/v1"),
            ("TIMEM_TIMEOUT", "99"),
            ("TIMEM_MAX_LLM_OUTPUT", "9999"),
            ("TIMEM_MAX_LLM_INPUT", "128K"),
            ("TIMEM_API_KEY", "env-key"),
        ]),
    )
    .unwrap();

    assert_eq!(config.provider, "custom");
    assert_eq!(config.api_protocol, ApiProtocol::Anthropic);
    assert_eq!(config.model, "cli-model");
    assert_eq!(config.base_url, "https://cli.example/v1");
    assert_eq!(config.timeout_secs, 33);
    assert_eq!(config.max_llm_output_tokens, 1234);
    assert_eq!(config.max_llm_input_tokens, 64_000);
    assert_eq!(config.api_key, "cli-key");
}

#[test]
fn gateway_provider_env_selects_gateway_and_context_window() {
    let config = provider_config_from_env(
        &CliOptions::default(),
        &env(&[
            ("TIMEM_API_KEY", "k"),
            ("TIMEM_GATEWAY_PROVIDER", "custom"),
            ("TIMEM_MAX_LLM_INPUT", "128K"),
        ]),
    )
    .unwrap();
    assert_eq!(config.provider, "custom");
    assert_eq!(config.max_llm_input_tokens, 128_000);
}

#[test]
fn chinese_backspace_removes_one_character() {
    let mut line = LineBuffer::default();
    line.push_str("中文测试");
    assert!(line.backspace());
    assert_eq!(line.as_string(), "中文测");
}

#[test]
fn compact_count_formats_token_numbers() {
    assert_eq!(compact_count(100), "100");
    assert_eq!(compact_count(1_220), "1.2K");
    assert_eq!(compact_count(1_000), "1K");
    assert_eq!(compact_count(1_210_000), "1.21M");
    assert_eq!(compact_count(1_200_000), "1.2M");
}

#[test]
fn token_status_uses_compact_numbers() {
    let rendered = render_final_response_at(
        "ok",
        &UsageStats {
            llm_calls: 3,
            prompt_tokens: 1_220,
            completion_tokens: 88,
            cached_tokens: 1_210_000,
            ..UsageStats::zero()
        },
        None,
        "aliyun",
        "qwen-plus",
        1,
        100_000,
        "10:52:57",
    );
    assert!(rendered.contains("aliyun:qwen-plus ⇌3 ║ ▲1.2K  ▼88  KVC(⌁1.21M)"));
}

#[test]
fn shell_renders_stopped_turn_text_from_core_summary() {
    let stopped = TurnStopSummary::model_error("provider_http_400").into_stopped_turn();
    let outcome = TurnOutcome::stopped("", stopped, Duration::from_secs(1));

    assert!(outcome.text.is_empty());
    assert_eq!(
        render_turn_outcome_text(&outcome),
        "模型调用失败：provider_http_400"
    );
}

#[test]
fn shell_appends_running_job_list_after_final_answer() {
    let outcome = TurnOutcome::final_response(
        "任务完成。",
        UsageStats::zero(),
        None,
        None,
        Duration::from_secs(1),
    )
    .with_running_jobs(vec![RunningShellJob {
        pid: 12345,
        kind: "timeout".to_string(),
        command: "sleep 30".to_string(),
        cwd: "/tmp".to_string(),
        session_id: "session_a".to_string(),
        turn_id: "turn_a".to_string(),
        created_at_ms: 1000,
    }]);

    let rendered = render_turn_outcome_text(&outcome);
    assert!(rendered.starts_with("任务完成。"));
    assert!(rendered.contains("RUNNING JOB LIST:"));
    assert!(rendered.contains("pid=12345, old job timeout, cmd=sleep 30, still running"));
}

#[test]
fn final_status_shows_repair_call_count_when_present() {
    let rendered = render_final_response_at(
        "ok",
        &UsageStats {
            llm_calls: 13,
            repair_calls: 3,
            prompt_tokens: 85_000,
            completion_tokens: 3_500,
            cached_tokens: 53_900,
            ..UsageStats::zero()
        },
        Some(&UsageStats {
            prompt_tokens: 80_000,
            completion_tokens: 321,
            ..UsageStats::zero()
        }),
        "custom",
        "aws-claude-opus-4-7",
        6,
        100_000,
        "22:29:07",
    );
    assert!(rendered
        .contains("custom:aws-claude-opus-4-7 ⇌13 (⚠3) ║ ctx[80%]  ▲85K  ▼3.5K  KVC(⌁53.9K)"));
}

#[test]
fn token_status_omits_zero_cache_and_shrink_annotations() {
    assert_eq!(
        token_status(&UsageStats {
            prompt_tokens: 22_200,
            completion_tokens: 1_400,
            ..UsageStats::zero()
        }),
        "Token: ▲22.2K ▼1.4K"
    );
}

#[test]
fn token_status_shows_cache_and_shrink_only_when_present() {
    assert_eq!(
        token_status(&UsageStats {
            prompt_tokens: 22_200,
            completion_tokens: 1_400,
            cached_tokens: 1_200,
            shrunk_tokens: 200,
            ..UsageStats::zero()
        }),
        "Token: ▲22.2K(KVC:⌁1.2K , ⇃200) ▼1.4K"
    );
}

#[test]
fn token_status_shows_latest_delta_and_context_window() {
    let total = UsageStats {
        prompt_tokens: 4_400,
        completion_tokens: 56,
        ..UsageStats::zero()
    };
    let latest = UsageStats {
        prompt_tokens: 2_000,
        completion_tokens: 32,
        ..UsageStats::zero()
    };
    assert_eq!(
        token_status_with_latest(&total, Some(&latest), TokenStatusMode::Thinking),
        "Token: ▲4.4K(+2K) ▼56(+32)"
    );
    assert_eq!(
        token_status_with_latest(&total, Some(&latest), TokenStatusMode::Final),
        "Token [ctx 2K] ▲4.4K ▼56"
    );
}

#[test]
fn token_status_groups_cache_creation_as_kvc() {
    let total = UsageStats {
        prompt_tokens: 4_900,
        completion_tokens: 39,
        cache_created_tokens: 4_900,
        ..UsageStats::zero()
    };
    let latest = UsageStats {
        prompt_tokens: 4_900,
        completion_tokens: 39,
        cache_created_tokens: 4_900,
        ..UsageStats::zero()
    };
    let view = runtime_token_status_view(&total, Some(&latest), 100_000, 0);
    assert_eq!(
        compact_token_totals(&view.total),
        "▲4.9K | ▼39 | KVC(✚4.9K)"
    );
    assert_eq!(
        compact_token_latest(view.latest.as_ref().unwrap()),
        "△4.9K  ▽39  KVC(✚4.9K)"
    );
    assert_eq!(
        final_status_line(&total, Some(&latest), "aliyun", "qwen-plus", 1, 100_000),
        " ↳  1s    aliyun:qwen-plus ⇌0 ║ ctx[5%]  ▲4.9K  ▼39  KVC(✚4.9K)"
    );
}

#[test]
fn token_status_uses_pending_request_as_current_when_total_is_zero() {
    let pending = UsageStats {
        prompt_tokens: 5_000,
        ..UsageStats::zero()
    };
    assert_eq!(
        token_status_with_latest(
            &UsageStats::zero(),
            Some(&pending),
            TokenStatusMode::Thinking
        ),
        "Token: ▲5K ▼0"
    );
    assert!(!token_status_with_latest(
        &UsageStats::zero(),
        Some(&pending),
        TokenStatusMode::Thinking
    )
    .contains("▲0(+5K)"));
}

#[test]
fn final_token_status_does_not_show_latest_output_delta() {
    let rendered = render_final_response_at(
        "Hi!",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 5_100,
            completion_tokens: 45,
            ..UsageStats::zero()
        },
        Some(&UsageStats {
            prompt_tokens: 5_100,
            completion_tokens: 45,
            ..UsageStats::zero()
        }),
        "custom",
        "aws-claude-sonnet-4-6",
        2,
        100_000,
        "09:24:00",
    );
    assert!(rendered.contains("custom:aws-claude-sonnet-4-6 ⇌1 ║ ctx[6%]  ▲5.1K  ▼45"));
    assert!(!rendered.contains("▼45(+45)"));
}

#[test]
fn thinking_block_visual_contract() {
    let block = render_thinking_block_at(
        &ShellStatusSnapshot {
            provider: "aliyun".into(),
            model: "qwen-plus".into(),
            intent: "查询记忆".into(),
            memory_activity: CoreMemoryActivity::Read,
            model_round: 2,
            direction: ModelDirection::Downstream,
            usage: UsageStats {
                prompt_tokens: 210,
                completion_tokens: 21,
                cached_tokens: 0,
                ..UsageStats::zero()
            },
            latest_usage: Some(UsageStats {
                prompt_tokens: 110,
                completion_tokens: 9,
                ..UsageStats::zero()
            }),
            tick: 0,
            elapsed_secs: 7,
            max_llm_input_tokens: 100_000,
            retry: None,
        },
        "08:56:33",
    );
    assert!(block.contains("[08:56:33] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
    assert!(block.contains("🦩 ◂⛃ 查询记忆..."));
    assert!(block.contains("aliyun:qwen-plus ⇌2 ║ ▲210 | ▼21"));
    assert!(block.contains("├─ context : ▰▱▱▱▱▱▱▱▱▱"));
    assert!(block.contains("└─ △110  ▽9"));
    assert!(!block.contains("已用 7s"));
    assert!(!block.contains("⚡cache"));
    assert_eq!(block.lines().count(), 5);
    assert!(!block.contains("thinking..."));
}

#[test]
fn thinking_block_compacts_long_model_intent_to_two_lines() {
    let block = render_thinking_block_at(
            &ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "Check local system date and calendar to verify current date context and compute June 12 significance (e.g., holiday, observance, personal memory).".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                latest_usage: Some(UsageStats {
                    prompt_tokens: 812,
                    ..UsageStats::zero()
                }),
                tick: 8,
                elapsed_secs: 65,
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            "23:33:05",
        );

    assert_eq!(block.lines().count(), 5);
    assert!(block.contains("Check local system"));
    assert!(block.contains("├─ context : ▰▱▱▱▱▱▱▱▱▱"));
    assert!(block.contains('…'));
    assert!(!block.contains("observance"));
}

#[test]
fn thinking_view_renders_observation_panel_and_status_line() {
    let mut observations = ObservationPanel::new(8, 60);
    observations.apply(ObservationEvent::Persistent("正在分析用户请求".into()));
    observations.apply(ObservationEvent::Active("rg --files | wc -l".into()));
    let view = render_thinking_view_at(
        &ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "ignored in panel mode".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 2,
                direction: ModelDirection::Downstream,
                usage: UsageStats {
                    prompt_tokens: 1200,
                    completion_tokens: 20,
                    cached_tokens: 300,
                    ..UsageStats::zero()
                },
                latest_usage: Some(UsageStats {
                    prompt_tokens: 800,
                    completion_tokens: 12,
                    ..UsageStats::zero()
                }),
                tick: 0,
                elapsed_secs: 12,
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            observations,
        },
        "12:00:00",
    );

    assert!(view.contains("[12:00:00] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
    assert!(view.contains("Thought / Action"));
    assert!(view.contains("Thought / Action  ⏳ 00:12"));
    assert!(view.contains("· 正在分析用户请求"));
    assert!(view.contains("\x1b[38;5;245m· rg --files | wc -l"));
    assert!(view.contains("aliyun:qwen-plus ⇌2 ║ ▲1.2K | ▼20 | KVC(⌁300)"));
    assert!(view.contains("├─ context : ▰▱▱▱▱▱▱▱▱▱"));
    assert!(view.contains("└─ △800  ▽12"));
    assert!(!view.contains("已用 12s"));
    assert!(!view.contains("ignored in panel mode"));
}

#[test]
fn multi_worker_thinking_view_keeps_identity_and_bounded_layout() {
    fn worker_snapshot(
        model_round: u32,
        repair_calls: u32,
        tick: usize,
        progress: &str,
        command: &str,
    ) -> ThinkingViewSnapshot {
        let mut observations = ObservationPanel::new(20, 84);
        observations.apply(ObservationEvent::Persistent(format!("⚙️ {progress}")));
        observations.apply(ObservationEvent::Persistent(
            "整理任务现场：保留用户目标、当前进度、下一步，不展示模型私有 thought。".into(),
        ));
        observations.apply(ObservationEvent::ActiveChild {
            text: format!("{command}"),
            is_last: true,
        });
        observations.apply(ObservationEvent::Transient("思考中...".into()));
        ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "private model thought should not render".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round,
                direction: ModelDirection::Upstream,
                usage: UsageStats {
                    repair_calls,
                    prompt_tokens: 85_000,
                    completion_tokens: 3_500,
                    cached_tokens: 53_900,
                    cache_created_tokens: 4_900,
                    ..UsageStats::zero()
                },
                latest_usage: Some(UsageStats {
                    prompt_tokens: 5_800,
                    completion_tokens: 123,
                    cached_tokens: 3_900,
                    ..UsageStats::zero()
                }),
                tick,
                elapsed_secs: 80 + u64::from(model_round),
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            observations,
        }
    }

    let ai1 = worker_snapshot(
            12,
            3,
            0,
            "正在做 5 worker / 30 turn 的压力回放，并检查 UI 是否在长进度下保持稳定。",
            "cargo test -p agent_core session_workers_stress_ui_threads_supplements_and_renames -- --nocapture",
        );
    let ai2 = worker_snapshot(
            22,
            0,
            1,
            "正在处理超长 action：命令会被折行展示，但不能撑破窗口或重复刷屏。",
            "printf '%s' 'very-long-command-with-cjk-参数-参数-参数-参数-参数-参数-参数-参数' && wc -c target/output.log",
        );
    let ai3 = worker_snapshot(
        7,
        1,
        2,
        "正在等待补充输入合入当前 turn，worker 身份必须一直可见，避免多 session 串台。",
        "rg -n 'user_supplement|CoreTopicEvent|TopicReply' agent_core timem_shell",
    );

    let rendered = render_worker_thinking_views_at(
        &[("ID0", &ai1), ("ID1", &ai2), ("Review", &ai3)],
        "09:30:00",
    );

    assert!(rendered.contains("[09:30:00] 𝓣𝓲𝓶𝓮𝓶 ID0  ⬇"));
    assert!(rendered.contains("[09:30:00] 𝓣𝓲𝓶𝓮𝓶 ID1  ⬇"));
    assert!(rendered.contains("[09:30:00] 𝓣𝓲𝓶𝓮𝓶 Review  ⬇"));
    assert_eq!(rendered.matches("Thought / Action  ⏳").count(), 3);
    assert_eq!(rendered.matches("思考中...").count(), 3);
    assert!(rendered.contains("aliyun:qwen-plus ⇌12 (⚠3)"));
    assert!(rendered.contains("KVC(⌁53.9K ✚4.9K)"));
    assert!(rendered.contains("└─ cargo test -p agent_core"));
    assert!(rendered.contains("└─ printf"));
    assert!(rendered.contains("└─ rg -n"));
    assert!(!rendered.contains("private model thought"));
    assert!(!rendered.contains("run_bash"));

    for line in rendered.lines() {
        assert!(
            visible_width_for_test(line) <= 110,
            "line too wide ({}): {line}\n{rendered}",
            visible_width_for_test(line)
        );
    }
}

#[test]
fn thinking_status_line_shows_retry_from_structured_fields() {
    let view = render_thinking_view_at(
        &ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "ignored in panel mode".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                latest_usage: None,
                tick: 0,
                elapsed_secs: 3,
                max_llm_input_tokens: 100_000,
                retry: Some(RuntimeRetryStatus {
                    until_epoch_ms: Some(current_epoch_ms() + 10_000),
                    error: None,
                    attempt: Some(1),
                    max_attempts: Some(5),
                }),
            },
            observations: ObservationPanel::default(),
        },
        "12:00:00",
    );

    assert!(view.contains("├─ context : ▱▱▱▱▱▱▱▱▱▱"));
    assert!(view.contains("├─ △0  ▽0"));
    assert!(view.contains("└─ 网络错误，10s 后重试（第1/5次）"));
}

#[test]
fn thinking_status_line_compacts_long_retry_detail_to_one_line() {
    let long_error = "provider_network_error: curl: (16) Error in the HTTP2 framing layer while reading response headers from upstream gateway after a long timeout";
    let view = render_thinking_view_at(
        &ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: String::new(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 1,
                direction: ModelDirection::Upstream,
                usage: UsageStats::zero(),
                latest_usage: None,
                tick: 0,
                elapsed_secs: 3,
                max_llm_input_tokens: 100_000,
                retry: Some(RuntimeRetryStatus {
                    until_epoch_ms: Some(current_epoch_ms() + 10_000),
                    error: Some(long_error.into()),
                    attempt: Some(1),
                    max_attempts: Some(5),
                }),
            },
            observations: ObservationPanel::default(),
        },
        "12:00:00",
    );

    let retry_lines: Vec<_> = view
        .lines()
        .filter(|line| line.contains("详情：provider_network_error"))
        .collect();
    assert_eq!(retry_lines.len(), 1, "{view}");
    assert!(retry_lines[0].contains('…'), "{view}");
    assert!(retry_lines[0].chars().count() < 120, "{view}");
    assert!(!view.contains("reading response headers from upstream gateway"));
}

#[test]
fn thinking_status_line_shows_network_retry_countdown_and_detail_line() {
    let view = render_thinking_view_at(
            &ThinkingViewSnapshot {
                status: ShellStatusSnapshot {
                    provider: "aliyun".into(),
                    model: "qwen-plus".into(),
                    intent: String::new(),
                    memory_activity: CoreMemoryActivity::None,
                    model_round: 1,
                    direction: ModelDirection::Upstream,
                    usage: UsageStats::zero(),
                    latest_usage: None,
                    tick: 0,
                    elapsed_secs: 3,
                    max_llm_input_tokens: 100_000,
                    retry: Some(RuntimeRetryStatus {
                        until_epoch_ms: Some(current_epoch_ms() + 10_000),
                        error: Some(
                            "provider_network_error: curl: (16) Error in the HTTP2 framing layer while reading response headers from upstream gateway"
                                .into(),
                        ),
                        attempt: Some(1),
                        max_attempts: Some(5),
                    }),
                },
                observations: ObservationPanel::default(),
            },
            "12:00:00",
        );

    assert!(
        view.contains("├─ 网络错误，10s 后重试（第1/5次）"),
        "{view}"
    );
    assert!(view.contains("└─ 详情：provider_network_error"), "{view}");
    assert!(!view.contains("网络错误，10s 后重试（第1次）"), "{view}");
    assert!(!view.contains("reading response headers from upstream gateway"));
}

#[test]
fn retry_status_renderer_consumes_core_retry_view() {
    let lines = retry_status_lines_from_view(&RuntimeRetryStatusView {
        remaining_secs: 7,
        attempt: 2,
        max_attempts: 5,
        error: Some("provider_timeout: upstream gateway timed out".to_string()),
    });

    assert_eq!(lines[0], "  ├─ 网络错误，7s 后重试（第2/5次）");
    assert_eq!(
        lines[1],
        "  └─ 详情：provider_timeout: upstream gateway timed out"
    );
}

#[test]
fn thinking_status_line_shows_repair_call_count_when_present() {
    let view = render_thinking_view_at(
        &ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: "custom".into(),
                model: "aws-claude-sonnet-4-6".into(),
                intent: "ignored in panel mode".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 13,
                direction: ModelDirection::Upstream,
                usage: UsageStats {
                    repair_calls: 3,
                    prompt_tokens: 85_000,
                    completion_tokens: 3_500,
                    cached_tokens: 53_900,
                    ..UsageStats::zero()
                },
                latest_usage: Some(UsageStats {
                    prompt_tokens: 5_800,
                    ..UsageStats::zero()
                }),
                tick: 0,
                elapsed_secs: 80,
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            observations: ObservationPanel::default(),
        },
        "12:00:00",
    );

    assert!(view.contains("custom:aws-claude-sonnet-4-6 ⇌13 (⚠3) ║ ▲85K | ▼3.5K | KVC(⌁53.9K)"));
}

#[test]
fn thinking_view_renders_protocol_repair_warning_in_observation_panel() {
    let mut observations = ObservationPanel::new(20, 84);
    observations.apply(ObservationEvent::Persistent(
        "⚠️ 模型回复偏离协议，重试 (2/5)...".into(),
    ));
    observations.apply(ObservationEvent::EnsureTransient("思考中...".into()));

    let view = render_thinking_view_at(
        &ThinkingViewSnapshot {
            status: ShellStatusSnapshot {
                provider: "aliyun".into(),
                model: "qwen-plus".into(),
                intent: "ignored in panel mode".into(),
                memory_activity: CoreMemoryActivity::None,
                model_round: 4,
                direction: ModelDirection::Upstream,
                usage: UsageStats {
                    repair_calls: 2,
                    prompt_tokens: 12_000,
                    completion_tokens: 300,
                    ..UsageStats::zero()
                },
                latest_usage: Some(UsageStats {
                    prompt_tokens: 4_000,
                    completion_tokens: 100,
                    ..UsageStats::zero()
                }),
                tick: 0,
                elapsed_secs: 15,
                max_llm_input_tokens: 100_000,
                retry: None,
            },
            observations,
        },
        "12:00:00",
    );

    assert!(view.contains("Thought / Action  ⏳ 00:15"), "{view}");
    assert!(
        view.contains("⚠️ 模型回复偏离协议，重试 (2/5)..."),
        "{view}"
    );
    assert!(view.contains("思考中..."), "{view}");
    assert!(view.contains("aliyun:qwen-plus ⇌4 (⚠2)"), "{view}");
}

#[test]
fn final_response_visual_contract() {
    let rendered = render_final_response_at(
        "测试代号是 ALPHA-42。",
        &UsageStats {
            llm_calls: 2,
            mem_reads: 1,
            mem_writes: 1,
            prompt_tokens: 812,
            completion_tokens: 52,
            cached_tokens: 384,
            ..UsageStats::zero()
        },
        Some(&UsageStats {
            prompt_tokens: 410,
            completion_tokens: 31,
            ..UsageStats::zero()
        }),
        "aliyun",
        "qwen-plus",
        2,
        100_000,
        "08:56:46",
    );
    assert!(rendered.contains("[08:56:46] 𝓣𝓲𝓶𝓮𝓶  ⬇"));
    assert!(rendered.contains("\x1b[92;1m"));
    assert!(rendered.contains("𝓣𝓲𝓶𝓮𝓶  ⬇"));
    assert!(rendered
        .lines()
        .nth(1)
        .is_some_and(|line| line == "测试代号是 ALPHA-42。"));
    assert!(rendered.contains("测试代号是 ALPHA-42。"));
    assert!(rendered.contains("aliyun:qwen-plus ⇌2 ║ ctx[1%]  ▲812  ▼52  KVC(⌁384)"));
    assert!(!rendered.contains("▼52(+31)"));
    assert!(!rendered.contains("你 >"));
    assert!(!rendered.contains("thinking..."));
}

#[test]
fn final_response_renders_simple_markdown_bold() {
    let rendered = render_final_response_at(
        "- **系统**：macOS",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 10,
            completion_tokens: 2,
            ..UsageStats::zero()
        },
        None,
        "custom",
        "aws-claude-sonnet-4-6",
        1,
        100_000,
        "17:20:00",
    );
    assert!(rendered.contains(&format!("{ANSI_BOLD}系统{ANSI_RESET}：macOS")));
    assert!(!rendered.contains("**系统**"));
}

#[test]
fn final_response_renders_common_markdown_shapes() {
    let rendered = render_final_response_at(
        "# 结论\n> 关键观察\n\n运行 `cargo test`：\n```text\nok 12 passed\n```",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 10,
            completion_tokens: 2,
            ..UsageStats::zero()
        },
        None,
        "custom",
        "qwen-plus",
        1,
        100_000,
        "17:20:00",
    );

    assert!(rendered.contains("结论"));
    assert!(rendered.contains("关键观察"));
    assert!(rendered.contains("cargo test"));
    assert!(rendered.contains("ok 12 passed"));
    assert!(!rendered.contains("# 结论"));
    assert!(!rendered.contains("```text"));
    assert!(!rendered.contains("`cargo test`"));
}

#[test]
fn final_response_markdown_renderer_resets_unclosed_inline_styles() {
    let rendered = render_terminal_markdown("先 `code\n再 **bold");
    assert!(rendered.contains("code"));
    assert!(rendered.contains("bold"));
    assert!(!rendered.contains("**bold"));
}

#[test]
fn final_status_line_is_always_dim_wrapped() {
    let rendered = render_final_response_at(
        "ok",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 10,
            completion_tokens: 2,
            ..UsageStats::zero()
        },
        None,
        "aliyun",
        "qwen-plus",
        1,
        100_000,
        "10:00:00",
    );
    let status_line = rendered.lines().nth(2).unwrap();
    assert!(status_line.starts_with(&format!("{ANSI_RESET}{ANSI_DIM}")));
    assert!(status_line.ends_with(ANSI_RESET));
    assert!(status_line.contains("↳  1s"));
}

#[test]
fn shell_status_bar_is_dim_wrapped_and_extensible() {
    let rendered = render_shell_status_bar(&HostStatusMessage {
        level: HostStatusLevel::Info,
        text: "已取消当前输入。Ctrl+D 退出。".to_string(),
    });
    assert!(rendered.starts_with(&format!("{ANSI_RESET}{ANSI_DIM}")));
    assert!(rendered.ends_with(ANSI_RESET));
    assert!(rendered.contains("ⓘ 已取消当前输入。Ctrl+D 退出。"));

    let warning = render_shell_status_bar(&HostStatusMessage {
        level: HostStatusLevel::Warning,
        text: "状态异常".to_string(),
    });
    assert!(warning.contains("! 状态异常"));

    let error = render_shell_status_bar(&HostStatusMessage {
        level: HostStatusLevel::Error,
        text: "状态失败".to_string(),
    });
    assert!(error.contains("× 状态失败"));
}

#[test]
fn shell_renders_core_lifecycle_topic_as_startup_status() {
    let profile = agent_core::CoreProfile {
        name: "test".to_string(),
        provider: "aliyun".to_string(),
        model: "qwen-plus".to_string(),
    };
    let event =
        agent_core::core_initialized_topic_event("session_a", &profile, "xml", 100_000, 50, 6, 0);

    let message = shell_status_message_from_core_topic(&event)
        .expect("shell should understand core lifecycle topic");
    assert_eq!(message.level, HostStatusLevel::Info);
    assert!(message.text.contains("Timem Core 启动成功"));
    assert!(message.text.contains("aliyun:qwen-plus"));
    assert!(message.text.contains("response protocol=xml"));
    assert!(message.text.contains("tools=6"));

    let rendered = render_shell_status_bar(&message);
    assert!(rendered.contains("ⓘ"));
    assert!(rendered.contains("Timem Core 启动成功"));
}

#[test]
fn shell_renders_work_instruction_load_topic_as_status() {
    let report = agent_core::WorkInstructionLoadReport {
        status: agent_core::WorkInstructionLoadStatus::Loaded,
        directory: "/tmp/project".into(),
        file_names: vec!["AGENTS.md".to_string()],
        context: Some("guide".to_string()),
        error: None,
    };
    let event = agent_core::work_instruction_load_topic_event("session_a", &report);

    let message = shell_status_message_from_core_topic(&event)
        .expect("shell should understand work instruction status topic");
    assert_eq!(message.level, HostStatusLevel::Info);
    assert_eq!(message.text, "已加载当前工作目录指令：AGENTS.md");

    let rendered = render_shell_status_bar(&message);
    assert!(rendered.contains("ⓘ"));
    assert!(rendered.contains("已加载当前工作目录指令：AGENTS.md"));
}

#[test]
fn shell_renders_worker_identity_from_lifecycle_topic() {
    let profile = agent_core::CoreProfile {
        name: "test".to_string(),
        provider: "local".to_string(),
        model: "fake-model".to_string(),
    };
    let identity = agent_core::CoreSessionWorkerIdentity::new(
        "session_worker",
        4,
        Some("日志分析".to_string()),
        Some("parent".to_string()),
    );
    let event = agent_core::core_initialized_topic_event_with_worker(
        "session_worker",
        &profile,
        "markdown",
        100_000,
        50,
        6,
        0,
        Some(&identity),
        None,
        None,
    );

    let message = shell_status_message_from_core_topic(&event)
        .expect("shell should render worker lifecycle topic");
    assert!(message.text.contains("Timem Core 日志分析 启动成功"));
    assert!(message.text.contains("local:fake-model"));
}

#[test]
fn no_memory_status_omits_memory_icon() {
    let rendered = render_final_response_at(
        "Hello",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 237,
            completion_tokens: 26,
            ..UsageStats::zero()
        },
        None,
        "aliyun",
        "qwen-plus",
        1,
        100_000,
        "10:08:43",
    );
    assert!(rendered.contains("aliyun:qwen-plus ⇌1 ║ ▲237  ▼26"));
    assert!(!rendered.contains("⛃"));
}

#[test]
fn memory_marker_visual_variants() {
    assert_eq!(
        memory_marker(&UsageStats {
            mem_reads: 1,
            ..UsageStats::zero()
        }),
        "◂⛃"
    );
    assert_eq!(
        memory_marker(&UsageStats {
            mem_writes: 1,
            ..UsageStats::zero()
        }),
        "▸⛃"
    );
    assert_eq!(
        memory_marker(&UsageStats {
            mem_reads: 1,
            mem_writes: 1,
            ..UsageStats::zero()
        }),
        "◂▸⛃"
    );
    assert_eq!(memory_marker(&UsageStats::zero()), "");
}
