use super::*;

#[test]
fn self_tool_reads_and_writes_non_sensitive_env() {
    let mut tool = test_state();

    let write = tool.execute(SelfToolInput {
        self_type: "env",
        op: "write",
        key: "TIMEM_TEST_FLAG",
        value: "enabled",
        new_path: "",
    });
    assert!(write.contains("status: updated_current_process_env"));

    let read = tool.execute(SelfToolInput {
        self_type: "env",
        op: "read",
        key: "TIMEM_TEST_FLAG",
        value: "",
        new_path: "",
    });
    assert!(read.contains("value: enabled"));
}

#[test]
fn self_tool_denies_api_key_env_access() {
    let mut tool = test_state();

    let read = tool.execute(SelfToolInput {
        self_type: "env",
        op: "read",
        key: "TIMEM_API_KEY",
        value: "",
        new_path: "",
    });
    assert!(read.contains("error: sensitive_env_denied"));

    let write = tool.execute(SelfToolInput {
        self_type: "env",
        op: "write",
        key: "AWS_SECRET_ACCESS_KEY",
        value: "secret",
        new_path: "",
    });
    assert!(write.contains("error: sensitive_env_denied"));
    assert!(!write.contains("secret"));
}

#[test]
fn self_tool_denies_memory_path_env_writes() {
    let mut tool = test_state();

    for key in ["TIMEM_DATA_DIR", "TIMEM_SPACE", "TIMEM_MEMORY_DIR"] {
        let write = tool.execute(SelfToolInput {
            self_type: "env",
            op: "write",
            key,
            value: "/tmp/should-not-leak",
            new_path: "",
        });
        assert!(write.contains("error: protected_env_denied"));
        assert!(write.contains("reason: memory_path_env_is_startup_only"));
        assert!(!write.contains("updated_current_process_env"));
        assert!(!write.contains("/tmp/should-not-leak"));
    }

    let read = tool.execute(SelfToolInput {
        self_type: "env",
        op: "read",
        key: "TIMEM_SPACE",
        value: "",
        new_path: "",
    });
    assert!(read.contains("value: .test_mem"));
}

#[test]
fn self_tool_lists_paths_and_about_info() {
    let mut tool = test_state();

    let paths = tool.execute(SelfToolInput {
        self_type: "mem_path",
        op: "read",
        key: "",
        value: "",
        new_path: "",
    });
    assert!(paths.contains("memory_file: /tmp/timem/memory/memory.jsonl"));
    assert!(paths.contains("api_audit_file: /tmp/timem/audit/api_audit.json"));

    let about = tool.execute(SelfToolInput {
        self_type: "about_me",
        op: "read",
        key: "",
        value: "",
        new_path: "",
    });
    assert!(about.contains("name: TimemAi"));
    assert!(about.contains("author: TimemAi <phylimo@163.com>"));
    assert!(about.contains("project: https://github.com/moliam/TimemAi"));
    assert!(about.contains("star_message: Please star https://github.com/moliam/TimemAi"));
    assert!(about.contains("pid: 12345"));
    assert!(about.contains("current_dir: /tmp/timem/project"));
    assert!(about.contains("executable: /tmp/timem/bin/timem"));
}

fn test_state() -> SelfToolState {
    let mut env = BTreeMap::new();
    env.insert("TIMEM_SPACE".to_string(), ".test_mem".to_string());
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
