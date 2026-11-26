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
fn diff_lines_spread_changes_in_large_files_stay_minimal() {
    // Changes far apart in a large file must not degrade to a whole-file
    // replace: unique-line anchors split the middle regardless of its size.
    let old: Vec<String> = (0..7200)
        .map(|index| format!("stable-unique-{index}"))
        .collect();
    let mut new = old.clone();
    new[51] = "replaced-head".to_string();
    new[7134] = "replaced-tail".to_string();
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
    assert_eq!(ops.len(), old.len() + 2);
    assert_eq!(ops[0], DiffOp::Keep);
    assert_eq!(ops[51], DiffOp::Delete(51));
    assert_eq!(ops[52], DiffOp::Insert(51));
    assert_eq!(ops.last(), Some(&DiffOp::Keep));
}

#[test]
fn modified_large_file_diff_stays_bounded_and_untruncated() {
    let old: Vec<String> = (0..7200)
        .map(|index| format!("stable-unique-{index}"))
        .collect();
    let mut new = old.clone();
    new[51] = "replaced-head".to_string();
    new[7134] = "replaced-tail".to_string();
    let entry = build_entry(
        "big.css",
        &FileContentSnapshot::Lines(old.join("\n")),
        &FileContentSnapshot::Lines(new.join("\n")),
        None,
    );
    assert_eq!(entry.op, FileEditOp::Modified);
    assert_eq!((entry.added_lines, entry.removed_lines), (2, 2));
    assert!(!entry.truncated);
    let diff = entry.diff.expect("diff");
    assert_eq!(
        diff.lines().filter(|line| line.starts_with("@@")).count(),
        2
    );
    assert!(diff.contains("+replaced-head"));
    assert!(diff.contains("+replaced-tail"));
}

#[test]
fn build_entry_reports_missing_paths_as_skipped_missing() {
    let entry = build_entry(
        "absent.txt",
        &FileContentSnapshot::Missing,
        &FileContentSnapshot::Missing,
        None,
    );
    assert_eq!(entry.op, FileEditOp::Skipped);
    assert_eq!(entry.skip_reason, Some("missing"));
    assert!(entry.diff.is_none());
    assert!(!entry.truncated);
}

#[test]
fn whitespace_only_rewrites_report_modified_with_meta_note() {
    let entry = build_entry(
        "notes.txt",
        &FileContentSnapshot::Lines("a\nb\n".to_string()),
        &FileContentSnapshot::Lines("a\nb".to_string()),
        None,
    );
    assert_eq!(entry.op, FileEditOp::Modified);
    assert_eq!((entry.added_lines, entry.removed_lines), (0, 0));
    assert!(!entry.truncated);
    let diff = entry.diff.expect("diff");
    assert!(diff.contains("# Whitespace-only change"));
    assert!(!diff.contains("@@"));

    let crlf = build_entry(
        "notes.txt",
        &FileContentSnapshot::Lines("a\nb\n".to_string()),
        &FileContentSnapshot::Lines("a\r\nb\r\n".to_string()),
        None,
    );
    assert_eq!(crlf.op, FileEditOp::Modified);
    assert!(crlf
        .diff
        .expect("diff")
        .contains("# Whitespace-only change"));

    let identical = build_entry(
        "notes.txt",
        &FileContentSnapshot::Lines("a\nb\n".to_string()),
        &FileContentSnapshot::Lines("a\nb\n".to_string()),
        None,
    );
    assert_eq!(identical.op, FileEditOp::Unchanged);
    assert!(identical.diff.is_none());
}

#[test]
fn diff_ops_reconstruct_the_new_side_for_tricky_inputs() {
    let rebuild = |old: &[String], new: &[String]| {
        let ops = diff_lines(old, new);
        let mut out: Vec<String> = Vec::new();
        let mut a_cursor = 0usize;
        let mut b_cursor = 0usize;
        for op in &ops {
            match *op {
                DiffOp::Keep => {
                    out.push(old[a_cursor].clone());
                    a_cursor += 1;
                    b_cursor += 1;
                }
                DiffOp::Delete(_) => a_cursor += 1,
                DiffOp::Insert(index) => {
                    assert_eq!(index, b_cursor, "inserts must stay in order");
                    out.push(new[b_cursor].clone());
                    b_cursor += 1;
                }
            }
        }
        assert_eq!(a_cursor, old.len());
        assert_eq!(b_cursor, new.len());
        out
    };
    let dup_old = lines(&["x", "dup", "x", "dup", "y", "dup"]);
    let dup_new = lines(&["dup", "x", "dup", "z", "dup", "y", "dup"]);
    assert_eq!(rebuild(&dup_old, &dup_new), dup_new);
    let moved_old = lines(&["a", "b", "c", "d", "e"]);
    let moved_new = lines(&["c", "d", "a", "b", "e"]);
    assert_eq!(rebuild(&moved_old, &moved_new), moved_new);
    let del_old = lines(&["k", "k", "k"]);
    let del_new = lines(&["k"]);
    assert_eq!(rebuild(&del_old, &del_new), del_new);
    assert_eq!(rebuild(&del_new, &del_old), del_old);
    let big_old: Vec<String> = (0..7200)
        .map(|index| format!("stable-unique-{index}"))
        .collect();
    let mut big_new = big_old.clone();
    big_new[3] = "head-edit".to_string();
    big_new[7000] = "mid-edit".to_string();
    big_new[7100] = "tail-edit".to_string();
    assert_eq!(rebuild(&big_old, &big_new), big_new);
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

        size_bytes: None,
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

        size_bytes: None,
    };
    entry.extend_from_diff("gone.txt", &lines(&["x", "y"]), &[]);
    let diff = entry.diff.expect("diff text");
    assert!(diff.starts_with("--- a/gone.txt\n+++ /dev/null\n"));
    assert!(diff.contains("@@ -1,2 +0,0 @@\n-x\n-y\n"));
    assert_eq!((entry.added_lines, entry.removed_lines), (0, 2));
}

#[test]
fn oversized_diff_text_is_truncated_but_stats_stay_full() {
    let old: Vec<String> = (0..6000)
        .map(|index| format!("{:0>4} {}", index, "x".repeat(200)))
        .collect();
    // Replace every line so the hunk itself exceeds the 1 MiB per-file cap.
    let new: Vec<String> = (0..6000)
        .map(|index| format!("changed {index} {}", "y".repeat(200)))
        .collect();
    let mut entry = FileEditEntry {
        path: "big.txt".to_string(),
        op: FileEditOp::Modified,
        diff: None,
        added_lines: 0,
        removed_lines: 0,
        truncated: false,
        skip_reason: None,

        size_bytes: None,
    };
    entry.extend_from_diff("big.txt", &old, &new);
    assert!(entry.truncated);
    assert_eq!((entry.added_lines, entry.removed_lines), (6000, 6000));
    let diff = entry.diff.expect("diff");
    assert!(diff.len() <= 1024 * 1024 + 4096);
}

#[test]
fn read_snapshot_classifies_missing_binary_and_oversized() {
    let dir = temp_dir("classify");
    let missing = dir.join("missing.txt");
    let (snapshot, _) = read_snapshot(&missing);
    assert_eq!(snapshot, FileContentSnapshot::Missing);

    let binary = dir.join("blob.bin");
    fs::write(&binary, [b'a', 0, b'b']).unwrap();
    let (snapshot, _) = read_snapshot(&binary);
    assert_eq!(snapshot, FileContentSnapshot::Binary);

    let invalid_utf8 = dir.join("latin.bin");
    fs::write(&invalid_utf8, [0xC3, 0x28]).unwrap();
    let (snapshot, _) = read_snapshot(&invalid_utf8);
    assert_eq!(snapshot, FileContentSnapshot::Binary);

    let (snapshot, _) = read_snapshot(&dir);
    assert_eq!(snapshot, FileContentSnapshot::Directory);

    // Sparse all-zero content sniffs as binary, so the oversized path reports
    // the more specific kind instead of the vaguer "too large".
    let big = dir.join("big.txt");
    let handle = fs::File::create(&big).unwrap();
    handle.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(handle);
    let (snapshot, _) = read_snapshot(&big);
    assert_eq!(snapshot, FileContentSnapshot::Binary);

    let big_text = dir.join("big_text.txt");
    {
        use std::io::Write;
        let mut file = fs::File::create(&big_text).unwrap();
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..64 {
            file.write_all(&chunk).unwrap();
        }
        file.write_all(b"x").unwrap();
    }
    let (snapshot, _) = read_snapshot(&big_text);
    assert_eq!(snapshot, FileContentSnapshot::TooLarge);

    let large_text = dir.join("large_text.txt");
    let content = "x".repeat(700 * 1024) + "\n";
    fs::write(&large_text, &content).unwrap();
    let (snapshot, _) = read_snapshot(&large_text);
    assert_eq!(snapshot, FileContentSnapshot::Lines(content));

    let empty = dir.join("empty.txt");
    fs::write(&empty, "").unwrap();
    let (snapshot, _) = read_snapshot(&empty);
    assert_eq!(snapshot, FileContentSnapshot::Lines(String::new()));
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
    // No baseline, but the paths exist now: a real op plus a "no baseline" tag.
    assert!(
        overflow
            .iter()
            .all(|entry| entry.op == FileEditOp::Modified
                && entry.skip_reason == Some("not_captured"))
    );
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
    // 非文本修改只有元信息可展示；文本修改也带一份最终大小，便于总结行显示。
    assert_eq!(created.size_bytes, Some("hello\nworld\n".len() as u64));
    let modified = by_path.next().unwrap();
    assert_eq!(modified.op, FileEditOp::Modified);
    assert_eq!((modified.added_lines, modified.removed_lines), (1, 1));
    assert!(modified.diff.as_deref().expect("diff").contains("+B\n"));
    let deleted = by_path.next().unwrap();
    assert_eq!(deleted.op, FileEditOp::Deleted);
    // 删除后没有 after 侧，大小取删除前的最后已知值。
    assert!(deleted.size_bytes.is_some_and(|size| size > 0));
    let same = by_path.next().unwrap();
    assert_eq!(same.op, FileEditOp::Unchanged);
    assert!(same.diff.is_none());
    // A declared path that never existed is dropped entirely: no "skipped"
    // row ships and the reader never sees an op outside 新建/修改/删除.
    assert!(by_path.next().is_none());
    assert_eq!(evidence.edits.len(), 4);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn compute_evidence_labels_binary_after_states_with_a_real_op() {
    let dir = temp_dir("skip");
    write_file(&dir.join("was_text.bin"), "text\n");
    let context = EditContext::capture(&dir, &["was_text.bin".to_string()]);
    fs::write(dir.join("was_text.bin"), [0u8, 1, 2]).unwrap();
    let evidence = context.compute_evidence();
    assert_eq!(evidence.edits.len(), 1);
    // 文本变成二进制仍是一次修改：op 保持 modified，类型作为标签。
    assert_eq!(evidence.edits[0].op, FileEditOp::Modified);
    assert_eq!(evidence.edits[0].skip_reason, Some("binary"));
    // 二进制文件没有 diff，只有路径/op/大小这组元信息。
    assert_eq!(evidence.edits[0].size_bytes, Some(3));
    let _ = fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn compute_evidence_labels_unreadable_files_with_a_real_op() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("unreadable");
    let locked = dir.join("locked.txt");
    write_file(&locked, "secret\n");
    let context = EditContext::capture(&dir, &["locked.txt".to_string()]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let evidence = context.compute_evidence();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(evidence.edits[0].op, FileEditOp::Modified);
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

            size_bytes: None,
        };
        entry.extend_from_diff(name, &old, &new);
        entries.push(entry);
    }
    let bounded = apply_payload_budget_with_limit(entries, 96 * 1024);
    let payload = edits_payload(&FileEditsEvidence { edits: bounded });
    let serialized = payload.to_string();
    assert!(serialized.len() <= 96 * 1024);
    // Overflowing entries degrade to slim (stats kept, diff dropped) or
    // minimal budget_exhausted placeholders; both keep the path visible.
    assert!(serialized.contains("budget_exhausted") || serialized.contains("\"diff\":null"));
    let _ = fs::remove_dir_all(&dir);
}

fn budget_entry(path: &str, diff_len: usize) -> FileEditEntry {
    FileEditEntry {
        path: path.to_string(),
        op: FileEditOp::Modified,
        diff: Some("x".repeat(diff_len)),
        added_lines: 1,
        removed_lines: 1,
        truncated: false,
        skip_reason: None,
        size_bytes: None,
    }
}

fn slim_budget_len(entry: &FileEditEntry) -> usize {
    let mut slim = entry.clone();
    slim.diff = None;
    slim.truncated = true;
    slim.skip_reason = Some(SKIP_BUDGET);
    entry_json(&slim).to_string().len() + 1
}

#[test]
fn payload_budget_spends_on_smallest_diffs_first() {
    let large = budget_entry("large.txt", 60_000);
    let small = budget_entry("small.txt", 10_000);
    let medium = budget_entry("medium.txt", 30_000);
    let base = 2 + slim_budget_len(&large) + slim_budget_len(&small) + slim_budget_len(&medium);
    let small_extra = entry_json(&small).to_string().len() + 1 - slim_budget_len(&small);
    let medium_extra = entry_json(&medium).to_string().len() + 1 - slim_budget_len(&medium);
    let budget = base + small_extra + medium_extra;

    let bounded = apply_payload_budget_with_limit(vec![large, small, medium], budget);
    let paths: Vec<&str> = bounded.iter().map(|entry| entry.path.as_str()).collect();
    assert_eq!(paths, vec!["large.txt", "small.txt", "medium.txt"]);
    let row = |path: &str| {
        bounded
            .iter()
            .find(|entry| entry.path == path)
            .unwrap_or_else(|| panic!("missing {path}"))
    };
    assert!(row("small.txt").diff.is_some());
    assert!(row("medium.txt").diff.is_some());
    let large = row("large.txt");
    assert!(large.diff.is_none());
    assert_eq!(large.skip_reason, Some(SKIP_BUDGET));
    let serialized = edits_payload(&FileEditsEvidence { edits: bounded }).to_string();
    assert!(serialized.len() <= budget);
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

        size_bytes: None,
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

#[test]
fn capture_stages_large_files_on_disk_and_diffs_small_edits() {
    let dir = temp_dir("bigstage");
    let name = "app.js";
    let old: Vec<String> = (0..24000)
        .map(|index| format!("console.log('line-{index}');"))
        .collect();
    let before_content = old.join("\n");
    write_file(&dir.join(name), &before_content);
    let context = EditContext::capture(&dir, &[name.to_string()]);
    let staged = context
        .snapshot_dir
        .as_ref()
        .expect("staging dir")
        .as_ref()
        .to_path_buf();
    let staged_copy = staged.join("before_00");
    assert_eq!(
        fs::metadata(&staged_copy).expect("staged copy").len(),
        before_content.len() as u64
    );
    let mut new = old.clone();
    new[12] = "console.log('edited-line-12');".to_string();
    write_file(&dir.join(name), &new.join("\n"));
    let evidence = context.compute_evidence();
    assert_eq!(evidence.edits.len(), 1);
    let entry = &evidence.edits[0];
    assert_eq!(entry.op, FileEditOp::Modified);
    assert_eq!((entry.added_lines, entry.removed_lines), (1, 1));
    assert!(!entry.truncated);
    assert!(entry.skip_reason.is_none());
    let diff = entry.diff.as_deref().expect("diff");
    assert!(diff.contains("-console.log('line-12');"));
    assert!(diff.contains("+console.log('edited-line-12');"));
    drop(context);
    assert!(!staged.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_dir_survives_until_the_last_clone_drops() {
    let dir = temp_dir("cloneshare");
    write_file(&dir.join("a.txt"), "one\n");
    let context = EditContext::capture(&dir, &["a.txt".to_string()]);
    let staged = context
        .snapshot_dir
        .as_ref()
        .expect("staging dir")
        .as_ref()
        .to_path_buf();
    let job_copy = context.clone();
    drop(context);
    assert!(staged.exists());
    let evidence = job_copy.compute_evidence();
    assert_eq!(evidence.edits[0].op, FileEditOp::Unchanged);
    drop(job_copy);
    assert!(!staged.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn snapshot_staging_dir_is_removed_on_drop() {
    let dir = temp_dir("staging");
    write_file(&dir.join("a.txt"), "content\n");
    let context = EditContext::capture(&dir, &["a.txt".to_string()]);
    let staged = context
        .snapshot_dir
        .as_ref()
        .expect("staging dir")
        .as_ref()
        .to_path_buf();
    assert!(staged.exists());
    drop(context);
    assert!(!staged.exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn capture_of_only_missing_paths_avoids_staging() {
    let dir = temp_dir("nostage");
    let context = EditContext::capture(&dir, &["absent.txt".to_string()]);
    assert!(context.snapshot_dir.is_none());
    let evidence = context.compute_evidence();
    // A path that never existed is not an edit: no row at all, not a "skipped"
    // row the reader would have to interpret.
    assert!(evidence.edits.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn directories_report_a_real_op_with_a_directory_tag() {
    // A declared folder is not an edit we can diff, but the row still says
    // 修改/新建 and trails a "文件夹" kind tag instead of "skipped".
    let dir = temp_dir("dircap");
    fs::create_dir(dir.join("sub")).unwrap();
    let existing = EditContext::capture(&dir, &["sub".to_string()]);
    let evidence = existing.compute_evidence();
    assert_eq!(evidence.edits.len(), 1);
    assert_eq!(evidence.edits[0].op, FileEditOp::Modified);
    assert_eq!(evidence.edits[0].skip_reason, Some("directory"));
    assert!(evidence.edits[0].diff.is_none());

    let fresh = temp_dir("dircap_created");
    let created = EditContext::capture(&fresh, &["newdir".to_string()]);
    fs::create_dir(fresh.join("newdir")).unwrap();
    let evidence = created.compute_evidence();
    assert_eq!(evidence.edits[0].op, FileEditOp::Created);
    assert_eq!(evidence.edits[0].skip_reason, Some("directory"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&fresh);
}

#[test]
fn capture_labels_oversized_files_by_sniffed_kind() {
    let dir = temp_dir("toobig");
    let name = "huge.bin";
    let handle = fs::File::create(dir.join(name)).unwrap();
    handle.set_len(64 * 1024 * 1024 + 1).unwrap();
    drop(handle);
    let context = EditContext::capture(&dir, &[name.to_string()]);
    let evidence = context.compute_evidence();
    // Sparse zeros sniff as binary; the op is still a real one.
    assert_eq!(evidence.edits[0].op, FileEditOp::Modified);
    assert_eq!(evidence.edits[0].skip_reason, Some("binary"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn capture_probe_skips_binary_files_before_copy() {
    let dir = temp_dir("probebin");
    let name = "asset.bin";
    let mut bytes = vec![b'x'; 4096];
    bytes.push(0);
    fs::write(dir.join(name), &bytes).unwrap();
    let context = EditContext::capture(&dir, &[name.to_string()]);
    let staged = context
        .snapshot_dir
        .as_ref()
        .expect("staging dir")
        .as_ref()
        .to_path_buf();
    assert_eq!(fs::read_dir(&*staged).unwrap().count(), 0);
    let evidence = context.compute_evidence();
    assert_eq!(evidence.edits[0].op, FileEditOp::Modified);
    assert_eq!(evidence.edits[0].skip_reason, Some("binary"));
    drop(context);
    assert!(!staged.exists());
    let _ = fs::remove_dir_all(&dir);
}

fn ledger_row<'a>(evidence: &'a FileEditsEvidence, path: &str) -> &'a FileEditEntry {
    evidence
        .edits
        .iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("no ledger row for {path}: {:?}", evidence.edits))
}

#[test]
fn turn_ledger_drops_created_then_deleted_tmp_tree() {
    // /tmp 场景：worktree 整树建了又删。目录根先登记（当时不存在 → Absent），
    // 其下的文件后来才首次声明（祖先 Absent ⇒ 推导基线也是 Absent），轮末
    // 一把算：双方都回到不存在，一行都不留。
    let dir = temp_dir("ledger_tmp_tree");
    let root = dir.join("tree");
    let root_declared = root.to_string_lossy().into_owned();
    let file = root.join("src").join("main.rs");
    let file_declared = file.to_string_lossy().into_owned();

    let mut ledger = TurnFileEditsLedger::default();
    ledger.register(&dir, "turn", std::slice::from_ref(&root_declared));
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    write_file(&file, "fn a() {}\nfn b() {}\n");
    ledger.register(&dir, "turn", std::slice::from_ref(&file_declared));

    let net = ledger.net_evidence_for_turn("turn").unwrap();
    let row = ledger_row(&net, &file_declared);
    assert_eq!(row.op, FileEditOp::Created);
    assert_eq!(row.added_lines, 2);

    fs::remove_dir_all(&root).unwrap();
    ledger.register(&dir, "turn", &[root_declared]);

    let net = ledger.net_evidence_for_turn("turn").unwrap();
    assert!(
        net.edits.is_empty(),
        "created-then-deleted tree must net out to nothing: {:?}",
        net.edits
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn turn_ledger_folds_repeated_edits_into_one_baseline_diff() {
    let dir = temp_dir("ledger_repeat");
    let file = dir.join("notes.txt");
    let declared = file.to_string_lossy().into_owned();
    write_file(&file, "one\n");
    let mut ledger = TurnFileEditsLedger::default();
    ledger.register(&dir, "turn", std::slice::from_ref(&declared));

    write_file(&file, "one\ntwo\n");
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    let row = ledger_row(&net, &declared);
    assert_eq!(row.op, FileEditOp::Modified);
    assert!(row.diff.as_deref().unwrap().contains("+two"));

    // 第二次改动后，diff 仍是 基线→终态 的全程变化，不是最后一跳。
    write_file(&file, "one\ntwo\nthree\n");
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    let row = ledger_row(&net, &declared);
    assert_eq!(row.added_lines, 2);
    assert!(row.diff.as_deref().unwrap().contains("+three"));
    assert_eq!(net.edits.len(), 1);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn turn_ledger_metadata_rows_omit_diff_text() {
    let dir = temp_dir("ledger_metadata");
    let file = dir.join("notes.txt");
    let declared = file.to_string_lossy().into_owned();
    write_file(&file, "one\n");
    let mut ledger = TurnFileEditsLedger::default();
    ledger.register(&dir, "turn", std::slice::from_ref(&declared));
    write_file(&file, "one\ntwo\n");

    let full = ledger.net_evidence_for_turn("turn").unwrap();
    let full_row = ledger_row(&full, &declared);
    assert!(full_row
        .diff
        .as_deref()
        .is_some_and(|diff| diff.contains("+two")));

    let metadata = ledger
        .net_evidence_for_turn_metadata("turn")
        .expect("metadata evidence");
    let metadata_row = ledger_row(&metadata, &declared);
    assert!(metadata_row.diff.is_none());
    assert_eq!(
        (metadata_row.added_lines, metadata_row.removed_lines),
        (1, 0)
    );
    assert!(ledger.net_evidence_for_turn_metadata("other").is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn turn_ledger_deleted_then_recreated_reads_modified() {
    let dir = temp_dir("ledger_rebuild");
    let file = dir.join("cfg.txt");
    let declared = file.to_string_lossy().into_owned();
    write_file(&file, "old\n");
    let mut ledger = TurnFileEditsLedger::default();
    ledger.register(&dir, "turn", std::slice::from_ref(&declared));

    fs::remove_file(&file).unwrap();
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    assert_eq!(ledger_row(&net, &declared).op, FileEditOp::Deleted);

    write_file(&file, "new\n");
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    let row = ledger_row(&net, &declared);
    assert_eq!(row.op, FileEditOp::Modified);
    assert!(row.diff.as_deref().unwrap().contains("+new"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn turn_ledger_non_text_paths_report_existence_only() {
    let dir = temp_dir("ledger_binary");
    let blob = dir.join("logo.bin");
    let declared = blob.to_string_lossy().into_owned();
    write_file(
        &blob,
        [0u8, 1, 2, 3]
            .iter()
            .map(|b| *b as char)
            .collect::<String>()
            .as_str(),
    );
    let mut ledger = TurnFileEditsLedger::default();
    ledger.register(&dir, "turn", std::slice::from_ref(&declared));

    // 仍然存在：无基线可对比，不产生行。
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    assert!(net.edits.is_empty(), "{:?}", net.edits);

    fs::remove_file(&blob).unwrap();
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    let row = ledger_row(&net, &declared);
    assert_eq!(row.op, FileEditOp::Deleted);
    assert_eq!(row.skip_reason, Some("binary"));
    assert!(row.diff.is_none());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn turn_ledger_resets_between_turns_and_ignores_stale_refreshes() {
    let dir = temp_dir("ledger_turns");
    let file = dir.join("f.txt");
    let declared = file.to_string_lossy().into_owned();
    write_file(&file, "a\n");
    let mut ledger = TurnFileEditsLedger::default();
    ledger.register(&dir, "turn-1", std::slice::from_ref(&declared));

    // 新一轮重新登记基线；旧轮次的刷新请求不再产出（总结已随旧轮下发）。
    ledger.register(&dir, "turn-2", std::slice::from_ref(&declared));
    assert!(ledger.net_evidence_for_turn("turn-1").is_none());

    write_file(&file, "b\n");
    let net = ledger.net_evidence_for_turn("turn-2").unwrap();
    assert!(net.edits[0].diff.as_deref().unwrap().contains("+b"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn turn_ledger_entry_cap_is_hard() {
    let dir = temp_dir("ledger_cap");
    let mut ledger = TurnFileEditsLedger::default();
    let declared: Vec<String> = (0..160)
        .map(|index| {
            dir.join(format!("f{index}.txt"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    ledger.register(&dir, "turn", &declared);
    assert_eq!(ledger.net_evidence_for_turn("turn").unwrap().edits.len(), 0);
    // 上限之后的新路径不再进入维护表，条目总数有界。
    let extra = dir.join("extra.txt");
    ledger.register(&dir, "turn", &[extra.to_string_lossy().into_owned()]);
    let net = ledger.net_evidence_for_turn("turn").unwrap();
    assert!(net.edits.is_empty());
    assert_eq!(ledger.tracked_len(), 128);
    let _ = fs::remove_dir_all(&dir);
}
