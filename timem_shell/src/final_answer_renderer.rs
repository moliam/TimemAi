pub trait FinalAnswerRenderer {
    fn render(&self, markdown: &str) -> String;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TermimadFinalAnswerRenderer;

impl FinalAnswerRenderer for TermimadFinalAnswerRenderer {
    fn render(&self, markdown: &str) -> String {
        let skin = termimad::MadSkin::default();
        skin.term_text(markdown).to_string()
    }
}

pub fn render_final_answer_markdown(markdown: &str) -> String {
    TermimadFinalAnswerRenderer.render(markdown)
}
