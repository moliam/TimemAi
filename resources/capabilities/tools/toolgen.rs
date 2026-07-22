use crate::response_protocol::ParsedAction;
use crate::{
    ActionExecution, AgentCore, ApprovalRequest, BashApprovalMode, PendingApproval,
    PendingApprovedAction, SessionToolRepo,
};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn execute_action(core: &mut AgentCore, action: &ParsedAction) -> ActionExecution {
    let op = action.input_lower("op");
    if op != "publish" {
        return ActionExecution::Completed(format!(
            "Action result: toolgen\nop: {op}\nerror: unsupported_op"
        ));
    }
    let draft_path = action.input_raw_str("draft_path");
    let repo = core.tool_repo();
    if core.bash_approval_mode == BashApprovalMode::Ask {
        return ActionExecution::NeedsApproval(PendingApproval {
            request: ApprovalRequest {
                approval_id: format!("approval_{}", now_ms()),
                action: "toolgen".to_string(),
                command: format!("validate and publish ToolGen draft: {}", draft_path.trim()),
                reason: "toolgen_self_test_requires_user_approval".to_string(),
                risk: "local_tool_self_test_execution".to_string(),
            },
            approved_action: PendingApprovedAction::ToolgenPublish {
                repo,
                draft_path: draft_path.trim().into(),
            },
            continuation: None,
        });
    }
    ActionExecution::Completed(publish_result(&repo, Path::new(draft_path.trim()), None))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(crate) fn execute_approved_publish(
    repo: &SessionToolRepo,
    draft_path: &Path,
    request: &ApprovalRequest,
) -> String {
    publish_result(repo, draft_path, Some(request))
}

fn publish_result(
    repo: &SessionToolRepo,
    draft_path: &Path,
    approval: Option<&ApprovalRequest>,
) -> String {
    let approval_suffix = approval
        .map(|request| {
            format!(
                "\napproval_id: {}\napproval_status: approved_by_user",
                request.approval_id
            )
        })
        .unwrap_or_default();
    match repo.publish(draft_path) {
        Ok(result) => format!(
            "Action result: toolgen\nop: publish\nstatus: ready\ntool_id: {}\nname: {}\npath: {}\nupdated_existing: {}\nvalidation_output:\n{}{}",
            result.summary.tool_id,
            result.summary.name,
            result.summary.path,
            result.updated_existing,
            if result.validation_output.trim().is_empty() {
                "(self-test passed without output)"
            } else {
                result.validation_output.trim()
            },
            approval_suffix,
        ),
        Err(error) => format!(
            "Action result: toolgen\nop: publish\nstatus: validation_failed\ndraft_path: {}\nerror: {}\nThe draft remains unpublished. Correct it before retrying.{}",
            draft_path.display(),
            error,
            approval_suffix,
        ),
    }
}

#[cfg(test)]
#[path = "../../../agent_core/tests/unit/capability_tool_toolgen_tests.rs"]
mod tests;
