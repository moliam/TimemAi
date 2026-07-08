use std::sync::OnceLock;
use syntect::{
    easy::HighlightLines,
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
    util::{as_24_bit_terminal_escaped, LinesWithEndings},
};

pub trait FinalAnswerRenderer {
    fn render(&self, markdown: &str) -> String;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TermimadFinalAnswerRenderer;

impl FinalAnswerRenderer for TermimadFinalAnswerRenderer {
    fn render(&self, markdown: &str) -> String {
        render_markdown_for_terminal(markdown)
    }
}

pub fn render_final_answer_markdown(markdown: &str) -> String {
    TermimadFinalAnswerRenderer.render(markdown)
}

fn render_markdown_for_terminal(markdown: &str) -> String {
    let mut out = String::new();
    let mut normal_chunk = String::new();
    let mut lines = markdown.lines();

    while let Some(line) = lines.next() {
        if let Some(fence) = parse_fence_start(line) {
            out.push_str(&render_termimad_chunk(&normal_chunk));
            normal_chunk.clear();

            let mut code = String::new();
            for code_line in lines.by_ref() {
                if is_matching_fence_end(code_line, &fence.marker) {
                    break;
                }
                code.push_str(code_line);
                code.push('\n');
            }
            out.push_str(&render_code_block(&code, fence.language.as_deref()));
        } else {
            normal_chunk.push_str(line);
            normal_chunk.push('\n');
        }
    }

    out.push_str(&render_termimad_chunk(&normal_chunk));
    if !markdown.ends_with('\n') && out.ends_with('\n') {
        out.pop();
    }
    out
}

fn render_termimad_chunk(markdown: &str) -> String {
    if markdown.is_empty() {
        return String::new();
    }
    let mut skin = termimad::MadSkin::default();
    skin.table_border_chars = termimad::ROUNDED_TABLE_BORDER_CHARS;
    let markdown = normalize_terminal_markdown(markdown);
    skin.term_text(&markdown).to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FenceStart {
    marker: String,
    language: Option<String>,
}

fn parse_fence_start(line: &str) -> Option<FenceStart> {
    let trimmed = line.trim_start();
    let marker = if trimmed.starts_with("```") {
        "```"
    } else if trimmed.starts_with("~~~") {
        "~~~"
    } else {
        return None;
    };
    let rest = trimmed.trim_start_matches(marker).trim();
    Some(FenceStart {
        marker: marker.to_string(),
        language: rest
            .split_whitespace()
            .next()
            .filter(|lang| !lang.is_empty())
            .map(str::to_string),
    })
}

fn is_matching_fence_end(line: &str, marker: &str) -> bool {
    line.trim_start().starts_with(marker)
}

fn render_code_block(code: &str, language: Option<&str>) -> String {
    if let Some(rendered) = render_code_block_with_syntect(code, language) {
        rendered
    } else {
        render_termimad_chunk(&format!("```text\n{code}```\n"))
    }
}

fn render_code_block_with_syntect(code: &str, language: Option<&str>) -> Option<String> {
    let syntax_set = syntax_set();
    let syntax = language
        .and_then(|lang| find_syntax_for_language(syntax_set, lang))
        .unwrap_or_else(|| syntax_set.find_syntax_plain_text());
    let theme = terminal_theme()?;
    let mut highlighter = HighlightLines::new(syntax, theme);
    let mut out = String::new();
    for line in LinesWithEndings::from(code) {
        let ranges = highlighter.highlight_line(line, syntax_set).ok()?;
        out.push_str(&as_24_bit_terminal_escaped(&ranges[..], false));
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    Some(out)
}

fn syntax_set() -> &'static SyntaxSet {
    static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme_set() -> &'static ThemeSet {
    static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

fn terminal_theme() -> Option<&'static Theme> {
    let themes = &theme_set().themes;
    themes
        .get("base16-ocean.dark")
        .or_else(|| themes.get("InspiredGitHub"))
        .or_else(|| themes.values().next())
}

fn find_syntax_for_language<'a>(
    syntax_set: &'a SyntaxSet,
    language: &str,
) -> Option<&'a SyntaxReference> {
    let language = language.trim().trim_start_matches('.').to_ascii_lowercase();
    let normalized = match language.as_str() {
        "c++" => "cpp",
        "c#" => "cs",
        "shell" | "zsh" | "sh" => "bash",
        "js" | "node" => "javascript",
        "ts" => "typescript",
        "py" => "python",
        "rs" => "rust",
        other => other,
    };
    syntax_set
        .find_syntax_by_token(normalized)
        .or_else(|| syntax_set.find_syntax_by_extension(normalized))
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

    #[test]
    fn rust_code_fence_uses_syntect_truecolor_highlighting() {
        let rendered =
            render_final_answer_markdown("```rust\nfn main() {\n    println!(\"hi\");\n}\n```");
        assert!(rendered.contains("fn"));
        assert!(rendered.contains("main"));
        assert!(
            rendered.contains("\x1b[38;2;"),
            "expected syntect 24-bit ANSI colors, got: {rendered:?}"
        );
    }

    #[test]
    fn bash_and_alias_code_fences_are_highlighted() {
        for lang in ["bash", "sh", "zsh", "shell"] {
            let rendered =
                render_final_answer_markdown(&format!("```{lang}\necho \"hello\" && exit 0\n```"));
            assert!(rendered.contains("echo"));
            assert!(
                rendered.contains("\x1b[38;2;"),
                "expected syntect colors for {lang}, got: {rendered:?}"
            );
        }
    }

    #[test]
    fn unknown_code_fence_language_does_not_crash() {
        let rendered = render_final_answer_markdown("```not-a-real-language\nalpha\nbeta\n```");
        assert!(rendered.contains("alpha"));
        assert!(rendered.contains("beta"));
    }

    #[test]
    fn mixed_markdown_and_code_preserves_surrounding_text() {
        let rendered = render_final_answer_markdown("前文\n\n```rust\nlet x = 1;\n```\n\n后文");
        assert!(rendered.contains("前文"));
        assert!(rendered.contains("let"));
        assert!(rendered.contains("后文"));
    }
}
