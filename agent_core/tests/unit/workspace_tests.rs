use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn tmp_workspace_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "timem_workspace_{name}_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir.join("workspace.json")
}

#[test]
fn state_adds_sorts_and_deduplicates_workspace_dirs() {
    let home = Path::new("/home/example");
    let mut state = WorkspaceState::new(vec!["/z".to_string(), "/a".to_string(), "/z".to_string()]);

    assert_eq!(state.dirs(), &["/a".to_string(), "/z".to_string()]);
    assert_eq!(
        state.add_dir("~/project", home),
        WorkspaceChange::Added("/home/example/project".to_string())
    );
    assert_eq!(
        state.add_dir("~/project", home),
        WorkspaceChange::Unchanged(WorkspaceUnchangedReason::Duplicate)
    );
    assert_eq!(
        state.dirs(),
        &[
            "/a".to_string(),
            "/home/example/project".to_string(),
            "/z".to_string()
        ]
    );
}

#[test]
fn state_removes_by_index_with_bounds_check() {
    let mut state = WorkspaceState::new(vec!["/a".to_string(), "/b".to_string()]);

    assert_eq!(
        state.remove_index(1),
        WorkspaceChange::Removed("/b".to_string())
    );
    assert_eq!(
        state.remove_index(2),
        WorkspaceChange::Unchanged(WorkspaceUnchangedReason::IndexOutOfRange)
    );
    assert_eq!(state.dirs(), &["/a".to_string()]);
}

#[test]
fn workspace_menu_report_is_ui_neutral_command_data() {
    let report = workspace_menu_report(&[
        "/z".to_string(),
        "".to_string(),
        "/a".to_string(),
        "/z".to_string(),
    ]);
    let debug = format!("{report:?}");
    for forbidden in ["\x1b[", "▶", "Add..."] {
        assert!(
            !debug.contains(forbidden),
            "core workspace report must stay UI-neutral and avoid terminal marker {forbidden:?}"
        );
    }
    assert_eq!(report.dirs, vec!["/a".to_string(), "/z".to_string()]);
    assert!(!report.is_empty);
    assert_eq!(report.add_index, 2);

    let empty = workspace_menu_report(&[]);
    assert!(empty.is_empty);
    assert_eq!(empty.add_index, 0);
}

#[test]
fn workspace_command_report_adds_removes_and_persists_dirs() {
    let path = tmp_workspace_path("command_report");
    let home_dir = PathBuf::from("/home/example");

    let added = apply_workspace_command_to_path(
        &path,
        WorkspaceCommand::AddDir {
            value: "~/project".to_string(),
            home_dir,
        },
    );
    assert_eq!(
        added.outcome,
        WorkspaceCommandOutcome::Added("/home/example/project".to_string())
    );
    assert_eq!(
        added.message(),
        WorkspaceCommandMessage {
            kind: WorkspaceCommandMessageKind::Added,
            level: HostStatusLevel::Info,
            subject: Some("/home/example/project".to_string()),
            error: None,
        }
    );
    assert_eq!(added.dirs, vec!["/home/example/project".to_string()]);
    assert!(added.changed);
    assert_eq!(
        load_workspace_dirs_from_path(&path),
        vec!["/home/example/project".to_string()]
    );

    let duplicate = apply_workspace_command_to_path(
        &path,
        WorkspaceCommand::AddDir {
            value: "/home/example/project".to_string(),
            home_dir: PathBuf::from("/home/example"),
        },
    );
    assert_eq!(duplicate.outcome, WorkspaceCommandOutcome::Duplicate);
    assert_eq!(
        duplicate.message().kind,
        WorkspaceCommandMessageKind::Duplicate
    );
    assert!(!duplicate.changed);

    let removed = apply_workspace_command_to_path(&path, WorkspaceCommand::RemoveIndex(0));
    assert_eq!(
        removed.outcome,
        WorkspaceCommandOutcome::Removed("/home/example/project".to_string())
    );
    assert_eq!(removed.message().kind, WorkspaceCommandMessageKind::Removed);
    assert!(removed.dirs.is_empty());
    assert!(removed.changed);

    let out_of_range = apply_workspace_command_to_path(&path, WorkspaceCommand::RemoveIndex(9));
    assert_eq!(
        out_of_range.outcome,
        WorkspaceCommandOutcome::IndexOutOfRange
    );
    assert_eq!(
        out_of_range.message().kind,
        WorkspaceCommandMessageKind::SelectionInvalid
    );
    assert!(!out_of_range.changed);

    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn workspace_command_message_is_ui_neutral() {
    let report = WorkspaceCommandReport {
        outcome: WorkspaceCommandOutcome::SaveFailed {
            attempted_change: WorkspaceChange::Added("/tmp/project".to_string()),
            error: "disk full".to_string(),
        },
        dirs: vec![],
        changed: false,
    };

    let message = report.message();
    assert_eq!(message.kind, WorkspaceCommandMessageKind::SaveFailed);
    assert_eq!(message.level, HostStatusLevel::Error);
    assert_eq!(message.error.as_deref(), Some("disk full"));
    let debug = format!("{message:?}");
    for forbidden in ["\x1b[", "已加入", "workspace：", "▶"] {
        assert!(
            !debug.contains(forbidden),
            "workspace command message must stay UI-neutral and avoid terminal copy {forbidden:?}"
        );
    }
}

#[test]
fn path_normalization_canonicalizes_existing_paths() {
    let dir = std::env::temp_dir().join(format!(
        "timem_workspace_core_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(dir.join("child")).unwrap();
    let nested = dir.join(".").join("child").join("..");

    assert_eq!(
        normalize_workspace_dir(nested.to_str().unwrap(), Path::new("/home/example")),
        dir.canonicalize().unwrap().to_string_lossy().to_string()
    );

    let missing = dir.join("missing").join("path");
    assert_eq!(
        normalize_workspace_dir(missing.to_str().unwrap(), Path::new("/home/example")),
        missing.to_string_lossy().to_string()
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_dirs_load_and_save_json_state() {
    let dir = std::env::temp_dir().join(format!(
        "timem_workspace_store_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let path = dir.join("workspace.json");
    let dirs = vec![
        "/z".to_string(),
        "".to_string(),
        "/a".to_string(),
        "/z".to_string(),
    ];

    save_workspace_dirs_to_path(&path, &dirs).unwrap();
    assert_eq!(
        load_workspace_dirs_from_path(&path),
        vec!["/a".to_string(), "/z".to_string()]
    );

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_dirs_load_empty_on_missing_or_malformed_file() {
    let dir = std::env::temp_dir().join(format!(
        "timem_workspace_malformed_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("missing.json");
    assert!(load_workspace_dirs_from_path(&missing).is_empty());

    let malformed = dir.join("workspace.json");
    std::fs::write(&malformed, "not json").unwrap();
    assert!(load_workspace_dirs_from_path(&malformed).is_empty());

    std::fs::write(&malformed, "{\"dirs\":[\"/b\", 123, \"/a\"]}").unwrap();
    assert_eq!(
        load_workspace_dirs_from_path(&malformed),
        vec!["/a".to_string(), "/b".to_string()]
    );

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn workspace_reference_context_is_core_owned_prompt_context() {
    assert_eq!(workspace_reference_context(&[]), None);
    assert_eq!(
        workspace_reference_context(&[
            "/z".to_string(),
            "".to_string(),
            "/a".to_string(),
            "/z".to_string(),
        ]),
        Some("workspace_dirs (model reference; not a host restriction):\n- /a\n- /z".to_string())
    );
    let context = workspace_reference_context(&["/a".to_string()]).unwrap();
    for forbidden in ["shell restriction", "terminal restriction"] {
        assert!(
            !context.contains(forbidden),
            "core workspace prompt context should stay host-neutral: {context}"
        );
    }
}
