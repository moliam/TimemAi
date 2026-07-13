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
#[path = "../tests/unit/final_answer_renderer_tests.rs"]
mod tests;
