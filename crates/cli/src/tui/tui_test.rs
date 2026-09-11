use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

use super::*;

/// `tui-markdown` sets a heading's style on the Line itself, not per-span.
/// Regression check for `ratatui_core_lines()` dropping that Line-level
/// style during the ratatui-core -> ratatui conversion.
#[test]
fn ratatui_core_lines_carries_heading_style() {
    let lines = ratatui_core_lines("# Heading\n");
    let heading_line = &lines[0];
    assert_eq!(
        heading_line.style.add_modifier,
        Modifier::BOLD | Modifier::UNDERLINED,
        "heading line lost its bold/underline style: {:?}",
        heading_line.style
    );
    assert_eq!(heading_line.style.bg, Some(Color::Cyan));
}

/// Fence markers are hidden (via NoFenceStyleSheet + code_block_lines'
/// boxed rendering) — the raw "```lang" line should never appear.
#[test]
fn code_block_fence_markers_are_hidden() {
    let lines = markdown_lines("```rust\nfn main() {}\n```\n", 80);
    let rendered: String = lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !rendered.contains("```"),
        "fence markers leaked into render:\n{rendered}"
    );
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
