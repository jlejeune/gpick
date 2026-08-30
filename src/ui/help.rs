use crate::app::AppState;
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Handles a key press for the help overlay. Returns `true` if the key was
/// consumed here (the caller must not dispatch it to the active screen).
pub fn handle_key_help(state: &mut AppState, key: KeyCode) -> bool {
    if state.show_help {
        if matches!(key, KeyCode::Char('?') | KeyCode::Esc) {
            state.show_help = false;
        }
        // Swallow every key while help is open so it can't leak through
        // to the underlying screen (e.g. 'q' quitting instead of closing help).
        true
    } else if key == KeyCode::Char('?') {
        state.show_help = true;
        true
    } else {
        false
    }
}

const HELP_TEXT: &str = "\
Branches
  \u{2191}/\u{2193}      move cursor
  type    filter by name
  Enter   select branch
  q/Esc   quit

Commits
  \u{2191}/\u{2193}      move cursor (preview follows)
  Space   toggle selection
  Enter   cherry-pick selected commits
  q/Esc   back to branches

Execution
  (runs automatically)
  q       quit

Conflict pause
  c       continue (resolve conflicts + git add first)
  a       abort
  q/Esc   quit

Global
  Ctrl+C  quit immediately
  ?       toggle this help
";

pub fn draw_help(frame: &mut Frame) {
    let area = centered_rect(60, 70, frame.area());
    frame.render_widget(Clear, area);
    let block = Block::default().title("Help (? or Esc to close)").borders(Borders::ALL);
    let paragraph = Paragraph::new(HELP_TEXT).block(block);
    frame.render_widget(paragraph, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppState;

    fn state() -> AppState {
        AppState::new("/tmp".into(), "main".into(), vec![])
    }

    #[test]
    fn question_mark_opens_help() {
        let mut state = state();
        let consumed = handle_key_help(&mut state, KeyCode::Char('?'));
        assert!(consumed);
        assert!(state.show_help);
    }

    #[test]
    fn question_mark_closes_open_help() {
        let mut state = state();
        state.show_help = true;
        let consumed = handle_key_help(&mut state, KeyCode::Char('?'));
        assert!(consumed);
        assert!(!state.show_help);
    }

    #[test]
    fn esc_closes_open_help() {
        let mut state = state();
        state.show_help = true;
        let consumed = handle_key_help(&mut state, KeyCode::Esc);
        assert!(consumed);
        assert!(!state.show_help);
    }

    #[test]
    fn other_keys_are_swallowed_while_help_is_open() {
        let mut state = state();
        state.show_help = true;
        let consumed = handle_key_help(&mut state, KeyCode::Char('q'));
        assert!(consumed);
        assert!(state.show_help);
    }

    #[test]
    fn other_keys_are_not_consumed_when_help_is_closed() {
        let mut state = state();
        let consumed = handle_key_help(&mut state, KeyCode::Char('q'));
        assert!(!consumed);
        assert!(!state.show_help);
    }
}
