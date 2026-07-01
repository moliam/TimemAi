use crate::capability::CapabilityRegistry;

#[derive(Debug, Clone, Copy)]
pub struct CapmgrActionInput<'a> {
    pub op: &'a str,
    pub kind: &'a str,
    pub id: &'a str,
}

pub fn execute(registry: &CapabilityRegistry, input: CapmgrActionInput<'_>) -> String {
    match input.op {
        "list" => registry.list_text(input.kind.trim()),
        "load" | "inspect" => registry.load_text(input.kind.trim(), input.id),
        other => format!("Action result: capmgr\nop: {other}\nerror: unsupported_op"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capmgr_lists_skill_headers() {
        let registry = CapabilityRegistry::builtin();
        let result = execute(
            &registry,
            CapmgrActionInput {
                op: "list",
                kind: "skill",
                id: "",
            },
        );

        assert!(result.contains("Action result: capmgr"));
        assert!(result.contains("release_quality_gate"));
    }

    #[test]
    fn capmgr_loads_skill_body() {
        let registry = CapabilityRegistry::builtin();
        let result = execute(
            &registry,
            CapmgrActionInput {
                op: "load",
                kind: "skill",
                id: "release_quality_gate",
            },
        );

        assert!(result.contains("Action result: capmgr"));
        assert!(result.contains("Release Quality Gate"));
        assert!(result.contains("body:"));
    }

    #[test]
    fn capmgr_reports_unsupported_operation() {
        let registry = CapabilityRegistry::builtin();
        let result = execute(
            &registry,
            CapmgrActionInput {
                op: "remove",
                kind: "skill",
                id: "release_quality_gate",
            },
        );

        assert!(result.contains("error: unsupported_op"));
    }
}
