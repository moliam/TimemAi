use super::*;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn capmgr_lists_skill_headers() {
    let registry = registry_with_release_skill();
    let result = execute(
        &registry,
        CapmgrActionInput {
            op: "list",
            kind: "skill",
            id: "",
        },
    );

    assert!(result.contains("Action result: capmgr"));
    assert!(result.contains("release_quality_gate"));
}

#[test]
fn capmgr_loads_skill_body() {
    let registry = registry_with_release_skill();
    let result = execute(
        &registry,
        CapmgrActionInput {
            op: "load",
            kind: "skill",
            id: "release_quality_gate",
        },
    );

    assert!(result.contains("Action result: capmgr"));
    assert!(result.contains("Release Quality Gate"));
    assert!(result.contains("body:"));
}

#[test]
fn capmgr_reports_unsupported_operation() {
    let registry = CapabilityRegistry::builtin();
    let result = execute(
        &registry,
        CapmgrActionInput {
            op: "remove",
            kind: "skill",
            id: "release_quality_gate",
        },
    );

    assert!(result.contains("error: unsupported_op"));
}

fn registry_with_release_skill() -> CapabilityRegistry {
    let dir = temp_release_quality_skill_overlay();
    CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap()
}

fn temp_release_quality_skill_overlay() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "timem_capmgr_release_skill_{}_{}",
        std::process::id(),
        nanos
    ));
    let skill_dir = dir.join("skills").join("release_quality_gate");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
            skill_dir.join("skill.yaml"),
            r#"kind: skill
id: release_quality_gate
title: Release quality gate
summary: Verify tests, CI, release notes, sensitive information, and version state before publishing a release.
entry: instructions.md
when_to_use: |
  Use when preparing, auditing, or deciding whether to publish a Timem release.
"#,
        )
        .unwrap();
    fs::write(
        skill_dir.join("instructions.md"),
        "# Release Quality Gate\n\nRun the relevant local tests.\n",
    )
    .unwrap();
    dir
}
