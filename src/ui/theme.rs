use ratatui::prelude::*;
use ratatui::widgets::{Block, BorderType, Borders, Scrollbar, ScrollbarOrientation, ScrollbarState};

pub const ACCENT: Color = Color::Cyan;
pub const LOCAL: Color = Color::Green;
pub const REMOTE: Color = Color::Blue;
pub const SUCCESS: Color = Color::Green;
pub const ERROR: Color = Color::Red;
pub const PENDING: Color = Color::Yellow;
pub const MUTED: Color = Color::DarkGray;

/// A bordered block with rounded corners and a bold title, used by every
/// screen so the whole app shares one visual language.
pub fn titled_block(title: &str) -> Block<'static> {
    Block::default()
        .title(Span::styled(title.to_string(), Style::default().add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
}

/// The cursor highlight style shared by every stateful list.
pub fn highlight_style() -> Style {
    Style::default().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)
}

/// Breadcrumb text shown in the header bar for a given navigation path,
/// e.g. `&["Branches", "feature-x", "Commits"]` -> "gpick › Branches › feature-x › Commits".
pub fn breadcrumb(segments: &[&str]) -> String {
    let mut s = String::from("gpick");
    for seg in segments {
        s.push_str(" › ");
        s.push_str(seg);
    }
    s
}

/// Draws a vertical scrollbar along the right edge of `area`, sized to
/// `content_len` items with the viewport currently at `position`. Renders
/// nothing (via a zero-length state) when everything already fits, so it's
/// safe to call unconditionally after drawing a list.
pub fn draw_scrollbar(frame: &mut Frame, area: Rect, content_len: usize, position: usize) {
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight).begin_symbol(None).end_symbol(None);
    let mut state = ScrollbarState::new(content_len).position(position);
    frame.render_stateful_widget(scrollbar, area.inner(Margin { vertical: 1, horizontal: 0 }), &mut state);
}

pub fn draw_header(frame: &mut Frame, area: Rect, segments: &[&str]) {
    use ratatui::widgets::Paragraph;
    let widget = Paragraph::new(breadcrumb(segments))
        .style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD));
    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breadcrumb_with_no_segments_is_just_the_app_name() {
        assert_eq!(breadcrumb(&[]), "gpick");
    }

    #[test]
    fn breadcrumb_joins_segments_with_separator() {
        assert_eq!(breadcrumb(&["Branches", "feature-x", "Commits"]), "gpick › Branches › feature-x › Commits");
    }
}
