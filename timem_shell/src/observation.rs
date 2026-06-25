use serde_json::Value;
use std::collections::VecDeque;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_RESET: &str = "\x1b[0m";
const ANSI_BLINK: &str = "\x1b[5m";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservationEvent {
    Persistent(String),
    Active(String),
    Transient(String),
    ClearTransient,
    SettleActive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationLineStyle {
    Normal,
    ActiveBlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationLine {
    pub text: String,
    pub style: ObservationLineStyle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationPanel {
    lines: VecDeque<ObservationLine>,
    transient: Option<ObservationLine>,
    max_lines: usize,
    max_width: usize,
}

impl Default for ObservationPanel {
    fn default() -> Self {
        Self::new(8, 72)
    }
}

impl ObservationPanel {
    pub fn new(max_lines: usize, max_width: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            transient: None,
            max_lines: max_lines.max(1),
            max_width: max_width.max(32),
        }
    }

    pub fn apply(&mut self, event: ObservationEvent) {
        match event {
            ObservationEvent::Persistent(text) => {
                self.push_line(text, ObservationLineStyle::Normal)
            }
            ObservationEvent::Active(text) => {
                self.push_line(text, ObservationLineStyle::ActiveBlink)
            }
            ObservationEvent::Transient(text) => {
                self.transient = Some(ObservationLine {
                    text,
                    style: ObservationLineStyle::ActiveBlink,
                });
            }
            ObservationEvent::ClearTransient => self.transient = None,
            ObservationEvent::SettleActive => {
                for line in &mut self.lines {
                    if line.style == ObservationLineStyle::ActiveBlink {
                        line.style = ObservationLineStyle::Normal;
                    }
                }
            }
        }
    }

    pub fn apply_all(&mut self, events: impl IntoIterator<Item = ObservationEvent>) {
        for event in events {
            self.apply(event);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty() && self.transient.is_none()
    }

    fn push_line(&mut self, text: String, style: ObservationLineStyle) {
        if text.trim().is_empty() {
            return;
        }
        self.lines.push_back(ObservationLine { text, style });
        while self.lines.len() > self.max_lines {
            self.lines.pop_front();
        }
    }

    fn visible_lines(&self) -> Vec<ObservationLine> {
        let mut lines: Vec<ObservationLine> = self.lines.iter().cloned().collect();
        if let Some(transient) = self.transient.clone() {
            lines.push(transient);
        }
        while lines.len() > self.max_lines {
            lines.remove(0);
        }
        lines
    }
}

pub fn render_observation_panel(panel: &ObservationPanel) -> String {
    if panel.is_empty() {
        return String::new();
    }
    let content_width = panel.max_width.saturating_sub(4).max(24);
    let title = " Thought / Action ";
    let mut out = String::new();
    out.push_str(ANSI_BOLD);
    out.push('┏');
    out.push('━');
    out.push_str(title);
    out.push_str(&"━".repeat(content_width.saturating_sub(display_width(title) + 1)));
    out.push('┓');
    out.push_str(ANSI_RESET);
    out.push('\n');
    for line in panel.visible_lines() {
        let fitted = fit_display_width(&line.text, content_width.saturating_sub(2));
        let padded = pad_display_width(&fitted, content_width.saturating_sub(2));
        out.push('┃');
        out.push(' ');
        match line.style {
            ObservationLineStyle::Normal => out.push_str(&padded),
            ObservationLineStyle::ActiveBlink => {
                out.push_str(ANSI_BLINK);
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

pub fn observation_events_from_model_response(content: &str) -> Vec<ObservationEvent> {
    let Ok(value) = serde_json::from_str::<Value>(content.trim()) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    if let Some(thought) = value
        .get("thought")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        events.push(ObservationEvent::Persistent(thought.to_string()));
    }
    if let Some(actions) = value.get("next_actions").and_then(Value::as_array) {
        for action in actions.iter().take(2) {
            if let Some(event) = observation_event_from_action(action) {
                events.push(event);
            }
        }
    }
    events
}

fn observation_event_from_action(action: &Value) -> Option<ObservationEvent> {
    let action_name = action.get("action").and_then(Value::as_str).unwrap_or("");
    let input = action.get("input").unwrap_or(&Value::Null);
    let intent = action
        .get("intent")
        .and_then(Value::as_str)
        .or_else(|| input.get("intent").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty());
    match action_name {
        "run_bash" => {
            let command = input
                .get("command")
                .or_else(|| action.get("command"))
                .and_then(Value::as_str)
                .or_else(|| {
                    input
                        .get("read_back_command")
                        .or_else(|| action.get("read_back_command"))
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|text| !text.is_empty())?;
            Some(ObservationEvent::Active(format!("执行 Bash: {command}")))
        }
        "shell_job_status" => Some(ObservationEvent::Active(format!(
            "检查后台任务: {}",
            input
                .get("job_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        ))),
        "query_memory" | "memory_query" | "memory_sql_query" | "sql_read" | "memory_schema" => {
            Some(ObservationEvent::Persistent(format!(
                "查询记忆: {}",
                intent.unwrap_or("读取相关记忆")
            )))
        }
        "chat_history_query" => Some(ObservationEvent::Persistent(format!(
            "查询聊天记录: {}",
            intent.unwrap_or("读取聊天记录")
        ))),
        "memory_write" | "write_memory" | "memory_update" => Some(ObservationEvent::Persistent(
            format!("更新记忆: {}", intent.unwrap_or("写入记忆")),
        )),
        "scratch_write" | "scratch_query" | "scratch_delete" => Some(ObservationEvent::Persistent(
            format!("处理草稿区: {}", intent.unwrap_or("更新草稿")),
        )),
        _ => intent.map(|text| ObservationEvent::Persistent(text.to_string())),
    }
}

fn fit_display_width(text: &str, width: usize) -> String {
    let one_line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if display_width(&one_line) <= width {
        return one_line;
    }
    let ellipsis = "…";
    let content_width = width.saturating_sub(display_width(ellipsis));
    let mut out = String::new();
    let mut used = 0;
    for ch in one_line.chars() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > content_width {
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
    if current >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - current))
    }
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panel_renders_heavy_border_and_blinking_transient() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("\x1b[1m┏━ Thought / Action"));
        assert!(rendered.contains("\x1b[5m思考中..."));
        assert!(rendered.contains('┗'));
    }

    #[test]
    fn panel_scrolls_when_lines_exceed_limit() {
        let mut panel = ObservationPanel::new(3, 48);
        for line in ["a", "b", "c", "d"] {
            panel.apply(ObservationEvent::Persistent(line.to_string()));
        }
        let rendered = render_observation_panel(&panel);
        assert!(!rendered.contains(" a "));
        assert!(rendered.contains(" b "));
        assert!(rendered.contains(" c "));
        assert!(rendered.contains(" d "));
    }

    #[test]
    fn transient_line_does_not_enter_history() {
        let mut panel = ObservationPanel::new(2, 48);
        panel.apply(ObservationEvent::Persistent("a".to_string()));
        panel.apply(ObservationEvent::Transient("思考中...".to_string()));
        panel.apply(ObservationEvent::ClearTransient);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains(" a "));
        assert!(!rendered.contains("思考中"));
    }

    #[test]
    fn active_line_can_settle_to_normal() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Active("执行 Bash: pwd".to_string()));
        assert!(render_observation_panel(&panel).contains("\x1b[5m执行 Bash"));
        panel.apply(ObservationEvent::SettleActive);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("执行 Bash: pwd"));
        assert!(!rendered.contains("\x1b[5m执行 Bash"));
    }

    #[test]
    fn model_response_maps_run_bash_to_user_facing_bash() {
        let events = observation_events_from_model_response(
            r#"{"thought":"先看代码","next_actions":[{"action":"run_bash","intent":"统计","input":{"command":"rg --files | wc -l"}}]}"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("先看代码".to_string()),
                ObservationEvent::Active("执行 Bash: rg --files | wc -l".to_string())
            ]
        );
    }

    #[test]
    fn model_response_does_not_expose_internal_action_name() {
        let mut panel = ObservationPanel::default();
        panel.apply_all(observation_events_from_model_response(
            r#"{"next_actions":[{"action":"run_bash","intent":"统计","input":{"command":"rg --files | wc -l"}}]}"#,
        ));
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("执行 Bash"));
        assert!(!rendered.contains("run_bash"));
    }

    #[test]
    fn panel_truncates_long_command_to_width() {
        let mut panel = ObservationPanel::new(8, 44);
        panel.apply(ObservationEvent::Active(format!(
            "执行 Bash: {}",
            "rg --files -g '*.rs' | xargs wc -l && echo very-long-tail"
        )));
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains('…'));
        assert!(!rendered.contains("very-long-tail"));
    }
}
