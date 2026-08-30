use super::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_root(label: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("timem_toolrepo_{label}_{stamp}"))
}

fn write_candidate(root: &Path, name: &str, script: &str, args: &[&str]) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("README.md"),
        format!("# {name}\n\n`{name} [value]`\n"),
    )
    .unwrap();
    fs::write(root.join("tool.sh"), script).unwrap();
    fs::write(
        root.join(".timem-tool.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "name": name,
            "type": "automation",
            "language": "bash",
            "entrypoint": "tool.sh",
            "synopsis": format!("{name} [value]"),
            "self_test": {"args": args, "timeout_ms": 2000}
        }))
        .unwrap(),
    )
    .unwrap();
}

#[test]
fn publishes_validated_tool_and_supports_detail_search_and_rename() {
    let root = temp_root("publish");
    let repo = SessionToolRepo::new(root.join("memory"), "session-a");
    let draft = repo.create_draft().unwrap();
    write_candidate(
        &draft,
        "summarize-build-log",
        "#!/bin/bash\nset -euo pipefail\n[[ ${1:-} == --self-test ]] && { echo ready; exit 0; }\necho summary\n",
        &["--self-test"],
    );

    let published = repo.publish(&draft).unwrap();
    assert_eq!(published.summary.name, "summarize-build-log");
    assert_eq!(published.summary.status, "ready");
    assert!(published.validation_output.contains("ready"));
    assert_eq!(repo.list().unwrap().len(), 1);
    assert_eq!(repo.search("summary", 10).unwrap().len(), 1);

    let detail = repo.detail(&published.summary.tool_id).unwrap();
    assert!(detail.readme.contains("summarize-build-log"));
    assert!(detail.files.iter().any(|file| file.path == "tool.sh"));

    let renamed = repo
        .rename(&published.summary.tool_id, "inspect-build-log")
        .unwrap();
    assert_eq!(renamed.tool_id, published.summary.tool_id);
    assert!(renamed.path.ends_with("inspect-build-log"));
    assert!(!Path::new(&published.summary.path).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn publishes_multiple_independent_tools_from_one_runtime_draft_root() {
    let root = temp_root("multi_publish");
    let repo = SessionToolRepo::new(root.join("memory"), "session-multi");
    let draft_root = repo.create_draft().unwrap();
    let first = draft_root.join("extract-log-fields");
    let second = draft_root.join("summarize-env-config");
    write_candidate(
        &first,
        "extract-log-fields",
        "#!/bin/bash\n[[ ${1:-} == --self-test ]] && { echo ready; exit 0; }\necho fields\n",
        &["--self-test"],
    );
    write_candidate(
        &second,
        "summarize-env-config",
        "#!/bin/bash\n[[ ${1:-} == --self-test ]] && { echo ready; exit 0; }\necho config\n",
        &["--self-test"],
    );

    let first_result = repo.publish(&first).unwrap();
    assert!(draft_root.is_dir());
    let second_result = repo.publish(&second).unwrap();
    assert_ne!(first_result.summary.tool_id, second_result.summary.tool_id);
    let tools = repo.list().unwrap();
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().any(|tool| tool.name == "extract-log-fields"));
    assert!(tools.iter().any(|tool| tool.name == "summarize-env-config"));

    repo.discard_draft(&draft_root).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_failed_self_test_and_does_not_publish() {
    let root = temp_root("failure");
    let repo = SessionToolRepo::new(root.join("memory"), "session-b");
    let draft = repo.create_draft().unwrap();
    write_candidate(
        &draft,
        "broken-log-tool",
        "#!/bin/bash\necho broken >&2\nexit 7\n",
        &[],
    );
    let error = repo.publish(&draft).unwrap_err();
    assert!(error.contains("tool_self_test_failed:exit_code=7"));
    assert!(repo.list().unwrap().is_empty());
    assert!(draft.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejects_path_escape_symlink_and_nonsemantic_name() {
    let root = temp_root("safety");
    let repo = SessionToolRepo::new(root.join("memory"), "session-c");
    assert_eq!(
        repo.publish(root.join("outside")).unwrap_err(),
        "tool_draft_outside_session_repo"
    );

    let draft = repo.create_draft().unwrap();
    write_candidate(&draft, "Tool 1", "#!/bin/bash\nexit 0\n", &[]);
    assert_eq!(
        repo.publish(&draft).unwrap_err(),
        "tool_name_must_be_semantic_kebab_case"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let draft = repo.create_draft().unwrap();
        write_candidate(&draft, "safe-name", "#!/bin/bash\nexit 0\n", &[]);
        symlink("/etc/passwd", draft.join("linked-secret")).unwrap();
        assert_eq!(
            repo.publish(&draft).unwrap_err(),
            "tool_symlink_not_allowed"
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn update_keeps_stable_id_and_replaces_files_after_validation() {
    let root = temp_root("update");
    let repo = SessionToolRepo::new(root.join("memory"), "session-d");
    let first = repo.create_draft().unwrap();
    write_candidate(&first, "inspect-latency", "#!/bin/bash\necho v1\n", &[]);
    let first = repo.publish(&first).unwrap();

    let second = repo.create_draft().unwrap();
    write_candidate(&second, "inspect-latency", "#!/bin/bash\necho v2\n", &[]);
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(second.join(".timem-tool.json")).unwrap())
            .unwrap();
    manifest["tool_id"] = serde_json::Value::String(first.summary.tool_id.clone());
    fs::write(
        second.join(".timem-tool.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let updated = repo.publish(&second).unwrap();
    assert!(updated.updated_existing);
    assert_eq!(updated.summary.tool_id, first.summary.tool_id);
    assert!(
        fs::read_to_string(Path::new(&updated.summary.path).join("tool.sh"))
            .unwrap()
            .contains("v2")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn self_test_drains_large_output_without_deadlock_and_bounds_result() {
    let root = temp_root("large_output");
    let repo = SessionToolRepo::new(root.join("memory"), "session-output");
    let draft = repo.create_draft().unwrap();
    write_candidate(
        &draft,
        "large-output-validator",
        "#!/bin/bash\nyes output-line | head -c 200000\n",
        &[],
    );
    let started = std::time::Instant::now();
    let published = repo.publish(&draft).unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
    assert!(published.validation_output.len() <= 2_003);
    assert!(published.validation_output.starts_with("output-line"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn manifest_can_use_a_separate_self_test_entrypoint() {
    let root = temp_root("separate_self_test");
    let repo = SessionToolRepo::new(root.join("memory"), "session-self-test");
    let draft = repo.create_draft().unwrap();
    write_candidate(
        &draft,
        "argument-required-tool",
        "#!/bin/bash\n[[ $# -eq 1 ]] || { echo usage >&2; exit 2; }\necho \"value=$1\"\n",
        &[],
    );
    fs::write(
        draft.join("self-test.sh"),
        "#!/bin/bash\nset -euo pipefail\n/bin/bash ./tool.sh fixture | grep -q 'value=fixture'\necho verified\n",
    )
    .unwrap();
    let mut manifest: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(draft.join(".timem-tool.json")).unwrap()).unwrap();
    manifest["self_test"]["entrypoint"] = serde_json::json!("self-test.sh");
    fs::write(
        draft.join(".timem-tool.json"),
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let published = repo.publish(&draft).unwrap();
    assert!(published.validation_output.contains("verified"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn concurrent_draft_creation_is_unique_within_one_session_repo() {
    let root = temp_root("concurrent_drafts");
    let repo = SessionToolRepo::new(root.join("memory"), "session-concurrent");
    let barrier = Arc::new(Barrier::new(24));
    let handles = (0..24)
        .map(|_| {
            let repo = repo.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                repo.create_draft().unwrap()
            })
        })
        .collect::<Vec<_>>();
    let paths = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(paths.len(), 24);
    assert!(paths.iter().all(|path| path.is_dir()));
    let _ = fs::remove_dir_all(root);
}
