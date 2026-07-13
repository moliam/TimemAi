use super::*;
use agent_core::{
    CoreMemoryActivity, CoreSessionState, CoreTopic, CORE_TOPIC_ACTION, CORE_TOPIC_CONTEXT_COMPACT,
    CORE_TOPIC_MODEL_REPAIR, CORE_TOPIC_MODEL_RESPONSE, CORE_TOPIC_WORK_INSTRUCTION_LOAD,
};
use serde_json::json;
use std::time::{Duration, Instant};

fn perf_guard_enabled() -> bool {
    std::env::var("TIMEM_PERF_GUARD").ok().as_deref() == Some("1")
}

fn assert_perf_under(label: &str, started: Instant, budget: Duration) {
    if perf_guard_enabled() {
        let elapsed = started.elapsed();
        assert!(
            elapsed <= budget,
            "{label} took {elapsed:?}, expected <= {budget:?}"
        );
    }
}

fn action_topic(action: &str, kind: CoreActionKind, active: bool) -> CoreTopicEvent {
    action_topic_with_status(action, kind, active, "start", "running")
}

fn action_topic_with_status(
    action: &str,
    kind: CoreActionKind,
    active: bool,
    event: &str,
    status: &str,
) -> CoreTopicEvent {
    action_topic_with_status_and_pid(action, kind, active, event, status, None)
}

fn action_topic_with_status_and_pid(
    action: &str,
    kind: CoreActionKind,
    active: bool,
    event: &str,
    status: &str,
    pid: Option<u32>,
) -> CoreTopicEvent {
    CoreTopicEvent::new(
        "session_test",
        CoreTopic::new(
            CORE_TOPIC_ACTION,
            json!({
                "name": CORE_TOPIC_ACTION,
                "action": action,
                "active": active,
                "event": event,
            }),
        ),
        CoreSessionState::Running,
        json!({
            "action": action,
            "input": serde_json::Value::Null,
            "kind": kind,
            "active": active,
            "event": event,
            "status": status,
            "pid": pid,
            "memory_activity": CoreMemoryActivity::None,
        }),
    )
}

fn bash_kind(command: &str) -> CoreActionKind {
    CoreActionKind::Bash {
        command: command.to_string(),
        mode: "normal".to_string(),
        interval_ms: None,
        timeout_ms: None,
        loop_timeout_ms: None,
        once_timeout_ms: None,
    }
}

fn polling_bash_kind(command: &str) -> CoreActionKind {
    CoreActionKind::Bash {
        command: command.to_string(),
        mode: "poll".to_string(),
        interval_ms: Some(5000),
        timeout_ms: None,
        loop_timeout_ms: Some(60000),
        once_timeout_ms: Some(5000),
    }
}

fn model_response_topic(free_talk: &str, _progress: &str) -> CoreTopicEvent {
    CoreTopicEvent::new(
        "session_test",
        CoreTopic::new(
            CORE_TOPIC_MODEL_RESPONSE,
            json!({
                "name": CORE_TOPIC_MODEL_RESPONSE,
            }),
        ),
        CoreSessionState::Running,
        json!({
            "status": "working",
            "free_talk": free_talk,
            "final_answer": "",
            "continue_work": true,
        }),
    )
}

fn model_response_topic_with_worker_count(
    free_talk: &str,
    _progress: &str,
    working_worker_count: usize,
) -> CoreTopicEvent {
    CoreTopicEvent::new(
        "session_test",
        CoreTopic::new(
            CORE_TOPIC_MODEL_RESPONSE,
            json!({
                "name": CORE_TOPIC_MODEL_RESPONSE,
            }),
        ),
        CoreSessionState::Running,
        json!({
            "status": "working",
            "free_talk": free_talk,
            "final_answer": "",
            "continue_work": true,
            "global": {
                "working_worker_count": working_worker_count,
            },
        }),
    )
}

fn model_repair_topic(issue: &str, attempt: u32, max_attempts: u32) -> CoreTopicEvent {
    CoreTopicEvent::new(
        "session_test",
        CoreTopic::new(
            CORE_TOPIC_MODEL_REPAIR,
            json!({
                "name": CORE_TOPIC_MODEL_REPAIR,
            }),
        ),
        CoreSessionState::WaitingModel,
        json!({
            "issue": issue,
            "attempt": attempt,
            "max_attempts": max_attempts,
        }),
    )
}

fn work_instruction_load_topic(status: &str, file_names: Vec<&str>) -> CoreTopicEvent {
    CoreTopicEvent::new(
        "session_test",
        CoreTopic::new(
            CORE_TOPIC_WORK_INSTRUCTION_LOAD,
            json!({
                "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
            }),
        ),
        CoreSessionState::Running,
        json!({
            "status": status,
            "directory": "/tmp/project",
            "file_names": file_names,
            "error": null,
        }),
    )
}

fn context_compact_topic(before: u32, after: u32) -> CoreTopicEvent {
    CoreTopicEvent::new(
        "session_test",
        CoreTopic::new(
            CORE_TOPIC_CONTEXT_COMPACT,
            json!({
                "name": CORE_TOPIC_CONTEXT_COMPACT,
            }),
        ),
        CoreSessionState::Running,
        json!({
            "estimated_before_tokens": before,
            "estimated_after_tokens": after,
            "discarded_delta_ids": ["pd_1"],
            "offloaded_delta_ids": ["pd_2"],
            "scratch_id": "scratch_1",
        }),
    )
}

#[test]
fn panel_renders_heavy_border_and_blinking_transient() {
    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));
    let rendered = render_observation_panel(&panel);
    assert!(rendered.contains("\x1b[1m┏━ Thought / Action"));
    assert!(rendered.contains("\x1b[38;5;245m· 思考中..."));
    assert!(rendered.contains('┗'));
}

#[test]
fn context_compact_topic_renders_highlighted_info_line() {
    let events =
        observation_events_from_core_topic_events(&[context_compact_topic(82_000, 14_000)]);
    assert_eq!(
        events,
        vec![ObservationEvent::Persistent(
            "[INFO] Context compacted : 82K --> 14K".to_string()
        )]
    );
    let mut panel = ObservationPanel::new(8, 88);
    panel.apply_all(events);
    let rendered = render_observation_panel(&panel);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("[INFO] Context compacted : 82K --> 14K"));
    assert!(
        rendered.contains("\x1b[94;1m[INFO]\x1b[0m"),
        "INFO marker should be highlighted: {rendered}"
    );
}

#[test]
fn scroll_mode_keeps_bounded_rows_but_accumulate_mode_keeps_history() {
    let mut scroll = ObservationPanel::new(3, 72);
    let mut accumulate = ObservationPanel::new(3, 72).with_mode(ObservationPanelMode::Accumulate);
    for idx in 0..5 {
        let event = ObservationEvent::Persistent(format!("line {idx}"));
        scroll.apply(event.clone());
        accumulate.apply(event);
    }

    let scroll_plain = strip_ansi(&render_observation_panel(&scroll));
    assert!(!scroll_plain.contains("line 0"));
    assert!(scroll_plain.contains("line 4"));

    let accumulate_plain = strip_ansi(&render_observation_panel(&accumulate));
    assert!(accumulate_plain.contains("line 0"));
    assert!(accumulate_plain.contains("line 4"));
}

#[test]
fn active_lines_cycle_text_depth_across_ticks() {
    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::Active("`pwd`".to_string()));

    let dark = render_observation_panel_at(&panel, 0);
    let mid = render_observation_panel_at(&panel, 1);
    let light = render_observation_panel_at(&panel, 2);
    let looped = render_observation_panel_at(&panel, 3);

    assert!(dark.contains("\x1b[38;5;245m"));
    assert!(mid.contains("\x1b[38;5;250m"));
    assert!(light.contains("\x1b[38;5;255m"));
    assert!(looped.contains("\x1b[38;5;245m"));
    assert!(strip_ansi(&dark).contains("· [.  ] pwd"));
    assert!(!strip_ansi(&dark).contains("`Bash"));
}

#[test]
fn panel_scrolls_when_lines_exceed_limit() {
    let mut panel = ObservationPanel::new(3, 48);
    for line in ["a", "b", "c", "d"] {
        panel.apply(ObservationEvent::Persistent(line.to_string()));
    }
    let rendered = render_observation_panel(&panel);
    assert!(!rendered.contains("· a "));
    assert!(rendered.contains("· b "));
    assert!(rendered.contains("· c "));
    assert!(rendered.contains("· d "));
}

#[test]
fn default_panel_allows_twenty_visible_rows() {
    let mut panel = ObservationPanel::default();
    for idx in 0..21 {
        panel.apply(ObservationEvent::Persistent(format!("line {idx}")));
    }
    let rendered = render_observation_panel(&panel);
    let content_rows = rendered.lines().filter(|line| line.contains('┃')).count();
    assert_eq!(content_rows, 20);
    assert!(!rendered.contains("line 0"));
    assert!(rendered.contains("line 20"));
}

#[test]
fn transient_line_does_not_enter_history() {
    let mut panel = ObservationPanel::new(2, 48);
    panel.apply(ObservationEvent::Persistent("a".to_string()));
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));
    panel.apply(ObservationEvent::ClearTransient);
    let rendered = render_observation_panel(&panel);
    assert!(rendered.contains("· a "));
    assert!(!rendered.contains("思考中"));
}

#[test]
fn persistent_update_keeps_unfinished_transient_at_bottom() {
    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));
    panel.apply(ObservationEvent::Persistent("后台 Bash 已完成".to_string()));

    let rendered = render_observation_panel(&panel);
    let persistent_pos = rendered.find("后台 Bash 已完成").unwrap();
    let transient_pos = rendered.find("思考中...").unwrap();
    assert!(persistent_pos < transient_pos);
}

#[test]
fn repeated_transient_merges_with_count_until_all_finish() {
    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));

    let rendered = render_observation_panel(&panel);
    assert_eq!(rendered.matches("思考中...").count(), 1);
    assert!(rendered.contains("思考中... x2"));

    panel.apply(ObservationEvent::FinishTransient("思考中...".to_string()));
    let rendered = render_observation_panel(&panel);
    assert!(rendered.contains("思考中..."));
    assert!(!rendered.contains("x2"));

    panel.apply(ObservationEvent::FinishTransient("思考中...".to_string()));
    let rendered = render_observation_panel(&panel);
    assert!(!rendered.contains("思考中..."));
}

#[test]
fn ensure_transient_does_not_increment_existing_status() {
    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));
    panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));

    let rendered = render_observation_panel(&panel);
    assert_eq!(rendered.matches("思考中...").count(), 1);
    assert!(!rendered.contains("x2"));
}

#[test]
fn active_line_can_settle_to_normal() {
    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::Active("`pwd`".to_string()));
    let active = render_observation_panel(&panel);
    assert!(active.contains("\x1b[38;5;245m"));
    assert!(strip_ansi(&active).contains("· [.  ] pwd"));
    panel.apply(ObservationEvent::SettleActive);
    let rendered = render_observation_panel(&panel);
    assert!(strip_ansi(&rendered).contains("· pwd"));
    assert!(!rendered.contains("\x1b[38;5;245m"));
}

#[test]
fn free_talk_renders_without_separate_progress_marker() {
    let events = observation_events_from_core_topic_events(&[
        model_response_topic("已经完成备份，继续写文件。", ""),
        action_topic("run_bash", bash_kind("printf ok"), true),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("💡 已经完成备份，继续写文件。".to_string()),
            ObservationEvent::Active("`printf ok`".to_string()),
        ]
    );
}

#[test]
fn free_talk_topic_renders_lightbulb_marker() {
    let events = observation_events_from_core_topic_events(&[model_response_topic(
        "先说明一下检查思路。",
        "正在检查项目状态。",
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Persistent(
            "💡 先说明一下检查思路。".to_string()
        )]
    );
}

#[test]
fn repair_topic_renders_warning_and_keeps_thinking() {
    let events =
        observation_events_from_core_topic_events(&[model_repair_topic("invalid_xml", 2, 5)]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("⚠️ 模型回复偏离协议，重试 (2/5)...".to_string()),
            ObservationEvent::EnsureTransient("思考中...".to_string()),
        ]
    );

    let mut panel = ObservationPanel::new(8, 72);
    panel.apply_all(events);
    let rendered = render_observation_panel(&panel);
    assert!(rendered.contains("模型回复偏离协议"));
    assert!(rendered.contains("(2/5)"));
    assert!(rendered.contains("思考中..."));
}

#[test]
fn work_instruction_status_topic_is_not_mixed_into_observation_panel() {
    let events = observation_events_from_core_topic_events(&[work_instruction_load_topic(
        "loaded",
        vec!["AGENTS.md"],
    )]);
    assert!(events.is_empty());
}

#[test]
fn model_response_keeps_thinking_when_global_workers_are_active() {
    let events =
        observation_events_from_core_topic_events(&[model_response_topic_with_worker_count(
            "正在继续执行另一个 worker。",
            "",
            2,
        )]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("💡 正在继续执行另一个 worker。".to_string()),
            ObservationEvent::EnsureTransient("思考中...".to_string()),
        ]
    );

    let mut panel = ObservationPanel::new(8, 48);
    panel.apply_all(events);
    let rendered = render_observation_panel(&panel);
    assert!(rendered.contains("正在继续执行另一个 worker"));
    assert!(rendered.contains("思考中..."));
    assert!(!rendered.contains("x2"));
}

#[test]
fn model_response_stops_thinking_when_global_workers_reach_zero() {
    let events =
        observation_events_from_core_topic_events(&[model_response_topic_with_worker_count(
            "全部 worker 已结束。",
            "",
            0,
        )]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("💡 全部 worker 已结束。".to_string()),
            ObservationEvent::FinishTransient("思考中...".to_string()),
        ]
    );

    let mut panel = ObservationPanel::new(8, 48);
    panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));
    panel.apply_all(events);
    let rendered = render_observation_panel(&panel);
    assert!(rendered.contains("全部 worker 已结束"));
    assert!(!rendered.contains("思考中..."));
}

#[test]
fn model_response_maps_run_bash_to_user_facing_bash() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("rg --files | wc -l"),
        true,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Active("`rg --files | wc -l`".to_string())]
    );
}

#[test]
fn model_response_maps_polling_run_bash_to_user_facing_poll() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        polling_bash_kind("gh run list --branch main"),
        true,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::ActiveWithTimer {
            text: "`gh run list --branch main`".to_string(),
            timer: ActionTimer {
                started_at_ms: events
                    .iter()
                    .find_map(|event| match event {
                        ObservationEvent::ActiveWithTimer { timer, .. } => {
                            Some(timer.started_at_ms)
                        }
                        _ => None,
                    })
                    .unwrap(),
                timeout_ms: None,
                loop_timeout_ms: Some(60000),
                interval_ms: Some(5000),
                once_timeout_ms: Some(5000),
            }
        }]
    );
}

#[test]
fn polling_action_topic_renders_active_countdown() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        polling_bash_kind("test -f /tmp/timem_poll_demo"),
        true,
    )]);
    let mut panel = ObservationPanel::new(8, 80);
    panel.apply_all(events);

    let rendered = render_observation_panel_at(&panel, 0);
    let plain = strip_ansi(&rendered);
    assert!(
        plain.contains("[⏱ 4/01:00] test -f /tmp/timem_poll_demo"),
        "{plain}"
    );
    assert!(!plain.contains("Poll"));
    assert!(rendered.contains("\x1b[38;5;245m"));
}

#[test]
fn action_finish_topic_updates_existing_bash_line() {
    let kind = bash_kind("printf done");
    let start =
        observation_events_from_core_topic_events(&[action_topic("run_bash", kind.clone(), true)]);
    let finish = observation_events_from_core_topic_events(&[action_topic_with_status(
        "run_bash",
        kind,
        false,
        "finish",
        "completed",
    )]);

    let mut panel = ObservationPanel::new(8, 80);
    panel.apply_all(start);
    panel.apply_all(finish);
    let rendered = render_observation_panel(&panel);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("[✔] printf done"));
    assert_eq!(plain.matches("printf done").count(), 1);
    assert!(!rendered.contains("\x1b[38;5;245m"));
}

#[test]
fn background_action_and_exit_status_render_user_facing_state() {
    let background_kind = CoreActionKind::Bash {
        command: "sleep 30".to_string(),
        mode: "background".to_string(),
        interval_ms: None,
        timeout_ms: None,
        loop_timeout_ms: None,
        once_timeout_ms: None,
    };
    let background = observation_events_from_core_topic_events(&[action_topic_with_status(
        "run_bash",
        background_kind,
        false,
        "finish",
        "background_running",
    )]);
    let mut panel = ObservationPanel::new(8, 80);
    panel.apply_all(background);
    let rendered = render_observation_panel(&panel);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("(后台执行) [后台执行] sleep 30"), "{plain}");
    assert!(rendered.contains(ANSI_BOLD) || rendered.contains("\x1b["));

    let finished = observation_events_from_core_topic_events(&[action_topic_with_status(
        "run_bash",
        CoreActionKind::Bash {
            command: "sleep 30".to_string(),
            mode: "background".to_string(),
            interval_ms: None,
            timeout_ms: None,
            loop_timeout_ms: None,
            once_timeout_ms: None,
        },
        false,
        "finish",
        "background_finished",
    )]);
    panel.apply_all(finished);
    let rendered = render_observation_panel(&panel);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("(后台执行) [后台完成] sleep 30"), "{plain}");
}

#[test]
fn timed_out_bash_finish_renders_still_running_pid() {
    let kind = CoreActionKind::Bash {
        command: "sleep 18".to_string(),
        mode: "normal".to_string(),
        interval_ms: None,
        timeout_ms: Some(10000),
        loop_timeout_ms: None,
        once_timeout_ms: None,
    };
    let start =
        observation_events_from_core_topic_events(&[action_topic("run_bash", kind.clone(), true)]);
    let finish = observation_events_from_core_topic_events(&[action_topic_with_status_and_pid(
        "run_bash",
        kind,
        false,
        "finish",
        "timeout",
        Some(49189),
    )]);

    let mut panel = ObservationPanel::new(8, 100);
    panel.apply_all(start);
    panel.apply_all(finish);
    let plain = strip_ansi(&render_observation_panel(&panel));
    assert!(
        plain.contains("[超时 pid=49189 仍在运行] sleep 18"),
        "{plain}"
    );
    assert_eq!(plain.matches("sleep 18").count(), 1, "{plain}");
}

#[test]
fn core_topic_events_map_action_without_protocol_parsing() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("git log --oneline v0.5.2..HEAD"),
        true,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Active(
            "`git log --oneline v0.5.2..HEAD`".to_string()
        )]
    );
}

#[test]
fn core_topic_events_map_free_talk_and_action_events() {
    let events = observation_events_from_core_topic_events(&[
        model_response_topic("正在检查项目状态。", ""),
        action_topic("run_bash", bash_kind("git status --short"), true),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("💡 正在检查项目状态。".to_string()),
            ObservationEvent::Active("`git status --short`".to_string()),
        ]
    );
}

#[test]
fn core_topic_events_wire_shape_maps_free_talk_and_action_events() {
    let topic_events = [
        model_response_topic("正在检查项目状态。", ""),
        action_topic("run_bash", bash_kind("git status --short"), true),
    ];
    let events = observation_events_from_core_topic_events(&topic_events);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("💡 正在检查项目状态。".to_string()),
            ObservationEvent::Active("`git status --short`".to_string()),
        ]
    );
}

#[test]
fn core_topic_events_map_multiple_action_events() {
    let events = observation_events_from_core_topic_events(&[
        model_response_topic("正在并行检查记忆和本地文件。", ""),
        action_topic(
            "memmgr",
            CoreActionKind::Memory {
                surface: "durable".to_string(),
                operation: "query".to_string(),
            },
            false,
        ),
        action_topic("run_bash", bash_kind("rg --files -g '*.rs'"), true),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("💡 正在并行检查记忆和本地文件。".to_string()),
            ObservationEvent::Persistent("长期记忆: 查询".to_string()),
            ObservationEvent::Active("`rg --files -g '*.rs'`".to_string()),
        ]
    );
}

#[test]
fn action_group_actions_render_directly() {
    let events = observation_events_from_core_topic_events(&[
        action_topic("run_bash", bash_kind("printf a"), true),
        action_topic("run_bash", bash_kind("printf b"), true),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Active("`printf a`".to_string()),
            ObservationEvent::Active("`printf b`".to_string()),
        ]
    );
}

#[test]
fn mixed_actions_render_directly_from_action_kind() {
    let events = observation_events_from_core_topic_events(&[
        action_topic("run_bash", bash_kind("printf named"), true),
        action_topic("run_bash", bash_kind("printf plain"), true),
        action_topic(
            "memmgr",
            CoreActionKind::Memory {
                surface: "durable".to_string(),
                operation: "query".to_string(),
            },
            false,
        ),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Active("`printf named`".to_string()),
            ObservationEvent::Active("`printf plain`".to_string()),
            ObservationEvent::Persistent("长期记忆: 查询".to_string()),
        ]
    );
}

#[test]
fn memmgr_actions_map_to_user_readable_observation_events() {
    let events = observation_events_from_core_topic_events(&[
        action_topic(
            "memmgr",
            CoreActionKind::Memory {
                surface: "durable".to_string(),
                operation: "query".to_string(),
            },
            false,
        ),
        action_topic(
            "memmgr",
            CoreActionKind::Memory {
                surface: "context".to_string(),
                operation: "shrink".to_string(),
            },
            false,
        ),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("长期记忆: 查询".to_string()),
            ObservationEvent::Persistent("上下文: 压缩".to_string()),
        ]
    );
}

#[test]
fn capmgr_action_maps_to_user_readable_observation_events() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "capmgr",
        CoreActionKind::Capability {
            op: "load".to_string(),
            kind: "skill".to_string(),
            id: "release_quality_gate".to_string(),
        },
        false,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Persistent(
            "能力: 加载 skill/release_quality_gate".to_string()
        )]
    );
}

#[test]
fn self_tool_action_maps_to_user_readable_observation_events() {
    let events = observation_events_from_core_topic_events(&[
        action_topic(
            "self_tool",
            CoreActionKind::SelfTool {
                self_type: "mem_path".to_string(),
                op: "read".to_string(),
            },
            false,
        ),
        action_topic(
            "self_tool",
            CoreActionKind::SelfTool {
                self_type: "about_me".to_string(),
                op: "read".to_string(),
            },
            false,
        ),
    ]);
    assert_eq!(
        events,
        vec![
            ObservationEvent::Persistent("Timem: 查看记忆路径".to_string()),
            ObservationEvent::Persistent("Timem: 查看自身信息".to_string()),
        ]
    );
}

#[test]
fn action_topic_with_json_like_command_keeps_command_intact() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("printf '{\"ok\":true}' > target/example.json"),
        true,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Active(
            "`printf '{\"ok\":true}' > target/example.json`".to_string()
        )]
    );
}

#[test]
fn unknown_action_uses_intent_without_exposing_action_name() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "future_tool",
        CoreActionKind::Other {
            action: "future_tool".to_string(),
        },
        false,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Persistent(
            "Action: future_tool".to_string()
        )]
    );
}

#[test]
fn empty_core_topic_events_create_no_observation_events() {
    let events = observation_events_from_core_topic_events(&[]);
    assert!(events.is_empty());
}

#[test]
fn action_topic_does_not_expose_internal_action_name() {
    let mut panel = ObservationPanel::default();
    panel.apply_all(observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("rg --files | wc -l"),
        true,
    )]));
    let rendered = render_observation_panel(&panel);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("· [.  ] rg --files | wc -l"));
    assert!(!plain.contains("`Bash"));
    assert!(!rendered.contains("run_bash"));
}

#[test]
fn tree_child_lines_render_under_intent_and_wrap_without_repeating_branch() {
    let mut panel = ObservationPanel::new(8, 44);
    panel.apply(ObservationEvent::Persistent("统计当前代码量".to_string()));
    panel.apply(ObservationEvent::ActiveChild {
        text: "`123456789012345678901234567890 tail`".to_string(),
        is_last: true,
    });

    let rendered = render_observation_panel(&panel);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("· 统计当前代码量"));
    assert!(plain.contains("└─ [.  ] 123456789012345"));
    assert_eq!(plain.matches("└─").count(), 1);
    assert!(plain.contains("tail"));
}

#[test]
fn run_bash_without_intent_shows_plain_label() {
    let events = observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("ls -la"),
        true,
    )]);
    assert_eq!(
        events,
        vec![ObservationEvent::Active("`ls -la`".to_string())]
    );
}

#[test]
fn panel_wraps_long_command_and_truncates_one_item_after_four_rows() {
    let mut panel = ObservationPanel::new(8, 44);
    panel.apply(ObservationEvent::Active(format!(
            "{}",
            "rg --files -g '*.rs' | xargs wc -l && echo very-long-tail && echo more-output && echo another-long-part && echo segment-four && echo segment-five && echo segment-six && echo hidden-tail-after-limit"
        )));
    let rendered = render_observation_panel(&panel);
    let content_rows = rendered.lines().filter(|line| line.contains('┃')).count();
    assert_eq!(content_rows, 4);
    assert!(rendered.contains('…'));
    assert!(!rendered.contains("hidden-tail-after-limit"));
}

#[test]
fn observation_width_follows_terminal_width_policy() {
    assert_eq!(observation_panel_width_for_terminal(120), 96);
    assert_eq!(observation_panel_width_for_terminal(100), 80);
    assert_eq!(observation_panel_width_for_terminal(90), 80);
    assert_eq!(observation_panel_width_for_terminal(70), 70);
}

#[test]
fn panel_width_can_be_updated_for_current_terminal() {
    let mut panel = ObservationPanel::new(8, 44);
    panel.set_max_width(observation_panel_width_for_terminal(120));
    panel.apply(ObservationEvent::Persistent("宽度检查".to_string()));
    let rendered = render_observation_panel(&panel);
    let first_line = rendered
        .lines()
        .next()
        .unwrap()
        .replace(ANSI_BOLD, "")
        .replace(ANSI_RESET, "");
    let first_line_width = display_width(&first_line);
    assert_eq!(first_line_width, 96);
}

#[test]
fn panel_ansi_sequences_do_not_affect_visible_width() {
    let mut panel = ObservationPanel::new(8, 80);
    panel.apply(ObservationEvent::Active(
        "正在执行长命令并刷新状态".to_string(),
    ));
    panel.apply(ObservationEvent::ActiveChild {
        text: "echo \"=== git status ===\"; git status; echo; git diff --cached".to_string(),
        is_last: true,
    });
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));

    let rendered = render_observation_panel_at_with_elapsed(&panel, 2, Some("00:57"));
    let visible_widths = rendered.lines().map(display_width).collect::<Vec<_>>();
    assert!(
        visible_widths.iter().all(|width| *width == 80),
        "all panel rows should have the same visible width: {visible_widths:?}\n{rendered}"
    );
    assert!(rendered.contains("\x1b[38;5;255m"));
}

#[test]
fn long_free_talk_and_command_render_as_bounded_aligned_rows() {
    let mut panel = ObservationPanel::new(20, 80);
    panel.apply(ObservationEvent::Persistent(format!(
            "💡 {}",
            "正在处理一个非常长的工作说明，需要确认观察窗会自动换行但不会把边框撑乱，也不会因为每秒刷新而产生宽度不一致的问题。".repeat(3)
        )));
    panel.apply(ObservationEvent::Persistent(
        "分析当前工作区状态并执行长命令".to_string(),
    ));
    panel.apply(ObservationEvent::ActiveChild {
        text: format!(
            "`{}`",
            format!(
                "{} tail-marker-should-not-render-after-limit",
                "echo start; git status --short; git diff --stat; printf '%s' very-long-segment; "
                    .repeat(12)
            )
        ),
        is_last: true,
    });
    panel.apply(ObservationEvent::Transient("思考中...".to_string()));

    let rendered = render_observation_panel_at_with_elapsed(&panel, 1, Some("12:34"));
    let visible_widths = rendered.lines().map(display_width).collect::<Vec<_>>();
    assert!(
        visible_widths.iter().all(|width| *width == 80),
        "all observation rows should stay aligned: {visible_widths:?}\n{rendered}"
    );
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("💡 正在处理"));
    assert!(plain.contains("└─"));
    assert!(rendered.contains('…'));
    assert!(!rendered.contains("run_bash"));
    assert!(!rendered.contains("tail-marker-should-not-render-after-limit"));
}

#[test]
fn performance_guard_many_observation_events_render_bounded() {
    let long_text = "这是一个很长的观察窗口内容 with ascii and 中文 ".repeat(80);
    let mut events = Vec::new();
    for idx in 0..600 {
        events.push(model_response_topic(
            &format!("计划 {idx}: {long_text}"),
            &format!("进度 {idx}: {long_text}"),
        ));
        events.push(action_topic(
            "run_bash",
            bash_kind(&format!("printf '{long_text}'")),
            true,
        ));
    }

    let started = Instant::now();
    let mut panel = ObservationPanel::new(20, 96);
    for chunk in events.chunks(8) {
        let observation_events = observation_events_from_core_topic_events(chunk);
        panel.apply_all(observation_events);
        panel.apply(ObservationEvent::SettleActive);
        let rendered = render_observation_panel_at_with_elapsed(&panel, 1, Some("09:59"));
        assert!(rendered.len() < 12_000);
        assert!(!rendered.contains("run_bash"));
    }
    assert_perf_under(
        "many observation events render bounded",
        started,
        Duration::from_millis(1200),
    );
}

#[test]
fn performance_guard_topic_interface_rate_mix_render_bounded() {
    let long_text = "topic pressure 内容 with ascii 中文 ".repeat(40);
    let mut topic_events = Vec::new();
    for idx in 0..20 {
        topic_events.push(model_response_topic(
            &format!("计划 {idx}: {long_text}"),
            &format!("进度 {idx}: {long_text}"),
        ));
    }
    for idx in 0..300 {
        topic_events.push(action_topic(
            "run_bash",
            bash_kind(&format!("printf '{}'", idx)),
            true,
        ));
    }
    let supplement_events = (0..20)
        .map(|idx| ObservationEvent::Persistent(format!("ⓘ 收到用户补充 {idx}: {long_text}")))
        .collect::<Vec<_>>();

    let started = Instant::now();
    let mut panel = ObservationPanel::new(20, 100);
    for chunk in topic_events.chunks(16) {
        panel.apply_all(observation_events_from_core_topic_events(chunk));
        panel.apply(ObservationEvent::SettleActive);
        let rendered = render_observation_panel_at_with_elapsed(&panel, 1, Some("00:09"));
        assert!(rendered.len() < 14_000);
        assert!(!rendered.contains("run_bash"));
    }
    panel.apply_all(supplement_events);
    let rendered = render_observation_panel_at_with_elapsed(&panel, 2, Some("00:10"));
    assert!(rendered.len() < 14_000);
    let widths = rendered.lines().map(display_width).collect::<Vec<_>>();
    assert!(
        widths.iter().all(|width| *width == 100),
        "topic pressure render should keep aligned rows: {widths:?}\n{rendered}"
    );
    assert_perf_under(
        "topic interface 20 response 300 action 20 supplement render bounded",
        started,
        Duration::from_millis(1200),
    );
}

#[test]
fn action_timer_created_for_bash_with_timeout() {
    let kind = CoreActionKind::Bash {
        command: "sleep 10".to_string(),
        mode: "normal".to_string(),
        interval_ms: None,
        timeout_ms: Some(10000),
        loop_timeout_ms: None,
        once_timeout_ms: None,
    };
    let (text, timer) = action_detail_for_shell(&kind);
    assert_eq!(text, "`sleep 10`");
    assert!(timer.is_some());
    let t = timer.unwrap();
    assert_eq!(t.timeout_ms, Some(10000));
    assert!(t.started_at_ms > 0);
}

#[test]
fn action_timer_created_for_polling_bash() {
    let kind = CoreActionKind::Bash {
        command: "check_status".to_string(),
        mode: "poll".to_string(),
        interval_ms: Some(5000),
        timeout_ms: None,
        loop_timeout_ms: Some(60000),
        once_timeout_ms: Some(10000),
    };
    let (text, timer) = action_detail_for_shell(&kind);
    assert_eq!(text, "`check_status`");
    assert!(timer.is_some());
    let t = timer.unwrap();
    assert_eq!(t.loop_timeout_ms, Some(60000));
    assert_eq!(t.interval_ms, Some(5000));
    assert_eq!(t.once_timeout_ms, Some(10000));
}

#[test]
fn no_timer_for_bash_without_timeout() {
    let kind = bash_kind("echo hello");
    let (text, timer) = action_detail_for_shell(&kind);
    assert_eq!(text, "`echo hello`");
    assert!(timer.is_none());
}

#[test]
fn normal_bash_without_timeout_renders_without_countdown() {
    let mut panel = ObservationPanel::new(8, 80);
    panel.apply_all(observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("sleep 10 && touch /tmp/timem_poll_demo2.txt"),
        true,
    )]));
    let rendered = render_observation_panel_at(&panel, 0);
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("sleep 10 && touch /tmp/timem_poll_demo2.txt"));
    assert!(!plain.contains("⏱"), "{plain}");
    assert!(rendered.contains("\x1b[38;5;245m"));
}

#[test]
fn normal_bash_without_timeout_still_blinks_while_active() {
    let mut panel = ObservationPanel::new(8, 80);
    panel.apply_all(observation_events_from_core_topic_events(&[action_topic(
        "run_bash",
        bash_kind("printf active"),
        true,
    )]));

    let dark = render_observation_panel_at(&panel, 0);
    let mid = render_observation_panel_at(&panel, 1);
    let light = render_observation_panel_at(&panel, 2);

    assert!(dark.contains("\x1b[38;5;245m"));
    assert!(mid.contains("\x1b[38;5;250m"));
    assert!(light.contains("\x1b[38;5;255m"));
    assert!(!strip_ansi(&dark).contains("⏱"));
}

#[test]
fn format_countdown_shows_remaining_seconds() {
    let now_ms = 1000000u64;
    let timer = ActionTimer {
        started_at_ms: now_ms - 3000,
        timeout_ms: Some(10000),
        loop_timeout_ms: None,
        interval_ms: None,
        once_timeout_ms: None,
    };
    let countdown = format_countdown(&timer, now_ms);
    assert_eq!(countdown, "⏱ 07s");
}

#[test]
fn format_countdown_uses_loop_timeout_for_polling() {
    let now_ms = 1000000u64;
    let timer = ActionTimer {
        started_at_ms: now_ms - 5000,
        timeout_ms: None,
        loop_timeout_ms: Some(60000),
        interval_ms: Some(5000),
        once_timeout_ms: None,
    };
    let countdown = format_countdown(&timer, now_ms);
    assert_eq!(countdown, "⏱ 4/55s");
}

#[test]
fn format_countdown_uses_shortest_time_shape_and_never_zero_while_running() {
    assert_eq!(format_duration_short(1, true), "01s");
    assert_eq!(format_duration_short(999, true), "01s");
    assert_eq!(format_duration_short(60_000, true), "01:00");
    assert_eq!(format_duration_short(3_661_000, true), "1:01:01");
    assert_eq!(format_duration_short(2_000, false), "02");

    let now_ms = 1000000u64;
    let timer = ActionTimer {
        started_at_ms: now_ms - 59_100,
        timeout_ms: Some(60_000),
        loop_timeout_ms: None,
        interval_ms: None,
        once_timeout_ms: None,
    };
    assert_eq!(format_countdown(&timer, now_ms), "⏱ 01s");
}

#[test]
fn format_countdown_uses_poll_pulse_for_one_second_interval() {
    let now_ms = 1000000u64;
    let timer = ActionTimer {
        started_at_ms: now_ms - 9000,
        timeout_ms: None,
        loop_timeout_ms: Some(10000),
        interval_ms: Some(1000),
        once_timeout_ms: Some(1000),
    };
    assert_eq!(format_countdown(&timer, now_ms), "⏱ ↻1s");
}

#[test]
fn format_countdown_shows_two_second_poll_interval_as_one_then_zero() {
    let now_ms = 1000000u64;
    let timer = ActionTimer {
        started_at_ms: now_ms,
        timeout_ms: None,
        loop_timeout_ms: Some(10000),
        interval_ms: Some(2000),
        once_timeout_ms: Some(1000),
    };
    assert_eq!(format_countdown(&timer, now_ms), "⏱ 1/10s");

    let timer = ActionTimer {
        started_at_ms: now_ms - 1000,
        timeout_ms: None,
        loop_timeout_ms: Some(10000),
        interval_ms: Some(2000),
        once_timeout_ms: Some(1000),
    };
    assert_eq!(format_countdown(&timer, now_ms), "⏱ 0/9s");
}

#[test]
fn format_countdown_rounds_ms_up_for_ui_display() {
    let now_ms = 1000000u64;
    let timer = ActionTimer {
        started_at_ms: now_ms - 1001,
        timeout_ms: Some(2001),
        loop_timeout_ms: None,
        interval_ms: None,
        once_timeout_ms: None,
    };
    assert_eq!(format_countdown(&timer, now_ms), "⏱ 01s");
}

#[test]
fn observation_line_with_timer_renders_countdown() {
    let mut panel = ObservationPanel::new(10, 80);
    let timer = ActionTimer {
        started_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 2000,
        timeout_ms: Some(10000),
        loop_timeout_ms: None,
        interval_ms: None,
        once_timeout_ms: None,
    };
    panel.apply(ObservationEvent::ActiveWithTimer {
        text: "`sleep 10`".to_string(),
        timer,
    });
    let rendered = render_observation_panel_at(&panel, 0);
    assert!(
        rendered.contains("\u{23f1}"),
        "Expected countdown symbol in output"
    );
    assert!(rendered.contains("\x1b[38;5;245m"));
    let plain = strip_ansi(&rendered);
    assert!(plain.contains("[⏱ 08s] sleep 10"));
    assert!(!plain.contains("`sleep 10`"));
}

#[test]
fn long_active_command_keeps_countdown_after_bash_label() {
    let mut panel = ObservationPanel::new(10, 88);
    let timer = ActionTimer {
        started_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            - 1000,
        timeout_ms: None,
        loop_timeout_ms: Some(15000),
        interval_ms: Some(3000),
        once_timeout_ms: Some(1000),
    };
    panel.apply(ObservationEvent::ActiveChildWithTimer {
            text: "`if [ ! -f /tmp/poll_start_c2.txt ]; then date +%s > /tmp/poll_start_c2.txt; fi; START=$(cat /tmp/poll_start_c2.txt); NOW=$(date +%s); ELAPSED=$((NOW - START)); echo \"已过 ${ELAPSED}s\"; [ $ELAPSED -ge 10 ] && echo '条件满足，提前退出' && exit 0 || exit 1`".to_string(),
            is_last: true,
            timer,
        });

    let plain = strip_ansi(&render_observation_panel_at(&panel, 0));
    assert!(plain.contains("[⏱ 1/14s] if [ ! -f"), "{plain}");
    assert!(!plain.contains("Bash:"), "{plain}");
}
