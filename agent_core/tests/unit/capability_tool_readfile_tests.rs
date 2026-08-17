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
    assert!(result.ends_with("content:\nA1B2B"), "{result}");
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
    assert!(result.ends_with("content:\né"), "{result}");
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
