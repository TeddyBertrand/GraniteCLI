use async_trait::async_trait;
use granite_core::{ConfirmHook, ConfirmRequest};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use tokio::sync::{mpsc, oneshot};
use tools::ToolRisk;

pub type ConfirmMsg = (ConfirmRequest, oneshot::Sender<bool>);

pub struct TuiConfirmHook {
    tx: mpsc::UnboundedSender<ConfirmMsg>,
}

impl TuiConfirmHook {
    pub fn new(tx: mpsc::UnboundedSender<ConfirmMsg>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ConfirmHook for TuiConfirmHook {
    async fn confirm(&self, request: ConfirmRequest) -> bool {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self.tx.send((request, reply_tx)).is_err() {
            return false;
        }
        reply_rx.await.unwrap_or(false)
    }
}

fn risk_color(risk: ToolRisk) -> Color {
    match risk {
        ToolRisk::Safe => Color::Green,
        ToolRisk::Mutating => Color::Yellow,
        ToolRisk::Dangerous => Color::Red,
    }
}

fn risk_label(risk: ToolRisk) -> &'static str {
    match risk {
        ToolRisk::Safe => "safe",
        ToolRisk::Mutating => "mutating",
        ToolRisk::Dangerous => "dangerous",
    }
}

fn build_lines(request: &ConfirmRequest, inner_width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![Line::from(vec![
        Span::styled("risk: ", Style::default().fg(Color::DarkGray)),
        Span::styled(risk_label(request.risk), Style::default().fg(risk_color(request.risk))),
    ])];
    lines.extend(super::markdown_lines(&request.detail, inner_width));
    lines.push(Line::from(vec![
        Span::styled("[y]", Style::default().fg(Color::Green)),
        Span::raw(" allow    "),
        Span::styled("[n]", Style::default().fg(Color::Red)),
        Span::raw(" deny"),
    ]));
    lines
}

pub fn height(request: &ConfirmRequest, area_width: u16, max_height: u16) -> u16 {
    let inner_width = area_width.saturating_sub(2).max(1) as usize;
    let content_lines = build_lines(request, inner_width).len() as u16;
    (content_lines + 2).min(max_height)
}

pub fn render(frame: &mut ratatui::Frame, area: Rect, request: &ConfirmRequest) {
    let border_color = risk_color(request.risk);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(format!("confirm: {}", request.tool_name));

    let inner_width = block.inner(area).width as usize;
    let lines = build_lines(request, inner_width);

    let widget = Paragraph::new(lines).block(block).wrap(Wrap { trim: false });
    frame.render_widget(widget, area);
}
