use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ACTION_BUCKET_MS: u64 = 20;
const ACTION_LAST_BUCKET_MS: u64 = 1_000;
const LLM_BUCKET_MS: u64 = 200;
const LLM_LAST_BUCKET_MS: u64 = 30_000;
const BAR_WIDTH: usize = 20;
const MAX_LLM_RESPONSES: usize = 10;

#[derive(Debug)]
pub(crate) struct DebugStore {
    root: PathBuf,
    sessions: Mutex<BTreeMap<String, SessionDebug>>,
}

#[derive(Debug)]
struct LlmResponseDumpEntry {
    sequence: u64,
    round: u32,
    received_at_ms: u128,
    content: String,
}

#[derive(Debug, Default)]
struct SessionDebug {
    request_sequence: u64,
    response_sequence: u64,
    responses: VecDeque<LlmResponseDumpEntry>,
    started_at_ms: u128,
    updated_at_ms: u128,
    action_cpu_ns: Vec<u64>,
    action_cpu_unavailable: u64,
    llm_latency_ms: Vec<u64>,
    tools_per_response: [u64; 11],
    repairs: BTreeMap<String, u64>,
    runtime_root_repair_help: u64,
}

impl DebugStore {
    pub(crate) fn create() -> Result<Self, String> {
        #[cfg(unix)]
        let temporary_root = PathBuf::from("/tmp");
        #[cfg(not(unix))]
        let temporary_root = std::env::temp_dir();

        let root = temporary_root.join(format!("timem-debug-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root)
                .map_err(|error| format!("debug_root_cleanup_failed:{error}"))?;
        }
        create_private_dir(&root)?;
        Ok(Self {
            root,
            sessions: Mutex::new(BTreeMap::new()),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn session_dir(&self, session_id: &str) -> Result<PathBuf, String> {
        let component = safe_session_component(session_id)?;
        let dir = self.root.join(component);
        create_private_dir(&dir)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "debug_store_poisoned".to_string())?;
        let now = now_ms();
        let inserted = match sessions.entry(session_id.to_string()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(SessionDebug {
                    started_at_ms: now,
                    updated_at_ms: now,
                    ..SessionDebug::default()
                });
                true
            }
            std::collections::btree_map::Entry::Occupied(_) => false,
        };
        drop(sessions);
        if inserted {
            self.render_statistics(session_id)?;
            self.render_llm_responses(session_id)?;
        }
        Ok(dir)
    }

    pub(crate) fn record_prompt(
        &self,
        session_id: &str,
        round: u32,
        prompt: &str,
    ) -> Result<(), String> {
        let dir = self.session_dir(session_id)?;
        let (sequence, generated_at_ms) = {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get_mut(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            stats.request_sequence = stats.request_sequence.saturating_add(1);
            stats.updated_at_ms = now_ms();
            (stats.request_sequence, stats.updated_at_ms)
        };
        let mut body = String::new();
        body.push_str("TIMEM LLM PROMPT DUMP\n");
        body.push_str(&format!("session_id: {session_id}\n"));
        body.push_str(&format!("request_sequence: {sequence}\n"));
        body.push_str(&format!("round: {round}\n"));
        body.push_str(&format!(
            "generated_at: {}\n",
            format_timestamp_ms(generated_at_ms)
        ));
        body.push_str(&format!("content_bytes: {}\n", prompt.len()));
        body.push_str("\n==================== MODEL INPUT ====================\n");
        body.push_str(prompt);
        if !prompt.ends_with('\n') {
            body.push('\n');
        }
        body.push_str("================== END MODEL INPUT ==================\n");
        atomic_private_write(&dir.join("llm_prompt.dump"), body.as_bytes())
    }

    pub(crate) fn record_llm_response(
        &self,
        session_id: &str,
        round: u32,
        content: &str,
    ) -> Result<(), String> {
        self.session_dir(session_id)?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get_mut(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            stats.response_sequence = stats.response_sequence.saturating_add(1);
            let received_at_ms = now_ms();
            stats.responses.push_front(LlmResponseDumpEntry {
                sequence: stats.response_sequence,
                round,
                received_at_ms,
                content: content.to_string(),
            });
            stats.responses.truncate(MAX_LLM_RESPONSES);
            stats.updated_at_ms = received_at_ms;
        }
        self.render_llm_responses(session_id)
    }

    pub(crate) fn record_llm_latency(
        &self,
        session_id: &str,
        latency: Duration,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            stats
                .llm_latency_ms
                .push(latency.as_millis().min(u64::MAX as u128) as u64);
        })
    }

    pub(crate) fn record_tools_per_response(
        &self,
        session_id: &str,
        count: usize,
    ) -> Result<(), String> {
        self.update(session_id, |stats| {
            stats.tools_per_response[count.min(10)] =
                stats.tools_per_response[count.min(10)].saturating_add(1);
        })
    }

    pub(crate) fn record_runtime_root_repair_help(&self, session_id: &str) -> Result<(), String> {
        self.update(session_id, |stats| {
            stats.runtime_root_repair_help = stats.runtime_root_repair_help.saturating_add(1);
        })
    }

    pub(crate) fn record_repair(&self, session_id: &str, issue: &str) -> Result<(), String> {
        let category = normalize_repair_category(issue);
        self.update(session_id, |stats| {
            let count = stats.repairs.entry(category).or_default();
            *count = count.saturating_add(1);
        })
    }

    pub(crate) fn record_action_cpu(
        &self,
        session_id: &str,
        cpu_time: Option<Duration>,
    ) -> Result<(), String> {
        self.update(session_id, |stats| match cpu_time {
            Some(duration) => stats
                .action_cpu_ns
                .push(duration.as_nanos().min(u64::MAX as u128) as u64),
            None => stats.action_cpu_unavailable = stats.action_cpu_unavailable.saturating_add(1),
        })
    }

    pub(crate) fn cleanup(&self) -> Result<(), String> {
        match fs::remove_dir_all(&self.root) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!("debug_root_remove_failed:{error}")),
        }
    }

    fn update(
        &self,
        session_id: &str,
        update: impl FnOnce(&mut SessionDebug),
    ) -> Result<(), String> {
        self.session_dir(session_id)?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get_mut(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            update(stats);
            stats.updated_at_ms = now_ms();
        }
        self.render_statistics(session_id)
    }

    fn render_statistics(&self, session_id: &str) -> Result<(), String> {
        let dir = self.root.join(safe_session_component(session_id)?);
        let body = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            render_statistics_markdown(session_id, stats)
        };
        atomic_private_write(&dir.join("statistics.md"), body.as_bytes())
    }

    fn render_llm_responses(&self, session_id: &str) -> Result<(), String> {
        let dir = self.root.join(safe_session_component(session_id)?);
        let body = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "debug_store_poisoned".to_string())?;
            let stats = sessions
                .get(session_id)
                .ok_or_else(|| "debug_session_not_found".to_string())?;
            render_llm_response_dump(session_id, &stats.responses)
        };
        atomic_private_write(&dir.join("llm_response.dump"), body.as_bytes())
    }
}

fn render_llm_response_dump(
    session_id: &str,
    responses: &VecDeque<LlmResponseDumpEntry>,
) -> String {
    let mut out = String::new();
    out.push_str("TIMEM LLM RESPONSE DUMP\n");
    out.push_str(&format!("session_id: {session_id}\n"));
    out.push_str(&format!("retained_responses: {}\n", responses.len()));
    out.push_str("order: newest_to_oldest\n");

    if responses.is_empty() {
        out.push_str("\n(no model responses recorded)\n");
        return out;
    }

    for response in responses {
        out.push('\n');
        out.push_str("============================================================\n");
        out.push_str(&format!("response_sequence: {}\n", response.sequence));
        out.push_str(&format!("round: {}\n", response.round));
        out.push_str(&format!(
            "received_at: {}\n",
            format_timestamp_ms(response.received_at_ms)
        ));
        out.push_str(&format!("content_bytes: {}\n", response.content.len()));
        out.push_str("------------------------------------------------------------\n");
        out.push_str(&response.content);
        if !response.content.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("====================== END RESPONSE ========================\n");
    }
    out
}

fn render_statistics_markdown(session_id: &str, stats: &SessionDebug) -> String {
    let action_ms = stats
        .action_cpu_ns
        .iter()
        .map(|value| value / 1_000_000)
        .collect::<Vec<_>>();
    let action_total_ns = stats.action_cpu_ns.iter().copied().sum::<u64>();
    let llm_total_ms = stats.llm_latency_ms.iter().copied().sum::<u64>();
    let repair_total = stats.repairs.values().copied().sum::<u64>();
    let mut out = "# Timem Session Statistics\n\n".to_string();
    let summary_rows = vec![
        vec!["Session".to_string(), format!("`{session_id}`")],
        vec![
            "Started".to_string(),
            format_timestamp_ms(stats.started_at_ms),
        ],
        vec![
            "Updated".to_string(),
            format_timestamp_ms(stats.updated_at_ms),
        ],
        vec![
            "LLM requests dumped".to_string(),
            stats.request_sequence.to_string(),
        ],
        vec![
            "Successful LLM API requests".to_string(),
            stats.llm_latency_ms.len().to_string(),
        ],
        vec![
            "Measured actions".to_string(),
            stats.action_cpu_ns.len().to_string(),
        ],
        vec![
            "Action CPU unavailable".to_string(),
            stats.action_cpu_unavailable.to_string(),
        ],
        vec!["Protocol repairs".to_string(), repair_total.to_string()],
        vec![
            "runtime_root_repair_help".to_string(),
            stats.runtime_root_repair_help.to_string(),
        ],
    ];
    render_markdown_table(
        &mut out,
        &["Metric", "Value"],
        &[false, true],
        &summary_rows,
    );

    out.push_str("\n## Action on-CPU time\n\n");
    let action_metric_rows = vec![
        vec![
            "Total on-CPU time".to_string(),
            format_duration_ns(action_total_ns),
        ],
        vec![
            "Mean".to_string(),
            format_mean_ns(action_total_ns, stats.action_cpu_ns.len()),
        ],
        vec![
            "Max".to_string(),
            stats
                .action_cpu_ns
                .iter()
                .copied()
                .max()
                .map(format_duration_ns)
                .unwrap_or_else(|| "n/a".to_string()),
        ],
    ];
    render_markdown_table(
        &mut out,
        &["Metric", "Value"],
        &[false, true],
        &action_metric_rows,
    );
    out.push('\n');
    let action_counts = fixed_histogram(&action_ms, ACTION_BUCKET_MS, ACTION_LAST_BUCKET_MS);
    render_histogram_slice(&mut out, "On-CPU time", &action_counts, |index| {
        if index + 1 == action_counts.len() {
            "1s+".to_string()
        } else {
            format!(
                "{}–{} ms",
                index as u64 * ACTION_BUCKET_MS,
                (index as u64 + 1) * ACTION_BUCKET_MS
            )
        }
    });

    out.push_str("\n## LLM API latency\n\n");
    let llm_metric_rows = vec![
        vec![
            "Total successful API time".to_string(),
            format_duration_ms(llm_total_ms),
        ],
        vec![
            "Mean".to_string(),
            format_mean_ms(llm_total_ms, stats.llm_latency_ms.len()),
        ],
        vec![
            "Max".to_string(),
            stats
                .llm_latency_ms
                .iter()
                .copied()
                .max()
                .map(format_duration_ms)
                .unwrap_or_else(|| "n/a".to_string()),
        ],
    ];
    render_markdown_table(
        &mut out,
        &["Metric", "Value"],
        &[false, true],
        &llm_metric_rows,
    );
    out.push('\n');
    let llm_counts = fixed_histogram(&stats.llm_latency_ms, LLM_BUCKET_MS, LLM_LAST_BUCKET_MS);
    render_histogram_slice(&mut out, "API latency", &llm_counts, |index| {
        if index + 1 == llm_counts.len() {
            "30s+".to_string()
        } else {
            format!(
                "{}–{} ms",
                index as u64 * LLM_BUCKET_MS,
                (index as u64 + 1) * LLM_BUCKET_MS
            )
        }
    });

    out.push_str("\n## Tools per LLM response\n\n");
    render_histogram_slice(&mut out, "Tools", &stats.tools_per_response, |index| {
        if index == 10 {
            "10+".to_string()
        } else {
            index.to_string()
        }
    });

    out.push_str("\n## Protocol repairs by category\n\n");
    let repair_rows = stats
        .repairs
        .iter()
        .map(|(category, count)| (category.clone(), *count))
        .collect::<Vec<_>>();
    render_named_histogram(&mut out, "Repair category", &repair_rows);
    out.push_str("\nNetwork failures, timeouts, retries, and cancellations are excluded.\n");
    out
}

fn fixed_histogram(values: &[u64], width: u64, last_start: u64) -> Vec<u64> {
    let normal = (last_start / width) as usize;
    let mut counts = vec![0_u64; normal + 1];
    for value in values {
        let index = if *value >= last_start {
            normal
        } else {
            (*value / width) as usize
        };
        counts[index] = counts[index].saturating_add(1);
    }
    counts
}

fn render_histogram_slice(
    out: &mut String,
    label: &str,
    counts: &[u64],
    name: impl Fn(usize) -> String,
) {
    let max = counts.iter().copied().max().unwrap_or(0);
    let rows = counts
        .iter()
        .copied()
        .enumerate()
        .map(|(index, count)| vec![name(index), count.to_string(), bar(count, max)])
        .collect::<Vec<_>>();
    render_markdown_table(
        out,
        &[label, "Count", "Distribution"],
        &[false, true, false],
        &rows,
    );
}

fn render_named_histogram(out: &mut String, label: &str, rows: &[(String, u64)]) {
    let max = rows.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let rows = if rows.is_empty() {
        vec![vec!["none".to_string(), "0".to_string(), String::new()]]
    } else {
        rows.iter()
            .map(|(name, count)| vec![name.clone(), count.to_string(), bar(*count, max)])
            .collect()
    };
    render_markdown_table(
        out,
        &[label, "Count", "Distribution"],
        &[false, true, false],
        &rows,
    );
}

fn render_markdown_table(
    out: &mut String,
    headers: &[&str],
    right_aligned: &[bool],
    rows: &[Vec<String>],
) {
    debug_assert_eq!(headers.len(), right_aligned.len());
    let mut widths = headers
        .iter()
        .map(|header| header.chars().count())
        .collect::<Vec<_>>();
    for row in rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }

    write_markdown_row(out, headers.iter().copied(), &widths, right_aligned);
    let separators = widths
        .iter()
        .enumerate()
        .map(|(index, width)| {
            if right_aligned[index] {
                format!("{}:", "-".repeat((*width).max(2) - 1))
            } else {
                format!(":{}", "-".repeat((*width).max(2) - 1))
            }
        })
        .collect::<Vec<_>>();
    write_markdown_row(
        out,
        separators.iter().map(String::as_str),
        &widths,
        right_aligned,
    );
    for row in rows {
        write_markdown_row(out, row.iter().map(String::as_str), &widths, right_aligned);
    }
}

fn write_markdown_row<'a>(
    out: &mut String,
    cells: impl IntoIterator<Item = &'a str>,
    widths: &[usize],
    right_aligned: &[bool],
) {
    out.push('|');
    for (index, cell) in cells.into_iter().enumerate() {
        let padding = widths[index].saturating_sub(cell.chars().count());
        out.push(' ');
        if right_aligned[index] {
            out.push_str(&" ".repeat(padding));
            out.push_str(cell);
        } else {
            out.push_str(cell);
            out.push_str(&" ".repeat(padding));
        }
        out.push_str(" |");
    }
    out.push('\n');
}

fn bar(count: u64, max: u64) -> String {
    if count == 0 || max == 0 {
        return String::new();
    }
    let width = (count as usize * BAR_WIDTH).div_ceil(max as usize);
    "█".repeat(width.max(1))
}

fn normalize_repair_category(issue: &str) -> String {
    let lower = issue.trim().to_ascii_lowercase();
    if lower.contains("truncated") || lower.contains("empty") {
        "empty_or_truncated_response"
    } else if lower.contains("response_root") || lower.contains("root") {
        "missing_or_invalid_response_root"
    } else if lower.contains("xml") || lower.contains("close_tag") {
        "invalid_xml"
    } else if lower.contains("parallel") {
        "invalid_parallel"
    } else if lower.contains("action") || lower.contains("tool") {
        "invalid_action"
    } else if lower.contains("finish_confirm") {
        "invalid_finish_confirm"
    } else if lower.contains("branch")
        || lower.contains("status")
        || lower.contains("final_answer")
        || lower.contains("next_actions")
    {
        "invalid_branch"
    } else {
        "unknown_protocol_error"
    }
    .to_string()
}

fn format_duration_ns(ns: u64) -> String {
    if ns >= 1_000_000_000 {
        format!("{:.3} s", ns as f64 / 1_000_000_000.0)
    } else {
        format!("{:.3} ms", ns as f64 / 1_000_000.0)
    }
}

fn format_mean_ns(total: u64, count: usize) -> String {
    if count == 0 {
        "n/a".to_string()
    } else {
        format_duration_ns(total / count as u64)
    }
}

fn format_duration_ms(ms: u64) -> String {
    if ms >= 1_000 {
        format!("{:.3} s", ms as f64 / 1_000.0)
    } else {
        format!("{ms} ms")
    }
}

fn format_mean_ms(total: u64, count: usize) -> String {
    if count == 0 {
        "n/a".to_string()
    } else {
        format_duration_ms(total / count as u64)
    }
}

fn safe_session_component(session_id: &str) -> Result<&str, String> {
    if !session_id.is_empty()
        && session_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        Ok(session_id)
    } else {
        Err("invalid_debug_session_id".to_string())
    }
}

fn create_private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| format!("debug_dir_create_failed:{error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("debug_dir_permissions_failed:{error}"))?;
    }
    Ok(())
}

fn atomic_private_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let suffix = now_ms();
    let temporary = path.with_extension(format!("tmp-{}-{suffix}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("debug_file_open_failed:{error}"))?;
    if let Err(error) = file.write_all(bytes).and_then(|_| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("debug_file_write_failed:{error}"));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("debug_file_replace_failed:{error}"));
    }
    Ok(())
}

fn format_timestamp_ms(timestamp_ms: u128) -> String {
    use chrono::{DateTime, Local, SecondsFormat, Utc};

    let Ok(timestamp_ms) = i64::try_from(timestamp_ms) else {
        return "invalid timestamp".to_string();
    };
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|timestamp| {
            timestamp
                .with_timezone(&Local)
                .to_rfc3339_opts(SecondsFormat::Millis, false)
        })
        .unwrap_or_else(|| "invalid timestamp".to_string())
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    static DEBUG_STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn timestamps_are_human_readable_rfc3339_with_milliseconds() {
        let formatted = format_timestamp_ms(1_787_066_216_855);
        let parsed = chrono::DateTime::parse_from_rfc3339(&formatted)
            .expect("formatted timestamp should be RFC 3339");
        assert_eq!(parsed.timestamp_millis(), 1_787_066_216_855);
        assert!(formatted.contains('T'));
        assert!(formatted.contains(".855"));
    }

    #[test]
    fn histograms_keep_fixed_last_bucket() {
        assert_eq!(fixed_histogram(&[19, 20, 999, 1_000], 20, 1_000)[50], 1);
        assert_eq!(
            fixed_histogram(&[199, 200, 29_999, 30_000], 200, 30_000)[150],
            1
        );
    }

    #[test]
    fn markdown_tables_are_aligned() {
        let mut stats = SessionDebug {
            started_at_ms: 1,
            updated_at_ms: 2,
            ..SessionDebug::default()
        };
        stats.tools_per_response[0] = 1;
        let markdown = render_statistics_markdown("session_1", &stats);
        assert!(markdown.contains("| Started"));
        assert!(markdown.contains("| Updated"));
        assert!(!markdown.contains("Unix ms"));
        assert!(!markdown.contains("Started (Unix ms)"));
        assert!(!markdown.contains("Updated (Unix ms)"));

        let tool_table = markdown
            .split("## Tools per LLM response\n\n")
            .nth(1)
            .expect("tools table")
            .split("\n\n")
            .next()
            .expect("tools table body");
        let rows = tool_table.lines().collect::<Vec<_>>();
        assert!(rows.len() >= 13);
        let pipe_columns = rows[0]
            .chars()
            .enumerate()
            .filter_map(|(index, ch)| (ch == '|').then_some(index))
            .collect::<Vec<_>>();
        assert!(
            rows.iter().all(|row| {
                row.chars()
                    .enumerate()
                    .filter_map(|(index, ch)| (ch == '|').then_some(index))
                    .collect::<Vec<_>>()
                    == pipe_columns
            }),
            "Markdown source columns should be padded to the same visible positions:\n{tool_table}"
        );

        let cells = |line: &str| {
            line.split('|')
                .map(str::trim)
                .filter(|cell| !cell.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert!(markdown.lines().any(|line| {
            let row = cells(line);
            row.first().map(String::as_str) == Some("10+")
                && row.get(1).map(String::as_str) == Some("0")
        }));
        assert!(markdown.lines().any(|line| {
            let row = cells(line);
            row.first().map(String::as_str) == Some("30s+")
                && row.get(1).map(String::as_str) == Some("0")
        }));
        assert!(markdown.lines().any(|line| {
            let row = cells(line);
            row.first().map(String::as_str) == Some("1s+")
                && row.get(1).map(String::as_str) == Some("0")
        }));
    }

    #[test]
    fn prompt_dump_replaces_the_previous_request_and_cleanup_removes_root() {
        let _guard = DEBUG_STORE_TEST_LOCK
            .lock()
            .expect("lock DebugStore filesystem test");
        let store = DebugStore::create().expect("create debug store");
        let root = store.root().to_path_buf();
        #[cfg(unix)]
        assert_eq!(
            root,
            PathBuf::from(format!("/tmp/timem-debug-{}", std::process::id()))
        );
        let session_dir = store
            .session_dir("session_test")
            .expect("create session dir");

        store
            .record_prompt("session_test", 1, "first\nprompt")
            .expect("write first prompt");
        let xml_prompt =
            "second\n\n<prompt_delta id=\"pd_2\" time_ms=\"2\">\n<ASSISTANT><free_talk>validated XML</free_talk></ASSISTANT>\n</prompt_delta>";
        store
            .record_prompt("session_test", 2, xml_prompt)
            .expect("replace prompt");

        let dump =
            fs::read_to_string(session_dir.join("llm_prompt.dump")).expect("read prompt dump");
        assert!(dump.contains("request_sequence: 2"));
        assert!(dump.contains("round: 2"));
        assert!(dump.contains("generated_at: "));
        assert!(!dump.contains("generated_at_ms:"));
        assert!(dump.contains(xml_prompt));
        assert!(dump.contains("<ASSISTANT><free_talk>validated XML</free_talk></ASSISTANT>"));
        assert!(!dump.contains("&lt;ASSISTANT&gt;"));
        assert!(!dump.contains("first\nprompt"));
        assert!(session_dir.join("statistics.md").is_file());
        assert!(session_dir.join("llm_response.dump").is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&root).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(session_dir.join("llm_prompt.dump"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(session_dir.join("llm_response.dump"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        store.cleanup().expect("cleanup debug root");
        assert!(!root.exists());
    }

    #[test]
    fn llm_response_dump_keeps_newest_ten_in_reverse_chronological_order() {
        let _guard = DEBUG_STORE_TEST_LOCK
            .lock()
            .expect("lock DebugStore filesystem test");
        let store = DebugStore::create().expect("create debug store");
        let root = store.root().to_path_buf();
        let session_dir = store
            .session_dir("session_responses")
            .expect("create response session");

        for index in 1..=12 {
            store
                .record_llm_response(
                    "session_responses",
                    index,
                    &format!("response-{index}\nsecond line {index}"),
                )
                .expect("record model response");
        }

        let dump =
            fs::read_to_string(session_dir.join("llm_response.dump")).expect("read response dump");
        assert!(dump.contains("retained_responses: 10"));
        assert!(dump.contains("order: newest_to_oldest"));
        assert!(dump.contains("received_at: "));
        assert!(!dump.contains("received_at_ms:"));
        assert!(dump.contains("content_bytes:"));
        assert!(dump.contains("response-12\nsecond line 12"));
        assert!(dump.contains("response-3\nsecond line 3"));
        assert!(!dump.contains("response-2\nsecond line 2"));
        assert!(!dump.contains("response-1\nsecond line 1"));

        let newest = dump.find("response_sequence: 12").expect("newest response");
        let next = dump.find("response_sequence: 11").expect("second response");
        let oldest = dump
            .find("response_sequence: 3")
            .expect("oldest retained response");
        assert!(newest < next);
        assert!(next < oldest);
        assert_eq!(dump.matches("response_sequence:").count(), 10);
        assert_eq!(
            dump.matches("====================== END RESPONSE").count(),
            10
        );

        store.cleanup().expect("cleanup response debug root");
        assert!(!root.exists());
    }
}
