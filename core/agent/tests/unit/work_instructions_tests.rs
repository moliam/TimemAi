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

fn write_candidate(dir: &Path, relative: &str, body: &str) -> PathBuf {
    let path = dir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn discovers_ecosystem_candidates_in_stable_order() {
    let dir = tmp_dir("candidates");
    let home = tmp_dir("candidates_home");
    let agents = write_candidate(&dir, "AGENTS.md", "agents rules\n");
    let claude = write_candidate(&dir, "CLAUDE.md", "claude rules\n");
    let nested_claude = write_candidate(&dir, ".claude/CLAUDE.md", "nested claude rules\n");
    let nested_codex = write_candidate(&dir, ".codex/AGENTS.md", "nested codex rules\n");
    let global_claude = write_candidate(&home, ".claude/CLAUDE.md", "global claude rules\n");
    let global_codex = write_candidate(&home, ".codex/AGENTS.md", "global codex rules\n");

    let candidates = discover_work_instruction_candidates(&dir, Some(&home));
    let paths: Vec<_> = candidates.iter().map(|c| c.path.clone()).collect();
    assert_eq!(
        paths,
        vec![
            agents.clone(),
            claude.clone(),
            nested_claude.clone(),
            nested_codex.clone(),
            global_claude.clone(),
            global_codex.clone(),
        ]
    );
    assert_eq!(
        candidates
            .iter()
            .map(|c| (c.filename.as_str(), c.source))
            .collect::<Vec<_>>(),
        vec![
            ("AGENTS.md", WorkInstructionSource::Project),
            ("CLAUDE.md", WorkInstructionSource::Project),
            ("CLAUDE.md", WorkInstructionSource::Project),
            ("AGENTS.md", WorkInstructionSource::Project),
            ("CLAUDE.md", WorkInstructionSource::Global),
            ("AGENTS.md", WorkInstructionSource::Global),
        ]
    );
    assert_eq!(WorkInstructionSource::Project.label(), "project");
    assert_eq!(WorkInstructionSource::Global.label(), "global");

    // Without a home, only project candidates exist.
    let project_only = discover_work_instruction_candidates(&dir, None);
    assert_eq!(project_only.len(), 4);

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn selection_none_keeps_everything_and_empty_selection_loads_nothing() {
    let dir = tmp_dir("selection_default");
    let home = tmp_dir("selection_default_home");
    write_candidate(&dir, "AGENTS.md", "agents rules\n");
    write_candidate(&home, ".claude/CLAUDE.md", "global claude rules\n");

    let default_context = load_work_instruction_context_selected(&dir, Some(&home), None)
        .unwrap()
        .unwrap();
    assert_eq!(default_context.files.len(), 2);
    assert!(default_context.context.contains("global claude rules"));

    let empty = BTreeSet::new();
    assert!(
        load_work_instruction_context_selected(&dir, Some(&home), Some(&empty))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        work_instruction_load_request_selected(&dir, Some(&home), Some(&empty)),
        None
    );
    let report = work_instruction_load_report_selected(&dir, Some(&home), Some(&empty));
    assert_eq!(report.status, WorkInstructionLoadStatus::NotFound);

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn selection_filters_nested_and_global_candidates() {
    let dir = tmp_dir("selection_filter");
    let home = tmp_dir("selection_filter_home");
    write_candidate(&dir, "AGENTS.md", "agents rules\n");
    let nested_claude = write_candidate(&dir, ".claude/CLAUDE.md", "nested claude rules\n");
    let global_codex = write_candidate(&home, ".codex/AGENTS.md", "global codex rules\n");

    let selection = BTreeSet::from([
        nested_claude.display().to_string(),
        global_codex.display().to_string(),
    ]);
    let context = load_work_instruction_context_selected(&dir, Some(&home), Some(&selection))
        .unwrap()
        .unwrap();
    assert_eq!(context.files.len(), 2);
    assert!(context.context.contains("nested claude rules"));
    assert!(context.context.contains("global codex rules"));
    assert!(!context.context.contains("agents rules"));
    // The source attribute rides on the begin marker; dedupe markers stay
    // compatible for the compaction re-injection parser.
    assert!(context
        .context
        .contains("[BEGIN PROJECT INSTRUCTION file=\"CLAUDE.md\" directory=\""));
    assert!(context.context.contains("source=\"project\""));
    assert!(context.context.contains("source=\"global\""));

    let request =
        work_instruction_load_request_selected(&dir, Some(&home), Some(&selection)).unwrap();
    assert_eq!(request.directory, dir);
    assert_eq!(
        request.file_names,
        vec!["CLAUDE.md".to_string(), "AGENTS.md".to_string()]
    );

    let report = work_instruction_load_report_selected(&dir, Some(&home), Some(&selection));
    assert_eq!(report.status, WorkInstructionLoadStatus::Loaded);
    assert_eq!(report.file_names, request.file_names);

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn validate_selection_fails_closed_on_unknown_paths() {
    let dir = tmp_dir("validate");
    let home = tmp_dir("validate_home");
    let agents = write_candidate(&dir, "AGENTS.md", "agents rules\n");
    let global_claude = write_candidate(&home, ".claude/CLAUDE.md", "global claude rules\n");
    let stranger = write_candidate(&dir, "OTHER.md", "not a candidate\n");

    let valid = validate_work_instruction_selection(
        &dir,
        Some(&home),
        &[
            global_claude.display().to_string(),
            agents.display().to_string(),
        ],
    )
    .unwrap();
    assert_eq!(
        valid,
        BTreeSet::from([
            agents.display().to_string(),
            global_claude.display().to_string(),
        ])
    );

    for bad in [
        vec![stranger.display().to_string()],
        vec![dir.join("missing.md").display().to_string()],
        vec![String::from("   ")],
        vec![format!("{}/../escape.md", dir.display())],
    ] {
        assert_eq!(
            validate_work_instruction_selection(&dir, Some(&home), &bad),
            Err("work_instruction_selection_invalid".to_string())
        );
    }

    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(home);
}

#[test]
fn oversized_instruction_file_is_truncated_to_the_hard_cap() {
    let dir = tmp_dir("cap");
    let body = "a".repeat(WORK_INSTRUCTION_FILE_CONTENT_LIMIT + 4096);
    write_candidate(&dir, "AGENTS.md", &body);

    let context = load_work_instruction_context_selected(&dir, None, None)
        .unwrap()
        .unwrap();
    assert!(context.files[0].content.contains("\n... [truncated]"));
    assert!(context.files[0].content.len() <= WORK_INSTRUCTION_FILE_CONTENT_LIMIT + 32);
    assert!(context.context.len() <= WORK_INSTRUCTION_FILE_CONTENT_LIMIT + 512);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn instruction_home_from_env_prefers_the_explicit_override() {
    let previous = std::env::var("TIMEM_WORK_INSTRUCTION_HOME").ok();
    std::env::set_var(
        "TIMEM_WORK_INSTRUCTION_HOME",
        "/tmp/instruction-home-override",
    );
    assert_eq!(
        work_instruction_home_from_env(),
        Some(PathBuf::from("/tmp/instruction-home-override"))
    );
    std::env::remove_var("TIMEM_WORK_INSTRUCTION_HOME");
    let fallback = work_instruction_home_from_env();
    std::env::remove_var("TIMEM_WORK_INSTRUCTION_HOME");
    assert_ne!(
        fallback,
        Some(PathBuf::from("/tmp/instruction-home-override"))
    );
    if let Some(value) = previous {
        std::env::set_var("TIMEM_WORK_INSTRUCTION_HOME", value);
    }
}
