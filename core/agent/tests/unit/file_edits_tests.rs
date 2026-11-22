use super::*;
use std::fs;
use std::sync::atomic::{AtomicU32, Ordering};

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "file_edits_tests_{}_{}_{}",
        tag,
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn write_file(path: &std::path::Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, content).expect("write file");
}

fn lines(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn ops_tags(ops: &[DiffOp]) -> Vec<&'static str> {
    ops.iter()
        .map(|op| match op {
            DiffOp::Keep => "keep",
            DiffOp::Delete(_) => "del",
            DiffOp::Insert(_) => "ins",
        })
        .collect()
}

#[test]
fn split_diff_lines_handles_trailing_newline_and_cr() {
    assert_eq!(split_diff_lines("a\r\nb\n"), lines(&["a", "b"]));
    assert_eq!(split_diff_lines("a\nb"), lines(&["a", "b"]));
    assert_eq!(split_diff_lines(""), Vec::<String>::new());
    assert_eq!(split_diff_lines("\n"), lines(&[""]));
}

#[test]
fn diff_lines_covers_insert_delete_and_equal() {
    let old = lines(&["a", "b", "c"]);
    let new = lines(&["a", "b", "c", "d"]);
    assert_eq!(
        ops_tags(&diff_lines(&old, &new)),
        vec!["keep", "keep", "keep", "ins"]
    );
    let new = lines(&["a", "c"]);
    assert_eq!(
        ops_tags(&diff_lines(&old, &new)),
        vec!["keep", "del", "keep"]
    );
    assert_eq!(
        ops_tags(&diff_lines(&old, &old)),
        vec!["keep", "keep", "keep"]
    );
    assert_eq!(ops_tags(&diff_lines(&[], &new)), vec!["ins", "ins"]);
}

#[test]
fn diff_lines_replaces_isolated_lines_without_noise() {
    let old = lines(&["a", "b", "c"]);
    let new = lines(&["a", "X", "c"]);
    let ops = diff_lines(&old, &new);
    assert_eq!(ops_tags(&ops), vec!["keep", "del", "ins", "keep"]);
}

#[test]
fn patience_lis_selects_the_longest_increasing_anchor_chain() {
    let pairs = vec![(0usize, 0usize), (1, 2), (2, 3), (3, 1)];
    let selected = patience_lis(&pairs);
    assert_eq!(selected, vec![(0, 0), (1, 2), (2, 3)]);
}

#[test]
fn patience_diff_ignores_duplicate_lines_when_anchoring() {
    let old = lines(&["a", "dup", "b", "dup", "c"]);
    let new = lines(&["a", "d1", "b", "d2", "c"]);
    let ops = diff_lines(&old, &new);
    let added = ops
        .iter()
        .filter(|op| matches!(op, DiffOp::Insert(_)))
        .count();
    let removed = ops
        .iter()
        .filter(|op| matches!(op, DiffOp::Delete(_)))
        .count();
    assert_eq!((added, removed), (2, 2));
}

#[test]
fn huge_unaligned_middle_falls_back_to_whole_replace() {
    let old: Vec<String> = (0..6000).map(|index| format!("old-{index}")).collect();
    let new: Vec<String> = (0..6000).map(|index| format!("new-{index}")).collect();
    let ops = diff_lines(&old, &new);
    assert_eq!(ops.len(), 12000);
    assert_eq!(ops[0], DiffOp::Delete(0));
    assert_eq!(ops[5999], DiffOp::Delete(5999));
    assert_eq!(ops[6000], DiffOp::Insert(0));
}

#[test]
fn group_hunks_merges_close_changes_and_numbers_ranges() {
    let old = lines(&["a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k"]);
    let new = lines(&["a", "B", "c", "d", "e", "f", "g", "H", "i", "j", "k"]);
    let ops = diff_lines(&old, &new);
    let hunks = group_hunks(&old, &new, &ops);
    // Changes at lines 2 and 8 sit within 2*context lines, so they merge
    // into a single hunk spanning the whole 11-line file.
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].a_start, 1);
    assert_eq!(hunks[0].a_len, 11);
    assert_eq!(hunks[0].b_start, 1);
    assert_eq!(hunks[0].b_len, 11);
    let rendered = render_hunk(&hunks[0]);
    assert!(rendered.starts_with("@@ -1,11 +1,11 @@\n"));
    assert!(rendered.contains(" a\n"));
    assert!(rendered.contains("-b\n"));
    assert!(rendered.contains("+B\n"));
}

#[test]
fn group_hunks_splits_far_apart_changes() {
    let old: Vec<String> = (0..40).map(|index| format!("line-{index}")).collect();
    let mut new = old.clone();
    new[0] = "changed-0".to_string();
    new[39] = "changed-39".to_string();
    let ops = diff_lines(&old, &new);
    let hunks = group_hunks(&old, &new, &ops);
    assert_eq!(hunks.len(), 2);
}

#[test]
fn created_file_uses_dev_null_header_and_zero_range() {
    let mut entry = FileEditEntry {
        path: "notes/a.txt".to_string(),
        op: FileEditOp::Created,
        diff: None,
        added_lines: 0,
        removed_lines: 0,
        truncated: false,
        skip_reason: None,
    };
    entry.extend_from_diff("notes/a.txt", &[], &lines(&["x", "y"]));
    let diff = entry.diff.expect("diff text");
    assert!(diff.starts_with("--- /dev/null\n+++ b/notes/a.txt\n"));
    assert!(diff.contains("@@ -0,0 +1,2 @@\n+x\n+y\n"));
    assert_eq!((entry.added_lines, entry.removed_lines), (2, 0));
    assert!(!entry.truncated);
}

#[test]
fn deleted_file_uses_dev_null_target() {
    let mut entry = FileEditEntry {
        path: "gone.txt".to_string(),
        op: FileEditOp::Deleted,
        diff: None,
        added_lines: 0,
        removed_lines: 0,
        truncated: false,
        skip_reason: None,
    };
    entry.extend_from_diff("gone.txt", &lines(&["x", "y"]), &[]);
    let diff = entry.diff.expect("diff text");
    assert!(diff.starts_with("--- a/gone.txt\n+++ /dev/null\n"));
    assert!(diff.contains("@@ -1,2 +0,0 @@\n-x\n-y\n"));
    assert_eq!((entry.added_lines, entry.removed_lines), (0, 2));
}

#[test]
fn oversized_diff_text_is_truncated_but_stats_stay_full() {
    let old: Vec<String> = (0..300)
        .map(|index| format!("{:0>3} {}", index, "x".repeat(200)))
        .collect();
    let mut new = old.clone();
    for index in (0..300).step_by(10) {
        new[index] = format!("changed {index}");
    }
    let mut entry = FileEditEntry {
        path: "big.txt".to_string(),
        op: FileEditOp::Modified,
        diff: None,
        added_lines: 0,
        removed_lines: 0,
        truncated: false,
        skip_reason: None,
    };
    entry.extend_from_diff("big.txt", &old, &new);
    assert!(entry.truncated);
    assert_eq!((entry.added_lines, entry.removed_lines), (30, 30));
    let diff = entry.diff.expect("diff");
    assert!(diff.len() <= 32 * 1024 + 4096);
}

#[test]
fn read_snapshot_classifies_missing_binary_and_oversized() {
    let dir = temp_dir("classify");
    let missing = dir.join("missing.txt");
    let (snapshot, _) = read_snapshot(&missing, None);
    assert_eq!(snapshot, FileContentSnapshot::Missing);

    let binary = dir.join("blob.bin");
    fs::write(&binary, [b'a', 0, b'b']).unwrap();
    let (snapshot, _) = read_snapshot(&binary, None);
    assert_eq!(snapshot, FileContentSnapshot::Binary);

    let invalid_utf8 = dir.join("latin.bin");
    fs::write(&invalid_utf8, [0xC3, 0x28]).unwrap();
    let (snapshot, _) = read_snapshot(&invalid_utf8, None);
    assert_eq!(snapshot, FileContentSnapshot::Binary);

    let (snapshot, _) = read_snapshot(&dir, None);
    assert_eq!(snapshot, FileContentSnapshot::Unreadable);

    let big = dir.join("big.txt");
    fs::write(&big, vec![b'x'; 512 * 1024 + 1]).unwrap();
    let (snapshot, _) = read_snapshot(&big, None);
    assert_eq!(snapshot, FileContentSnapshot::TooLarge);

    let empty = dir.join("empty.txt");
    fs::write(&empty, "").unwrap();
    let (snapshot, _) = read_snapshot(&empty, None);
    assert_eq!(snapshot, FileContentSnapshot::Lines(Vec::new()));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn capture_resolves_relative_paths_and_deduplicates() {
    let dir = temp_dir("resolve");
    write_file(&dir.join("src/a.txt"), "one\n");
    let context = EditContext::capture(
        &dir,
        &[
            "src/a.txt".to_string(),
            "src/a.txt".to_string(),
            "".to_string(),
        ],
    );
    assert_eq!(context.files.len(), 1);
    assert_eq!(context.files[0].declared, "src/a.txt");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn capture_caps_file_count_and_reports_not_captured() {
    let dir = temp_dir("capcount");
    for index in 0..(MAX_EDIT_FILES as i32 + 2) {
        write_file(&dir.join(format!("f{index}.txt")), "x\n");
    }
    let declared: Vec<String> = (0..MAX_EDIT_FILES + 2)
        .map(|index| format!("f{index}.txt"))
        .collect();
    let context = EditContext::capture(&dir, &declared);
    assert_eq!(context.files.len(), MAX_EDIT_FILES + 2);
    let evidence = context.compute_evidence();
    assert_eq!(evidence.edits.len(), MAX_EDIT_FILES + 2);
    let overflow = &evidence.edits[MAX_EDIT_FILES..];
    assert!(overflow
        .iter()
        .all(|entry| entry.op == FileEditOp::Skipped && entry.skip_reason == Some("not_captured")));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compute_evidence_tracks_create_modify_delete_and_unchanged() {
    let dir = temp_dir("evidence");
    write_file(&dir.join("modified.txt"), "a\nb\n");
    write_file(&dir.join("deleted.txt"), "gone\n");
    write_file(&dir.join("same.txt"), "same\n");
    let context = EditContext::capture(
        &dir,
        &[
            "created.txt".to_string(),
            "modified.txt".to_string(),
            "deleted.txt".to_string(),
            "same.txt".to_string(),
            "never.txt".to_string(),
        ],
    );
    write_file(&dir.join("created.txt"), "hello\nworld\n");
    write_file(&dir.join("modified.txt"), "a\nB\n");
    fs::remove_file(dir.join("deleted.txt")).unwrap();
    let evidence = context.compute_evidence();
    let mut by_path = evidence.edits.iter();
    let created = by_path.next().unwrap();
    assert_eq!(created.op, FileEditOp::Created);
    assert_eq!((created.added_lines, created.removed_lines), (2, 0));
    let modified = by_path.next().unwrap();
    assert_eq!(modified.op, FileEditOp::Modified);
    assert_eq!((modified.added_lines, modified.removed_lines), (1, 1));
    assert!(modified.diff.as_deref().expect("diff").contains("+B\n"));
    let deleted = by_path.next().unwrap();
    assert_eq!(deleted.op, FileEditOp::Deleted);
    let same = by_path.next().unwrap();
    assert_eq!(same.op, FileEditOp::Unchanged);
    assert!(same.diff.is_none());
    let never = by_path.next().unwrap();
    assert_eq!(never.op, FileEditOp::Unchanged);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compute_evidence_skips_binary_and_oversized_after_states() {
    let dir = temp_dir("skip");
    write_file(&dir.join("was_text.bin"), "text\n");
    let context = EditContext::capture(&dir, &["was_text.bin".to_string()]);
    fs::write(dir.join("was_text.bin"), [0u8, 1, 2]).unwrap();
    let evidence = context.compute_evidence();
    assert_eq!(evidence.edits.len(), 1);
    assert_eq!(evidence.edits[0].op, FileEditOp::Skipped);
    assert_eq!(evidence.edits[0].skip_reason, Some("binary"));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn compute_evidence_skips_unreadable_files() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("unreadable");
    let locked = dir.join("locked.txt");
    write_file(&locked, "secret\n");
    let context = EditContext::capture(&dir, &["locked.txt".to_string()]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let evidence = context.compute_evidence();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(evidence.edits[0].op, FileEditOp::Skipped);
    assert_eq!(evidence.edits[0].skip_reason, Some("unreadable"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn payload_budget_degrades_large_sets_without_losing_paths() {
    let dir = temp_dir("budget");
    let mut declared = Vec::new();
    for file in 0..6 {
        let name = format!("f{file}.txt");
        let old: Vec<String> = (0..400)
            .map(|index| format!("{file}-{index} {}", "y".repeat(48)))
            .collect();
        let mut new = old.clone();
        for index in (0..400).step_by(7) {
            new[index] = format!("CHANGED {} {index}", &name[..2]);
        }
        write_file(&dir.join(&name), &old.join("\n"));
        let context = EditContext::capture(&dir, std::slice::from_ref(&name));
        write_file(&dir.join(&name), &new.join("\n"));
        let evidence = context.compute_evidence();
        declared.push(name);
        assert!(
            evidence.edits[0].diff.as_deref().expect("diff").len() > 20_000,
            "diff should stay near the per-file cap for budget pressure"
        );
    }
    let mut entries = Vec::new();
    for name in &declared {
        let old: Vec<String> = (0..400)
            .map(|index| format!("{}-{index} {}", &name[..2], "y".repeat(48)))
            .collect();
        let mut new = old.clone();
        for index in (0..400).step_by(7) {
            new[index] = format!("CHANGED {} {index}", &name[..2]);
        }
        let mut entry = FileEditEntry {
            path: name.clone(),
            op: FileEditOp::Modified,
            diff: None,
            added_lines: 0,
            removed_lines: 0,
            truncated: false,
            skip_reason: None,
        };
        entry.extend_from_diff(name, &old, &new);
        entries.push(entry);
    }
    let bounded = apply_payload_budget(entries);
    let payload = edits_payload(&FileEditsEvidence { edits: bounded });
    let serialized = payload.to_string();
    assert!(serialized.len() <= 96 * 1024);
    // Overflowing entries degrade to slim (stats kept, diff dropped) or
    // minimal budget_exhausted placeholders; both keep the path visible.
    assert!(serialized.contains("budget_exhausted") || serialized.contains("\"diff\":null"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn edits_payload_shape_is_stable_for_interfaces() {
    let entry = FileEditEntry {
        path: "p.txt".to_string(),
        op: FileEditOp::Modified,
        diff: Some("--- a/p.txt\n+++ b/p.txt\n".to_string()),
        added_lines: 1,
        removed_lines: 2,
        truncated: true,
        skip_reason: None,
    };
    let payload = edits_payload(&FileEditsEvidence { edits: vec![entry] });
    let object = payload.as_array().expect("array")[0]
        .as_object()
        .expect("object")
        .clone();
    for key in [
        "path",
        "op",
        "diff",
        "added_lines",
        "removed_lines",
        "truncated",
        "skip_reason",
    ] {
        assert!(object.contains_key(key), "missing {key}");
    }
    assert_eq!(object["op"], "modified");
    assert_eq!(object["skip_reason"], Value::Null);
}

#[test]
fn empty_context_has_no_evidence() {
    let context = EditContext::default();
    assert!(context.is_empty());
    let evidence = context.compute_evidence();
    assert!(evidence.edits.is_empty());
    assert_eq!(edits_payload(&evidence), Value::Array(Vec::new()));
}
