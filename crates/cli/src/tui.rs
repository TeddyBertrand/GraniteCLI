use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use granite_core::agent::Agent;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

/// Enters raw mode + alt-screen on construction, restores the terminal on
/// drop (covers normal return, `?` early-return, and panics via the panic
/// hook installed in `enter`).
pub struct AltScreenGuard;

impl AltScreenGuard {
    pub fn enter() -> anyhow::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;

        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = crossterm::terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            default_hook(info);
        }));

        Ok(Self)
    }
}

impl Drop for AltScreenGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

/// `tui-markdown` renders against `ratatui-core` types, which are structurally
/// identical to but a distinct crate from the `ratatui` 0.29 types used here.
/// Round-trip colors/modifiers through their matching string reprs to convert.
fn convert_style(style: ratatui_core::style::Style) -> Style {
    let mut out = Style::default();
    if let Some(fg) = style.fg {
        out.fg = fg.to_string().parse().ok();
    }
    if let Some(bg) = style.bg {
        out.bg = bg.to_string().parse().ok();
    }
    out = out.add_modifier(convert_modifier(style.add_modifier));
    out
}

fn convert_modifier(modifier: ratatui_core::style::Modifier) -> ratatui::style::Modifier {
    use ratatui::style::Modifier;
    let flags = [
        (ratatui_core::style::Modifier::BOLD, Modifier::BOLD),
        (ratatui_core::style::Modifier::DIM, Modifier::DIM),
        (ratatui_core::style::Modifier::ITALIC, Modifier::ITALIC),
        (ratatui_core::style::Modifier::UNDERLINED, Modifier::UNDERLINED),
        (ratatui_core::style::Modifier::SLOW_BLINK, Modifier::SLOW_BLINK),
        (ratatui_core::style::Modifier::RAPID_BLINK, Modifier::RAPID_BLINK),
        (ratatui_core::style::Modifier::REVERSED, Modifier::REVERSED),
        (ratatui_core::style::Modifier::HIDDEN, Modifier::HIDDEN),
        (ratatui_core::style::Modifier::CROSSED_OUT, Modifier::CROSSED_OUT),
    ];
    flags.into_iter().fold(Modifier::empty(), |acc, (src, dst)| {
        if modifier.contains(src) {
            acc | dst
        } else {
            acc
        }
    })
}

fn markdown_lines(text: &str) -> Vec<Line<'static>> {
    tui_markdown::from_str(text)
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content.into_owned(), convert_style(span.style)))
                .collect();
            Line::from(spans)
        })
        .collect()
}

enum Speaker {
    User,
    Agent,
    Error,
}

struct HistoryEntry {
    speaker: Speaker,
    text: String,
}

/// Runs the interactive chat loop: a bordered scrollable history panel with
/// a prompt input line pinned to the bottom. Keeps prompting until the user
/// quits (`/exit`, `/quit`, Esc, or Ctrl+C) instead of returning after one turn.
pub async fn run_chat_loop(agent: &mut Agent) -> anyhow::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let mut history: Vec<HistoryEntry> = Vec::new();
    let mut input = String::new();
    let mut status: Option<String> = None;

    loop {
        terminal.draw(|frame| draw(frame, &history, &input, status.as_deref()))?;

        if !event::poll(Duration::from_millis(100))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        match key.code {
            KeyCode::Esc => break,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::Enter => {
                let line = input.trim().to_string();
                input.clear();
                if line.is_empty() {
                    continue;
                }
                if line == "/exit" || line == "/quit" {
                    break;
                }

                history.push(HistoryEntry {
                    speaker: Speaker::User,
                    text: line.clone(),
                });
                status = Some("thinking...".to_string());
                terminal.draw(|frame| draw(frame, &history, &input, status.as_deref()))?;

                match agent.run(line).await {
                    Ok(answer) => history.push(HistoryEntry {
                        speaker: Speaker::Agent,
                        text: answer,
                    }),
                    Err(err) => history.push(HistoryEntry {
                        speaker: Speaker::Error,
                        text: err.to_string(),
                    }),
                }
                status = None;
            }
            KeyCode::Backspace => {
                input.pop();
            }
            KeyCode::Char(c) => input.push(c),
            _ => {}
        }
    }

    Ok(())
}

fn draw(
    frame: &mut ratatui::Frame,
    history: &[HistoryEntry],
    input: &str,
    status: Option<&str>,
) {
    let [history_area, input_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(3)]).areas(frame.area());

    let lines: Vec<Line> = history
        .iter()
        .flat_map(|entry| {
            let (prefix, color) = match entry.speaker {
                Speaker::User => ("you", Color::Cyan),
                Speaker::Agent => ("granite", Color::Green),
                Speaker::Error => ("error", Color::Red),
            };
            let mut lines = vec![Line::from(Span::styled(
                format!("{prefix}:"),
                Style::default().fg(color),
            ))];
            match entry.speaker {
                Speaker::Agent => {
                    lines.extend(markdown_lines(&entry.text));
                }
                Speaker::User | Speaker::Error => {
                    lines.extend(entry.text.lines().map(|l| Line::from(l.to_string())));
                }
            }
            lines.push(Line::from(""));
            lines
        })
        .collect();

    let history_block = Block::default().borders(Borders::ALL).title("granite");
    let inner_height = history_block.inner(history_area).height as usize;
    let scroll = lines.len().saturating_sub(inner_height) as u16;

    let history_widget = Paragraph::new(lines)
        .block(history_block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(history_widget, history_area);

    let input_title = status.unwrap_or("prompt (Enter to send, /exit to quit)");
    let input_widget = Paragraph::new(input)
        .block(Block::default().borders(Borders::ALL).title(input_title));
    frame.render_widget(input_widget, input_area);

    frame.set_cursor_position((
        input_area.x + 1 + input.len() as u16,
        input_area.y + 1,
    ));
}
