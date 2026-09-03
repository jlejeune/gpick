use crate::app::Screen;
use crate::ui::theme;
use ratatui::prelude::*;
use ratatui::widgets::{Paragraph, Wrap};

/// Keybinding hints for the given screen, shown in a persistent one-line
/// footer so a first-time user always knows what to press.
pub fn footer_text(screen: &Screen) -> &'static str {
    match screen {
        Screen::BranchList => {
            "↑/↓ move  / search  a show all  Space select  Enter open  Del delete  p push to master  q/Esc quit"
        }
        Screen::CommitList => "↑/↓ move  Space toggle  Enter cherry-pick  q/Esc back",
        Screen::Execution => "running…  q quit",
        Screen::ConflictPause => "c continue (after resolving + git add)  a abort  q/Esc quit",
        Screen::Quit => "",
    }
}

/// Splits `"key description  key description  ..."` hint text into styled
/// spans — bold accent for the leading key token of each hint, dim for the
/// rest — so the footer is easier to scan than one flat line.
fn styled_footer_line(text: &str) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, part) in text.split("  ").enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        match part.split_once(' ') {
            Some((key, rest)) => {
                spans.push(Span::styled(
                    key.to_string(),
                    Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                ));
                spans.push(Span::raw(" "));
                spans.push(Span::styled(rest.to_string(), Style::default().add_modifier(Modifier::DIM)));
            }
            None => spans.push(Span::styled(part.to_string(), Style::default().add_modifier(Modifier::DIM))),
        }
    }
    Line::from(spans)
}

/// Renders the plain keybinding-hint text for the current screen, styled as
/// bold-key/dim-action pairs.
pub fn draw_footer_text(frame: &mut Frame, area: Rect, text: &str) {
    let lines: Vec<Line> = text.lines().map(styled_footer_line).collect();
    let widget = Paragraph::new(lines).wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

/// Renders a one-off confirmation or error message in the footer area as a
/// single styled block, without the key/action split (there's no shortcut
/// pattern to split — it's a sentence).
pub fn draw_message_text(frame: &mut Frame, area: Rect, text: &str, color: Color) {
    let widget = Paragraph::new(text)
        .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
        .wrap(Wrap { trim: true });
    frame.render_widget(widget, area);
}

/// How many terminal rows the footer needs to show `text` without
/// truncating, given it will render into a box `width` columns wide.
pub fn footer_height(text: &str, width: u16) -> u16 {
    let width = width.max(1) as usize;
    text.lines()
        .map(|line| ((line.chars().count().max(1) - 1) / width + 1) as u16)
        .sum::<u16>()
        .max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_screen_has_non_empty_footer_text_except_quit() {
        for screen in [
            Screen::BranchList,
            Screen::CommitList,
            Screen::Execution,
            Screen::ConflictPause,
        ] {
            assert!(!footer_text(&screen).is_empty(), "{screen:?} has empty footer text");
        }
        assert_eq!(footer_text(&Screen::Quit), "");
    }

    #[test]
    fn footer_height_is_one_when_text_fits() {
        assert_eq!(footer_height("short", 80), 1);
    }

    #[test]
    fn footer_height_grows_for_long_wrapped_text() {
        let text = "a".repeat(85);
        assert_eq!(footer_height(&text, 40), 3);
    }

    #[test]
    fn footer_height_never_zero_for_empty_text() {
        assert_eq!(footer_height("", 80), 1);
    }
}
