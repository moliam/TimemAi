use agent_core::UsageStats;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::Duration;

const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_RESET: &str = "\x1b[0m";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeProfiler {
    models: BTreeMap<String, ModelProfile>,
    model_wait: Duration,
    local_work: Duration,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelProfile {
    pub llm_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    pub wait: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageProfile {
    pub durable_entries: usize,
    pub durable_bytes: u64,
    pub scratch_entries: usize,
    pub scratch_bytes: u64,
    pub audit_bytes: u64,
}

impl RuntimeProfiler {
    pub fn record_model_wait(
        &mut self,
        provider: &str,
        model: &str,
        usage: &UsageStats,
        wait: Duration,
    ) {
        let key = format!("{}:{}", provider, model);
        let profile = self.models.entry(key).or_default();
        profile.llm_calls = profile.llm_calls.saturating_add(usage.llm_calls);
        profile.input_tokens = profile
            .input_tokens
            .saturating_add(usage.prompt_tokens as u64);
        profile.output_tokens = profile
            .output_tokens
            .saturating_add(usage.completion_tokens as u64);
        profile.cached_tokens = profile
            .cached_tokens
            .saturating_add(usage.cached_tokens as u64);
        profile.wait = profile.wait.saturating_add(wait);
        self.model_wait = self.model_wait.saturating_add(wait);
    }

    pub fn record_turn(&mut self, elapsed: Duration, model_wait: Duration) {
        let local = elapsed.saturating_sub(model_wait);
        self.local_work = self.local_work.saturating_add(local);
    }

    pub fn model_wait(&self) -> Duration {
        self.model_wait
    }

    pub fn local_work(&self) -> Duration {
        self.local_work
    }

    pub fn models(&self) -> &BTreeMap<String, ModelProfile> {
        &self.models
    }
}

pub fn collect_storage_profile(memory_dir: &Path, audit_file: &Path) -> StorageProfile {
    let durable_file = memory_dir.join("memory.jsonl");
    let scratch_file = memory_dir.join("scratch_notes.jsonl");
    StorageProfile {
        durable_entries: count_jsonl_entries(&durable_file),
        durable_bytes: file_size(&durable_file),
        scratch_entries: count_jsonl_entries(&scratch_file),
        scratch_bytes: file_size(&scratch_file),
        audit_bytes: file_size(audit_file),
    }
}

pub fn render_prof_report(
    profiler: &RuntimeProfiler,
    memory_dir: &Path,
    audit_file: &Path,
) -> String {
    let storage = collect_storage_profile(memory_dir, audit_file);
    let mut out = String::new();
    out.push_str(&format!(
        "{ANSI_BOLD}Timem runtime profiling{ANSI_RESET}\n\n"
    ));
    out.push_str(&section_title("Token 监控（per model）"));
    if profiler.models().is_empty() {
        out.push_str("  暂无模型调用。\n");
    } else {
        for (model, profile) in profiler.models() {
            out.push_str(&format!("  {}\n", model));
            out.push_str(&format!(
                "    │─ calls: {}  ||  kvc hit rate(⌁): {}\n",
                profile.llm_calls,
                format_percent(profile.cached_tokens, profile.input_tokens)
            ));
            out.push_str(&format!(
                "    └─ ▲{} (⌁{})  ▼{}  |  sec/▼1K: {}\n",
                format_count(profile.input_tokens),
                format_count(profile.cached_tokens),
                format_count(profile.output_tokens),
                format_wait_per_1k_output(profile.wait, profile.output_tokens)
            ));
        }
    }
    out.push('\n');
    out.push_str(&section_title("底层性能"));
    out.push_str(&format!(
        "  等待模型回复: {}\n",
        format_duration(profiler.model_wait())
    ));
    out.push_str(&format!(
        "  本地执行: {}\n",
        format_duration(profiler.local_work())
    ));
    out.push_str("  预留: 后续可加入 bash 执行、文件读写、增删文件等监控。\n");
    out.push('\n');
    out.push_str(&section_title("存储"));
    out.push_str(&format!(
        "  durable_mem: {} 条, {}\n",
        storage.durable_entries,
        format_bytes(storage.durable_bytes)
    ));
    out.push_str(&format!(
        "  scratch_mem: {} 条, {}\n",
        storage.scratch_entries,
        format_bytes(storage.scratch_bytes)
    ));
    out.push_str(&format!(
        "  api_audit: {}\n",
        format_bytes(storage.audit_bytes)
    ));
    out
}

fn section_title(title: &str) -> String {
    format!("{ANSI_BOLD}▸ {title}{ANSI_RESET}\n")
}

fn count_jsonl_entries(path: &Path) -> usize {
    fs::read_to_string(path)
        .map(|text| text.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

fn file_size(path: &Path) -> u64 {
    fs::metadata(path).map(|meta| meta.len()).unwrap_or(0)
}

fn format_count(value: u64) -> String {
    if value < 1_000 {
        return value.to_string();
    }
    if value < 1_000_000 {
        return trim_decimal(format!("{:.1}", value as f64 / 1_000.0)) + "K";
    }
    trim_decimal(format!("{:.2}", value as f64 / 1_000_000.0)) + "M"
}

fn format_bytes(value: u64) -> String {
    if value < 1024 {
        return format!("{} B", value);
    }
    if value < 1024 * 1024 {
        return trim_decimal(format!("{:.1}", value as f64 / 1024.0)) + " KiB";
    }
    trim_decimal(format!("{:.2}", value as f64 / 1024.0 / 1024.0)) + " MiB"
}

fn format_percent(part: u64, whole: u64) -> String {
    if whole == 0 {
        return "0%".to_string();
    }
    trim_decimal(format!("{:.1}", part as f64 * 100.0 / whole as f64)) + "%"
}

fn format_wait_per_1k_output(wait: Duration, output_tokens: u64) -> String {
    if output_tokens == 0 {
        return "n/a".to_string();
    }
    let millis = wait.as_millis() as f64 * 1000.0 / output_tokens as f64;
    format_duration(Duration::from_millis(millis.round() as u64))
}

fn format_duration(duration: Duration) -> String {
    let millis = duration.as_millis();
    if millis < 1000 {
        return format!("{} ms", millis);
    }
    trim_decimal(format!("{:.1}", millis as f64 / 1000.0)) + " s"
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
        assert_eq!(profiler.model_wait(), Duration::from_millis(1500));
        assert_eq!(profiler.local_work(), Duration::from_millis(500));

        let report = render_prof_report(
            &profiler,
            std::path::Path::new("/missing/memory"),
            std::path::Path::new("/missing/audit.jsonl"),
        );
        assert!(report.contains("\x1b[1m▸ Token 监控（per model）\x1b[0m"));
        assert!(report.contains("  aliyun:qwen-plus"));
        assert!(report.contains("│─ calls: 2  ||  kvc hit rate(⌁): 53.3%"));
        assert!(report.contains("└─ ▲1.5K (⌁800)  ▼500  |  sec/▼1K: 3 s"));
        assert!(report.contains("53.3%"));
        assert!(!report.contains("total input"));
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
        let audit = dir.join("api_audit.jsonl");
        fs::write(&memory, "{}\n\n{}\n").unwrap();
        fs::write(&scratch, "{}\n").unwrap();
        fs::write(&audit, "audit\n").unwrap();

        let profile = collect_storage_profile(&memory_dir, &audit);
        assert_eq!(profile.durable_entries, 2);
        assert_eq!(profile.scratch_entries, 1);
        assert!(profile.durable_bytes > 0);
        assert_eq!(profile.audit_bytes, 6);

        let _ = fs::remove_dir_all(dir);
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

        let report = render_prof_report(
            &profiler,
            std::path::Path::new("/missing/memory"),
            std::path::Path::new("/missing/audit.jsonl"),
        );
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
