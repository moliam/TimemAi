use super::*;
use crate::CoreProfile;
use serde_json::json;
use std::fs;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn publish_action_returns_ready_only_after_runtime_self_test() {
    let root = std::env::temp_dir().join(format!(
        "timem_toolgen_action_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            name: "test".to_string(),
            provider: "test".to_string(),
            model: "test".to_string(),
        },
        &root,
    );
    core.set_bash_approval_mode(BashApprovalMode::Approve);
    core.set_tool_repo_session_id("session-action");
    let draft = core.tool_repo().create_draft().unwrap();
    fs::write(
        draft.join("README.md"),
        "# inspect-log\n\n`inspect-log --self-test`\n",
    )
    .unwrap();
    fs::write(draft.join("tool.sh"), "#!/bin/bash\necho verified\n").unwrap();
    fs::write(
        draft.join(".timem-tool.json"),
        serde_json::to_string_pretty(&json!({
            "name": "inspect-log",
            "type": "debug",
            "language": "bash",
            "entrypoint": "tool.sh",
            "synopsis": "inspect-log <file>",
            "self_test": {"args": ["--self-test"], "timeout_ms": 2000}
        }))
        .unwrap(),
    )
    .unwrap();
    let action = ParsedAction {
        action: "toolgen".to_string(),
        raw_input: json!({"op":"publish", "draft_path":draft}),
    };
    let ActionExecution::Completed(result) = execute_action(&mut core, &action) else {
        panic!("automatic approval mode must execute ToolGen directly");
    };
    assert!(result.contains("status: ready"));
    assert!(result.contains("validation_output:\nverified"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn publish_action_uses_structured_approval_before_executing_self_test() {
    let root = std::env::temp_dir().join(format!(
        "timem_toolgen_approval_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut core = AgentCore::new(
        "prompt",
        CoreProfile {
            name: "test".into(),
            provider: "test".into(),
            model: "test".into(),
        },
        &root,
    );
    core.set_tool_repo_session_id("session-approval");
    let draft = core.tool_repo().create_draft().unwrap();
    let action = ParsedAction {
        action: "toolgen".into(),
        raw_input: json!({"op":"publish", "draft_path":draft}),
    };
    let ActionExecution::NeedsApproval(pending) = execute_action(&mut core, &action) else {
        panic!("ask mode must request approval");
    };
    assert_eq!(pending.request.action, "toolgen");
    assert_eq!(pending.request.risk, "local_tool_self_test_execution");
    assert!(!draft.join(".timem-tool.json").exists());
    let _ = fs::remove_dir_all(root);
}
