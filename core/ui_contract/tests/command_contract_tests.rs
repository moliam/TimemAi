use timem_ui_contract::commands::ToolGenRequest;

#[test]
fn toolgen_request_discards_only_empty_or_whitespace_guidance() {
    assert_eq!(ToolGenRequest::new(None).user_instruction, None);
    assert_eq!(
        ToolGenRequest::new(Some("   ".to_string())).user_instruction,
        None
    );
    assert_eq!(
        ToolGenRequest::new(Some("  keep context  ".to_string())).user_instruction,
        Some("  keep context  ".to_string())
    );
}
