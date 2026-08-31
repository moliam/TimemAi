use timem_ui_contract::preferences::{AssistantResponseFormat, InterfacePreferences};

#[test]
fn interface_preferences_have_stable_wire_values_and_backward_compatible_defaults() {
    assert_eq!(
        serde_json::to_string(&AssistantResponseFormat::Markdown).unwrap(),
        r#""markdown""#
    );
    assert_eq!(
        serde_json::to_string(&AssistantResponseFormat::PlainText).unwrap(),
        r#""plain_text""#
    );
    assert_eq!(
        serde_json::from_str::<InterfacePreferences>("{}").unwrap(),
        InterfacePreferences::default()
    );
    assert_eq!(
        serde_json::from_str::<InterfacePreferences>(r#"{"assistant_response_format":"markdown"}"#)
            .unwrap(),
        InterfacePreferences::markdown()
    );
}
