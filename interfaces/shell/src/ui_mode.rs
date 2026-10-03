use crate::{ANSI_BOLD, ANSI_BRIGHT_TIMEM, ANSI_DIM, ANSI_RESET};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use std::io::Write;

pub const UI_MODE_ENV: &str = "TIMEM_UI_MODE";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellUiMode {
    #[default]
    Ordinary,
    Stream,
}

impl ShellUiMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ordinary" => Ok(Self::Ordinary),
            "stream" => Ok(Self::Stream),
            _ => Err(format!("invalid_ui_mode:{value}; expected ordinary|stream")),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Ordinary => "Ordinary integrated",
            Self::Stream => "Streaming response",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Ordinary => "compact thinking, tools, and final answer",
            Self::Stream => "live provisional response with tool activity",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiModeResolution {
    Resolved(ShellUiMode),
    Select,
}

pub fn resolve_ui_mode(
    explicit: Option<&str>,
    env_value: Option<&str>,
    interactive_tty: bool,
    once_json: bool,
) -> Result<UiModeResolution, String> {
    if once_json {
        return Ok(UiModeResolution::Resolved(ShellUiMode::Ordinary));
    }
    if let Some(value) = explicit {
        return ShellUiMode::parse(value).map(UiModeResolution::Resolved);
    }
    if let Some(value) = env_value.filter(|value| !value.trim().is_empty()) {
        return ShellUiMode::parse(value).map(UiModeResolution::Resolved);
    }
    if interactive_tty {
        Ok(UiModeResolution::Select)
    } else {
        Ok(UiModeResolution::Resolved(ShellUiMode::Ordinary))
    }
}

pub fn ui_mode_selector(selected: ShellUiMode) -> String {
    let mut output = format!(
        "{ANSI_BOLD}Select session UI{ANSI_RESET} {ANSI_DIM}(↑/↓ move · Enter continue · Esc exit){ANSI_RESET}\r\n"
    );
    for mode in [ShellUiMode::Ordinary, ShellUiMode::Stream] {
        let marker = if mode == selected {
            format!("{ANSI_BRIGHT_TIMEM}❯{ANSI_RESET}")
        } else {
            " ".to_string()
        };
        output.push_str(&format!(
            "{marker} {ANSI_BOLD}{}{ANSI_RESET}  {ANSI_DIM}{}{ANSI_RESET}\r\n",
            mode.label(),
            mode.description()
        ));
    }
    output
}

pub fn select_ui_mode() -> Option<ShellUiMode> {
    use crossterm::terminal::{disable_raw_mode, enable_raw_mode};

    let mut selected = ShellUiMode::Ordinary;
    let _ = enable_raw_mode();
    let line_count = 3u16;
    let mut rendered_once = false;
    let render = |selected: ShellUiMode, rendered_once: bool| {
        use crossterm::cursor::MoveUp;
        use crossterm::queue;
        use crossterm::terminal::{Clear, ClearType};
        let mut stdout = std::io::stdout();
        if rendered_once {
            let _ = queue!(stdout, MoveUp(line_count), Clear(ClearType::FromCursorDown));
        }
        print!("{}", ui_mode_selector(selected));
        let _ = stdout.flush();
    };
    render(selected, rendered_once);
    rendered_once = true;
    let result = loop {
        match crossterm::event::read() {
            Ok(Event::Key(KeyEvent {
                code, modifiers, ..
            })) => match (code, modifiers) {
                (KeyCode::Char('c'), KeyModifiers::CONTROL) | (KeyCode::Esc, _) => break None,
                (KeyCode::Up | KeyCode::Down, _) => {
                    selected = match selected {
                        ShellUiMode::Ordinary => ShellUiMode::Stream,
                        ShellUiMode::Stream => ShellUiMode::Ordinary,
                    };
                    render(selected, rendered_once);
                }
                (KeyCode::Enter, _) => break Some(selected),
                _ => {}
            },
            Ok(_) => {}
            Err(_) => break None,
        }
    };
    let _ = disable_raw_mode();
    println!();
    result
}

#[cfg(test)]
#[path = "../tests/unit/ui_mode_tests.rs"]
mod tests;
