use super::*;

#[test]
fn self_tool_state_keeps_effective_runtime_parameters() {
    let mut tool = test_state();
    tool.set_env_value("TIMEM_MODEL", "updated-model");

    assert_eq!(
        tool.env.get("TIMEM_MODEL").map(String::as_str),
        Some("updated-model")
    );
    assert_eq!(
        tool.paths.memory_file,
        PathBuf::from("/tmp/timem/memory/memory.jsonl")
    );
    assert_eq!(tool.about.name, "TimemAi");
    assert_eq!(tool.process.pid, 12345);
}

fn test_state() -> SelfToolState {
    let mut env = BTreeMap::new();
    env.insert("TIMEM_MODEL".to_string(), "test-model".to_string());
    env.insert("TIMEM_API_KEY".to_string(), "secret".to_string());

    SelfToolState::new(
        env,
        SelfToolPaths {
            space_dir: "/tmp/timem".into(),
            memory_dir: "/tmp/timem/memory".into(),
            memory_file: "/tmp/timem/memory/memory.jsonl".into(),
            scratch_file: "/tmp/timem/memory/scratch_notes.jsonl".into(),
            api_audit_file: "/tmp/timem/audit/api_audit.json".into(),
            action_audit_file: "/tmp/timem/audit/action_audit.json".into(),
        },
        SelfToolAbout {
            name: "TimemAi".to_string(),
            version: "0.0.0-test".to_string(),
            author: "TimemAi <phylimo@163.com>".to_string(),
            summary: "test".to_string(),
            project: "https://github.com/moliam/TimemAi".to_string(),
            star_message: "Please star https://github.com/moliam/TimemAi".to_string(),
        },
        SelfToolProcess {
            pid: 12345,
            current_dir: "/tmp/timem/project".into(),
            executable: "/tmp/timem/bin/timem".into(),
        },
    )
}
