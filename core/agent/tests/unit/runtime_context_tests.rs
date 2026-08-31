use super::*;

#[test]
fn formats_runtime_time_context_with_bilingual_weekday() {
    let context = format_runtime_time_context(LocalTimeParts {
        year: 2026,
        month: 7,
        day: 4,
        hour: 9,
        minute: 8,
        second: 7,
        weekday: 6,
    });

    assert_eq!(
        context,
        "2026-07-04 09:08:07 local_time, weekday=周六/Saturday"
    );
}

#[test]
fn weekday_labels_handle_unknown_values() {
    assert_eq!(weekday_zh(9), "未知");
    assert_eq!(weekday_en(9), "Unknown");
}
