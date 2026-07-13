use super::*;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.push(format!("timem_work_instructions_{name}_{stamp}"));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn work_instruction_mode_defaults_to_silent_and_accepts_sources() {
    let env = HashMap::new();
    assert_eq!(
        work_instruction_mode_from_sources(None, &env),
        WorkInstructionLoadMode::Silent
    );

    let mut env = HashMap::new();
    env.insert("TIMEM_WORK_INSTRUCTIONS".to_string(), "ask".to_string());
    assert_eq!(
        work_instruction_mode_from_sources(None, &env),
        WorkInstructionLoadMode::Ask
    );
    assert_eq!(
        work_instruction_mode_from_sources(Some("off"), &env),
        WorkInstructionLoadMode::Off
    );
}

#[test]
fn discovers_and_formats_agents_and_claude_context() {
    let dir = tmp_dir("format");
    fs::write(dir.join("AGENTS.md"), "Use focused tests.\n").unwrap();
    fs::write(dir.join("CLAUDE.md"), "Keep changes scoped.\n").unwrap();

    let context = load_work_instruction_context(&dir).unwrap().unwrap();
    assert_eq!(context.files.len(), 2);
    assert!(context.context.contains("work_directory_instructions"));
    assert!(context.context.contains("file=\"AGENTS.md\""));
    assert!(context.context.contains("file=\"CLAUDE.md\""));
    assert!(context
        .context
        .contains(&format!("directory=\"{}\"", dir.display())));
    assert!(context.context.contains("Use focused tests."));
    assert!(context.context.contains("Keep changes scoped."));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn load_report_is_structured_for_host_rendering() {
    let dir = tmp_dir("report");
    fs::write(dir.join("AGENTS.md"), "Run focused tests.\n").unwrap();

    let report = work_instruction_load_report(&dir);
    assert_eq!(report.status, WorkInstructionLoadStatus::Loaded);
    assert_eq!(report.directory, dir);
    assert_eq!(report.file_names, vec!["AGENTS.md".to_string()]);
    assert!(report
        .context
        .as_deref()
        .unwrap()
        .contains("Run focused tests."));
    assert_eq!(report.error, None);
    assert_eq!(
        report.message(),
        WorkInstructionLoadMessage {
            kind: WorkInstructionLoadMessageKind::Loaded,
            level: Some(HostStatusLevel::Info),
            directory: report.directory.clone(),
            file_names: vec!["AGENTS.md".to_string()],
            error: None,
        }
    );

    let _ = fs::remove_dir_all(report.directory);
}

#[test]
fn load_request_is_structured_for_host_confirmation() {
    let dir = tmp_dir("request");
    fs::write(dir.join("AGENTS.md"), "Run focused tests.\n").unwrap();
    fs::write(dir.join("CLAUDE.md"), "Keep edits scoped.\n").unwrap();

    let request = work_instruction_load_request(&dir).unwrap();
    assert_eq!(request.directory, dir);
    assert_eq!(
        request.file_names,
        vec!["AGENTS.md".to_string(), "CLAUDE.md".to_string()]
    );
    let debug = format!("{request:?}");
    for forbidden in ["是否加载", "跳过", "\x1b"] {
        assert!(
            !debug.contains(forbidden),
            "core load request leaked shell UI text {forbidden:?}: {debug}"
        );
    }

    let _ = fs::remove_dir_all(request.directory);
}

#[test]
fn missing_report_is_not_an_error() {
    let dir = tmp_dir("missing");
    let report = work_instruction_load_report(&dir);
    assert_eq!(report.status, WorkInstructionLoadStatus::NotFound);
    assert!(report.file_names.is_empty());
    assert_eq!(report.context, None);
    assert_eq!(report.error, None);
    assert_eq!(
        report.message().kind,
        WorkInstructionLoadMessageKind::NotFound
    );
    assert_eq!(report.message().level, None);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn missing_load_request_is_none() {
    let dir = tmp_dir("missing_request");
    assert_eq!(work_instruction_load_request(&dir), None);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn combines_additional_contexts_without_empty_sections() {
    assert_eq!(combine_additional_contexts([None, Some("  "), None]), None);
    assert_eq!(
        combine_additional_contexts([Some("work instructions"), None, Some("workspace")]),
        Some("work instructions\n\nworkspace".to_string())
    );
}
