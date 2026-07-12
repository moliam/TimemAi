use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn builtin_registry_loads_manifest_tools() {
    let registry = CapabilityRegistry::builtin();

    assert!(registry.contains_tool("memmgr"));
    assert!(registry.contains_tool("capmgr"));
    assert!(registry.contains_tool("run_bash"));
    assert!(!registry.contains_tool("shell_job_status"));
    assert!(registry.contains_tool("self_tool"));
    assert!(!registry.contains_tool("tool_job_status"));
    assert!(!registry.contains_tool("query_memory"));
}

#[test]
fn host_profile_filters_local_command_capabilities() {
    let registry = CapabilityRegistry::builtin_for_host(
        CapabilityHostProfile::without_local_command_execution(),
    );

    assert!(registry.contains_tool("memmgr"));
    assert!(registry.contains_tool("capmgr"));
    assert!(registry.contains_tool("self_tool"));
    assert!(!registry.contains_tool("run_bash"));
    assert!(!registry.contains_tool("shell_job_status"));
    assert!(!registry.contains_tool("tool_job_status"));
    assert_eq!(registry.binding_name("run_bash"), None);
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([("cmd", Value::String("pwd".to_string()))])
        )
        .unwrap_err()
        .contains("unsupported_action:run_bash"));

    let rendered = registry.render_tool_catalog_markdown();
    assert!(!rendered.contains("#### `run_bash`"));
    assert!(!rendered.contains("#### `shell_job_status`"));
    assert!(!rendered.contains("#### `tool_job_status`"));
}

#[test]
fn host_profile_can_enable_local_command_capabilities_without_shell_ui() {
    let registry =
        CapabilityRegistry::builtin_for_host(CapabilityHostProfile::with_local_command_execution());

    assert!(registry.contains_tool("run_bash"));
    assert!(!registry.contains_tool("shell_job_status"));
    assert!(!registry.contains_tool("tool_job_status"));
    assert_eq!(registry.binding_name("run_bash"), Some("run_bash"));
}

#[test]
fn registry_renders_prompt_tool_catalog_from_manifests() {
    let registry = CapabilityRegistry::builtin();
    let rendered = registry.render_tool_catalog_markdown();

    assert!(rendered.contains("#### `memmgr`"));
    assert!(rendered.contains("#### `capmgr`"));
    assert!(rendered.contains("#### `run_bash`"));
    assert!(!rendered.contains("#### `shell_job_status`"));
    assert!(!rendered.contains("#### `tool_job_status`"));
    assert!(rendered.contains("#### `self_tool`"));
    assert!(rendered.contains("interval_ms"));
    assert!(rendered.contains("loop_timeout_ms"));
    assert!(rendered.contains("once_timeout_ms"));
    assert!(rendered.contains("exits with code 0"));
    assert!(rendered.contains("**Synopsis**"));
    assert!(rendered.contains("**Options**"));
    assert!(rendered.contains("Unified local memory manager"));
    assert!(rendered.contains("Use when the user asks about Timem itself"));
    assert!(rendered.contains("Conditional:"));
    assert!(rendered.contains("Use sql for durable reads"));
    assert!(!rendered.contains("durable: query|schema"));
    assert!(!rendered.contains("empty is allowed for durable/raw_chat/scratch recent listing"));
    assert!(!rendered.contains("type=context"));
    assert!(!rendered.contains("context_offload"));
    assert!(!rendered.contains("when `` is"));
    assert!(rendered.contains("**Result**"));
    assert!(!rendered.contains("```"));
    assert!(!rendered.contains("**Example action**"));
    assert!(!rendered.contains("read_back_command"));
    assert!(!rendered.contains("large_readback"));
    assert!(!rendered.contains("check_timeout_ms"));
    assert!(rendered.contains("`background`:"));
    assert!(rendered.contains("Normal/Polling returns status and bounded output"));
    assert!(rendered.contains("Background returns"));
    assert!(rendered.contains("Timeout command won't be killed automatically"));
    assert!(rendered.contains("`timeout_ms` is only how long Timem waits"));
    assert!(rendered.contains("It is not a kill deadline"));
    assert!(rendered.contains("Use loop_cmd with interval_ms"));
    assert!(rendered.contains("`op`:"));
    assert!(rendered.contains("`kind`:"));
    assert!(rendered.contains("`id`:"));
    assert!(rendered.contains("`inspect`"));
    assert!(rendered.contains("memory_conflict"));
    assert!(!rendered.contains("\"output\": {"));
    assert!(!rendered.contains("\"description\""));
    assert!(!rendered.contains("Background job id when background=true."));
}

#[test]
fn registry_exposes_executor_binding_names() {
    let registry = CapabilityRegistry::builtin();

    assert_eq!(registry.binding_name("memmgr"), Some("memmgr"));
    assert_eq!(registry.binding_name("capmgr"), Some("capmgr"));
    assert_eq!(registry.binding_name("run_bash"), Some("run_bash"));
    assert_eq!(registry.binding_name("shell_job_status"), None);
    assert_eq!(registry.binding_name("tool_job_status"), None);
    assert_eq!(registry.binding_name("self_tool"), Some("self_tool"));
    assert_eq!(registry.binding_name("future_tool"), None);
}

#[test]
fn registry_validates_required_input_fields_from_manifest() {
    let registry = CapabilityRegistry::builtin();

    assert!(registry
        .validate_action_input("capmgr", &json_object([]))
        .unwrap_err()
        .contains("input.op_required"));
    assert!(registry
        .validate_action_input(
            "memmgr",
            &json_object([("type", Value::String("durable".to_string()))])
        )
        .unwrap_err()
        .contains("input.op_required"));
    assert!(registry
        .validate_action_input(
            "memmgr",
            &json_object([
                ("type", Value::String("durable".to_string())),
                ("op", Value::String("sql".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.sql_required_when_op=sql,type=durable"));
    assert!(registry
        .validate_action_input(
            "memmgr",
            &json_object([
                ("type", Value::String("scratch".to_string())),
                ("op", Value::String("write".to_string())),
                ("kind", Value::String("notes".to_string())),
                ("content", Value::String("checkpoint".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.label_required_when_op=write,type=scratch"));
    assert!(registry
        .validate_action_input(
            "memmgr",
            &json_object([
                ("type", Value::String("scratch".to_string())),
                ("op", Value::String("write".to_string())),
                ("kind", Value::String("notes".to_string())),
                ("label", Value::String("checkpoint".to_string())),
                ("content", Value::String("checkpoint".to_string())),
            ])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "memmgr",
            &json_object([
                ("type", Value::String("scratch".to_string())),
                ("op", Value::String("write".to_string())),
                ("kind", Value::String("context_offload".to_string())),
                ("label", Value::String("large context".to_string())),
            ])
        )
        .is_err());
    assert!(registry
        .validate_action_input(
            "memmgr",
            &json_object([
                ("type", Value::String("context".to_string())),
                ("op", Value::String("discard".to_string())),
            ])
        )
        .is_err());
    assert!(registry
        .validate_action_input(
            "shell_job_status",
            &json_object([("job_id", Value::String("job_1".to_string()))])
        )
        .unwrap_err()
        .contains("unsupported_action:shell_job_status"));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([
                ("op", Value::String("job_cancel".to_string())),
                ("job_id", Value::String("tool_job_1".to_string())),
            ])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "shell_job_status",
            &json_object([
                ("job_id", Value::String("job_1".to_string())),
                ("op", Value::String("cancel".to_string())),
            ])
        )
        .unwrap_err()
        .contains("unsupported_action:shell_job_status"));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([
                ("op", Value::String("job_status".to_string())),
                ("job_id", Value::String("tool_job_1".to_string())),
            ])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "tool_job_status",
            &json_object([("job_id", Value::String("tool_job_1".to_string()))])
        )
        .unwrap_err()
        .contains("unsupported_action:tool_job_status"));
    assert!(registry
        .validate_action_input("run_bash", &json_object([]))
        .unwrap_err()
        .contains("input.any_required:cmd|loop_cmd"));
    assert!(registry
        .validate_action_input("self_tool", &json_object([]))
        .unwrap_err()
        .contains("input.type_required"));
    assert!(registry
        .validate_action_input(
            "self_tool",
            &json_object([
                ("type", Value::String("env".to_string())),
                ("op", Value::String("write".to_string())),
                ("key", Value::String("TIMEM_TEST_FLAG".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.value_required_when_op=write"));
    assert!(registry
        .validate_action_input(
            "self_tool",
            &json_object([
                ("type", Value::String("mem_path".to_string())),
                ("op", Value::String("read".to_string())),
            ])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([("read_back_command", Value::String("pwd".to_string()))])
        )
        .unwrap_err()
        .contains("input.read_back_command_unsupported"));
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([
                ("cmd", Value::String("pwd".to_string())),
                (
                    "large_readback_opt_in",
                    Value::String("need full output".to_string())
                ),
            ])
        )
        .unwrap_err()
        .contains("input.large_readback_opt_in_unsupported"));
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([
                ("cmd", Value::String("test -s output.txt".to_string())),
                ("timeout_ms", Value::Number(5000.into())),
            ])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([("op", Value::String("load".to_string()))])
        )
        .unwrap_err()
        .contains("input.kind_required_when_op=load"));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([
                ("op", Value::String("inspect".to_string())),
                ("kind", Value::String("skill".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.id_required_when_op=inspect"));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([("op", Value::String("list".to_string()))])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([("op", Value::String("remove".to_string()))])
        )
        .unwrap_err()
        .contains("input.op_unsupported:remove"));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([
                ("op", Value::String("list".to_string())),
                ("kind", Value::String("resource".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.kind_unsupported:resource"));
}

#[test]
fn registry_derives_validation_rules_from_json_schema_idl() {
    let registry = CapabilityRegistry::builtin();
    let catalog = registry.tool_catalog_value();
    let capmgr = catalog
        .get("capmgr")
        .and_then(Value::as_object)
        .expect("capmgr catalog entry");

    let op_enum = capmgr
        .get("input_schema")
        .and_then(Value::as_object)
        .and_then(|schema| schema.get("properties"))
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("op"))
        .and_then(Value::as_object)
        .and_then(|op| op.get("enum"))
        .and_then(Value::as_array)
        .expect("capmgr op enum");
    assert!(op_enum.contains(&Value::String("list".to_string())));
    assert!(op_enum.contains(&Value::String("load".to_string())));
    assert!(capmgr
        .get("required_when")
        .and_then(Value::as_array)
        .is_some_and(|rules| !rules.is_empty()));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([("op", Value::String("load".to_string()))])
        )
        .unwrap_err()
        .contains("input.kind_required_when_op=load"));
    assert!(registry
        .validate_action_input(
            "capmgr",
            &json_object([("op", Value::String("inspect".to_string()))])
        )
        .unwrap_err()
        .contains("input.kind_required_when_op=inspect"));
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([
                ("cmd", Value::String("pwd".to_string())),
                ("mode", Value::String("normal".to_string())),
            ])
        )
        .is_ok());
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([
                ("cmd", Value::String("pwd".to_string())),
                ("mode", Value::String("foreground".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.mode_unsupported:foreground"));
    assert!(registry
        .validate_action_input(
            "run_bash",
            &json_object([
                ("cmd", Value::String("pwd".to_string())),
                ("mode", Value::String("daemon".to_string())),
            ])
        )
        .unwrap_err()
        .contains("input.mode_unsupported:daemon"));
}

#[test]
fn registry_enriches_static_prompt_tool_catalog() {
    let registry = CapabilityRegistry::builtin();
    let enriched =
        registry.enrich_static_prompt("## Tools\n{{TOOL_CATALOG}}\n## Skills\n{{SKILL_HEADERS}}");

    assert!(enriched.contains("#### `memmgr`"));
    assert!(!enriched.contains("\"release_quality_gate\""));
    assert!(enriched.contains("#### `run_bash`"));
    assert!(enriched.contains("No optional skills are currently loaded."));
    assert!(!enriched.contains("{{TOOL_CATALOG}}"));
    assert!(!enriched.contains("{{SKILL_HEADERS}}"));
}

#[test]
fn run_bash_idl_uses_cmd_loop_cmd_without_removed_expect_fields() {
    let registry = CapabilityRegistry::builtin();
    let catalog = registry.tool_catalog_value();
    let run_bash = catalog
        .get("run_bash")
        .and_then(Value::as_object)
        .expect("run_bash catalog entry");
    let input_schema = run_bash
        .get("input_schema")
        .and_then(Value::as_object)
        .expect("run_bash input schema");
    let input_properties = input_schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("run_bash input schema properties");

    assert!(input_properties.contains_key("cmd"));
    assert!(input_properties.contains_key("loop_cmd"));
    assert!(input_properties.contains_key("loop_timeout_ms"));
    assert!(input_properties.contains_key("once_timeout_ms"));
    assert!(!input_properties.contains_key("command"));
    assert!(!input_properties.contains_key("read_back_command"));
    assert!(!input_properties.contains_key("large_readback_opt_in"));
    assert!(!input_properties.contains_key("check_timeout_ms"));
    assert!(!input_properties.contains_key("expect"));
    assert!(!input_properties.contains_key("expect_timeout_ms"));

    let required_any = input_schema
        .get("required_any")
        .and_then(Value::as_array)
        .expect("run_bash required_any");
    assert!(required_any.iter().any(|group| {
        group
            .as_array()
            .map(|fields| fields.iter().any(|field| field == "cmd"))
            .unwrap_or(false)
    }));

    let prompt = registry.render_tool_catalog_markdown();
    assert!(prompt.contains("run_bash cmd=<shell_command>"));
    assert!(prompt.contains("run_bash loop_cmd=<check_command>"));
    assert!(prompt.contains("loop_timeout_ms"));
    assert!(prompt.contains("once_timeout_ms"));
    assert!(!prompt.contains("check_timeout_ms"));
    assert!(!prompt.contains("`expect`:"));
    assert!(!prompt.contains("expect_timeout_ms"));
}

#[test]
fn capmgr_can_list_and_load_skill_content() {
    let dir = temp_release_quality_skill_overlay("capmgr_skill_load");
    let registry = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap();

    let list = registry.list_text("skill");
    assert!(list.contains("Action result: capmgr"));
    assert!(list.contains("release_quality_gate"));
    assert!(list.contains("Release quality gate"));

    let loaded = registry.load_text("skill", "release_quality_gate");
    assert!(loaded.contains("op: load"));
    assert!(loaded.contains("# Release Quality Gate"));
    assert!(loaded.contains("Run the relevant local tests"));

    let loaded_tool = registry.load_text("tool", "run_bash");
    assert!(loaded_tool.contains("kind: tool"));
    assert!(loaded_tool.contains("manual:"));
    assert!(loaded_tool.contains("#### `run_bash`"));
    assert!(loaded_tool.contains("**Options**"));
    assert!(loaded_tool.contains("run_bash cmd=<shell_command>"));
    assert!(!loaded_tool.contains("read_back_command"));
    assert!(!loaded_tool.contains("large_readback"));
    assert!(!loaded_tool.contains("expect_timeout_ms"));
    assert!(!loaded_tool.contains("**Example action**"));
}

#[test]
fn registry_loads_runtime_overlay_tools_and_skills_from_files() {
    let dir = temp_capability_dir("runtime_overlay");
    let tools_dir = dir.join("tools");
    let skill_dir = dir.join("skills").join("log_review");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        tools_dir.join("local_echo.yaml"),
        r#"kind: tool
id: local_echo
binding_type: builtin
binding_name: run_bash
summary: Echo a bounded local string through Bash.
description: |
  Use this runtime overlay tool only when a bounded echo command is enough.
input_properties:
  command: string
required:
  - command
example_json: |
  {
    "action": "local_echo",
    "args": {
      "command": "printf hello"
    }
  }
"#,
    )
    .unwrap();
    fs::write(
        skill_dir.join("skill.yaml"),
        r#"kind: skill
id: log_review
title: Log review
summary: Runtime-loaded log review checklist.
entry: instructions.md
when_to_use: |
  Use for structured log review.
"#,
    )
    .unwrap();
    fs::write(
        skill_dir.join("instructions.md"),
        "# Runtime Log Review\n\nCheck timestamps.",
    )
    .unwrap();

    let registry = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap();

    assert_eq!(registry.binding_name("local_echo"), Some("run_bash"));
    assert!(registry
        .tool_catalog_value()
        .get("local_echo")
        .and_then(|tool| tool.get("output"))
        .is_none());
    assert!(registry
        .render_tool_catalog_markdown()
        .contains("#### `local_echo`"));
    assert!(registry
        .load_text("skill", "log_review")
        .contains("# Runtime Log Review"));
    assert!(registry
        .render_skill_headers_markdown()
        .contains("Runtime-loaded log review checklist"));
}

#[test]
fn no_local_command_profile_filters_overlay_command_tools() {
    let dir = temp_capability_dir("no_local_command_overlay");
    let tools_dir = dir.join("tools");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::write(
        tools_dir.join("local_echo.yaml"),
        r#"kind: tool
id: local_echo
binding_type: command
binding_name: echo.sh
summary: Echo through a local command.
description: |
  Uses a host command process.
input_properties:
  text: string
required:
  - text
example_json: |
  {
    "action": "local_echo",
    "args": {
      "text": "hello"
    }
  }
"#,
    )
    .unwrap();
    fs::write(
        tools_dir.join("local_echo_builtin.yaml"),
        r#"kind: tool
id: local_echo_builtin
binding_type: builtin
binding_name: run_bash
requires_host: local_command_execution
summary: Echo through the built-in local command executor.
description: |
  Uses the built-in local command executor through an overlay alias.
input_properties:
  command: string
required:
  - command
example_json: |
  {
    "action": "local_echo_builtin",
    "args": {
      "command": "printf hello"
    }
  }
"#,
    )
    .unwrap();
    fs::write(dir.join("echo.sh"), "#!/bin/sh\nprintf '%s\\n' \"$1\"\n").unwrap();

    let registry = CapabilityRegistry::builtin_with_overlay_dir_for_host(
        &dir,
        CapabilityHostProfile::without_local_command_execution(),
    )
    .unwrap();

    assert!(!registry.contains_tool("local_echo"));
    assert!(!registry.contains_tool("local_echo_builtin"));
    assert!(!registry.contains_tool("run_bash"));
    assert!(!registry.contains_tool("shell_job_status"));
    assert!(!registry
        .render_tool_catalog_markdown()
        .contains("local_echo"));
}

#[test]
fn no_local_command_profile_filters_run_bash_builtin_alias_even_without_requires_host() {
    let dir = temp_capability_dir("no_local_command_builtin_alias_overlay");
    let tools_dir = dir.join("tools");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::write(
        tools_dir.join("local_shell_alias.yaml"),
        r#"kind: tool
id: local_shell_alias
binding_type: builtin
binding_name: run_bash
summary: Alias for local shell execution.
description: |
  This intentionally omits requires_host to prove the target binding is filtered.
input_properties:
  cmd: string
required:
  - cmd
example_json: |
  {
    "action": "local_shell_alias",
    "args": {
      "cmd": "pwd"
    }
  }
"#,
    )
    .unwrap();

    let registry = CapabilityRegistry::builtin_with_overlay_dir_for_host(
        &dir,
        CapabilityHostProfile::without_local_command_execution(),
    )
    .unwrap();

    assert!(!registry.contains_tool("run_bash"));
    assert!(!registry.contains_tool("local_shell_alias"));
    assert!(!registry
        .render_tool_catalog_markdown()
        .contains("local_shell_alias"));
    assert!(registry
        .validate_action_input(
            "local_shell_alias",
            &json_object([("cmd", Value::String("pwd".to_string()))])
        )
        .unwrap_err()
        .contains("unsupported_action:local_shell_alias"));
}

#[test]
fn registry_stress_loads_many_overlay_tools_and_skills_and_filters_removals() {
    let dir = temp_capability_dir("many_runtime_overlay");
    let tools_dir = dir.join("tools");
    let skills_dir = dir.join("skills");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::create_dir_all(&skills_dir).unwrap();
    for i in 0..80 {
        fs::write(
            tools_dir.join(format!("overlay_tool_{i:03}.yaml")),
            format!(
                r#"kind: tool
id: overlay_tool_{i:03}
binding_type: builtin
binding_name: self_tool
summary: Runtime overlay self inspection tool {i}.
description: |
  Runtime-added test tool {i} for capability registry stress coverage.
input_properties:
  type: string
  op: string
required:
  - type
  - op
example_json: |
  {{
    "action": "overlay_tool_{i:03}",
    "args": {{
      "type": "about_me",
      "op": "read"
    }}
  }}
"#
            ),
        )
        .unwrap();
    }
    for i in 0..25 {
        let skill_dir = skills_dir.join(format!("skill_{i:03}"));
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("skill.yaml"),
            format!(
                r#"kind: skill
id: skill_{i:03}
title: Overlay Skill {i}
summary: Runtime overlay skill {i}.
entry: instructions.md
when_to_use: |
  Use this synthetic skill {i} during capability stress tests.
"#
            ),
        )
        .unwrap();
        fs::write(
            skill_dir.join("instructions.md"),
            format!("# Overlay Skill {i}\n\nStress body {i}.\n"),
        )
        .unwrap();
    }

    let registry = CapabilityRegistry::builtin_with_overlay_dir_for_host(
        &dir,
        CapabilityHostProfile::with_local_command_execution(),
    )
    .unwrap();
    assert!(registry.tool_count() >= 84);
    assert_eq!(registry.skill_count(), 25);
    assert!(registry.contains_tool("overlay_tool_000"));
    assert!(registry.contains_tool("overlay_tool_079"));
    assert!(registry
        .validate_action_input(
            "overlay_tool_079",
            &json_object([
                ("type", Value::String("about_me".to_string())),
                ("op", Value::String("read".to_string())),
            ])
        )
        .is_ok());
    let rendered = registry.render_tool_catalog_markdown();
    assert!(rendered.contains("#### `overlay_tool_000`"));
    assert!(rendered.contains("#### `overlay_tool_079`"));
    assert!(registry
        .load_text("skill", "skill_024")
        .contains("# Overlay Skill 24"));

    let filtered = CapabilityRegistry::builtin_with_overlay_dir_for_host(
        &dir,
        CapabilityHostProfile::without_local_command_execution(),
    )
    .unwrap();
    assert!(filtered.contains_tool("overlay_tool_000"));
    assert!(!filtered.contains_tool("run_bash"));
    assert_eq!(filtered.skill_count(), 25);
}

#[test]
fn registry_rejects_overlay_tool_without_executor_binding() {
    let dir = temp_capability_dir("bad_runtime_overlay");
    let tools_dir = dir.join("tools");
    fs::create_dir_all(&tools_dir).unwrap();
    fs::write(
        tools_dir.join("ghost.yaml"),
        r#"kind: tool
id: ghost
binding_type: builtin
binding_name: missing_executor
summary: This tool has no executor.
description: |
  Should not load.
input_properties:
  query: string
example_json: |
  {
    "action": "ghost",
    "args": {
      "query": "x"
    }
  }
"#,
    )
    .unwrap();

    let err = CapabilityRegistry::builtin_with_overlay_dir(&dir).unwrap_err();
    assert!(err.contains("ghost:unsupported_builtin_binding"));
}

fn temp_capability_dir(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("timem_capability_test_{name}_{nanos}"))
}

fn temp_release_quality_skill_overlay(name: &str) -> PathBuf {
    let dir = temp_capability_dir(name);
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
