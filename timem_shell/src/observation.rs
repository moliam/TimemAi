use agent_core::{CoreActionKind, CoreTopicEvent};
use std::collections::VecDeque;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_RESET: &str = "\x1b[0m";
const ACTIVE_TEXT_COLORS: [&str; 3] = ["\x1b[38;5;245m", "\x1b[38;5;250m", "\x1b[38;5;255m"];
const OBSERVATION_LINE_PREFIX: &str = "· ";
const OBSERVATION_CHILD_MID_PREFIX: &str = "  ├─ ";
const OBSERVATION_CHILD_LAST_PREFIX: &str = "  └─ ";
const MAX_WRAPPED_LINES_PER_ITEM: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationEvent {
    Persistent(String),
    Active(String),
    PersistentChild {
        text: String,
        is_last: bool,
    },
    ActiveChild {
        text: String,
        is_last: bool,
    },
    Transient(String),
    EnsureTransient(String),
    FinishTransient(String),
    ClearTransient,
    SettleActive,
    ActiveWithTimer {
        text: String,
        timer: ActionTimer,
    },
    UpdateActionStatus {
        active_text: String,
        status_text: String,
    },
    ActiveChildWithTimer {
        text: String,
        is_last: bool,
        timer: ActionTimer,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationLineStyle {
    Normal,
    ActiveBlink,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionTimer {
    pub started_at_ms: u64,
    pub timeout_ms: Option<u64>,
    pub loop_timeout_ms: Option<u64>,
    pub interval_ms: Option<u64>,
    pub once_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationLine {
    pub text: String,
    pub style: ObservationLineStyle,
    pub prefix: String,
    pub timer: Option<ActionTimer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationPanel {
    lines: VecDeque<ObservationLine>,
    transients: Vec<TransientObservation>,
    max_lines: usize,
    max_width: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TransientObservation {
    text: String,
    count: usize,
}

impl Default for ObservationPanel {
    fn default() -> Self {
        Self::new(20, 72)
    }
}

impl ObservationPanel {
    pub fn new(max_lines: usize, max_width: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            transients: Vec::new(),
            max_lines: max_lines.max(1),
            max_width: max_width.max(10),
        }
    }

    pub fn set_max_width(&mut self, max_width: usize) {
        self.max_width = max_width.max(10);
    }

    pub fn apply(&mut self, event: ObservationEvent) {
        match event {
            ObservationEvent::Persistent(text) => {
                let prefix = if text.starts_with("💡 ") || text.starts_with("⚙️ ") {
                    String::new()
                } else {
                    OBSERVATION_LINE_PREFIX.to_string()
                };
                self.push_line(text, ObservationLineStyle::Normal, prefix);
            }
            ObservationEvent::Active(text) => self.push_line(
                text,
                ObservationLineStyle::ActiveBlink,
                OBSERVATION_LINE_PREFIX.to_string(),
            ),
            ObservationEvent::PersistentChild { text, is_last } => self.push_line(
                text,
                ObservationLineStyle::Normal,
                child_prefix(is_last).to_string(),
            ),
            ObservationEvent::ActiveChild { text, is_last } => self.push_line(
                text,
                ObservationLineStyle::ActiveBlink,
                child_prefix(is_last).to_string(),
            ),
            ObservationEvent::Transient(text) => self.increment_transient(text),
            ObservationEvent::EnsureTransient(text) => self.ensure_transient(text),
            ObservationEvent::FinishTransient(text) => self.decrement_transient(&text),
            ObservationEvent::ClearTransient => self.transients.clear(),
            ObservationEvent::SettleActive => {
                for line in &mut self.lines {
                    if line.style == ObservationLineStyle::ActiveBlink {
                        line.style = ObservationLineStyle::Normal;
                        line.timer = None;
                    }
                }
            }
            ObservationEvent::ActiveWithTimer { text, timer } => {
                self.push_line_with_timer(
                    text,
                    ObservationLineStyle::ActiveBlink,
                    OBSERVATION_LINE_PREFIX.to_string(),
                    Some(timer),
                );
            }
            ObservationEvent::UpdateActionStatus {
                active_text,
                status_text,
            } => self.update_action_status(&active_text, status_text),
            ObservationEvent::ActiveChildWithTimer {
                text,
                is_last,
                timer,
            } => {
                self.push_line_with_timer(
                    text,
                    ObservationLineStyle::ActiveBlink,
                    child_prefix(is_last).to_string(),
                    Some(timer),
                );
            }
        }
    }

    fn update_action_status(&mut self, active_text: &str, status_text: String) {
        if status_text.trim().is_empty() {
            return;
        }
        if let Some(line) = self
            .lines
            .iter_mut()
            .rev()
            .find(|line| line.text == active_text)
        {
            line.text = status_text;
            line.style = ObservationLineStyle::Normal;
            line.timer = None;
            return;
        }
        self.push_line(
            status_text,
            ObservationLineStyle::Normal,
            OBSERVATION_LINE_PREFIX.to_string(),
        );
    }

    pub fn apply_all(&mut self, events: impl IntoIterator<Item = ObservationEvent>) {
        for event in events {
            self.apply(event);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.transients.is_empty()
    }

    fn push_line(&mut self, text: String, style: ObservationLineStyle, prefix: String) {
        self.push_line_with_timer(text, style, prefix, None);
    }

    fn push_line_with_timer(
        &mut self,
        text: String,
        style: ObservationLineStyle,
        prefix: String,
        timer: Option<ActionTimer>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        self.lines.push_back(ObservationLine {
            text,
            style,
            prefix,
            timer,
        });
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
        }
    }

    fn visible_lines(&self) -> Vec<ObservationLine> {
        let mut lines: Vec<ObservationLine> = self.lines.iter().cloned().collect();
        for transient in &self.transients {
            lines.push(ObservationLine {
                text: transient_label(transient),
                style: ObservationLineStyle::ActiveBlink,
                prefix: OBSERVATION_LINE_PREFIX.to_string(),
                timer: None,
            });
        }
        if lines.len() > self.max_lines {
            let overflow = lines.len() - self.max_lines;
            lines.drain(0..overflow);
        }
        lines
    }

    fn increment_transient(&mut self, text: String) {
        let normalized = text.trim();
        if normalized.is_empty() {
            return;
        }
        if let Some(transient) = self
            .transients
            .iter_mut()
            .find(|transient| transient.text == normalized)
        {
            transient.count = transient.count.saturating_add(1);
            return;
        }
        self.transients.push(TransientObservation {
            text: normalized.to_string(),
            count: 1,
        });
    }

    fn ensure_transient(&mut self, text: String) {
        let normalized = text.trim();
        if normalized.is_empty()
            || self
                .transients
                .iter()
                .any(|transient| transient.text == normalized)
        {
            return;
        }
        self.transients.push(TransientObservation {
            text: normalized.to_string(),
            count: 1,
        });
    }

    fn decrement_transient(&mut self, text: &str) {
        let normalized = text.trim();
        if normalized.is_empty() {
            return;
        }
        let Some(index) = self
            .transients
            .iter()
            .position(|transient| transient.text == normalized)
        else {
            return;
        };
        if self.transients[index].count <= 1 {
            self.transients.remove(index);
        } else {
            self.transients[index].count -= 1;
        }
    }
}

fn child_prefix(is_last: bool) -> &'static str {
    if is_last {
        OBSERVATION_CHILD_LAST_PREFIX
    } else {
        OBSERVATION_CHILD_MID_PREFIX
    }
}

fn transient_label(transient: &TransientObservation) -> String {
    if transient.count <= 1 {
        transient.text.clone()
    } else {
        format!("{} x{}", transient.text, transient.count)
    }
}

fn format_countdown(timer: &ActionTimer, now_ms: u64) -> String {
    let elapsed_ms = now_ms.saturating_sub(timer.started_at_ms);

    if let (Some(loop_timeout), Some(interval_ms)) = (timer.loop_timeout_ms, timer.interval_ms) {
        let total_remaining = loop_timeout.saturating_sub(elapsed_ms);
        if interval_ms == 0 {
            return format!(
                "⏱ ↻{}",
                format_duration_short_unpadded(total_remaining, true)
            );
        }
        let interval_remaining = interval_ms.saturating_sub(elapsed_ms % interval_ms);
        if interval_ms < 2000 {
            return format!(
                "⏱ ↻{}",
                format_duration_short_unpadded(total_remaining, true)
            );
        }
        return format!(
            "⏱ {}/{}",
            format_interval_countdown(interval_remaining),
            format_duration_short_unpadded(total_remaining, true)
        );
    }

    if let Some(timeout) = timer.timeout_ms {
        let remaining = timeout.saturating_sub(elapsed_ms);
        return format!("⏱ {}", format_duration_short(remaining, true));
    }
    String::new()
}

fn format_spinner_label(tick: usize) -> String {
    match tick % 3 {
        0 => "[.  ]".to_string(),
        1 => "[.. ]".to_string(),
        _ => "[...]".to_string(),
    }
}

fn bracket_label(label: &str) -> String {
    if label.starts_with('[') {
        label.to_string()
    } else {
        format!("[{label}]")
    }
}

fn inject_action_activity(text: &str, activity: &str) -> Option<String> {
    let activity = bracket_label(activity);
    if let Some(rest) = text.strip_prefix('`') {
        if let Some(command) = rest.strip_suffix('`') {
            return Some(format!("`{activity} {command}`"));
        }
    }
    if let Some(rest) = text.strip_prefix("**(后台执行)** `") {
        if let Some(command) = rest.strip_suffix('`') {
            return Some(format!("**(后台执行)** `{activity} {command}`"));
        }
    }
    None
}

fn format_interval_countdown(ms: u64) -> String {
    let secs = ms.saturating_add(999) / 1000;
    secs.saturating_sub(1).to_string()
}

fn format_duration_short(ms: u64, suffix_seconds: bool) -> String {
    let mut secs = ms.saturating_add(999) / 1000;
    if ms > 0 && secs == 0 {
        secs = 1;
    }
    if secs < 60 {
        if suffix_seconds {
            format!("{secs:02}s")
        } else {
            format!("{secs:02}")
        }
    } else if secs < 3600 {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

fn format_duration_short_unpadded(ms: u64, suffix_seconds: bool) -> String {
    let secs = ms.saturating_add(999) / 1000;
    if secs < 60 {
        if suffix_seconds {
            format!("{secs}s")
        } else {
            secs.to_string()
        }
    } else if secs < 3600 {
        format!("{:02}:{:02}", secs / 60, secs % 60)
    } else {
        format!("{}:{:02}:{:02}", secs / 3600, (secs % 3600) / 60, secs % 60)
    }
}

pub fn render_observation_panel(panel: &ObservationPanel) -> String {
    render_observation_panel_at(panel, 0)
}

pub fn render_observation_panel_at(panel: &ObservationPanel, tick: usize) -> String {
    render_observation_panel_at_with_elapsed(panel, tick, None)
}

pub fn render_observation_panel_at_with_elapsed(
    panel: &ObservationPanel,
    tick: usize,
    elapsed_label: Option<&str>,
) -> String {
    if panel.is_empty() {
        return String::new();
    }
    let active_color = ACTIVE_TEXT_COLORS[tick % ACTIVE_TEXT_COLORS.len()];
    let content_width = panel.max_width.saturating_sub(2).max(8);
    let title = elapsed_label
        .map(|elapsed| format!(" Thought / Action  ⏳ {elapsed} "))
        .unwrap_or_else(|| " Thought / Action ".to_string());
    let mut out = String::new();
    out.push_str(ANSI_BOLD);
    out.push('┏');
    out.push('━');
    out.push_str(&title);
    out.push_str(&"━".repeat(content_width.saturating_sub(display_width(&title) + 1)));
    out.push('┓');
    out.push_str(ANSI_RESET);
    out.push('\n');
    let line_width = content_width.saturating_sub(2);
    let mut render_rows = Vec::new();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    for line in panel.visible_lines() {
        let activity = if line.style == ObservationLineStyle::ActiveBlink {
            line.timer
                .as_ref()
                .map(|t| format_countdown(t, now_ms))
                .unwrap_or_else(|| format_spinner_label(tick))
        } else {
            String::new()
        };
        let line_text = if activity.is_empty() {
            line.text.clone()
        } else {
            inject_action_activity(&line.text, &activity).unwrap_or_else(|| line.text.clone())
        };
        let marker_extra_width = terminal_marker_extra_width(&line.text);
        let text_width = line_width
            .saturating_sub(display_width(&line.prefix))
            .saturating_sub(marker_extra_width);
        let rendered_lines =
            render_markdown_lines_limited(&line_text, text_width, MAX_WRAPPED_LINES_PER_ITEM);
        for (idx, rendered) in rendered_lines.into_iter().enumerate() {
            let prefix = if idx == 0 {
                line.prefix.clone()
            } else {
                " ".repeat(display_width(&line.prefix))
            };
            render_rows.push((line.style, format!("{prefix}{rendered}")));
        }
    }
    if render_rows.len() > panel.max_lines {
        let overflow = render_rows.len() - panel.max_lines;
        render_rows.drain(0..overflow);
    }
    for (style, content) in render_rows {
        let padded = pad_display_width(&content, line_width);
        out.push('┃');
        out.push(' ');
        match style {
            ObservationLineStyle::Normal => out.push_str(&padded),
            ObservationLineStyle::ActiveBlink => {
                out.push_str(active_color);
                out.push_str(&padded);
                out.push_str(ANSI_RESET);
            }
        }
        out.push(' ');
        out.push('┃');
        out.push('\n');
    }
    out.push_str(ANSI_BOLD);
    out.push('┗');
    out.push_str(&"━".repeat(content_width));
    out.push('┛');
    out.push_str(ANSI_RESET);
    out.push('\n');
    out
}

pub fn observation_panel_width_for_terminal(terminal_width: usize) -> usize {
    let window = terminal_width.max(1);
    let eighty_percent = window.saturating_mul(80) / 100;
    if eighty_percent >= 80 {
        eighty_percent
    } else if window > 80 {
        80
    } else {
        window
    }
}

pub fn observation_events_from_core_topic_events(
    events: &[CoreTopicEvent],
) -> Vec<ObservationEvent> {
    let mut observations = Vec::new();
    for event in events {
        if let Some(model_response) = event.as_model_response() {
            let free_talk = model_response.free_talk.trim();
            if !free_talk.is_empty() {
                observations.push(ObservationEvent::Persistent(format!("💡 {free_talk}")));
            }
            if event.payload.get("global").is_some() {
                if model_response.global.working_worker_count > 0 {
                    observations.push(ObservationEvent::EnsureTransient("思考中...".to_string()));
                } else {
                    observations.push(ObservationEvent::FinishTransient("思考中...".to_string()));
                }
            }
            continue;
        }
        if let Some(repair) = event.as_model_repair() {
            let attempt = repair.attempt.max(1);
            let max_attempts = repair.max_attempts.max(attempt);
            observations.push(ObservationEvent::Persistent(format!(
                "⚠️ 模型回复偏离协议，重试 ({attempt}/{max_attempts})..."
            )));
            observations.push(ObservationEvent::EnsureTransient("思考中...".to_string()));
            continue;
        }
        if event.as_work_instruction_load().is_some() {
            continue;
        }
        if let Some(action) = event.as_action() {
            let (detail_text, timer) = action_detail_for_shell(&action.kind);
            if action.event == "finish" {
                observations.push(ObservationEvent::UpdateActionStatus {
                    active_text: detail_text,
                    status_text: action_status_detail_for_shell(
                        &action.kind,
                        &action.status,
                        action.pid,
                    ),
                });
                continue;
            }
            let child_style = if action.active {
                ObservationLineStyle::ActiveBlink
            } else {
                ObservationLineStyle::Normal
            };
            observations.extend(action_observation_pair(
                None,
                child_style,
                detail_text,
                timer,
            ));
        }
    }
    observations
}

fn action_detail_for_shell(kind: &CoreActionKind) -> (String, Option<ActionTimer>) {
    match kind {
        CoreActionKind::Bash {
            command,
            mode,
            interval_ms,
            timeout_ms,
            loop_timeout_ms,
            once_timeout_ms,
        } => {
            let timer =
                if timeout_ms.is_some() || loop_timeout_ms.is_some() || once_timeout_ms.is_some() {
                    Some(ActionTimer {
                        started_at_ms: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64,
                        timeout_ms: timeout_ms.map(|t| t as u64),
                        loop_timeout_ms: loop_timeout_ms.map(|t| t as u64),
                        interval_ms: *interval_ms,
                        once_timeout_ms: *once_timeout_ms,
                    })
                } else {
                    None
                };
            let text = if mode == "poll" {
                format!("`{}`", command.trim())
            } else if mode == "background" {
                format!("**(后台执行)** `{}`", command.trim())
            } else {
                format!("`{}`", command.trim())
            };
            (text, timer)
        }
        CoreActionKind::ShellJob { job_id } => (
            format!("后台任务: {}", fallback_unknown(job_id.trim())),
            None,
        ),
        CoreActionKind::Memory { surface, operation } => {
            (memory_action_detail(surface.trim(), operation.trim()), None)
        }
        CoreActionKind::Capability { op, kind, id } => (
            capmgr_action_detail(op.trim(), kind.trim(), id.trim()),
            None,
        ),
        CoreActionKind::SelfTool { self_type, op } => {
            (self_tool_action_detail(self_type.trim(), op.trim()), None)
        }
        CoreActionKind::ChatHistory { operation } => {
            (memory_action_detail("raw_chat", operation.trim()), None)
        }
        CoreActionKind::Other { action } => (format!("Action: {action}"), None),
    }
}

fn action_status_detail_for_shell(kind: &CoreActionKind, status: &str, pid: Option<u32>) -> String {
    match kind {
        CoreActionKind::Bash { command, mode, .. } => {
            let label = action_status_label(status, pid);
            if mode == "poll" {
                format!("`[{label}] {}`", command.trim())
            } else if status == "background_running" || mode == "background" {
                format!("**(后台执行)** `[{label}] {}`", command.trim())
            } else {
                format!("`[{label}] {}`", command.trim())
            }
        }
        CoreActionKind::ShellJob { job_id } => match status {
            "background_finished" => format!("后台命令退出: {}", fallback_unknown(job_id.trim())),
            "cancelled" => format!("后台命令取消: {}", fallback_unknown(job_id.trim())),
            "background_running" => {
                format!("后台命令仍在执行: {}", fallback_unknown(job_id.trim()))
            }
            _ => format!("后台任务状态: {}", fallback_unknown(job_id.trim())),
        },
        _ => match status {
            "completed" => "Action: 已完成".to_string(),
            "timeout" => "Action: 超时".to_string(),
            "cancelled" => "Action: 已取消".to_string(),
            "failed" => "Action: 失败".to_string(),
            _ => "Action: 已结束".to_string(),
        },
    }
}

fn action_status_label(status: &str, pid: Option<u32>) -> String {
    match status {
        "completed" => "✔".to_string(),
        "timeout" => pid
            .map(|pid| format!("超时 pid={pid} 仍在运行"))
            .unwrap_or_else(|| "超时".to_string()),
        "cancelled" => "已取消".to_string(),
        "failed" => "失败".to_string(),
        "background_running" => pid
            .map(|pid| format!("后台执行 pid={pid}"))
            .unwrap_or_else(|| "后台执行".to_string()),
        "background_finished" => "后台完成".to_string(),
        _ => "已结束".to_string(),
    }
}

fn capmgr_action_detail(op: &str, kind: &str, id: &str) -> String {
    match (op, kind, id) {
        ("load", "", _) => "能力: 加载".to_string(),
        ("load", kind, "") => format!("能力: 加载 {kind}"),
        ("load", kind, id) => format!("能力: 加载 {kind}/{id}"),
        ("list", "", _) => "能力: 列出".to_string(),
        ("list", kind, _) => format!("能力: 列出 {kind}"),
        ("inspect", kind, id) if !kind.is_empty() && !id.is_empty() => {
            format!("能力: 查看 {kind}/{id}")
        }
        ("job_status", _, id) if !id.is_empty() => format!("后台工具任务: {}", id.trim()),
        ("job_status", _, _) => "后台工具任务: 查询".to_string(),
        ("job_cancel", _, id) if !id.is_empty() => format!("后台工具任务: 取消 {}", id.trim()),
        ("job_cancel", _, _) => "后台工具任务: 取消".to_string(),
        _ => "能力: 管理".to_string(),
    }
}

fn self_tool_action_detail(self_type: &str, op: &str) -> String {
    match (self_type, op) {
        ("env", "read") => "Timem: 查看环境".to_string(),
        ("env", "write") => "Timem: 更新环境".to_string(),
        ("mem_path", "read") => "Timem: 查看记忆路径".to_string(),
        ("about_me", "read") => "Timem: 查看自身信息".to_string(),
        _ => "Timem: 自身工具".to_string(),
    }
}

fn memory_action_detail(mem_type: &str, op: &str) -> String {
    let target = match mem_type {
        "durable" => "长期记忆",
        "raw_chat" => "聊天记录",
        "scratch" => "草稿区",
        "context" => "上下文",
        _ => "记忆",
    };
    let action = match op {
        "query" | "sql" | "schema" | "read" => "查询",
        "write" | "insert" | "update" | "upsert" => "更新",
        "delete" => "删除",
        "shrink" => "压缩",
        _ => match mem_type {
            "durable" => "更新",
            "raw_chat" => "查询",
            "scratch" => "处理",
            "context" => "整理",
            _ => "处理",
        },
    };
    format!("{target}: {action}")
}

fn fallback_unknown(text: &str) -> String {
    if text.is_empty() {
        "unknown".to_string()
    } else {
        text.to_string()
    }
}

fn action_observation_pair(
    parent_label: Option<&str>,
    child_style: ObservationLineStyle,
    child_text: String,
    timer: Option<ActionTimer>,
) -> Vec<ObservationEvent> {
    let Some(parent_label) = parent_label else {
        return match (child_style, timer) {
            (ObservationLineStyle::Normal, _) => vec![ObservationEvent::Persistent(child_text)],
            (ObservationLineStyle::ActiveBlink, Some(t)) => {
                vec![ObservationEvent::ActiveWithTimer {
                    text: child_text,
                    timer: t,
                }]
            }
            (ObservationLineStyle::ActiveBlink, None) => vec![ObservationEvent::Active(child_text)],
        };
    };
    let mut events = vec![ObservationEvent::Persistent(parent_label.to_string())];
    events.push(match (child_style, timer) {
        (ObservationLineStyle::Normal, _) => ObservationEvent::PersistentChild {
            text: child_text,
            is_last: true,
        },
        (ObservationLineStyle::ActiveBlink, Some(t)) => ObservationEvent::ActiveChildWithTimer {
            text: child_text,
            is_last: true,
            timer: t,
        },
        (ObservationLineStyle::ActiveBlink, None) => ObservationEvent::ActiveChild {
            text: child_text,
            is_last: true,
        },
    });
    events
}

fn wrap_display_width(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    for source_line in text.lines() {
        let mut current = String::new();
        let mut used = 0usize;
        for ch in source_line.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if used > 0 && used + ch_width > width {
                lines.push(current);
                current = String::new();
                used = 0;
            }
            current.push(ch);
            used += ch_width;
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn wrap_display_width_limited(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let mut lines = wrap_display_width(text, width);
    if lines.len() <= max_lines {
        return lines;
    }
    lines.truncate(max_lines);
    if let Some(last) = lines.last_mut() {
        *last = fit_with_ellipsis(last, width);
    }
    lines
}

fn render_markdown_lines_limited(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    if let Some(code) = single_inline_code_span(text) {
        return wrap_display_width_limited(code, width, max_lines)
            .into_iter()
            .map(|line| termimad::inline(&format!("`{line}`")).to_string())
            .collect();
    }

    wrap_display_width_limited(text, width, max_lines)
        .into_iter()
        .map(|line| termimad::inline(&line).to_string())
        .collect()
}

fn single_inline_code_span(text: &str) -> Option<&str> {
    let stripped = text.strip_prefix('`')?.strip_suffix('`')?;
    if stripped.contains('`') {
        None
    } else {
        Some(stripped)
    }
}

fn fit_with_ellipsis(text: &str, width: usize) -> String {
    let width = width.max(1);
    let ellipsis = "…";
    if width <= display_width(ellipsis) {
        return ellipsis.to_string();
    }
    let target_width = width.saturating_sub(display_width(ellipsis));
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > target_width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out.push_str(ellipsis);
    out
}

fn pad_display_width(text: &str, width: usize) -> String {
    let current = display_width(text);
    if current > width {
        truncate_display_width(text, width)
    } else if current == width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - current))
    }
}

fn truncate_display_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(strip_ansi(text).as_str())
}

fn terminal_marker_extra_width(text: &str) -> usize {
    if text.starts_with("⚙️ ") || text.starts_with("💡 ") {
        1
    } else {
        0
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code_ch in chars.by_ref() {
                if code_ch.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
#[path = "../tests/unit/observation_tests.rs"]
mod tests;
