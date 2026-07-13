use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalTimeParts {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub second: i32,
    pub weekday: i32,
}

pub fn local_time_label() -> String {
    local_time_parts()
        .map(|parts| format!("{:02}:{:02}:{:02}", parts.hour, parts.minute, parts.second))
        .unwrap_or_else(|| "00:00:00".to_string())
}

pub fn runtime_time_context() -> String {
    local_time_parts()
        .map(format_runtime_time_context)
        .unwrap_or_else(|| "local_time_unavailable".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SupportingContextInput<'a> {
    pub provider: &'a str,
    pub model: &'a str,
    pub runtime: &'a str,
    pub run_bash_target: &'a str,
}

pub fn supporting_context(input: SupportingContextInput<'_>) -> String {
    format!(
        "provider: {}, model: {}\nruntime: {}\nrun_bash_target: {}\nruntime_time: {}",
        input.provider,
        input.model,
        input.runtime,
        input.run_bash_target,
        runtime_time_context()
    )
}

pub fn turn_supporting_context(
    input: SupportingContextInput<'_>,
    additional_context: Option<&str>,
) -> String {
    let mut context = supporting_context(input);
    if let Some(extra) = additional_context
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        context.push_str("\n\n");
        context.push_str(extra);
    }
    context
}

pub fn runtime_info_context(entries: &[impl AsRef<str>]) -> Option<String> {
    let mut lines = entries
        .iter()
        .map(AsRef::as_ref)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }
    lines.insert(0, "runtime_info:".to_string());
    Some(lines.join("\n"))
}

pub fn format_supporting_context(input: SupportingContextInput<'_>, runtime_time: &str) -> String {
    format!(
        "provider: {}, model: {}\nruntime: {}\nrun_bash_target: {}\nruntime_time: {}",
        input.provider, input.model, input.runtime, input.run_bash_target, runtime_time
    )
}

pub fn format_runtime_time_context(parts: LocalTimeParts) -> String {
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02} local_time, weekday={}/{}",
        parts.year,
        parts.month,
        parts.day,
        parts.hour,
        parts.minute,
        parts.second,
        weekday_zh(parts.weekday),
        weekday_en(parts.weekday)
    )
}

pub fn local_time_parts() -> Option<LocalTimeParts> {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs() as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let ptr = unsafe { libc::localtime_r(&secs, tm.as_mut_ptr()) };
    if ptr.is_null() {
        return None;
    }
    let tm = unsafe { tm.assume_init() };
    Some(LocalTimeParts {
        year: tm.tm_year + 1900,
        month: tm.tm_mon + 1,
        day: tm.tm_mday,
        hour: tm.tm_hour,
        minute: tm.tm_min,
        second: tm.tm_sec,
        weekday: tm.tm_wday,
    })
}

pub fn weekday_zh(weekday: i32) -> &'static str {
    match weekday {
        0 => "周日",
        1 => "周一",
        2 => "周二",
        3 => "周三",
        4 => "周四",
        5 => "周五",
        6 => "周六",
        _ => "未知",
    }
}

pub fn weekday_en(weekday: i32) -> &'static str {
    match weekday {
        0 => "Sunday",
        1 => "Monday",
        2 => "Tuesday",
        3 => "Wednesday",
        4 => "Thursday",
        5 => "Friday",
        6 => "Saturday",
        _ => "Unknown",
    }
}

#[cfg(test)]
#[path = "../tests/unit/runtime_context_tests.rs"]
mod tests;
