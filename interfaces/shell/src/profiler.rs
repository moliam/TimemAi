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
#[path = "../tests/unit/profiler_tests.rs"]
mod tests;
