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
    let mut active_parent_intent: Option<String> = None;
    let mut last_child_index_for_active_parent: Option<usize> = None;
    for event in events {
        if let Some(model_response) = event.as_model_response() {
            active_parent_intent = None;
            last_child_index_for_active_parent = None;
            let free_talk = model_response.free_talk.trim();
            if !free_talk.is_empty() {
                observations.push(ObservationEvent::Persistent(format!("💡 {free_talk}")));
            }
            let progress = model_response.progress.trim();
            if !progress.is_empty() {
                observations.push(ObservationEvent::Persistent(format!("⚙️ {progress}")));
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
            active_parent_intent = None;
            last_child_index_for_active_parent = None;
            let attempt = repair.attempt.max(1);
            let max_attempts = repair.max_attempts.max(attempt);
            observations.push(ObservationEvent::Persistent(format!(
                "⚠️ 模型回复偏离协议，重试 ({attempt}/{max_attempts})..."
            )));
            observations.push(ObservationEvent::EnsureTransient("思考中...".to_string()));
            continue;
        }
        if event.as_work_instruction_load().is_some() {
            active_parent_intent = None;
            last_child_index_for_active_parent = None;
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
                active_parent_intent = None;
                last_child_index_for_active_parent = None;
                continue;
            }
            let child_style = if action.active {
                ObservationLineStyle::ActiveBlink
            } else {
                ObservationLineStyle::Normal
            };
            if let Some(parent_intent) = action
                .parent_intent
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                if active_parent_intent.as_deref() != Some(parent_intent) {
                    active_parent_intent = Some(parent_intent.to_string());
                    last_child_index_for_active_parent = None;
                    observations.push(ObservationEvent::Persistent(parent_intent.to_string()));
                }
                if let Some(idx) = last_child_index_for_active_parent {
                    set_child_is_last(&mut observations[idx], false);
                }
                observations.push(action_child_observation(
                    child_style,
                    detail_text,
                    timer,
                    true,
                ));
                last_child_index_for_active_parent = observations.len().checked_sub(1);
            } else {
                active_parent_intent = None;
                last_child_index_for_active_parent = None;
                observations.extend(action_observation_pair(
                    action.intent.as_deref(),
                    child_style,
                    detail_text,
                    timer,
                ));
            }
        }
    }
    observations
}

fn set_child_is_last(event: &mut ObservationEvent, value: bool) {
    match event {
        ObservationEvent::PersistentChild { is_last, .. }
        | ObservationEvent::ActiveChild { is_last, .. }
        | ObservationEvent::ActiveChildWithTimer { is_last, .. } => {
            *is_last = value;
        }
        _ => {}
    }
}

fn action_child_observation(
    child_style: ObservationLineStyle,
    child_text: String,
    timer: Option<ActionTimer>,
    is_last: bool,
) -> ObservationEvent {
    match (child_style, timer) {
        (ObservationLineStyle::Normal, _) => ObservationEvent::PersistentChild {
            text: child_text,
            is_last,
        },
        (ObservationLineStyle::ActiveBlink, Some(t)) => ObservationEvent::ActiveChildWithTimer {
            text: child_text,
            is_last,
            timer: t,
        },
        (ObservationLineStyle::ActiveBlink, None) => ObservationEvent::ActiveChild {
            text: child_text,
            is_last,
        },
    }
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
    intent: Option<&str>,
    child_style: ObservationLineStyle,
    child_text: String,
    timer: Option<ActionTimer>,
) -> Vec<ObservationEvent> {
    let Some(intent) = intent else {
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
    let mut events = vec![ObservationEvent::Persistent(intent.to_string())];
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
mod tests {
    use super::*;
    use agent_core::{
        CoreMemoryActivity, CoreSessionState, CoreTopic, CORE_TOPIC_ACTION,
        CORE_TOPIC_MODEL_REPAIR, CORE_TOPIC_MODEL_RESPONSE, CORE_TOPIC_WORK_INSTRUCTION_LOAD,
    };
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn perf_guard_enabled() -> bool {
        std::env::var("TIMEM_PERF_GUARD").ok().as_deref() == Some("1")
    }

    fn assert_perf_under(label: &str, started: Instant, budget: Duration) {
        if perf_guard_enabled() {
            let elapsed = started.elapsed();
            assert!(
                elapsed <= budget,
                "{label} took {elapsed:?}, expected <= {budget:?}"
            );
        }
    }

    fn action_topic(
        action: &str,
        intent: Option<&str>,
        kind: CoreActionKind,
        active: bool,
    ) -> CoreTopicEvent {
        action_topic_with_parent(action, intent, None, kind, active)
    }

    fn action_topic_with_parent(
        action: &str,
        intent: Option<&str>,
        parent_intent: Option<&str>,
        kind: CoreActionKind,
        active: bool,
    ) -> CoreTopicEvent {
        action_topic_with_status(
            action,
            intent,
            parent_intent,
            kind,
            active,
            "start",
            "running",
        )
    }

    fn action_topic_with_status(
        action: &str,
        intent: Option<&str>,
        parent_intent: Option<&str>,
        kind: CoreActionKind,
        active: bool,
        event: &str,
        status: &str,
    ) -> CoreTopicEvent {
        action_topic_with_status_and_pid(
            action,
            intent,
            parent_intent,
            kind,
            active,
            event,
            status,
            None,
        )
    }

    fn action_topic_with_status_and_pid(
        action: &str,
        intent: Option<&str>,
        parent_intent: Option<&str>,
        kind: CoreActionKind,
        active: bool,
        event: &str,
        status: &str,
        pid: Option<u32>,
    ) -> CoreTopicEvent {
        CoreTopicEvent::new(
            "session_test",
            CoreTopic::new(
                CORE_TOPIC_ACTION,
                json!({
                    "name": CORE_TOPIC_ACTION,
                    "action": action,
                    "active": active,
                    "event": event,
                }),
            ),
            CoreSessionState::Running,
            json!({
                "intent": intent,
                "parent_intent": parent_intent,
                "action": action,
                "input": serde_json::Value::Null,
                "kind": kind,
                "active": active,
                "event": event,
                "status": status,
                "pid": pid,
                "memory_activity": CoreMemoryActivity::None,
            }),
        )
    }

    fn bash_kind(command: &str) -> CoreActionKind {
        CoreActionKind::Bash {
            command: command.to_string(),
            mode: "normal".to_string(),
            interval_ms: None,
            timeout_ms: None,
            loop_timeout_ms: None,
            once_timeout_ms: None,
        }
    }

    fn polling_bash_kind(command: &str) -> CoreActionKind {
        CoreActionKind::Bash {
            command: command.to_string(),
            mode: "poll".to_string(),
            interval_ms: Some(5000),
            timeout_ms: None,
            loop_timeout_ms: Some(60000),
            once_timeout_ms: Some(5000),
        }
    }

    fn model_response_topic(free_talk: &str, progress: &str) -> CoreTopicEvent {
        CoreTopicEvent::new(
            "session_test",
            CoreTopic::new(
                CORE_TOPIC_MODEL_RESPONSE,
                json!({
                    "name": CORE_TOPIC_MODEL_RESPONSE,
                }),
            ),
            CoreSessionState::Running,
            json!({
                "status": "working",
                "free_talk": free_talk,
                "progress": progress,
                "final_answer": "",
                "continue_work": true,
            }),
        )
    }

    fn model_response_topic_with_worker_count(
        free_talk: &str,
        progress: &str,
        working_worker_count: usize,
    ) -> CoreTopicEvent {
        CoreTopicEvent::new(
            "session_test",
            CoreTopic::new(
                CORE_TOPIC_MODEL_RESPONSE,
                json!({
                    "name": CORE_TOPIC_MODEL_RESPONSE,
                }),
            ),
            CoreSessionState::Running,
            json!({
                "status": "working",
                "free_talk": free_talk,
                "progress": progress,
                "final_answer": "",
                "continue_work": true,
                "global": {
                    "working_worker_count": working_worker_count,
                },
            }),
        )
    }

    fn model_repair_topic(issue: &str, attempt: u32, max_attempts: u32) -> CoreTopicEvent {
        CoreTopicEvent::new(
            "session_test",
            CoreTopic::new(
                CORE_TOPIC_MODEL_REPAIR,
                json!({
                    "name": CORE_TOPIC_MODEL_REPAIR,
                }),
            ),
            CoreSessionState::WaitingModel,
            json!({
                "issue": issue,
                "attempt": attempt,
                "max_attempts": max_attempts,
            }),
        )
    }

    fn work_instruction_load_topic(status: &str, file_names: Vec<&str>) -> CoreTopicEvent {
        CoreTopicEvent::new(
            "session_test",
            CoreTopic::new(
                CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                json!({
                    "name": CORE_TOPIC_WORK_INSTRUCTION_LOAD,
                }),
            ),
            CoreSessionState::Running,
            json!({
                "status": status,
                "directory": "/tmp/project",
                "file_names": file_names,
                "error": null,
            }),
        )
    }

    #[test]
    fn panel_renders_heavy_border_and_blinking_transient() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("\x1b[1m┏━ Thought / Action"));
        assert!(rendered.contains("\x1b[38;5;245m· 思考中..."));
        assert!(rendered.contains('┗'));
    }

    #[test]
    fn active_lines_cycle_text_depth_across_ticks() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Active("`pwd`".to_string()));

        let dark = render_observation_panel_at(&panel, 0);
        let mid = render_observation_panel_at(&panel, 1);
        let light = render_observation_panel_at(&panel, 2);
        let looped = render_observation_panel_at(&panel, 3);

        assert!(dark.contains("\x1b[38;5;245m"));
        assert!(mid.contains("\x1b[38;5;250m"));
        assert!(light.contains("\x1b[38;5;255m"));
        assert!(looped.contains("\x1b[38;5;245m"));
        assert!(strip_ansi(&dark).contains("· [.  ] pwd"));
        assert!(!strip_ansi(&dark).contains("`Bash"));
    }

    #[test]
    fn panel_scrolls_when_lines_exceed_limit() {
        let mut panel = ObservationPanel::new(3, 48);
        for line in ["a", "b", "c", "d"] {
            panel.apply(ObservationEvent::Persistent(line.to_string()));
        }
        let rendered = render_observation_panel(&panel);
        assert!(!rendered.contains("· a "));
        assert!(rendered.contains("· b "));
        assert!(rendered.contains("· c "));
        assert!(rendered.contains("· d "));
    }

    #[test]
    fn default_panel_allows_twenty_visible_rows() {
        let mut panel = ObservationPanel::default();
        for idx in 0..21 {
            panel.apply(ObservationEvent::Persistent(format!("line {idx}")));
        }
        let rendered = render_observation_panel(&panel);
        let content_rows = rendered.lines().filter(|line| line.contains('┃')).count();
        assert_eq!(content_rows, 20);
        assert!(!rendered.contains("line 0"));
        assert!(rendered.contains("line 20"));
    }

    #[test]
    fn transient_line_does_not_enter_history() {
        let mut panel = ObservationPanel::new(2, 48);
        panel.apply(ObservationEvent::Persistent("a".to_string()));
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));
        panel.apply(ObservationEvent::ClearTransient);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("· a "));
        assert!(!rendered.contains("思考中"));
    }

    #[test]
    fn persistent_update_keeps_unfinished_transient_at_bottom() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));
        panel.apply(ObservationEvent::Persistent("后台 Bash 已完成".to_string()));

        let rendered = render_observation_panel(&panel);
        let persistent_pos = rendered.find("后台 Bash 已完成").unwrap();
        let transient_pos = rendered.find("思考中...").unwrap();
        assert!(persistent_pos < transient_pos);
    }

    #[test]
    fn repeated_transient_merges_with_count_until_all_finish() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));

        let rendered = render_observation_panel(&panel);
        assert_eq!(rendered.matches("思考中...").count(), 1);
        assert!(rendered.contains("思考中... x2"));

        panel.apply(ObservationEvent::FinishTransient("思考中...".to_string()));
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("思考中..."));
        assert!(!rendered.contains("x2"));

        panel.apply(ObservationEvent::FinishTransient("思考中...".to_string()));
        let rendered = render_observation_panel(&panel);
        assert!(!rendered.contains("思考中..."));
    }

    #[test]
    fn ensure_transient_does_not_increment_existing_status() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));
        panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));

        let rendered = render_observation_panel(&panel);
        assert_eq!(rendered.matches("思考中...").count(), 1);
        assert!(!rendered.contains("x2"));
    }

    #[test]
    fn active_line_can_settle_to_normal() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Active("`pwd`".to_string()));
        let active = render_observation_panel(&panel);
        assert!(active.contains("\x1b[38;5;245m"));
        assert!(strip_ansi(&active).contains("· [.  ] pwd"));
        panel.apply(ObservationEvent::SettleActive);
        let rendered = render_observation_panel(&panel);
        assert!(strip_ansi(&rendered).contains("· pwd"));
        assert!(!rendered.contains("\x1b[38;5;245m"));
    }

    #[test]
    fn continuing_progress_renders_progress_marker() {
        let events = observation_events_from_core_topic_events(&[
            model_response_topic("", "已经完成备份，继续写文件。"),
            action_topic("run_bash", Some("写入文件"), bash_kind("printf ok"), true),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚙️ 已经完成备份，继续写文件。".to_string()),
                ObservationEvent::Persistent("写入文件".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`printf ok`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn free_talk_topic_renders_lightbulb_marker_before_progress() {
        let events = observation_events_from_core_topic_events(&[model_response_topic(
            "先说明一下检查思路。",
            "正在检查项目状态。",
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("💡 先说明一下检查思路。".to_string()),
                ObservationEvent::Persistent("⚙️ 正在检查项目状态。".to_string()),
            ]
        );
    }

    #[test]
    fn repair_topic_renders_warning_and_keeps_thinking() {
        let events =
            observation_events_from_core_topic_events(&[model_repair_topic("invalid_xml", 2, 5)]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚠️ 模型回复偏离协议，重试 (2/5)...".to_string()),
                ObservationEvent::EnsureTransient("思考中...".to_string()),
            ]
        );

        let mut panel = ObservationPanel::new(8, 72);
        panel.apply_all(events);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("模型回复偏离协议"));
        assert!(rendered.contains("(2/5)"));
        assert!(rendered.contains("思考中..."));
    }

    #[test]
    fn work_instruction_status_topic_is_not_mixed_into_observation_panel() {
        let events = observation_events_from_core_topic_events(&[work_instruction_load_topic(
            "loaded",
            vec!["AGENTS.md"],
        )]);
        assert!(events.is_empty());
    }

    #[test]
    fn model_response_keeps_thinking_when_global_workers_are_active() {
        let events =
            observation_events_from_core_topic_events(&[model_response_topic_with_worker_count(
                "",
                "正在继续执行另一个 worker。",
                2,
            )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚙️ 正在继续执行另一个 worker。".to_string()),
                ObservationEvent::EnsureTransient("思考中...".to_string()),
            ]
        );

        let mut panel = ObservationPanel::new(8, 48);
        panel.apply_all(events);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("正在继续执行另一个 worker"));
        assert!(rendered.contains("思考中..."));
        assert!(!rendered.contains("x2"));
    }

    #[test]
    fn model_response_stops_thinking_when_global_workers_reach_zero() {
        let events =
            observation_events_from_core_topic_events(&[model_response_topic_with_worker_count(
                "",
                "全部 worker 已结束。",
                0,
            )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚙️ 全部 worker 已结束。".to_string()),
                ObservationEvent::FinishTransient("思考中...".to_string()),
            ]
        );

        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::EnsureTransient("思考中...".to_string()));
        panel.apply_all(events);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("全部 worker 已结束"));
        assert!(!rendered.contains("思考中..."));
    }

    #[test]
    fn model_response_maps_run_bash_to_user_facing_bash() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("统计当前代码量"),
            bash_kind("rg --files | wc -l"),
            true,
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("统计当前代码量".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`rg --files | wc -l`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn model_response_maps_polling_run_bash_to_user_facing_poll() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("等待 CI 完成"),
            polling_bash_kind("gh run list --branch main"),
            true,
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("等待 CI 完成".to_string()),
                ObservationEvent::ActiveChildWithTimer {
                    text: "`gh run list --branch main`".to_string(),
                    is_last: true,
                    timer: ActionTimer {
                        started_at_ms: events
                            .iter()
                            .find_map(|event| match event {
                                ObservationEvent::ActiveChildWithTimer { timer, .. } => {
                                    Some(timer.started_at_ms)
                                }
                                _ => None,
                            })
                            .unwrap(),
                        timeout_ms: None,
                        loop_timeout_ms: Some(60000),
                        interval_ms: Some(5000),
                        once_timeout_ms: Some(5000),
                    }
                }
            ]
        );
    }

    #[test]
    fn polling_action_topic_renders_active_countdown() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("等待文件出现"),
            polling_bash_kind("test -f /tmp/timem_poll_demo"),
            true,
        )]);
        let mut panel = ObservationPanel::new(8, 80);
        panel.apply_all(events);

        let rendered = render_observation_panel_at(&panel, 0);
        let plain = strip_ansi(&rendered);
        assert!(
            plain.contains("[⏱ 4/01:00] test -f /tmp/timem_poll_demo"),
            "{plain}"
        );
        assert!(!plain.contains("Poll"));
        assert!(rendered.contains("\x1b[38;5;245m"));
    }

    #[test]
    fn action_finish_topic_updates_existing_bash_line() {
        let kind = bash_kind("printf done");
        let start = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("执行检查"),
            kind.clone(),
            true,
        )]);
        let finish = observation_events_from_core_topic_events(&[action_topic_with_status(
            "run_bash",
            Some("执行检查"),
            None,
            kind,
            false,
            "finish",
            "completed",
        )]);

        let mut panel = ObservationPanel::new(8, 80);
        panel.apply_all(start);
        panel.apply_all(finish);
        let rendered = render_observation_panel(&panel);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("[✔] printf done"));
        assert_eq!(plain.matches("printf done").count(), 1);
        assert!(!rendered.contains("\x1b[38;5;245m"));
    }

    #[test]
    fn background_action_and_exit_status_render_user_facing_state() {
        let background_kind = CoreActionKind::Bash {
            command: "sleep 30".to_string(),
            mode: "background".to_string(),
            interval_ms: None,
            timeout_ms: None,
            loop_timeout_ms: None,
            once_timeout_ms: None,
        };
        let background = observation_events_from_core_topic_events(&[action_topic_with_status(
            "run_bash",
            Some("启动后台任务"),
            None,
            background_kind,
            false,
            "finish",
            "background_running",
        )]);
        let mut panel = ObservationPanel::new(8, 80);
        panel.apply_all(background);
        let rendered = render_observation_panel(&panel);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("(后台执行) [后台执行] sleep 30"), "{plain}");
        assert!(rendered.contains(ANSI_BOLD) || rendered.contains("\x1b["));

        let finished = observation_events_from_core_topic_events(&[action_topic_with_status(
            "run_bash",
            None,
            None,
            CoreActionKind::Bash {
                command: "sleep 30".to_string(),
                mode: "background".to_string(),
                interval_ms: None,
                timeout_ms: None,
                loop_timeout_ms: None,
                once_timeout_ms: None,
            },
            false,
            "finish",
            "background_finished",
        )]);
        panel.apply_all(finished);
        let rendered = render_observation_panel(&panel);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("(后台执行) [后台完成] sleep 30"), "{plain}");
    }

    #[test]
    fn timed_out_bash_finish_renders_still_running_pid() {
        let kind = CoreActionKind::Bash {
            command: "sleep 18".to_string(),
            mode: "normal".to_string(),
            interval_ms: None,
            timeout_ms: Some(10000),
            loop_timeout_ms: None,
            once_timeout_ms: None,
        };
        let start = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("运行 sleep 并等待 timeout"),
            kind.clone(),
            true,
        )]);
        let finish =
            observation_events_from_core_topic_events(&[action_topic_with_status_and_pid(
                "run_bash",
                Some("运行 sleep 并等待 timeout"),
                None,
                kind,
                false,
                "finish",
                "timeout",
                Some(49189),
            )]);

        let mut panel = ObservationPanel::new(8, 100);
        panel.apply_all(start);
        panel.apply_all(finish);
        let plain = strip_ansi(&render_observation_panel(&panel));
        assert!(
            plain.contains("[超时 pid=49189 仍在运行] sleep 18"),
            "{plain}"
        );
        assert_eq!(plain.matches("sleep 18").count(), 1, "{plain}");
    }

    #[test]
    fn core_topic_events_map_action_without_protocol_parsing() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("整理 v0.5.2 之后的提交"),
            bash_kind("git log --oneline v0.5.2..HEAD"),
            true,
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("整理 v0.5.2 之后的提交".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`git log --oneline v0.5.2..HEAD`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn core_topic_events_map_progress_and_action_events() {
        let events = observation_events_from_core_topic_events(&[
            model_response_topic("", "正在检查项目状态。"),
            action_topic(
                "run_bash",
                Some("查看当前 git 状态"),
                bash_kind("git status --short"),
                true,
            ),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚙️ 正在检查项目状态。".to_string()),
                ObservationEvent::Persistent("查看当前 git 状态".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`git status --short`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn core_topic_events_wire_shape_maps_progress_and_action_events() {
        let topic_events = [
            model_response_topic("", "正在检查项目状态。"),
            action_topic(
                "run_bash",
                Some("查看当前 git 状态"),
                bash_kind("git status --short"),
                true,
            ),
        ];
        let events = observation_events_from_core_topic_events(&topic_events);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚙️ 正在检查项目状态。".to_string()),
                ObservationEvent::Persistent("查看当前 git 状态".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`git status --short`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn core_topic_events_map_multiple_action_events() {
        let events = observation_events_from_core_topic_events(&[
            model_response_topic("", "正在并行检查记忆和本地文件。"),
            action_topic(
                "memmgr",
                Some("查询测试记忆"),
                CoreActionKind::Memory {
                    surface: "durable".to_string(),
                    operation: "query".to_string(),
                },
                false,
            ),
            action_topic(
                "run_bash",
                Some("列出源码文件"),
                bash_kind("rg --files -g '*.rs'"),
                true,
            ),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("⚙️ 正在并行检查记忆和本地文件。".to_string()),
                ObservationEvent::Persistent("查询测试记忆".to_string()),
                ObservationEvent::PersistentChild {
                    text: "长期记忆: 查询".to_string(),
                    is_last: true
                },
                ObservationEvent::Persistent("列出源码文件".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`rg --files -g '*.rs'`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn action_group_parent_intent_groups_child_bash_lines() {
        let events = observation_events_from_core_topic_events(&[
            action_topic_with_parent(
                "run_bash",
                None,
                Some("先做项目检查"),
                bash_kind("printf a"),
                true,
            ),
            action_topic_with_parent(
                "run_bash",
                None,
                Some("先做项目检查"),
                bash_kind("printf b"),
                true,
            ),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("先做项目检查".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`printf a`".to_string(),
                    is_last: false
                },
                ObservationEvent::ActiveChild {
                    text: "`printf b`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn action_intent_overrides_group_parent_and_unlabeled_actions_render_directly() {
        let events = observation_events_from_core_topic_events(&[
            action_topic_with_parent(
                "run_bash",
                Some("进行 yyy 的分任务"),
                None,
                bash_kind("printf named"),
                true,
            ),
            action_topic("run_bash", None, bash_kind("printf plain"), true),
            action_topic(
                "memmgr",
                None,
                CoreActionKind::Memory {
                    surface: "durable".to_string(),
                    operation: "query".to_string(),
                },
                false,
            ),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("进行 yyy 的分任务".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`printf named`".to_string(),
                    is_last: true
                },
                ObservationEvent::Active("`printf plain`".to_string()),
                ObservationEvent::Persistent("长期记忆: 查询".to_string()),
            ]
        );
    }

    #[test]
    fn memmgr_actions_map_to_user_readable_observation_events() {
        let events = observation_events_from_core_topic_events(&[
            action_topic(
                "memmgr",
                Some("查询测试代号记忆"),
                CoreActionKind::Memory {
                    surface: "durable".to_string(),
                    operation: "query".to_string(),
                },
                false,
            ),
            action_topic(
                "memmgr",
                Some("移除过期上下文"),
                CoreActionKind::Memory {
                    surface: "context".to_string(),
                    operation: "shrink".to_string(),
                },
                false,
            ),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("查询测试代号记忆".to_string()),
                ObservationEvent::PersistentChild {
                    text: "长期记忆: 查询".to_string(),
                    is_last: true
                },
                ObservationEvent::Persistent("移除过期上下文".to_string()),
                ObservationEvent::PersistentChild {
                    text: "上下文: 压缩".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn capmgr_action_maps_to_user_readable_observation_events() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "capmgr",
            Some("加载发布检查能力"),
            CoreActionKind::Capability {
                op: "load".to_string(),
                kind: "skill".to_string(),
                id: "release_quality_gate".to_string(),
            },
            false,
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("加载发布检查能力".to_string()),
                ObservationEvent::PersistentChild {
                    text: "能力: 加载 skill/release_quality_gate".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn self_tool_action_maps_to_user_readable_observation_events() {
        let events = observation_events_from_core_topic_events(&[
            action_topic(
                "self_tool",
                Some("查看 Timem 记忆路径"),
                CoreActionKind::SelfTool {
                    self_type: "mem_path".to_string(),
                    op: "read".to_string(),
                },
                false,
            ),
            action_topic(
                "self_tool",
                Some("查看 Timem 软件信息"),
                CoreActionKind::SelfTool {
                    self_type: "about_me".to_string(),
                    op: "read".to_string(),
                },
                false,
            ),
        ]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("查看 Timem 记忆路径".to_string()),
                ObservationEvent::PersistentChild {
                    text: "Timem: 查看记忆路径".to_string(),
                    is_last: true
                },
                ObservationEvent::Persistent("查看 Timem 软件信息".to_string()),
                ObservationEvent::PersistentChild {
                    text: "Timem: 查看自身信息".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn action_topic_with_json_like_command_keeps_command_intact() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("写入包含 JSON 的示例"),
            bash_kind("printf '{\"ok\":true}' > target/example.json"),
            true,
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("写入包含 JSON 的示例".to_string()),
                ObservationEvent::ActiveChild {
                    text: "`printf '{\"ok\":true}' > target/example.json`".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn unknown_action_uses_intent_without_exposing_action_name() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "future_tool",
            Some("执行未来扩展动作"),
            CoreActionKind::Other {
                action: "future_tool".to_string(),
            },
            false,
        )]);
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("执行未来扩展动作".to_string()),
                ObservationEvent::PersistentChild {
                    text: "Action: future_tool".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn empty_core_topic_events_create_no_observation_events() {
        let events = observation_events_from_core_topic_events(&[]);
        assert!(events.is_empty());
    }

    #[test]
    fn action_topic_does_not_expose_internal_action_name() {
        let mut panel = ObservationPanel::default();
        panel.apply_all(observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("统计"),
            bash_kind("rg --files | wc -l"),
            true,
        )]));
        let rendered = render_observation_panel(&panel);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("· 统计"));
        assert!(plain.contains("└─"));
        assert!(!plain.contains("`Bash"));
        assert!(!rendered.contains("run_bash"));
    }

    #[test]
    fn tree_child_lines_render_under_intent_and_wrap_without_repeating_branch() {
        let mut panel = ObservationPanel::new(8, 44);
        panel.apply(ObservationEvent::Persistent("统计当前代码量".to_string()));
        panel.apply(ObservationEvent::ActiveChild {
            text: "`123456789012345678901234567890 tail`".to_string(),
            is_last: true,
        });

        let rendered = render_observation_panel(&panel);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("· 统计当前代码量"));
        assert!(plain.contains("└─ [.  ] 123456789012345"));
        assert_eq!(plain.matches("└─").count(), 1);
        assert!(plain.contains("tail"));
    }

    #[test]
    fn run_bash_without_intent_shows_plain_label() {
        let events = observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            None,
            bash_kind("ls -la"),
            true,
        )]);
        assert_eq!(
            events,
            vec![ObservationEvent::Active("`ls -la`".to_string())]
        );
    }

    #[test]
    fn panel_wraps_long_command_and_truncates_one_item_after_four_rows() {
        let mut panel = ObservationPanel::new(8, 44);
        panel.apply(ObservationEvent::Active(format!(
            "{}",
            "rg --files -g '*.rs' | xargs wc -l && echo very-long-tail && echo more-output && echo another-long-part && echo segment-four && echo segment-five && echo segment-six && echo hidden-tail-after-limit"
        )));
        let rendered = render_observation_panel(&panel);
        let content_rows = rendered.lines().filter(|line| line.contains('┃')).count();
        assert_eq!(content_rows, 4);
        assert!(rendered.contains('…'));
        assert!(!rendered.contains("hidden-tail-after-limit"));
    }

    #[test]
    fn observation_width_follows_terminal_width_policy() {
        assert_eq!(observation_panel_width_for_terminal(120), 96);
        assert_eq!(observation_panel_width_for_terminal(100), 80);
        assert_eq!(observation_panel_width_for_terminal(90), 80);
        assert_eq!(observation_panel_width_for_terminal(70), 70);
    }

    #[test]
    fn panel_width_can_be_updated_for_current_terminal() {
        let mut panel = ObservationPanel::new(8, 44);
        panel.set_max_width(observation_panel_width_for_terminal(120));
        panel.apply(ObservationEvent::Persistent("宽度检查".to_string()));
        let rendered = render_observation_panel(&panel);
        let first_line = rendered
            .lines()
            .next()
            .unwrap()
            .replace(ANSI_BOLD, "")
            .replace(ANSI_RESET, "");
        let first_line_width = display_width(&first_line);
        assert_eq!(first_line_width, 96);
    }

    #[test]
    fn panel_ansi_sequences_do_not_affect_visible_width() {
        let mut panel = ObservationPanel::new(8, 80);
        panel.apply(ObservationEvent::Active(
            "正在执行长命令并刷新状态".to_string(),
        ));
        panel.apply(ObservationEvent::ActiveChild {
            text: "echo \"=== git status ===\"; git status; echo; git diff --cached".to_string(),
            is_last: true,
        });
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));

        let rendered = render_observation_panel_at_with_elapsed(&panel, 2, Some("00:57"));
        let visible_widths = rendered.lines().map(display_width).collect::<Vec<_>>();
        assert!(
            visible_widths.iter().all(|width| *width == 80),
            "all panel rows should have the same visible width: {visible_widths:?}\n{rendered}"
        );
        assert!(rendered.contains("\x1b[38;5;255m"));
    }

    #[test]
    fn long_progress_and_command_render_as_bounded_aligned_rows() {
        let mut panel = ObservationPanel::new(20, 80);
        panel.apply(ObservationEvent::Persistent(format!(
            "⚙️ {}",
            "正在处理一个非常长的进度汇报，需要确认观察窗会自动换行但不会把边框撑乱，也不会因为每秒刷新而产生宽度不一致的问题。".repeat(3)
        )));
        panel.apply(ObservationEvent::Persistent(
            "分析当前工作区状态并执行长命令".to_string(),
        ));
        panel.apply(ObservationEvent::ActiveChild {
            text: format!(
                "`{}`",
                format!(
                    "{} tail-marker-should-not-render-after-limit",
                    "echo start; git status --short; git diff --stat; printf '%s' very-long-segment; "
                        .repeat(12)
                )
            ),
            is_last: true,
        });
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));

        let rendered = render_observation_panel_at_with_elapsed(&panel, 1, Some("12:34"));
        let visible_widths = rendered.lines().map(display_width).collect::<Vec<_>>();
        assert!(
            visible_widths.iter().all(|width| *width == 80),
            "all observation rows should stay aligned: {visible_widths:?}\n{rendered}"
        );
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("⚙️ 正在处理"));
        assert!(plain.contains("└─"));
        assert!(rendered.contains('…'));
        assert!(!rendered.contains("run_bash"));
        assert!(!rendered.contains("tail-marker-should-not-render-after-limit"));
    }

    #[test]
    fn performance_guard_many_observation_events_render_bounded() {
        let long_text = "这是一个很长的观察窗口内容 with ascii and 中文 ".repeat(80);
        let mut events = Vec::new();
        for idx in 0..600 {
            events.push(model_response_topic(
                &format!("计划 {idx}: {long_text}"),
                &format!("进度 {idx}: {long_text}"),
            ));
            events.push(action_topic(
                "run_bash",
                Some(&format!("执行第 {idx} 个本地检查")),
                bash_kind(&format!("printf '{long_text}'")),
                true,
            ));
        }

        let started = Instant::now();
        let mut panel = ObservationPanel::new(20, 96);
        for chunk in events.chunks(8) {
            let observation_events = observation_events_from_core_topic_events(chunk);
            panel.apply_all(observation_events);
            panel.apply(ObservationEvent::SettleActive);
            let rendered = render_observation_panel_at_with_elapsed(&panel, 1, Some("09:59"));
            assert!(rendered.len() < 12_000);
            assert!(!rendered.contains("run_bash"));
        }
        assert_perf_under(
            "many observation events render bounded",
            started,
            Duration::from_millis(1200),
        );
    }

    #[test]
    fn performance_guard_topic_interface_rate_mix_render_bounded() {
        let long_text = "topic pressure 内容 with ascii 中文 ".repeat(40);
        let mut topic_events = Vec::new();
        for idx in 0..20 {
            topic_events.push(model_response_topic(
                &format!("计划 {idx}: {long_text}"),
                &format!("进度 {idx}: {long_text}"),
            ));
        }
        for idx in 0..300 {
            topic_events.push(action_topic(
                "run_bash",
                Some(&format!("执行压力动作 {idx}")),
                bash_kind(&format!("printf '{}'", idx)),
                true,
            ));
        }
        let supplement_events = (0..20)
            .map(|idx| ObservationEvent::Persistent(format!("ⓘ 收到用户补充 {idx}: {long_text}")))
            .collect::<Vec<_>>();

        let started = Instant::now();
        let mut panel = ObservationPanel::new(20, 100);
        for chunk in topic_events.chunks(16) {
            panel.apply_all(observation_events_from_core_topic_events(chunk));
            panel.apply(ObservationEvent::SettleActive);
            let rendered = render_observation_panel_at_with_elapsed(&panel, 1, Some("00:09"));
            assert!(rendered.len() < 14_000);
            assert!(!rendered.contains("run_bash"));
        }
        panel.apply_all(supplement_events);
        let rendered = render_observation_panel_at_with_elapsed(&panel, 2, Some("00:10"));
        assert!(rendered.len() < 14_000);
        let widths = rendered.lines().map(display_width).collect::<Vec<_>>();
        assert!(
            widths.iter().all(|width| *width == 100),
            "topic pressure render should keep aligned rows: {widths:?}\n{rendered}"
        );
        assert_perf_under(
            "topic interface 20 response 300 action 20 supplement render bounded",
            started,
            Duration::from_millis(1200),
        );
    }

    #[test]
    fn action_timer_created_for_bash_with_timeout() {
        let kind = CoreActionKind::Bash {
            command: "sleep 10".to_string(),
            mode: "normal".to_string(),
            interval_ms: None,
            timeout_ms: Some(10000),
            loop_timeout_ms: None,
            once_timeout_ms: None,
        };
        let (text, timer) = action_detail_for_shell(&kind);
        assert_eq!(text, "`sleep 10`");
        assert!(timer.is_some());
        let t = timer.unwrap();
        assert_eq!(t.timeout_ms, Some(10000));
        assert!(t.started_at_ms > 0);
    }

    #[test]
    fn action_timer_created_for_polling_bash() {
        let kind = CoreActionKind::Bash {
            command: "check_status".to_string(),
            mode: "poll".to_string(),
            interval_ms: Some(5000),
            timeout_ms: None,
            loop_timeout_ms: Some(60000),
            once_timeout_ms: Some(10000),
        };
        let (text, timer) = action_detail_for_shell(&kind);
        assert_eq!(text, "`check_status`");
        assert!(timer.is_some());
        let t = timer.unwrap();
        assert_eq!(t.loop_timeout_ms, Some(60000));
        assert_eq!(t.interval_ms, Some(5000));
        assert_eq!(t.once_timeout_ms, Some(10000));
    }

    #[test]
    fn no_timer_for_bash_without_timeout() {
        let kind = bash_kind("echo hello");
        let (text, timer) = action_detail_for_shell(&kind);
        assert_eq!(text, "`echo hello`");
        assert!(timer.is_none());
    }

    #[test]
    fn normal_bash_without_timeout_renders_without_countdown() {
        let mut panel = ObservationPanel::new(8, 80);
        panel.apply_all(observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            Some("执行普通命令"),
            bash_kind("sleep 10 && touch /tmp/timem_poll_demo2.txt"),
            true,
        )]));
        let rendered = render_observation_panel_at(&panel, 0);
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("sleep 10 && touch /tmp/timem_poll_demo2.txt"));
        assert!(!plain.contains("⏱"), "{plain}");
        assert!(rendered.contains("\x1b[38;5;245m"));
    }

    #[test]
    fn normal_bash_without_timeout_still_blinks_while_active() {
        let mut panel = ObservationPanel::new(8, 80);
        panel.apply_all(observation_events_from_core_topic_events(&[action_topic(
            "run_bash",
            None,
            bash_kind("printf active"),
            true,
        )]));

        let dark = render_observation_panel_at(&panel, 0);
        let mid = render_observation_panel_at(&panel, 1);
        let light = render_observation_panel_at(&panel, 2);

        assert!(dark.contains("\x1b[38;5;245m"));
        assert!(mid.contains("\x1b[38;5;250m"));
        assert!(light.contains("\x1b[38;5;255m"));
        assert!(!strip_ansi(&dark).contains("⏱"));
    }

    #[test]
    fn format_countdown_shows_remaining_seconds() {
        let now_ms = 1000000u64;
        let timer = ActionTimer {
            started_at_ms: now_ms - 3000,
            timeout_ms: Some(10000),
            loop_timeout_ms: None,
            interval_ms: None,
            once_timeout_ms: None,
        };
        let countdown = format_countdown(&timer, now_ms);
        assert_eq!(countdown, "⏱ 07s");
    }

    #[test]
    fn format_countdown_uses_loop_timeout_for_polling() {
        let now_ms = 1000000u64;
        let timer = ActionTimer {
            started_at_ms: now_ms - 5000,
            timeout_ms: None,
            loop_timeout_ms: Some(60000),
            interval_ms: Some(5000),
            once_timeout_ms: None,
        };
        let countdown = format_countdown(&timer, now_ms);
        assert_eq!(countdown, "⏱ 4/55s");
    }

    #[test]
    fn format_countdown_uses_shortest_time_shape_and_never_zero_while_running() {
        assert_eq!(format_duration_short(1, true), "01s");
        assert_eq!(format_duration_short(999, true), "01s");
        assert_eq!(format_duration_short(60_000, true), "01:00");
        assert_eq!(format_duration_short(3_661_000, true), "1:01:01");
        assert_eq!(format_duration_short(2_000, false), "02");

        let now_ms = 1000000u64;
        let timer = ActionTimer {
            started_at_ms: now_ms - 59_100,
            timeout_ms: Some(60_000),
            loop_timeout_ms: None,
            interval_ms: None,
            once_timeout_ms: None,
        };
        assert_eq!(format_countdown(&timer, now_ms), "⏱ 01s");
    }

    #[test]
    fn format_countdown_uses_poll_pulse_for_one_second_interval() {
        let now_ms = 1000000u64;
        let timer = ActionTimer {
            started_at_ms: now_ms - 9000,
            timeout_ms: None,
            loop_timeout_ms: Some(10000),
            interval_ms: Some(1000),
            once_timeout_ms: Some(1000),
        };
        assert_eq!(format_countdown(&timer, now_ms), "⏱ ↻1s");
    }

    #[test]
    fn format_countdown_shows_two_second_poll_interval_as_one_then_zero() {
        let now_ms = 1000000u64;
        let timer = ActionTimer {
            started_at_ms: now_ms,
            timeout_ms: None,
            loop_timeout_ms: Some(10000),
            interval_ms: Some(2000),
            once_timeout_ms: Some(1000),
        };
        assert_eq!(format_countdown(&timer, now_ms), "⏱ 1/10s");

        let timer = ActionTimer {
            started_at_ms: now_ms - 1000,
            timeout_ms: None,
            loop_timeout_ms: Some(10000),
            interval_ms: Some(2000),
            once_timeout_ms: Some(1000),
        };
        assert_eq!(format_countdown(&timer, now_ms), "⏱ 0/9s");
    }

    #[test]
    fn format_countdown_rounds_ms_up_for_ui_display() {
        let now_ms = 1000000u64;
        let timer = ActionTimer {
            started_at_ms: now_ms - 1001,
            timeout_ms: Some(2001),
            loop_timeout_ms: None,
            interval_ms: None,
            once_timeout_ms: None,
        };
        assert_eq!(format_countdown(&timer, now_ms), "⏱ 01s");
    }

    #[test]
    fn observation_line_with_timer_renders_countdown() {
        let mut panel = ObservationPanel::new(10, 80);
        let timer = ActionTimer {
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                - 2000,
            timeout_ms: Some(10000),
            loop_timeout_ms: None,
            interval_ms: None,
            once_timeout_ms: None,
        };
        panel.apply(ObservationEvent::ActiveWithTimer {
            text: "`sleep 10`".to_string(),
            timer,
        });
        let rendered = render_observation_panel_at(&panel, 0);
        assert!(
            rendered.contains("\u{23f1}"),
            "Expected countdown symbol in output"
        );
        assert!(rendered.contains("\x1b[38;5;245m"));
        let plain = strip_ansi(&rendered);
        assert!(plain.contains("[⏱ 08s] sleep 10"));
        assert!(!plain.contains("`sleep 10`"));
    }

    #[test]
    fn long_active_command_keeps_countdown_after_bash_label() {
        let mut panel = ObservationPanel::new(10, 88);
        let timer = ActionTimer {
            started_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64
                - 1000,
            timeout_ms: None,
            loop_timeout_ms: Some(15000),
            interval_ms: Some(3000),
            once_timeout_ms: Some(1000),
        };
        panel.apply(ObservationEvent::ActiveChildWithTimer {
            text: "`if [ ! -f /tmp/poll_start_c2.txt ]; then date +%s > /tmp/poll_start_c2.txt; fi; START=$(cat /tmp/poll_start_c2.txt); NOW=$(date +%s); ELAPSED=$((NOW - START)); echo \"已过 ${ELAPSED}s\"; [ $ELAPSED -ge 10 ] && echo '条件满足，提前退出' && exit 0 || exit 1`".to_string(),
            is_last: true,
            timer,
        });

        let plain = strip_ansi(&render_observation_panel_at(&panel, 0));
        assert!(plain.contains("[⏱ 1/14s] if [ ! -f"), "{plain}");
        assert!(!plain.contains("Bash:"), "{plain}");
    }
}
