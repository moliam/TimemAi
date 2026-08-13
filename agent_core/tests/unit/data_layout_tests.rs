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
