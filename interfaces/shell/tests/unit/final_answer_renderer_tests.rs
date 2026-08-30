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
