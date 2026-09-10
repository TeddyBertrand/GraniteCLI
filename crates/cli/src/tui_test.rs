use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

use super::*;

/// `tui-markdown` sets a heading's style on the Line itself, not per-span.
/// Regression check for markdown_lines() dropping that Line-level style
/// during the ratatui-core -> ratatui conversion.
#[test]
fn markdown_lines_carries_heading_style() {
    let lines = markdown_lines("# Heading\n");
    let heading_line = &lines[0];
    assert_eq!(
        heading_line.style.add_modifier,
        Modifier::BOLD | Modifier::UNDERLINED,
        "heading line lost its bold/underline style: {:?}",
        heading_line.style
    );
    assert_eq!(heading_line.style.bg, Some(Color::Cyan));
}

/// Renders a heading + table agent reply into the history panel and checks
/// the table's box-drawing rows survive the panel's word-wrap intact.
#[test]
fn markdown_heading_and_table_render() {
    let backend = TestBackend::new(40, 20);
    let mut terminal = Terminal::new(backend).unwrap();

    let history = vec![HistoryEntry {
        speaker: Speaker::Agent,
        text: "# Heading\n\n| a | b |\n|---|---|\n| 1 | 2 |\n".to_string(),
    }];

    terminal
        .draw(|frame| draw(frame, &history, "", None, 0))
        .unwrap();

    let buffer = terminal.backend().buffer().clone();
    let rendered: String = (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n");

    for line in rendered.lines() {
        // Skip the outer history-panel frame itself (┌granite──┐ / └──┘),
        // only inspect markdown-table border rows inside it.
        if line.starts_with('┌') || line.starts_with('└') {
            continue;
        }
        if line.contains('─') {
            assert!(
                line.contains('│') || line.contains('┬') || line.contains('┼'),
                "table border row split awkwardly: {line:?}"
            );
        }
    }

    assert!(
        rendered.contains("Heading"),
        "heading text missing from render:\n{rendered}"
    );
}
