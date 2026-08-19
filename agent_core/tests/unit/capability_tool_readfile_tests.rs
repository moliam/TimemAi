use super::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "timem_readfile_{label}_{}_{}",
            std::process::id(),
            id
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn default_execution_timeout_is_ten_seconds() {
    assert_eq!(DEFAULT_TIMEOUT, std::time::Duration::from_secs(10));
}

#[test]
fn successful_read_status_is_independent_of_payload_words() {
    let dir = TempDir::new("structured_status_payload");
    let payload = "timeout_secs=30\nerror: documentation example\ncancelled is a word\n";
    fs::write(dir.path().join("status-words.txt"), payload).unwrap();

    let outcome = execute_outcome(dir.path(), &json!({"path": "status-words.txt"}));

    assert_eq!(outcome.status, crate::ActionStatus::Completed);
    assert!(outcome.text.contains("timeout_secs=30"), "{}", outcome.text);
    assert!(
        outcome.text.contains("error: documentation example"),
        "{}",
        outcome.text
    );
    assert!(
        outcome.text.contains("cancelled is a word"),
        "{}",
        outcome.text
    );
    let evidence = outcome.readfile_result.expect("readfile evidence");
    assert_eq!(
        evidence.path,
        fs::canonicalize(dir.path().join("status-words.txt"))
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(evidence.content, payload);
    assert_eq!(evidence.error_type, None);
    assert_eq!(evidence.limited, Some(false));
}

#[test]
fn blocked_read_has_structured_timeout_status() {
    let dir = TempDir::new("structured_timeout");
    fs::write(dir.path().join("slow.txt"), "eventual content").unwrap();
    let probe = install_test_parallel_read_probe(
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(100),
    );

    let outcome = execute_with_timeout_outcome(
        dir.path(),
        &json!({"path": "slow.txt"}),
        std::time::Duration::from_millis(10),
    );

    assert_eq!(outcome.status, crate::ActionStatus::Timeout);
    assert!(outcome.text.contains("error: timeout"), "{}", outcome.text);
    let evidence = outcome.readfile_result.expect("timeout readfile evidence");
    assert_eq!(evidence.path, "slow.txt");
    assert_eq!(evidence.error_type, None);
    assert!(evidence.content.contains("timeout"));

    thread::sleep(std::time::Duration::from_millis(120));
    drop(probe);
}

#[test]
fn parallel_read_probes_keep_independent_state_and_cleanup() {
    let first_dir = TempDir::new("independent_probe_first");
    let second_dir = TempDir::new("independent_probe_second");
    fs::write(first_dir.path().join("slow.txt"), "first content").unwrap();
    fs::write(second_dir.path().join("slow.txt"), "second content").unwrap();

    let first_probe = install_test_parallel_read_probe(
        first_dir.path().to_path_buf(),
        std::time::Duration::from_millis(100),
    );
    let second_probe = install_test_parallel_read_probe(
        second_dir.path().to_path_buf(),
        std::time::Duration::from_millis(100),
    );

    // Releasing one probe must not clear another probe installed for a
    // different path. The former global probe state violated this invariant.
    drop(second_probe);

    let outcome = execute_with_timeout_outcome(
        first_dir.path(),
        &json!({"path": "slow.txt"}),
        std::time::Duration::from_millis(10),
    );

    assert_eq!(outcome.status, crate::ActionStatus::Timeout);
    assert!(outcome.text.contains("error: timeout"), "{}", outcome.text);

    thread::sleep(std::time::Duration::from_millis(120));
    drop(first_probe);
}

#[test]
fn blocked_read_returns_timeout_error_after_wait_limit() {
    let dir = TempDir::new("timeout");
    fs::write(dir.path().join("slow.txt"), "eventual content").unwrap();
    let probe = install_test_parallel_read_probe(
        dir.path().to_path_buf(),
        std::time::Duration::from_millis(100),
    );

    let started = std::time::Instant::now();
    let outcome = execute_with_timeout_outcome(
        dir.path(),
        &json!({"path": "slow.txt"}),
        std::time::Duration::from_millis(10),
    );
    let elapsed = started.elapsed();

    assert_eq!(outcome.status, crate::ActionStatus::Timeout);
    assert!(outcome.text.contains("status: error"), "{}", outcome.text);
    assert!(outcome.text.contains("error: timeout"), "{}", outcome.text);
    assert!(
        elapsed < std::time::Duration::from_millis(80),
        "readfile timeout returned too late: {elapsed:?}"
    );

    // The timed-out worker is detached because a blocking filesystem read cannot
    // be safely cancelled. Let this test probe finish before resetting its globals.
    thread::sleep(std::time::Duration::from_millis(120));
    drop(probe);
}

#[test]
fn reads_relative_utf8_file_from_session_cwd() {
    let dir = TempDir::new("relative");
    fs::write(dir.path().join("notes.txt"), "alpha\nbeta\n").unwrap();

    let result = execute(dir.path(), &json!({"path": "notes.txt"}));

    assert!(
        result.contains("Action result: readfile\nstatus: ok"),
        "{result}"
    );
    assert!(result.contains("encoding: UTF-8"), "{result}");
    assert!(result.contains("file_bytes: 11"), "{result}");
    assert!(result.ends_with("content:\nalpha\nbeta\n"), "{result}");
}

#[test]
fn default_read_is_limited_to_the_thirty_two_kibibyte_output_budget() {
    assert_eq!(DEFAULT_MAX_BYTES, 32 * 1024);
    assert_eq!(MAX_RETURN_BYTES, 32 * 1024);
    let dir = TempDir::new("default_output_budget");
    fs::write(
        dir.path().join("large.txt"),
        "x".repeat(MAX_RETURN_BYTES + 500),
    )
    .unwrap();

    let result = execute(dir.path(), &json!({"path": "large.txt"}));

    assert!(
        result.contains(&format!("content_bytes: {MAX_RETURN_BYTES}")),
        "{result}"
    );
    assert!(result.contains("limited: true"), "{result}");
}

#[test]
fn line_selectors_are_one_based_inclusive_and_keep_line_endings() {
    let dir = TempDir::new("lines");
    fs::write(dir.path().join("lines.txt"), "one\r\ntwo\nthree").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "lines.txt",
            "starter": {"line_nr": 2},
            "ender": {"line_nr": 2}
        }),
    );

    assert!(result.ends_with("content:\ntwo\n"), "{result}");
}

#[test]
fn end_line_beyond_eof_clamps_to_the_actual_last_line() {
    let dir = TempDir::new("end_line_beyond_eof");
    fs::write(dir.path().join("short.txt"), "first\nsecond").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "short.txt",
            "starter": {"line_nr": 1},
            "ender": {"line_nr": 240},
            "max_bytes": 32768
        }),
    );

    assert!(result.contains("status: ok"), "{result}");
    assert!(!result.contains("end_line_not_found"), "{result}");
    assert!(result.contains("limited: false"), "{result}");
    assert!(result.ends_with("content:\nfirst\nsecond"), "{result}");
}

#[test]
fn end_line_beyond_eof_still_respects_max_bytes() {
    let dir = TempDir::new("end_line_beyond_eof_budget");
    fs::write(dir.path().join("short.txt"), "first\nsecond\nthird").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "short.txt",
            "starter": {"line_nr": 1},
            "ender": {"line_nr": 240},
            "max_bytes": 8
        }),
    );

    assert!(result.contains("status: ok"), "{result}");
    assert!(result.contains("content_bytes: 8"), "{result}");
    assert!(result.contains("limited: true"), "{result}");
    assert!(
        result.contains("content:\nfirst\nse\n!!!Too long, 2 words truncated after."),
        "{result}"
    );
}

#[test]
fn starter_line_beyond_eof_remains_an_error() {
    let dir = TempDir::new("start_line_beyond_eof");
    fs::write(dir.path().join("short.txt"), "first\nsecond").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "short.txt",
            "starter": {"line_nr": 240},
            "ender": {"line_nr": 300}
        }),
    );

    assert!(result.contains("status: error"), "{result}");
    assert!(result.contains("error: start_line_not_found"), "{result}");
}

#[test]
fn line_selectors_support_lone_carriage_return_files() {
    let dir = TempDir::new("cr_lines");
    fs::write(dir.path().join("lines.txt"), "one\rtwo\rthree").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "lines.txt",
            "starter": {"line_nr": 2},
            "ender": {"line_nr": 2}
        }),
    );

    assert!(result.ends_with("content:\ntwo\r"), "{result}");
}

#[test]
fn match_selectors_use_first_start_and_last_complete_end_in_window() {
    let dir = TempDir::new("matches");
    fs::write(dir.path().join("matches.txt"), "xxA1B2BzzA3Btail").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "matches.txt",
            "starter": {"match": "A"},
            "ender": {"match": "B"},
            "max_bytes": 8
        }),
    );

    assert!(result.contains("limited: true"), "{result}");
    assert!(
        result.contains("content:\nA1B2B\n!!!Too long, 0 words truncated after."),
        "{result}"
    );
}

#[test]
fn match_selectors_are_unicode_safe() {
    let dir = TempDir::new("unicode_matches");
    fs::write(
        dir.path().join("matches.txt"),
        "前缀【开始】甲【结束】乙【结束】尾部",
    )
    .unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "matches.txt",
            "starter": {"match": "【开始】"},
            "ender": {"match": "【结束】"},
            "max_bytes": 128
        }),
    );

    assert!(
        result.ends_with("content:\n【开始】甲【结束】乙【结束】"),
        "{result}"
    );
}

#[test]
fn byte_selectors_address_original_bytes_and_are_inclusive() {
    let dir = TempDir::new("bytes");
    fs::write(dir.path().join("bytes.txt"), "abcdef").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "bytes.txt",
            "starter": {"byte_nr": 1},
            "ender": {"byte_nr": 3}
        }),
    );

    assert!(result.ends_with("content:\nbcd"), "{result}");
}

#[test]
fn byte_selectors_reject_multibyte_character_splits() {
    let dir = TempDir::new("byte_split");
    fs::write(dir.path().join("utf8.txt"), "aéz").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "utf8.txt",
            "starter": {"byte_nr": 1},
            "ender": {"byte_nr": 1}
        }),
    );

    assert!(
        result.contains("error: byte_selector_splits_character"),
        "{result}"
    );
}

#[test]
fn max_bytes_never_splits_returned_utf8() {
    let dir = TempDir::new("utf8_limit");
    fs::write(dir.path().join("utf8.txt"), "éé").unwrap();

    let result = execute(dir.path(), &json!({"path": "utf8.txt", "max_bytes": 3}));

    assert!(result.contains("content_bytes: 2"), "{result}");
    assert!(result.contains("limited: true"), "{result}");
    assert!(
        result.contains("content:\né\n!!!Too long, 1 words truncated after."),
        "{result}"
    );
}

#[test]
fn bom_detection_handles_utf8_and_utf16_without_returning_bom() {
    let dir = TempDir::new("bom");
    fs::write(dir.path().join("utf8.txt"), b"\xEF\xBB\xBFhello").unwrap();
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "你好".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    fs::write(dir.path().join("utf16.txt"), utf16).unwrap();

    let utf8_result = execute(dir.path(), &json!({"path": "utf8.txt"}));
    let utf16_result = execute(dir.path(), &json!({"path": "utf16.txt"}));

    assert!(utf8_result.ends_with("content:\nhello"), "{utf8_result}");
    assert!(
        utf16_result.contains("encoding: UTF-16LE"),
        "{utf16_result}"
    );
    assert!(utf16_result.ends_with("content:\n你好"), "{utf16_result}");
}

#[test]
fn explicit_gbk_decodes_strictly() {
    let dir = TempDir::new("gbk");
    let (encoded, _, had_errors) = encoding_rs::GBK.encode("中文内容");
    assert!(!had_errors);
    fs::write(dir.path().join("gbk.txt"), encoded.as_ref()).unwrap();

    let result = execute(dir.path(), &json!({"path": "gbk.txt", "encoding": "gbk"}));

    assert!(result.contains("encoding: GBK"), "{result}");
    assert!(result.ends_with("content:\n中文内容"), "{result}");
}

#[test]
fn malformed_or_mismatched_encoding_never_returns_replacement_text() {
    let dir = TempDir::new("encoding_errors");
    fs::write(dir.path().join("invalid.txt"), [0xFF, 0xFF]).unwrap();
    fs::write(dir.path().join("bom.txt"), b"\xEF\xBB\xBFhello").unwrap();

    let malformed = execute(dir.path(), &json!({"path": "invalid.txt"}));
    let mismatch = execute(
        dir.path(),
        &json!({"path": "bom.txt", "encoding": "utf-16le"}),
    );
    let unsupported = execute(
        dir.path(),
        &json!({"path": "bom.txt", "encoding": "made-up-encoding"}),
    );

    assert!(
        malformed.contains("error: invalid_text_encoding"),
        "{malformed}"
    );
    assert!(
        mismatch.contains("error: encoding_bom_mismatch"),
        "{mismatch}"
    );
    assert!(
        unsupported.contains("error: unsupported_encoding"),
        "{unsupported}"
    );
    assert!(!malformed.contains('�'));
}

#[test]
fn rejects_binary_directory_missing_and_oversized_files() {
    let dir = TempDir::new("bad_files");
    fs::write(dir.path().join("binary.bin"), b"abc\0def").unwrap();
    let huge = fs::File::create(dir.path().join("huge.txt")).unwrap();
    huge.set_len(MAX_SCAN_BYTES + 1).unwrap();

    let binary = execute(dir.path(), &json!({"path": "binary.bin"}));
    let directory = execute(dir.path(), &json!({"path": "."}));
    let missing = execute(dir.path(), &json!({"path": "missing.txt"}));
    let oversized = execute(dir.path(), &json!({"path": "huge.txt"}));

    assert!(binary.contains("error: binary_file"), "{binary}");
    assert!(directory.contains("error: not_regular_file"), "{directory}");
    assert!(missing.contains("error: path_not_found"), "{missing}");
    assert!(
        oversized.contains("error: scan_limit_exceeded"),
        "{oversized}"
    );
}

#[cfg(unix)]
#[test]
fn rejects_fifo_without_blocking_on_macos_or_linux() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let dir = TempDir::new("fifo");
    let fifo = dir.path().join("input.pipe");
    let path = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);

    let result = execute(dir.path(), &json!({"path": "input.pipe"}));

    assert!(result.contains("error: not_regular_file"), "{result}");
}

#[cfg(unix)]
#[test]
fn follows_symlinks_only_when_the_target_is_a_regular_file() {
    use std::os::unix::fs::symlink;

    let dir = TempDir::new("symlinks");
    fs::write(dir.path().join("target.txt"), "linked text").unwrap();
    fs::create_dir(dir.path().join("target-dir")).unwrap();
    symlink("target.txt", dir.path().join("file-link")).unwrap();
    symlink("target-dir", dir.path().join("dir-link")).unwrap();

    let file_link = execute(dir.path(), &json!({"path": "file-link"}));
    let dir_link = execute(dir.path(), &json!({"path": "dir-link"}));

    assert!(file_link.ends_with("content:\nlinked text"), "{file_link}");
    assert!(dir_link.contains("error: not_regular_file"), "{dir_link}");
}

#[test]
fn selector_and_range_errors_are_explicit() {
    let dir = TempDir::new("selector_errors");
    fs::write(dir.path().join("text.txt"), "first\nsecond\n").unwrap();

    let multiple = execute(
        dir.path(),
        &json!({"path": "text.txt", "starter": {"line_nr": 1, "byte_nr": 0}}),
    );
    let missing_match = execute(
        dir.path(),
        &json!({"path": "text.txt", "starter": {"match": "absent"}}),
    );
    let reversed = execute(
        dir.path(),
        &json!({
            "path": "text.txt",
            "starter": {"line_nr": 2},
            "ender": {"line_nr": 1}
        }),
    );
    let invalid_limit = execute(
        dir.path(),
        &json!({"path": "text.txt", "max_bytes": MAX_RETURN_BYTES + 1}),
    );
    let unsupported = execute(dir.path(), &json!({"path": "text.txt", "surprise": true}));

    assert!(multiple.contains("error: invalid_selector"), "{multiple}");
    assert!(
        missing_match.contains("error: start_match_not_found"),
        "{missing_match}"
    );
    assert!(reversed.contains("error: range_before_start"), "{reversed}");
    assert!(
        invalid_limit.contains("error: invalid_max_bytes"),
        "{invalid_limit}"
    );
    assert!(
        unsupported.contains("error: unsupported_input"),
        "{unsupported}"
    );
}

#[test]
fn concurrent_reads_are_independent_and_deterministic() {
    let dir = TempDir::new("concurrent");
    fs::write(dir.path().join("shared.txt"), "stable evidence\n").unwrap();
    let cwd = Arc::new(dir.path().to_path_buf());
    let mut readers = Vec::new();
    for _ in 0..16 {
        let cwd = Arc::clone(&cwd);
        readers.push(thread::spawn(move || {
            execute(&cwd, &json!({"path": "shared.txt"}))
        }));
    }

    for reader in readers {
        let result = reader.join().unwrap();
        assert!(result.ends_with("content:\nstable evidence\n"), "{result}");
    }
}

#[test]
fn empty_regular_file_returns_an_empty_success() {
    let dir = TempDir::new("empty");
    fs::write(dir.path().join("empty.txt"), []).unwrap();

    let result = execute(dir.path(), &json!({"path": "empty.txt"}));

    assert!(result.contains("status: ok"), "{result}");
    assert!(result.contains("file_bytes: 0"), "{result}");
    assert!(result.ends_with("content:\n"), "{result}");
}

#[test]
fn forward_read_retains_beginning_and_places_notice_after_content() {
    let dir = TempDir::new("forward_tail_option");
    fs::write(dir.path().join("large.txt"), "BEGIN alpha beta gamma END").unwrap();

    let result = execute(dir.path(), &json!({"path": "large.txt", "max_bytes": 11}));

    assert!(result.contains("tail_out: false"), "{result}");
    assert!(result.contains("content_bytes: 11"), "{result}");
    assert!(
        result.contains("content:\nBEGIN alpha\n!!!Too long,"),
        "{result}"
    );
    assert!(result.contains("truncated after"), "{result}");
    assert!(!result.contains("gamma END"), "{result}");
}

#[test]
fn tail_read_retains_ending_and_places_notice_before_content() {
    let dir = TempDir::new("tail_option");
    fs::write(dir.path().join("large.txt"), "BEGIN alpha beta gamma END").unwrap();

    let result = execute(
        dir.path(),
        &json!({"path": "large.txt", "max_bytes": 9, "tail_out": true}),
    );

    assert!(result.contains("tail_out: true"), "{result}");
    assert!(result.contains("content_bytes: 9"), "{result}");
    assert!(result.contains("content:\n!!!Too long,"), "{result}");
    assert!(result.contains("truncated before"), "{result}");
    assert!(result.ends_with("gamma END"), "{result}");
    assert!(!result.ends_with("BEGIN alpha"), "{result}");
}

#[test]
fn tail_read_is_utf8_safe_and_never_exceeds_byte_budget() {
    let dir = TempDir::new("tail_utf8");
    fs::write(dir.path().join("utf8.txt"), "甲乙丙丁").unwrap();

    let result = execute(
        dir.path(),
        &json!({"path": "utf8.txt", "max_bytes": 7, "tail_out": true}),
    );

    assert!(result.contains("content_bytes: 6"), "{result}");
    assert!(result.contains("limited: true"), "{result}");
    assert!(result.ends_with("丙丁"), "{result}");
    assert!(!result.contains('�'), "{result}");
}

#[test]
fn tail_read_applies_to_the_selected_range_only() {
    let dir = TempDir::new("tail_selected_range");
    fs::write(
        dir.path().join("selected.txt"),
        "outside START one two three END outside-tail",
    )
    .unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "selected.txt",
            "starter": {"match": "START"},
            "ender": {"match": "END"},
            "max_bytes": 13,
            "tail_out": true
        }),
    );

    assert!(result.contains("truncated before"), "{result}");
    assert!(result.ends_with("two three END"), "{result}");
    assert!(!result.contains("outside-tail"), "{result}");
}

#[test]
fn tail_read_without_truncation_has_no_notice() {
    let dir = TempDir::new("tail_without_truncation");
    fs::write(dir.path().join("short.txt"), "short text").unwrap();

    let result = execute(dir.path(), &json!({"path": "short.txt", "tail_out": true}));

    assert!(result.contains("tail_out: true"), "{result}");
    assert!(result.contains("limited: false"), "{result}");
    assert!(!result.contains("!!!Too long,"), "{result}");
    assert!(result.ends_with("short text"), "{result}");
}

#[test]
fn tail_out_must_be_boolean() {
    let dir = TempDir::new("invalid_tail_out");
    fs::write(dir.path().join("text.txt"), "text").unwrap();

    let result = execute(dir.path(), &json!({"path": "text.txt", "tail_out": "yes"}));

    assert!(result.contains("status: error"), "{result}");
    assert!(result.contains("error: invalid_tail_out"), "{result}");
}

#[test]
fn content_heading_reports_file_name_and_actual_line_range() {
    let dir = TempDir::new("content_heading_lines");
    fs::write(dir.path().join("lines.txt"), "one\ntwo\nthree\nfour\n").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "lines.txt",
            "starter": {"line_nr": 2},
            "ender": {"line_nr": 3}
        }),
    );
    assert!(
        result.contains("lines.txt, line [2, 3]:\ncontent:\ntwo\nthree\n"),
        "{result}"
    );
}

#[test]
fn content_heading_reports_matcher_expression_and_actual_lines() {
    let dir = TempDir::new("content_heading_matcher");
    fs::write(
        dir.path().join("matches.txt"),
        "before\nSTART\nmiddle\nEND\nafter\n",
    )
    .unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "matches.txt",
            "starter": {"match": "START"},
            "ender": {"match": "END"}
        }),
    );
    assert!(
        result.contains(
            "matches.txt, matcher 'START ... END' line is [2, 4]:\ncontent:\nSTART\nmiddle\nEND",
        ),
        "{result}"
    );
}

#[test]
fn content_heading_uses_lines_of_returned_truncated_content() {
    let dir = TempDir::new("content_heading_truncated");
    fs::write(
        dir.path().join("large.txt"),
        "first\nsecond\nthird\nfourth\n",
    )
    .unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "large.txt",
            "max_bytes": 12
        }),
    );
    assert!(
        result.contains("large.txt, line [1, 2]:\ncontent:\nfirst\nsecond"),
        "{result}"
    );
}

#[test]
fn matcher_heading_escapes_single_quotes() {
    let dir = TempDir::new("content_heading_quote");
    fs::write(dir.path().join("quote.txt"), "before\nit's here\nafter").unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "quote.txt",
            "starter": {"match": "it's"},
            "ender": {"match": "here"}
        }),
    );

    assert!(
        result.contains("matcher 'it\\'s ... here' line is [2, 2]:"),
        "{result}"
    );
}

#[test]
fn matcher_heading_keeps_control_characters_on_one_line() {
    let dir = TempDir::new("content_heading_control_chars");
    fs::write(
        dir.path().join("control.txt"),
        "before\nSTART\tvalue\nEND\nafter",
    )
    .unwrap();

    let result = execute(
        dir.path(),
        &json!({
            "path": "control.txt",
            "starter": {"match": "START\t"},
            "ender": {"match": "\nEND"}
        }),
    );

    assert!(
        result.contains("matcher 'START\\t ... \\nEND' line is [2, 3]:"),
        "{result}"
    );
}
