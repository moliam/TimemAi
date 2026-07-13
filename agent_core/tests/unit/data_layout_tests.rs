use super::*;

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
