use std::io;
use std::sync::Arc;

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

use crate::args::Provider;
use crate::commands::{default_registry, CommandOutcome};
use crate::config::Config;

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

/// Default `tui-markdown` style sheet prints the ```` ``` ```` fence lines as literal text.
/// Hide them by overriding `code_block_fence` to empty, keeping syntax highlighting.
#[derive(Debug, Clone)]
struct NoFenceStyleSheet;

impl tui_markdown::StyleSheet for NoFenceStyleSheet {
    fn code_block_fence(&self) -> &str {
        ""
    }
}

const CODE_BORDER: Color = Color::DarkGray;
const CODE_BG: Color = Color::Rgb(40, 40, 40);

fn ratatui_core_lines(text: &str) -> Vec<Line<'static>> {
    let options = tui_markdown::Options::new(NoFenceStyleSheet);
    tui_markdown::from_str_with_options(text, &options)
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

enum MarkdownSegment<'a> {
    Text(&'a str),
    Code { lang: &'a str, body: &'a str },
}

/// Splits on ` ``` ` fence lines ourselves so code blocks can be boxed separately from
/// prose — tui-markdown renders everything as one flat `Text`, with no way to tell a
/// consumer where a code block starts/ends after the fact.
fn split_code_blocks(text: &str) -> Vec<MarkdownSegment<'_>> {
    let mut segments = Vec::new();
    let mut rest = text;
    loop {
        let Some(fence_start) = rest.find("```") else {
            if !rest.is_empty() {
                segments.push(MarkdownSegment::Text(rest));
            }
            break;
        };
        if fence_start > 0 {
            segments.push(MarkdownSegment::Text(&rest[..fence_start]));
        }
        let after_open = &rest[fence_start + 3..];
        let lang_end = after_open.find('\n').unwrap_or(after_open.len());
        let lang = &after_open[..lang_end];
        let body_start = &after_open[lang_end..].trim_start_matches('\n');
        let Some(fence_end) = body_start.find("```") else {
            // Unterminated fence (still streaming) — treat the rest as code.
            segments.push(MarkdownSegment::Code {
                lang,
                body: body_start,
            });
            break;
        };
        let body = body_start[..fence_end].trim_end_matches('\n');
        segments.push(MarkdownSegment::Code { lang, body });
        rest = &body_start[fence_end + 3..];
    }
    segments
}

const PAD: &str = "    ";
const RIGHT_MARGIN: usize = 3;

/// Fills the rest of a code-block line with background so the box reads as one
/// panel, not just an outline around the text. Leaves `RIGHT_MARGIN` columns
/// unbackgrounded on the right, mirroring the left `PAD`.
fn pad_bg(mut line: Line<'static>, width: usize) -> Line<'static> {
    let target_width = width.saturating_sub(RIGHT_MARGIN);
    let filled = line.width();
    if filled < target_width {
        line.spans.push(Span::styled(
            " ".repeat(target_width - filled),
            Style::default().bg(CODE_BG),
        ));
    }
    line
}

/// `+`/`-` prefixed lines in a diff/patch block are diff markup, not syntax to
/// highlight — color them by prefix instead of running them through syntect.
fn diff_line_style(line: &str) -> Style {
    let base = Style::default().bg(CODE_BG);
    if line.starts_with('+') && !line.starts_with("+++") {
        base.fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        base.fg(Color::Red)
    } else {
        base
    }
}

fn code_block_lines(lang: &str, body: &str, width: usize) -> Vec<Line<'static>> {
    let is_diff = matches!(lang, "diff" | "patch");
    let fenced = format!("```{lang}\n{body}\n```");
    let border = Style::default().fg(CODE_BORDER).bg(CODE_BG);

    let mut lines = Vec::new();
    let header = if lang.is_empty() {
        "┌─ code ─".to_string()
    } else {
        format!("┌─ {lang} ─")
    };
    lines.push(pad_bg(
        Line::from(vec![Span::raw(PAD), Span::styled(header, border)]),
        width,
    ));
    if is_diff {
        for source_line in body.lines() {
            let spans = vec![
                Span::raw(PAD),
                Span::styled("│ ", border),
                Span::styled(source_line.to_string(), diff_line_style(source_line)),
            ];
            lines.push(pad_bg(Line::from(spans), width));
        }
    } else {
        for line in ratatui_core_lines(&fenced) {
            let mut spans = vec![Span::raw(PAD), Span::styled("│ ", border)];
            spans.extend(
                line.spans.into_iter().map(|mut span| {
                    span.style = span.style.bg(CODE_BG);
                    span
                }),
            );
            lines.push(pad_bg(Line::from(spans), width));
        }
    }
    lines.push(pad_bg(
        Line::from(vec![
            Span::raw(PAD),
            Span::styled("└─".to_string(), border),
        ]),
        width,
    ));
    lines.push(Line::from(""));
    lines
}

fn markdown_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    split_code_blocks(text)
        .into_iter()
        .flat_map(|segment| match segment {
            MarkdownSegment::Text(text) => ratatui_core_lines(text),
            MarkdownSegment::Code { lang, body } => code_block_lines(lang, body, width),
        })
        .collect()
}

pub(crate) enum Speaker {
    User,
    Agent,
    Error,
    Info,
}

pub(crate) struct HistoryEntry {
    pub(crate) speaker: Speaker,
    pub(crate) text: String,
}

const SCROLL_STEP: u16 = 1;
const PAGE_SCROLL_STEP: u16 = 10;

/// Which full-screen view the loop is currently rendering.
enum AppMode {
    Chat,
    /// Dedicated full-screen API key entry, replacing the chat view
    /// entirely. `mandatory` is true when this is the unavoidable
    /// first-run screen shown because no key resolved at startup — Esc
    /// there quits instead of falling back to a keyless chat.
    ApiKeySetup {
        input: String,
        mandatory: bool,
        error: Option<String>,
    },
}

/// Runs the interactive chat loop: a bordered scrollable history panel with
/// a prompt input line pinned to the bottom. Keeps prompting until the user
/// quits (`/exit`, `/quit`, Esc, or Ctrl+C while idle) instead of returning
/// after one turn.
///
/// If `agent` has no provider set (no API key resolved at startup), the
/// loop opens straight into the full-screen API key setup view instead of
/// the chat view — see issue #67.
pub async fn run_chat_loop(
    agent: &mut Agent,
    cfg: &Config,
    current_provider: &mut Provider,
) -> anyhow::Result<()> {
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

    let mut mode = if agent.has_provider() {
        AppMode::Chat
    } else {
        AppMode::ApiKeySetup {
            input: String::new(),
            mandatory: true,
            error: None,
        }
    };

    loop {
        match &mode {
            AppMode::Chat => {
                terminal.draw(|frame| draw(frame, &history, &input, status.as_deref(), scroll_up))?;
            }
            AppMode::ApiKeySetup { input: key_input, error, mandatory } => {
                terminal.draw(|frame| {
                    draw_api_key_setup(frame, *current_provider, key_input, error.as_deref(), *mandatory)
                })?;
            }
        }

        let Some(key) = next_key(&mut events).await? else {
            break;
        };

        if let AppMode::ApiKeySetup { input: key_input, mandatory, error } = &mut mode {
            match key.code {
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => break,
                KeyCode::Esc if *mandatory => break,
                KeyCode::Esc => mode = AppMode::Chat,
                KeyCode::Backspace => {
                    key_input.pop();
                }
                KeyCode::Char(c) => key_input.push(c),
                KeyCode::Enter => {
                    let raw = key_input.trim().to_string();
                    if raw.is_empty() {
                        continue;
                    }
                    match apply_api_key(*current_provider, raw) {
                        Ok(new_provider) => {
                            agent.set_provider(new_provider);
                            history.push(HistoryEntry {
                                speaker: Speaker::Agent,
                                text: format!("{} API key saved.", current_provider.label()),
                            });
                            mode = AppMode::Chat;
                        }
                        Err(err) => {
                            *error = Some(err.to_string());
                            key_input.clear();
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

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
                        Some(CommandOutcome::PromptApiKey) => {
                            mode = AppMode::ApiKeySetup {
                                input: String::new(),
                                mandatory: false,
                                error: None,
                            };
                            continue;
                        }
                        Some(CommandOutcome::Info(msg)) => {
                            history.push(HistoryEntry {
                                speaker: Speaker::Error,
                                text: msg,
                            });
                            scroll_up = 0;
                            continue;
                        }
                        Some(CommandOutcome::SwitchProvider(provider)) => {
                            match crate::build_provider(provider, None, None, cfg) {
                                Ok(Some((new_provider, new_model))) => {
                                    agent.set_provider(new_provider);
                                    agent.set_model(new_model.clone());
                                    *current_provider = provider;
                                    history.push(HistoryEntry {
                                        speaker: Speaker::Info,
                                        text: format!(
                                            "switched to provider {provider} (model {new_model})"
                                        ),
                                    });
                                }
                                Ok(None) => {
                                    *current_provider = provider;
                                    mode = AppMode::ApiKeySetup {
                                        input: String::new(),
                                        mandatory: false,
                                        error: None,
                                    };
                                }
                                Err(err) => history.push(HistoryEntry {
                                    speaker: Speaker::Error,
                                    text: err.to_string(),
                                }),
                            }
                            scroll_up = 0;
                            continue;
                        }
                        Some(CommandOutcome::SwitchModel(model)) => {
                            agent.set_model(model.clone());
                            history.push(HistoryEntry {
                                speaker: Speaker::Info,
                                text: format!("switched to model {model}"),
                            });
                            scroll_up = 0;
                            continue;
                        }
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

                if !agent.has_provider() {
                    history.push(HistoryEntry {
                        speaker: Speaker::Error,
                        text: format!(
                            "no {} API key configured — run /apikey to set one",
                            current_provider.label()
                        ),
                    });
                    scroll_up = 0;
                    continue;
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

/// Persists `key` to config as the given provider's API key and builds a
/// live provider instance for it.
fn apply_api_key(provider: Provider, key: String) -> anyhow::Result<Arc<dyn provider::LlmProvider>> {
    let mut cfg = Config::load()?;
    cfg.set(&format!("{}.api_key", provider.config_prefix()), &key)?;
    cfg.save()?;
    crate::provider_from_key(provider, key)
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

/// Full-screen (not inline) API key entry — replaces the chat view
/// entirely while active. Input is masked with `*` since it's a secret.
fn draw_api_key_setup(
    frame: &mut ratatui::Frame,
    provider: Provider,
    input: &str,
    error: Option<&str>,
    mandatory: bool,
) {
    let [banner_area, input_area, help_area, error_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .areas(frame.area());

    let banner = Paragraph::new(format!("Set your {} API key", provider.label())).block(
        Block::default()
            .borders(Borders::ALL)
            .title("granite — API key setup"),
    );
    frame.render_widget(banner, banner_area);

    let masked: String = "*".repeat(input.chars().count());
    let input_widget = Paragraph::new(masked).block(
        Block::default()
            .borders(Borders::ALL)
            .title("paste key, Enter to save"),
    );
    frame.render_widget(input_widget, input_area);

    let help_text = if mandatory {
        "Esc to quit · Ctrl+C to quit"
    } else {
        "Esc to cancel and return to chat · Ctrl+C to quit"
    };
    let help = Paragraph::new(Line::from(Span::styled(
        help_text,
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(help, help_area);

    if let Some(err) = error {
        let error_widget = Paragraph::new(err.to_string())
            .style(Style::default().fg(Color::Red))
            .wrap(Wrap { trim: false });
        frame.render_widget(error_widget, error_area);
    }

    frame.set_cursor_position((input_area.x + 1 + input.chars().count() as u16, input_area.y + 1));
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

    let history_block = Block::default().borders(Borders::ALL).title("granite");
    let inner_width = history_block.inner(history_area).width as usize;

    let lines: Vec<Line> = history
        .iter()
        .flat_map(|entry| {
            let (prefix, color) = match entry.speaker {
                Speaker::User => ("you", Color::Cyan),
                Speaker::Agent => ("granite", Color::Green),
                Speaker::Error => ("error", Color::Red),
                Speaker::Info => ("info", Color::Yellow),
            };
            let mut lines = vec![Line::from(Span::styled(
                format!("{prefix}:"),
                Style::default().fg(color),
            ))];
            match entry.speaker {
                Speaker::Agent => {
                    lines.extend(markdown_lines(&entry.text, inner_width));
                }
                Speaker::User | Speaker::Error | Speaker::Info => {
                    lines.extend(entry.text.lines().map(|l| Line::from(l.to_string())));
                }
            }
            lines.push(Line::from(""));
            lines
        })
        .collect();

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
