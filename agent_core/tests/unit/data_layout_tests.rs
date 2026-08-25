use super::*;

fn unique_test_root(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("timem_{label}_{}_{}", std::process::id(), nanos))
}

#[test]
fn builds_runtime_data_layout_paths() {
    let layout = RuntimeDataLayout::new("/tmp/timem-data", ".test_mem");

    assert_eq!(layout.data_root(), Path::new("/tmp/timem-data"));
    assert_eq!(layout.space(), ".test_mem");
    assert_eq!(
        layout.space_dir(),
        PathBuf::from("/tmp/timem-data/.test_mem")
    );
    assert_eq!(
        layout.memory_dir(),
        PathBuf::from("/tmp/timem-data/.test_mem/memory")
    );
    assert_eq!(
        layout.api_audit_file(),
        PathBuf::from("/tmp/timem-data/.test_mem/audit/api_audit.json")
    );
    assert_eq!(
        layout.action_audit_file(),
        PathBuf::from("/tmp/timem-data/.test_mem/audit/action_audit.json")
    );
    assert_eq!(
        layout.workspace_config_file(),
        PathBuf::from("/tmp/timem-data/workspace.json")
    );
}

#[test]
fn new_environment_uses_hidden_data_root() {
    let root = unique_test_root("hidden_data_root");
    std::fs::create_dir_all(&root).unwrap();

    assert_eq!(
        default_unconfigured_data_root(&root),
        PathBuf::from(".timem_data")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn existing_legacy_data_root_is_preserved_until_hidden_root_exists() {
    let root = unique_test_root("legacy_data_root");
    std::fs::create_dir_all(root.join("data")).unwrap();
    assert_eq!(
        default_unconfigured_data_root(&root),
        PathBuf::from(".timem_data"),
        "an unrelated directory named data is not a Timem legacy root"
    );

    std::fs::write(
        root.join("data/workspace.json"),
        r#"{"dirs":["/tmp/project"]}"#,
    )
    .unwrap();
    assert_eq!(default_unconfigured_data_root(&root), PathBuf::from("data"));

    std::fs::create_dir_all(root.join(".timem_data")).unwrap();
    assert_eq!(
        default_unconfigured_data_root(&root),
        PathBuf::from(".timem_data")
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_session_layout_is_a_timem_fingerprint() {
    let root = unique_test_root("legacy_data_fingerprint");
    let legacy = root.join("data/.project/memory/sessions");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("index.jsonl"), "").unwrap();
    assert_eq!(default_unconfigured_data_root(&root), PathBuf::from("data"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn direct_memory_layout_does_not_append_memory_directory() {
    let layout = RuntimeDataLayout::from_memory_dir("/tmp/timem-data", "/tmp/custom Timem MEM");

    assert_eq!(layout.space(), "/tmp/custom Timem MEM");
    assert_eq!(layout.space_dir(), PathBuf::from("/tmp/custom Timem MEM"));
    assert_eq!(layout.memory_dir(), PathBuf::from("/tmp/custom Timem MEM"));
    assert_eq!(
        layout.api_audit_file(),
        PathBuf::from("/tmp/custom Timem MEM/audit/api_audit.json")
    );
    assert_eq!(
        layout.action_audit_file(),
        PathBuf::from("/tmp/custom Timem MEM/audit/action_audit.json")
    );
    assert_eq!(
        layout.workspace_config_file(),
        PathBuf::from("/tmp/timem-data/workspace.json")
    );
}

#[test]
fn default_memory_directory_is_under_home() {
    assert_eq!(
        default_memory_dir_from_home(Some(std::ffi::OsStr::new("/tmp/test-home"))).unwrap(),
        PathBuf::from("/tmp/test-home/.timem/mem")
    );
    assert_eq!(
        default_memory_dir_from_home(None).unwrap_err(),
        "home_directory_unavailable"
    );
}

#[test]
fn space_resolves_only_absolute_memory_paths() {
    assert_eq!(
        resolve_memory_dir(Some("/tmp/custom-mem")).unwrap(),
        PathBuf::from("/tmp/custom-mem")
    );
    assert_eq!(
        resolve_memory_dir(Some("relative-mem")).unwrap_err(),
        "space_must_be_absolute_path"
    );
    assert_eq!(resolve_memory_dir(Some("")).unwrap_err(), "mem_path_empty");
}

#[test]
fn memory_directory_is_created_recursively() {
    let root = unique_test_root("create_memory_dir");
    let memory_dir = root.join("nested/mem");

    create_memory_dir(&memory_dir).unwrap();
    assert!(memory_dir.is_dir());

    let _ = std::fs::remove_dir_all(root);
}
