use std::io;

use crossterm::event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen};
use futures_util::StreamExt;
use granite_core::agent::Agent;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;

use crate::commands::{default_registry, CommandOutcome};

#[cfg(test)]
#[path = "tui_test.rs"]
mod tests;

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
            // `tui-markdown` sets heading style on the Line itself (see its
            // `start_heading`), not per-span — carry it over or headings
            // render with no color/weight at all.
            Line::from(spans).style(convert_style(line.style))
        })
        .collect()
}

pub(crate) enum Speaker {
    User,
    Agent,
    Error,
}

pub(crate) struct HistoryEntry {
    pub(crate) speaker: Speaker,
    pub(crate) text: String,
}

const SCROLL_STEP: u16 = 1;
const PAGE_SCROLL_STEP: u16 = 10;

/// Runs the interactive chat loop: a bordered scrollable history panel with
/// a prompt input line pinned to the bottom. Keeps prompting until the user
/// quits (`/exit`, `/quit`, Esc, or Ctrl+C while idle) instead of returning
/// after one turn.
pub async fn run_chat_loop(agent: &mut Agent) -> anyhow::Result<()> {
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
    let mut events = EventStream::new();

    let mut history: Vec<HistoryEntry> = Vec::new();
    let mut input = String::new();
    let mut status: Option<String> = None;

    // Lines scrolled up from the bottom of the history panel; 0 auto-follows
    // the latest output. Reset to 0 whenever new content is appended.
    let mut scroll_up: u16 = 0;

    let mut prompt_history: Vec<String> = Vec::new();
    // Index into `prompt_history` while recalling with Up/Down; `None` means
    // the prompt input isn't currently showing a recalled entry.
    let mut recall_index: Option<usize> = None;

    let registry = default_registry();

    loop {
        terminal.draw(|frame| draw(frame, &history, &input, status.as_deref(), scroll_up))?;

        let Some(key) = next_key(&mut events).await? else {
            break;
        };

        match key.code {
            KeyCode::Esc => break,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
            KeyCode::PageUp => scroll_up = scroll_up.saturating_add(PAGE_SCROLL_STEP),
            KeyCode::PageDown => scroll_up = scroll_up.saturating_sub(PAGE_SCROLL_STEP),
            KeyCode::Up if input.is_empty() && !prompt_history.is_empty() => {
                let next_index = match recall_index {
                    Some(i) => i.saturating_sub(1),
                    None => prompt_history.len() - 1,
                };
                recall_index = Some(next_index);
                input = prompt_history[next_index].clone();
            }
            KeyCode::Up => scroll_up = scroll_up.saturating_add(SCROLL_STEP),
            KeyCode::Down if recall_index.is_some() => {
                let current = recall_index.unwrap();
                if current + 1 < prompt_history.len() {
                    recall_index = Some(current + 1);
                    input = prompt_history[current + 1].clone();
                } else {
                    recall_index = None;
                    input.clear();
                }
            }
            KeyCode::Down => scroll_up = scroll_up.saturating_sub(SCROLL_STEP),
            KeyCode::Enter => {
                let line = input.trim().to_string();
                input.clear();
                recall_index = None;
                if line.is_empty() {
                    continue;
                }
                if line.starts_with('/') {
                    match registry.dispatch(&line) {
                        Some(CommandOutcome::Exit) => break,
                        None => {
                            history.push(HistoryEntry {
                                speaker: Speaker::Error,
                                text: format!("unknown command: {line}"),
                            });
                            scroll_up = 0;
                            continue;
                        }
                    }
                }

                prompt_history.push(line.clone());
                history.push(HistoryEntry {
                    speaker: Speaker::User,
                    text: line.clone(),
                });
                status = Some("thinking...".to_string());
                scroll_up = 0;
                terminal.draw(|frame| draw(frame, &history, &input, status.as_deref(), scroll_up))?;

                let turn = run_turn(agent, line, &mut events).await;
                match turn {
                    TurnOutcome::Answer(answer) => history.push(HistoryEntry {
                        speaker: Speaker::Agent,
                        text: answer,
                    }),
                    TurnOutcome::Error(err) => history.push(HistoryEntry {
                        speaker: Speaker::Error,
                        text: err,
                    }),
                    TurnOutcome::Cancelled => history.push(HistoryEntry {
                        speaker: Speaker::Error,
                        text: "cancelled".to_string(),
                    }),
                }
                status = None;
                scroll_up = 0;
            }
            KeyCode::Backspace => {
                input.pop();
                recall_index = None;
            }
            KeyCode::Char(c) => {
                input.push(c);
                recall_index = None;
            }
            _ => {}
        }
    }

    Ok(())
}

enum TurnOutcome {
    Answer(String),
    Error(String),
    Cancelled,
}

/// Runs one agent turn, racing it against incoming terminal events so a
/// Ctrl+C during the turn cancels it (dropping the in-flight future) instead
/// of exiting the app.
async fn run_turn(agent: &mut Agent, line: String, events: &mut EventStream) -> TurnOutcome {
    tokio::select! {
        result = agent.run(line) => match result {
            Ok(answer) => TurnOutcome::Answer(answer),
            Err(err) => TurnOutcome::Error(err.to_string()),
        },
        () = wait_for_cancel(events) => TurnOutcome::Cancelled,
    }
}

async fn wait_for_cancel(events: &mut EventStream) {
    loop {
        let Some(Ok(Event::Key(key))) = events.next().await else {
            return;
        };
        if key.kind == KeyEventKind::Press
            && key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
        {
            return;
        }
    }
}

async fn next_key(events: &mut EventStream) -> anyhow::Result<Option<crossterm::event::KeyEvent>> {
    loop {
        let Some(event) = events.next().await else {
            return Ok(None);
        };
        let Event::Key(key) = event? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        return Ok(Some(key));
    }
}

fn draw(
    frame: &mut ratatui::Frame,
    history: &[HistoryEntry],
    input: &str,
    status: Option<&str>,
    scroll_up: u16,
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
    let bottom_scroll = lines.len().saturating_sub(inner_height) as u16;
    let scroll = bottom_scroll.saturating_sub(scroll_up);

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
