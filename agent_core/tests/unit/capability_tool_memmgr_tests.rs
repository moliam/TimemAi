use super::*;

#[test]
fn scratch_kind_aliases_are_normalized() {
    assert_eq!(normalize_scratch_kind("note"), "notes");
    assert_eq!(normalize_scratch_kind("custom"), "custom");
}
