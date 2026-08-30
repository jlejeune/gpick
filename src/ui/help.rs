use crate::app::Screen;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

/// Keybinding hints for the given screen, shown in a persistent one-line
/// footer so a first-time user always knows what to press.
pub fn footer_text(screen: &Screen) -> &'static str {
    match screen {
        Screen::BranchList => "↑/↓ move  type to filter  Enter select  Del delete (local only)  q/Esc quit",
        Screen::CommitList => "↑/↓ move  Space toggle  Enter cherry-pick  q/Esc back",
        Screen::Execution => "running…  q quit",
        Screen::ConflictPause => "c continue (after resolving + git add)  a abort  q/Esc quit",
        Screen::Quit => "",
    }
}

pub fn draw_footer(frame: &mut Frame, area: Rect, screen: &Screen) {
    draw_footer_text(frame, area, footer_text(screen));
}

pub fn draw_footer_text(frame: &mut Frame, area: Rect, text: &str) {
    let widget = Paragraph::new(text).style(Style::default().add_modifier(Modifier::DIM));
    frame.render_widget(widget, area);
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
}
