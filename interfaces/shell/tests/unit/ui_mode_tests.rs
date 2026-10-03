use super::*;

fn strip_ansi(text: &str) -> String {
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            output.push(ch);
        }
    }
    output
}

#[test]
fn ui_mode_parser_is_strict_and_case_insensitive() {
    assert_eq!(
        ShellUiMode::parse(" ordinary ").unwrap(),
        ShellUiMode::Ordinary
    );
    assert_eq!(ShellUiMode::parse("STREAM").unwrap(), ShellUiMode::Stream);
    assert!(ShellUiMode::parse("streaming")
        .unwrap_err()
        .contains("ordinary|stream"));
    assert!(ShellUiMode::parse("").is_err());
}

#[test]
fn resolution_obeys_cli_env_tty_and_once_json_precedence() {
    assert_eq!(
        resolve_ui_mode(Some("ordinary"), Some("stream"), true, false).unwrap(),
        UiModeResolution::Resolved(ShellUiMode::Ordinary)
    );
    assert_eq!(
        resolve_ui_mode(None, Some("stream"), true, false).unwrap(),
        UiModeResolution::Resolved(ShellUiMode::Stream)
    );
    assert_eq!(
        resolve_ui_mode(None, Some("  "), true, false).unwrap(),
        UiModeResolution::Select
    );
    assert_eq!(
        resolve_ui_mode(None, None, false, false).unwrap(),
        UiModeResolution::Resolved(ShellUiMode::Ordinary)
    );
    assert_eq!(
        resolve_ui_mode(Some("stream"), Some("stream"), true, true).unwrap(),
        UiModeResolution::Resolved(ShellUiMode::Ordinary)
    );
}

#[test]
fn invalid_explicit_or_environment_value_is_not_silently_defaulted() {
    assert!(resolve_ui_mode(Some("bad"), None, true, false).is_err());
    assert!(resolve_ui_mode(None, Some("bad"), false, false).is_err());
}

#[test]
fn selector_describes_both_modes_and_marks_the_selection() {
    let ordinary = strip_ansi(&ui_mode_selector(ShellUiMode::Ordinary));
    assert!(ordinary.contains("Select session UI"));
    assert!(ordinary.contains("❯ Ordinary integrated"));
    assert!(ordinary.contains("Streaming response"));
    let stream = strip_ansi(&ui_mode_selector(ShellUiMode::Stream));
    assert!(stream.contains("❯ Streaming response"));
}
