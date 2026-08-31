/// Request to run ToolGen for the current completed task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolGenRequest {
    pub user_instruction: Option<String>,
}

impl ToolGenRequest {
    pub fn new(user_instruction: Option<String>) -> Self {
        Self {
            user_instruction: user_instruction.filter(|value| !value.trim().is_empty()),
        }
    }
}
