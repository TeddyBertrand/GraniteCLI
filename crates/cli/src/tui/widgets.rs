use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub const PAD: &str = "    ";
pub const RIGHT_MARGIN: usize = 3;

pub struct BoxStyle {
    pub border_color: Color,
    pub bg: Color,
    pub title: Option<String>,
}

fn pad_bg(mut line: Line<'static>, width: usize, bg: Color) -> Line<'static> {
    let target_width = width.saturating_sub(RIGHT_MARGIN);
    let filled = line.width();
    if filled < target_width {
        line.spans.push(Span::styled(
            " ".repeat(target_width - filled),
            Style::default().bg(bg),
        ));
    }
    line
}

pub fn render_box(body: Vec<Line<'static>>, style: &BoxStyle, width: usize) -> Vec<Line<'static>> {
    let border = Style::default().fg(style.border_color).bg(style.bg);
    let mut lines = Vec::new();

    let header = match &style.title {
        Some(t) => format!("┌─ {t} ─"),
        None => "┌─".to_string(),
    };
    lines.push(pad_bg(
        Line::from(vec![Span::raw(PAD), Span::styled(header, border)]),
        width,
        style.bg,
    ));

    for line in body {
        let mut spans = vec![Span::raw(PAD), Span::styled("│ ", border)];
        spans.extend(line.spans.into_iter().map(|mut span| {
            span.style = span.style.bg(style.bg);
            span
        }));
        lines.push(pad_bg(Line::from(spans), width, style.bg));
    }

    lines.push(pad_bg(
        Line::from(vec![Span::raw(PAD), Span::styled("└─".to_string(), border)]),
        width,
        style.bg,
    ));
    lines.push(Line::from(""));
    lines
}
