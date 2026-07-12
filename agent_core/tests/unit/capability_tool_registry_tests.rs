use super::*;

#[test]
fn builtin_registry_lists_all_compiled_tool_callbacks() {
    for binding in BUILTIN_TOOL_BINDINGS {
        assert!(
            builtin_tool_callback(binding).is_some(),
            "missing builtin callback for {binding}"
        );
    }
    assert!(builtin_tool_callback("ghost_tool").is_none());
}

#[test]
fn builtin_execution_contains_callback_panics() {
    let result = catch_builtin_execution(|| -> ActionExecution {
        panic!("injected builtin failure");
    });

    assert_eq!(result, Err(BuiltinToolFailure));
}
