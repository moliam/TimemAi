pub trait FinalAnswerRenderer {
    fn render(&self, markdown: &str) -> String;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TermimadFinalAnswerRenderer;

impl FinalAnswerRenderer for TermimadFinalAnswerRenderer {
    fn render(&self, markdown: &str) -> String {
        let mut skin = termimad::MadSkin::default();
        skin.table_border_chars = termimad::ROUNDED_TABLE_BORDER_CHARS;
        let markdown = normalize_terminal_markdown(markdown);
        skin.term_text(&markdown).to_string()
    }
}

pub fn render_final_answer_markdown(markdown: &str) -> String {
    TermimadFinalAnswerRenderer.render(markdown)
}

fn normalize_terminal_markdown(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut in_fence = false;

    for line in markdown.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            in_fence = !in_fence;
            out.push_str(line);
        } else if !in_fence && is_standalone_separator(trimmed) {
            out.push_str("────────");
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }

    if !markdown.ends_with('\n') {
        out.pop();
    }
    out
}

fn is_standalone_separator(trimmed: &str) -> bool {
    let mut chars = trimmed.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    let min_len = match first {
        '-' | '*' | '_' => 3,
        '―' | '─' | '—' | '–' => 8,
        _ => return false,
    };
    let len = 1 + chars.clone().count();
    len >= min_len && chars.all(|c| c == first)
}

#[cfg(test)]
mod tests {
    use super::{normalize_terminal_markdown, render_final_answer_markdown};

    #[test]
    fn markdown_horizontal_rules_are_shortened_before_termimad() {
        let normalized = normalize_terminal_markdown("示例1\n\n---\n\n示例2");
        assert_eq!(normalized, "示例1\n\n────────\n\n示例2");

        let rendered = render_final_answer_markdown("示例1\n\n---\n\n示例2");
        assert!(rendered.contains("────────"));
        assert!(!rendered.contains("――――――――――――――――"));
    }

    #[test]
    fn code_fence_separators_are_preserved() {
        let normalized = normalize_terminal_markdown("```text\n---\n```\n\n---");
        assert_eq!(normalized, "```text\n---\n```\n\n────────");
    }

    #[test]
    fn already_expanded_separator_lines_are_shortened() {
        let normalized = normalize_terminal_markdown("a\n――――――――――――――――――――\nb");
        assert_eq!(normalized, "a\n────────\nb");
    }

    #[test]
    fn final_answer_renderer_uses_termimad_rounded_table_rules() {
        let rendered = render_final_answer_markdown(
            "| --- | --- |\n| 文件 | 变更规模 |\n| --- | --- |\n| src/main.rs | 少量调整 |\n| --- | --- |\n",
        );
        assert!(rendered.contains("╭"));
        assert!(rendered.contains("╮"));
        assert!(rendered.contains("╰"));
        assert!(rendered.contains("╯"));
    }
}
