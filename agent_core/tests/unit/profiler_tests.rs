use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn profiler_aggregates_tokens_by_model_and_wait_time() {
    let mut profiler = RuntimeProfiler::default();
    profiler.record_model_wait(
        "aliyun",
        "qwen-plus",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 1000,
            completion_tokens: 200,
            cached_tokens: 700,
            cache_created_tokens: 120,
            ..UsageStats::zero()
        },
        Duration::from_millis(500),
    );
    profiler.record_model_wait(
        "aliyun",
        "qwen-plus",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 500,
            completion_tokens: 300,
            cached_tokens: 100,
            cache_created_tokens: 80,
            ..UsageStats::zero()
        },
        Duration::from_millis(1000),
    );
    profiler.record_turn(Duration::from_millis(2000), Duration::from_millis(1500));

    let profile = profiler.models().get("aliyun:qwen-plus").unwrap();
    assert_eq!(profile.llm_calls, 2);
    assert_eq!(profile.input_tokens, 1500);
    assert_eq!(profile.output_tokens, 500);
    assert_eq!(profile.cached_tokens, 800);
    assert_eq!(profile.cache_created_tokens, 200);
    assert_eq!(profiler.model_wait(), Duration::from_millis(1500));
    assert_eq!(profiler.local_work(), Duration::from_millis(500));

    let report = profiler.report(StorageProfile {
        durable_entries: 0,
        durable_bytes: 0,
        scratch_entries: 0,
        scratch_bytes: 0,
        api_audit_bytes: 0,
        action_audit_bytes: 0,
    });
    assert_eq!(report.models.len(), 1);
    assert_eq!(report.models[0].model, "aliyun:qwen-plus");
    assert_eq!(report.models[0].cached_tokens, 800);
    assert_eq!(report.model_wait, Duration::from_millis(1500));
    assert_eq!(report.local_work, Duration::from_millis(500));

    assert_eq!(report.models[0].llm_calls, 2);
    assert_eq!(report.models[0].input_tokens, 1500);
    assert_eq!(report.models[0].output_tokens, 500);
    assert_eq!(report.models[0].cache_created_tokens, 200);
    assert_eq!(report.models[0].wait, Duration::from_millis(1500));
}

#[test]
fn storage_profile_counts_entries_and_sizes() {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "timem_profiler_test_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    fs::create_dir_all(&dir).unwrap();
    let memory_dir = dir.join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    let memory = memory_dir.join("memory.jsonl");
    let scratch = memory_dir.join("scratch_notes.jsonl");
    let api_audit = dir.join("api_audit.json");
    let action_audit = dir.join("action_audit.json");
    fs::write(&memory, "{}\n\n{}\n").unwrap();
    fs::write(&scratch, "{}\n").unwrap();
    fs::write(&api_audit, "{\n  \"version\": 1,\n  \"events\": []\n}\n").unwrap();
    fs::write(&action_audit, "{\n  \"turns\": []\n}\n").unwrap();

    let profile = collect_storage_profile(&memory_dir, &api_audit, &action_audit);
    assert_eq!(profile.durable_entries, 2);
    assert_eq!(profile.scratch_entries, 1);
    assert!(profile.durable_bytes > 0);
    assert!(profile.api_audit_bytes > 0);
    assert!(profile.action_audit_bytes > 0);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn storage_profile_counts_large_jsonl_without_loading_whole_file() {
    let dir = std::env::temp_dir().join(format!(
        "timem_profiler_large_jsonl_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    let memory_dir = dir.join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    let memory = memory_dir.join("memory.jsonl");
    let scratch = memory_dir.join("scratch_notes.jsonl");
    let mut text = String::new();
    for index in 0..10_000 {
        text.push_str(&format!(r#"{{"id":"m_{index}","content":"test"}}"#));
        text.push('\n');
    }
    fs::write(&memory, text).unwrap();
    fs::write(&scratch, "{}\n\n{}\n").unwrap();
    let api_audit = dir.join("api_audit.json");
    let action_audit = dir.join("action_audit.json");
    fs::write(&api_audit, "{}").unwrap();
    fs::write(&action_audit, "{}").unwrap();

    let profile = collect_storage_profile(&memory_dir, &api_audit, &action_audit);

    assert_eq!(profile.durable_entries, 10_000);
    assert_eq!(profile.scratch_entries, 2);
    assert!(profile.durable_bytes > 100_000);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn runtime_profile_report_collects_storage_and_raw_profile_data() {
    let dir = std::env::temp_dir().join(format!(
        "timem_profile_report_test_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    ));
    let memory_dir = dir.join("memory");
    fs::create_dir_all(&memory_dir).unwrap();
    fs::write(memory_dir.join("memory.jsonl"), "{}\n{}\n").unwrap();
    let api_audit = dir.join("api_audit.json");
    let action_audit = dir.join("action_audit.json");
    fs::write(&api_audit, "{}").unwrap();
    fs::write(&action_audit, "{}").unwrap();

    let mut profiler = RuntimeProfiler::default();
    profiler.record_model_wait(
        "test",
        "model",
        &UsageStats {
            llm_calls: 1,
            prompt_tokens: 100,
            completion_tokens: 20,
            ..UsageStats::zero()
        },
        Duration::from_millis(50),
    );

    let report = runtime_profile_report(&profiler, &memory_dir, &api_audit, &action_audit);
    assert_eq!(report.models.len(), 1);
    assert_eq!(report.models[0].model, "test:model");
    assert_eq!(report.storage.durable_entries, 2);
    assert!(report.storage.api_audit_bytes > 0);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn profile_report_keeps_raw_storage_and_zero_output_data() {
    let report = RuntimeProfileReport {
        models: vec![ModelProfileReport {
            model: "test:model".to_string(),
            llm_calls: 1,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cache_created_tokens: 0,
            wait: Duration::from_millis(250),
        }],
        model_wait: Duration::from_millis(250),
        local_work: Duration::from_millis(42),
        storage: StorageProfile {
            durable_entries: 3,
            durable_bytes: 327,
            scratch_entries: 7,
            scratch_bytes: 1600,
            api_audit_bytes: 6_553_600,
            action_audit_bytes: 8192,
        },
    };

    assert_eq!(report.models[0].output_tokens, 0);
    assert_eq!(report.models[0].wait, Duration::from_millis(250));
    assert_eq!(report.storage.durable_entries, 3);
    assert_eq!(report.storage.durable_bytes, 327);
    assert_eq!(report.storage.scratch_entries, 7);
    assert_eq!(report.storage.scratch_bytes, 1600);
    assert_eq!(report.storage.api_audit_bytes, 6_553_600);
    assert_eq!(report.storage.action_audit_bytes, 8192);
}

#[test]
fn profile_metrics_are_core_owned_and_ui_neutral() {
    let profile = ModelProfileReport {
        model: "provider:model".to_string(),
        llm_calls: 3,
        input_tokens: 1_500,
        output_tokens: 500,
        cached_tokens: 800,
        cache_created_tokens: 200,
        wait: Duration::from_millis(1500),
    };

    assert_eq!(profile.cache_hit_percent_tenths(), Some(533));
    assert_eq!(profile.wait_per_1k_output(), Some(Duration::from_secs(3)));
    assert_eq!(profile_cache_hit_percent_tenths(0, 0), None);
    assert_eq!(profile_wait_per_1k_output(Duration::from_secs(1), 0), None);
}
