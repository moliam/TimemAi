use super::*;

#[test]
fn formats_runtime_time_context_with_bilingual_weekday() {
    let context = format_runtime_time_context(LocalTimeParts {
        year: 2026,
        month: 7,
        day: 4,
        hour: 9,
        minute: 8,
        second: 7,
        weekday: 6,
    });

    assert_eq!(
        context,
        "2026-07-04 09:08:07 local_time, weekday=周六/Saturday"
    );
}

#[test]
fn weekday_labels_handle_unknown_values() {
    assert_eq!(weekday_zh(9), "未知");
    assert_eq!(weekday_en(9), "Unknown");
}

#[test]
fn supporting_context_formats_host_supplied_runtime_identity() {
    let context = format_supporting_context(
        SupportingContextInput {
            provider: "aliyun",
            model: "qwen-plus",
            runtime: "timem_native_shell",
            run_bash_target: "user_local_machine",
        },
        "2026-07-04 09:08:07 local_time, weekday=周六/Saturday",
    );

    assert_eq!(
            context,
            "provider: aliyun, model: qwen-plus\nruntime: timem_native_shell\nrun_bash_target: user_local_machine\nruntime_time: 2026-07-04 09:08:07 local_time, weekday=周六/Saturday"
        );
}

#[test]
fn turn_supporting_context_combines_runtime_and_additional_context() {
    let context = turn_supporting_context(
        SupportingContextInput {
            provider: "aliyun",
            model: "qwen-plus",
            runtime: "timem_native_shell",
            run_bash_target: "user_local_machine",
        },
        Some("  work instructions\nworkspace refs  "),
    );

    assert!(context.contains("provider: aliyun, model: qwen-plus"));
    assert!(context.contains("\n\nwork instructions\nworkspace refs"));
    assert!(!context.contains("  work instructions"));
}

#[test]
fn runtime_info_context_uses_host_supplied_entries_without_cwd() {
    let context = runtime_info_context(&[
        "ui: shell",
        "run_bash: available; executes on user_local_machine",
        "",
    ])
    .unwrap();

    assert_eq!(
        context,
        "runtime_info:\n- ui: shell\n- run_bash: available; executes on user_local_machine"
    );
    assert!(!context.contains("cwd:"));
}
