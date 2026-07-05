use agent_core::RuntimeProfileReport;

const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_RESET: &str = "\x1b[0m";

pub fn render_prof_report_data(report: &RuntimeProfileReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{ANSI_BOLD}Timem runtime profiling{ANSI_RESET}\n\n"
    ));
    out.push_str(&section_title("Token 监控（per model）"));
    if report.models.is_empty() {
        out.push_str("  暂无模型调用。\n");
    } else {
        for profile in &report.models {
            out.push_str(&format!("  {}\n", profile.model));
            out.push_str(&format!(
                "    │─ calls: {}  ||  kvc hit rate(⌁): {}\n",
                profile.llm_calls,
                compact_profile_percent_tenths(profile.cache_hit_percent_tenths())
            ));
            out.push_str(&format!(
                "    └─ ▲{} (⌁{} / ✚{})  ▼{}  |  sec/▼1K: {}\n",
                compact_profile_count(profile.input_tokens),
                compact_profile_count(profile.cached_tokens),
                compact_profile_count(profile.cache_created_tokens),
                compact_profile_count(profile.output_tokens),
                compact_optional_profile_duration(profile.wait_per_1k_output())
            ));
        }
    }
    out.push('\n');
    out.push_str(&section_title("底层性能"));
    out.push_str(&format!(
        "  等待模型回复: {}\n",
        compact_profile_duration(report.model_wait)
    ));
    out.push_str(&format!(
        "  本地执行: {}\n",
        compact_profile_duration(report.local_work)
    ));
    out.push_str("  预留: 后续可加入 bash 执行、文件读写、增删文件等监控。\n");
    out.push('\n');
    out.push_str(&section_title("存储"));
    out.push_str(&format!(
        "  durable_mem: {} 条, {}\n",
        report.storage.durable_entries,
        compact_profile_bytes(report.storage.durable_bytes)
    ));
    out.push_str(&format!(
        "  scratch_mem: {} 条, {}\n",
        report.storage.scratch_entries,
        compact_profile_bytes(report.storage.scratch_bytes)
    ));
    out.push_str(&format!(
        "  api_audit: {}\n",
        compact_profile_bytes(report.storage.api_audit_bytes)
    ));
    out.push_str(&format!(
        "  action_audit: {}\n",
        compact_profile_bytes(report.storage.action_audit_bytes)
    ));
    out
}

fn section_title(title: &str) -> String {
    format!("{ANSI_BOLD}▸ {title}{ANSI_RESET}\n")
}

fn compact_profile_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    if value < 1_000_000 {
        return trim_decimal(format!("{:.1}", value as f64 / 1_000.0)) + "K";
    }
    trim_decimal(format!("{:.2}", value as f64 / 1_000_000.0)) + "M"
}

fn compact_profile_bytes(value: u64) -> String {
    if value < 1024 {
        return format!("{value} B");
    }
    if value < 1024 * 1024 {
        return trim_decimal(format!("{:.1}", value as f64 / 1024.0)) + " KiB";
    }
    trim_decimal(format!("{:.2}", value as f64 / 1024.0 / 1024.0)) + " MiB"
}

fn compact_profile_percent_tenths(value: Option<u32>) -> String {
    let Some(value) = value else {
        return "0%".to_string();
    };
    let whole = value / 10;
    let tenth = value % 10;
    if tenth == 0 {
        format!("{whole}%")
    } else {
        format!("{whole}.{tenth}%")
    }
}

fn compact_profile_duration(duration: std::time::Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        return format!("{millis} ms");
    }
    trim_decimal(format!("{:.1}", millis as f64 / 1000.0)) + " s"
}

fn compact_optional_profile_duration(duration: Option<std::time::Duration>) -> String {
    duration
        .map(compact_profile_duration)
        .unwrap_or_else(|| "n/a".to_string())
}

fn trim_decimal(mut text: String) -> String {
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{RuntimeProfiler, StorageProfile, UsageStats};
    use std::time::Duration;

    #[test]
    fn profiler_token_cards_render_aggregated_core_data() {
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

        let report = crate::runtime_profile_report(
            &profiler,
            std::path::Path::new("/missing/memory"),
            std::path::Path::new("/missing/audit.json"),
            std::path::Path::new("/missing/action_audit.json"),
        );
        let report = render_prof_report_data(&report);
        assert!(report.contains("\x1b[1m▸ Token 监控（per model）\x1b[0m"));
        assert!(report.contains("  aliyun:qwen-plus"));
        assert!(report.contains("│─ calls: 2  ||  kvc hit rate(⌁): 53.3%"));
        assert!(report.contains("└─ ▲1.5K (⌁800 / ✚200)  ▼500  |  sec/▼1K: 3 s"));
        assert!(report.contains("53.3%"));
        assert!(!report.contains("total input"));
    }

    #[test]
    fn profiler_report_renderer_uses_core_data_without_collecting_files() {
        let mut profiler = RuntimeProfiler::default();
        profiler.record_model_wait(
            "test",
            "model",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 2000,
                completion_tokens: 100,
                cached_tokens: 500,
                cache_created_tokens: 1000,
                ..UsageStats::zero()
            },
            Duration::from_millis(250),
        );
        profiler.record_turn(Duration::from_millis(400), Duration::from_millis(250));
        let report = profiler.report(StorageProfile {
            durable_entries: 2,
            durable_bytes: 42,
            scratch_entries: 3,
            scratch_bytes: 2048,
            api_audit_bytes: 4096,
            action_audit_bytes: 8192,
        });

        let rendered = render_prof_report_data(&report);
        assert!(rendered.contains("test:model"));
        assert!(rendered.contains("calls: 1"));
        assert!(rendered.contains("kvc hit rate(⌁): 25%"));
        assert!(rendered.contains("durable_mem: 2 条, 42 B"));
        assert!(rendered.contains("scratch_mem: 3 条, 2 KiB"));
        assert!(rendered.contains("api_audit: 4 KiB"));
        assert!(rendered.contains("action_audit: 8 KiB"));
    }

    #[test]
    fn profiler_token_cards_keep_compact_structure_across_models() {
        let mut profiler = RuntimeProfiler::default();
        profiler.record_model_wait(
            "a",
            "short",
            &UsageStats {
                llm_calls: 12,
                prompt_tokens: 99,
                completion_tokens: 7,
                cached_tokens: 1,
                ..UsageStats::zero()
            },
            Duration::from_millis(100),
        );
        profiler.record_model_wait(
            "aliyun",
            "very-long-model-name",
            &UsageStats {
                llm_calls: 1,
                prompt_tokens: 123_456,
                completion_tokens: 4567,
                cached_tokens: 12_345,
                ..UsageStats::zero()
            },
            Duration::from_millis(2000),
        );

        let report = crate::runtime_profile_report(
            &profiler,
            std::path::Path::new("/missing/memory"),
            std::path::Path::new("/missing/audit.json"),
            std::path::Path::new("/missing/action_audit.json"),
        );
        let report = render_prof_report_data(&report);
        let call_lines: Vec<&str> = report
            .lines()
            .filter(|line| line.contains("│─ calls:"))
            .collect();
        let token_lines: Vec<&str> = report
            .lines()
            .filter(|line| line.contains("└─ ▲"))
            .collect();
        assert_eq!(call_lines.len(), 2);
        assert_eq!(token_lines.len(), 2);
        assert!(call_lines
            .iter()
            .all(|line| line.contains("  ||  kvc hit rate(⌁): ")));
        assert!(token_lines
            .iter()
            .all(|line| line.contains("  |  sec/▼1K: ")));
        assert!(!report.contains("(⌁0       )"));
        assert!(!report.contains("(⌁1       )"));
    }
}
