use serde_json::Value;
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
    PersistentChild { text: String, is_last: bool },
    ActiveChild { text: String, is_last: bool },
    Transient(String),
    FinishTransient(String),
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
    pub prefix: String,
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
            ObservationEvent::Persistent(text) => self.push_line(
                text,
                ObservationLineStyle::Normal,
                OBSERVATION_LINE_PREFIX.to_string(),
            ),
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
            ObservationEvent::FinishTransient(text) => self.decrement_transient(&text),
            ObservationEvent::ClearTransient => self.transients.clear(),
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
        self.lines.is_empty() && self.transients.is_empty()
    }

    fn push_line(&mut self, text: String, style: ObservationLineStyle, prefix: String) {
        if text.trim().is_empty() {
            return;
        }
        self.lines.push_back(ObservationLine {
            text,
            style,
            prefix,
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
            });
        }
        while lines.len() > self.max_lines {
            lines.remove(0);
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

pub fn render_observation_panel(panel: &ObservationPanel) -> String {
    render_observation_panel_at(panel, 0)
}

pub fn render_observation_panel_at(panel: &ObservationPanel, tick: usize) -> String {
    if panel.is_empty() {
        return String::new();
    }
    let active_color = ACTIVE_TEXT_COLORS[tick % ACTIVE_TEXT_COLORS.len()];
    let content_width = panel.max_width.saturating_sub(2).max(8);
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
    let line_width = content_width.saturating_sub(2);
    let mut render_rows = Vec::new();
    for line in panel.visible_lines() {
        let text_width = line_width.saturating_sub(display_width(&line.prefix));
        let wrapped =
            wrap_display_width_limited(&line.text, text_width, MAX_WRAPPED_LINES_PER_ITEM);
        for (idx, wrapped) in wrapped.into_iter().enumerate() {
            let prefix = if idx == 0 {
                line.prefix.clone()
            } else {
                " ".repeat(display_width(&line.prefix))
            };
            render_rows.push((line.style, format!("{prefix}{wrapped}")));
        }
    }
    while render_rows.len() > panel.max_lines {
        render_rows.remove(0);
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

pub fn observation_events_from_model_response(content: &str) -> Vec<ObservationEvent> {
    let Some(value) = parse_observation_json_value(content) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    let should_continue = value
        .get("continue")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if should_continue {
        if let Some(progress) = value
            .get("report_job_progress")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            events.push(ObservationEvent::Persistent(format!("▰▱ {progress}")));
        }
    }
    if let Some(actions) = value.get("next_actions").and_then(Value::as_array) {
        for action in actions.iter().take(2) {
            events.extend(observation_events_from_action(action));
        }
    }
    events
}

pub(crate) fn parse_observation_json_value(content: &str) -> Option<Value> {
    let trimmed = content.trim();
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return Some(value);
    }
    let mut last_envelope = None;
    for (idx, ch) in trimmed.char_indices() {
        if ch != '{' {
            continue;
        }
        let candidate = &trimmed[idx..];
        if let Some(object_text) = extract_balanced_json_object(candidate) {
            if let Ok(value) = serde_json::from_str::<Value>(&object_text) {
                if is_likely_observation_envelope(&value) {
                    last_envelope = Some(value);
                }
            }
        }
    }
    last_envelope
}

fn is_likely_observation_envelope(value: &Value) -> bool {
    value.as_object().is_some_and(|object| {
        object.contains_key("next_actions")
            || object.contains_key("report_job_progress")
            || object.contains_key("thought")
    })
}

fn extract_balanced_json_object(input: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, ch) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
                if depth == 0 {
                    return Some(input[..idx + ch.len_utf8()].to_string());
                }
            }
            _ => {}
        }
    }
    None
}

fn observation_events_from_action(action: &Value) -> Vec<ObservationEvent> {
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
            let Some(command) = input
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
                .filter(|text| !text.is_empty())
            else {
                return Vec::new();
            };
            action_observation_pair(
                intent,
                ObservationLineStyle::ActiveBlink,
                format!("Bash: {command}"),
            )
        }
        "shell_job_status" => action_observation_pair(
            intent,
            ObservationLineStyle::ActiveBlink,
            format!(
                "后台任务: {}",
                input
                    .get("job_id")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
            ),
        ),
        "memmgr" => {
            let mem_type = input.get("type").and_then(Value::as_str).unwrap_or("");
            let op = input.get("op").and_then(Value::as_str).unwrap_or("");
            let default = match (mem_type, op) {
                ("durable", "query" | "schema" | "sql") => "查询记忆",
                ("durable", _) => "更新记忆",
                ("raw_chat", "query" | "sql") => "查询聊天记录",
                ("raw_chat", "delete") => "删除聊天记录",
                ("scratch", _) => "处理草稿区",
                ("context", "shrink") => "整理上下文",
                _ => "处理记忆",
            };
            let detail = memory_action_detail(mem_type, op, default);
            action_observation_pair(intent, ObservationLineStyle::Normal, detail)
        }
        "query_memory" | "memory_query" | "memory_sql_query" | "sql_read" | "memory_schema" => {
            action_observation_pair(
                intent,
                ObservationLineStyle::Normal,
                "记忆: 查询".to_string(),
            )
        }
        "chat_history_query" => action_observation_pair(
            intent,
            ObservationLineStyle::Normal,
            "聊天记录: 查询".to_string(),
        ),
        "memory_write" | "write_memory" | "memory_update" => action_observation_pair(
            intent,
            ObservationLineStyle::Normal,
            "记忆: 更新".to_string(),
        ),
        "scratch_write" | "scratch_read" | "scratch_query" | "scratch_delete" => {
            action_observation_pair(
                intent,
                ObservationLineStyle::Normal,
                "草稿区: 处理".to_string(),
            )
        }
        _ => match intent {
            Some(text) => vec![ObservationEvent::Persistent(text.to_string())],
            None => Vec::new(),
        },
    }
}

fn action_observation_pair(
    intent: Option<&str>,
    child_style: ObservationLineStyle,
    child_text: String,
) -> Vec<ObservationEvent> {
    let Some(intent) = intent else {
        return match child_style {
            ObservationLineStyle::Normal => vec![ObservationEvent::Persistent(child_text)],
            ObservationLineStyle::ActiveBlink => vec![ObservationEvent::Active(child_text)],
        };
    };
    let mut events = vec![ObservationEvent::Persistent(intent.to_string())];
    events.push(match child_style {
        ObservationLineStyle::Normal => ObservationEvent::PersistentChild {
            text: child_text,
            is_last: true,
        },
        ObservationLineStyle::ActiveBlink => ObservationEvent::ActiveChild {
            text: child_text,
            is_last: true,
        },
    });
    events
}

fn memory_action_detail(mem_type: &str, op: &str, fallback: &str) -> String {
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
        _ => fallback,
    };
    format!("{target}: {action}")
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
        assert!(rendered.contains("\x1b[38;5;245m· 思考中..."));
        assert!(rendered.contains('┗'));
    }

    #[test]
    fn active_lines_cycle_text_depth_across_ticks() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Active("Bash: pwd".to_string()));

        let dark = render_observation_panel_at(&panel, 0);
        let mid = render_observation_panel_at(&panel, 1);
        let light = render_observation_panel_at(&panel, 2);
        let looped = render_observation_panel_at(&panel, 3);

        assert!(dark.contains("\x1b[38;5;245m· Bash"));
        assert!(mid.contains("\x1b[38;5;250m· Bash"));
        assert!(light.contains("\x1b[38;5;255m· Bash"));
        assert!(looped.contains("\x1b[38;5;245m· Bash"));
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
    fn active_line_can_settle_to_normal() {
        let mut panel = ObservationPanel::new(8, 48);
        panel.apply(ObservationEvent::Active("Bash: pwd".to_string()));
        assert!(render_observation_panel(&panel).contains("\x1b[38;5;245m· Bash"));
        panel.apply(ObservationEvent::SettleActive);
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("· Bash: pwd"));
        assert!(!rendered.contains("\x1b[38;5;245m· Bash"));
    }

    #[test]
    fn continuing_report_job_progress_renders_progress_marker() {
        let events = observation_events_from_model_response(
            r#"{"report_job_progress":"已经完成备份，继续写文件。","next_actions":[{"action":"run_bash","intent":"写入文件","input":{"command":"printf ok"}}]}"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("▰▱ 已经完成备份，继续写文件。".to_string()),
                ObservationEvent::Persistent("写入文件".to_string()),
                ObservationEvent::ActiveChild {
                    text: "Bash: printf ok".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn model_response_maps_run_bash_to_user_facing_bash() {
        let events = observation_events_from_model_response(
            r#"{"thought":"不要展示的模型思考","next_actions":[{"action":"run_bash","intent":"统计当前代码量","input":{"command":"rg --files | wc -l"}}]}"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("统计当前代码量".to_string()),
                ObservationEvent::ActiveChild {
                    text: "Bash: rg --files | wc -l".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn fenced_model_response_still_maps_observation_events() {
        let events = observation_events_from_model_response(
            r#"
```json
{
  "thought": {
    "content": "内部推理，不应该显示",
    "durable": false
  },
  "next_actions": [
    {
      "action": "run_bash",
      "intent": "整理 v0.5.2 之后的提交",
      "input": {
        "command": "git log --oneline v0.5.2..HEAD"
      }
    }
  ]
}
```
"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("整理 v0.5.2 之后的提交".to_string()),
                ObservationEvent::ActiveChild {
                    text: "Bash: git log --oneline v0.5.2..HEAD".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn prose_wrapped_model_response_maps_last_valid_envelope() {
        let events = observation_events_from_model_response(
            r#"
先说明一下：{"not":"an envelope"}

```json
{
  "report_job_progress": "",
  "next_actions": [
    {
      "action": "query_memory",
      "intent": "查询用户姓名记忆",
      "input": {"query": "名字", "limit": 5}
    }
  ]
}
```
"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("查询用户姓名记忆".to_string()),
                ObservationEvent::PersistentChild {
                    text: "记忆: 查询".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn memmgr_actions_map_to_user_readable_observation_events() {
        let events = observation_events_from_model_response(
            r#"{"next_actions":[
                {"action":"memmgr","intent":"查询用户姓名记忆","input":{"type":"durable","op":"query","query":"名字"}},
                {"action":"memmgr","intent":"移除过期上下文","input":{"type":"context","op":"shrink","delta_ids":["pd_1"]}}
            ]}"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("查询用户姓名记忆".to_string()),
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
    fn model_output_with_json_like_command_keeps_command_intact() {
        let events = observation_events_from_model_response(
            r#"
```json
{
  "next_actions": [
    {
      "action": "run_bash",
      "intent": "写入包含 JSON 的示例",
      "input": {
        "command": "printf '{\"ok\":true}' > target/example.json"
      }
    }
  ]
}
```
"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("写入包含 JSON 的示例".to_string()),
                ObservationEvent::ActiveChild {
                    text: "Bash: printf '{\"ok\":true}' > target/example.json".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn model_output_maps_first_two_actions_and_ignores_extra_for_compact_ui() {
        let events = observation_events_from_model_response(
            r#"{"next_actions":[
                {"action":"query_memory","intent":"查名字","input":{"query":"名字"}},
                {"action":"run_bash","intent":"看状态","input":{"command":"git status --short"}},
                {"action":"chat_history_query","intent":"查聊天","input":{"query":"昨天"}}
            ]}"#,
        );
        assert_eq!(
            events,
            vec![
                ObservationEvent::Persistent("查名字".to_string()),
                ObservationEvent::PersistentChild {
                    text: "记忆: 查询".to_string(),
                    is_last: true
                },
                ObservationEvent::Persistent("看状态".to_string()),
                ObservationEvent::ActiveChild {
                    text: "Bash: git status --short".to_string(),
                    is_last: true
                }
            ]
        );
    }

    #[test]
    fn final_only_model_response_creates_no_observation_events() {
        let events = observation_events_from_model_response(
            r#"{"thought":"内部思考","report_job_progress":"已经完成","continue":false}"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn truncated_fenced_model_response_creates_no_observation_events() {
        let events = observation_events_from_model_response(
            r#"
```json
{"next_actions":[{"action":"run_bash","intent":"坏掉了","input":{"command":"git status"}}
"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn unknown_action_uses_intent_without_exposing_action_name() {
        let events = observation_events_from_model_response(
            r#"{"next_actions":[{"action":"future_tool","intent":"执行未来扩展动作","input":{}}]}"#,
        );
        assert_eq!(
            events,
            vec![ObservationEvent::Persistent("执行未来扩展动作".to_string())]
        );
    }

    #[test]
    fn model_thought_is_hidden_from_observation_panel() {
        let events = observation_events_from_model_response(
            r#"{"thought":{"content":"内部推理，不给用户看","durable":true},"report_job_progress":"ok","continue":false}"#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn model_response_does_not_expose_internal_action_name() {
        let mut panel = ObservationPanel::default();
        panel.apply_all(observation_events_from_model_response(
            r#"{"next_actions":[{"action":"run_bash","intent":"统计","input":{"command":"rg --files | wc -l"}}]}"#,
        ));
        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("· 统计"));
        assert!(rendered.contains("└─ Bash:"));
        assert!(!rendered.contains("run_bash"));
    }

    #[test]
    fn tree_child_lines_render_under_intent_and_wrap_without_repeating_branch() {
        let mut panel = ObservationPanel::new(8, 44);
        panel.apply(ObservationEvent::Persistent("统计当前代码量".to_string()));
        panel.apply(ObservationEvent::ActiveChild {
            text: "Bash: 123456789012345678901234567890 tail".to_string(),
            is_last: true,
        });

        let rendered = render_observation_panel(&panel);
        assert!(rendered.contains("· 统计当前代码量"));
        assert!(rendered.contains("└─ Bash: 123456789012345678901234"));
        assert_eq!(rendered.matches("└─").count(), 1);
        assert!(rendered.contains("tail"));
    }

    #[test]
    fn run_bash_without_intent_shows_plain_label() {
        let events = observation_events_from_model_response(
            r#"{"next_actions":[{"action":"run_bash","input":{"command":"ls -la"}}]}"#,
        );
        assert_eq!(
            events,
            vec![ObservationEvent::Active("Bash: ls -la".to_string())]
        );
    }

    #[test]
    fn malformed_model_response_does_not_create_observation_events() {
        let events = observation_events_from_model_response(
            r#"{"thought":"partial","next_actions":[{"action":"run_bash""#,
        );
        assert!(events.is_empty());
    }

    #[test]
    fn panel_wraps_long_command_and_truncates_one_item_after_four_rows() {
        let mut panel = ObservationPanel::new(8, 44);
        panel.apply(ObservationEvent::Active(format!(
            "Bash: {}",
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
}
