use super::*;

#[test]
fn head_retention_is_utf8_safe_and_respects_the_complete_budget() {
    let result = fit(
        &format!("HEAD{}TAIL", "你".repeat(100)),
        96,
        Retention::Head,
    );
    assert!(result.starts_with("HEAD"));
    assert!(result.contains("words truncated."));
    assert!(result.len() <= 96);
}

#[test]
fn tail_retention_is_utf8_safe_and_respects_the_complete_budget() {
    let result = fit(
        &format!("HEAD{}TAIL", "你".repeat(100)),
        96,
        Retention::Tail,
    );
    assert!(result.ends_with("TAIL"));
    assert!(result.contains("truncated before"));
    assert!(result.len() <= 96);
}

#[test]
fn exact_boundary_is_not_modified() {
    assert_eq!(fit("1234", 4, Retention::Tail), "1234");
}
