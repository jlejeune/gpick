use crate::app::{AppState, Screen};
use crate::git::Branch;
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem};

pub fn visible_branches(state: &AppState) -> Vec<&Branch> {
    let filter = state.branch_filter.to_lowercase();
    state
        .branches
        .iter()
        .filter(|b| b.name.to_lowercase().contains(&filter))
        .collect()
}

pub fn handle_key_branch_list(state: &mut AppState, key: KeyCode) {
    let visible_len = visible_branches(state).len();
    match key {
        KeyCode::Down => {
            if visible_len > 0 && state.branch_cursor + 1 < visible_len {
                state.branch_cursor += 1;
            }
        }
        KeyCode::Up => {
            state.branch_cursor = state.branch_cursor.saturating_sub(1);
        }
        KeyCode::Char('q') => state.screen = Screen::Quit,
        KeyCode::Esc => state.screen = Screen::Quit,
        KeyCode::Backspace => {
            state.branch_filter.pop();
            state.branch_cursor = 0;
        }
        KeyCode::Char(c) => {
            state.branch_filter.push(c);
            state.branch_cursor = 0;
        }
        KeyCode::Enter => {
            if let Some(branch) = visible_branches(state).get(state.branch_cursor) {
                state.selected_branch = Some(branch.name.clone());
                state.screen = Screen::CommitList;
            }
        }
        _ => {}
    }
}

pub fn draw_branch_list(frame: &mut Frame, state: &AppState) {
    let visible = visible_branches(state);
    let items: Vec<ListItem> = if visible.is_empty() {
        let msg = state
            .last_error
            .clone()
            .unwrap_or_else(|| "No branches found".to_string());
        vec![ListItem::new(msg)]
    } else {
        visible.iter().map(|b| ListItem::new(b.name.clone())).collect()
    };
    let title = if state.branch_filter.is_empty() {
        format!("Branches (base: {})", state.base)
    } else {
        format!("Branches (base: {}, filter: {})", state.base, state.branch_filter)
    };
    let list = List::new(items).block(Block::default().title(title).borders(Borders::ALL));
    frame.render_widget(list, frame.area());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, Screen};
    use crate::git::Branch;
    use crossterm::event::KeyCode;

    fn state_with_branches(names: &[&str]) -> AppState {
        AppState::new(
            "/tmp".into(),
            "main".into(),
            names.iter().map(|n| Branch { name: n.to_string(), last_commit_epoch: 0 }).collect(),
        )
    }

    #[test]
    fn down_moves_cursor_and_clamps_at_end() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Down);
        assert_eq!(state.branch_cursor, 1);
        handle_key_branch_list(&mut state, KeyCode::Down);
        assert_eq!(state.branch_cursor, 1); // clamped
    }

    #[test]
    fn up_clamps_at_zero() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Up);
        assert_eq!(state.branch_cursor, 0);
    }

    #[test]
    fn typing_filters_visible_branches() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('f'));
        handle_key_branch_list(&mut state, KeyCode::Char('e'));
        let visible = visible_branches(&state);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "feature-x");
    }

    #[test]
    fn enter_selects_branch_and_moves_to_commit_list() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Down);
        handle_key_branch_list(&mut state, KeyCode::Enter);
        assert_eq!(state.selected_branch, Some("b".to_string()));
        assert_eq!(state.screen, Screen::CommitList);
    }

    #[test]
    fn q_quits() {
        let mut state = state_with_branches(&["a"]);
        handle_key_branch_list(&mut state, KeyCode::Char('q'));
        assert_eq!(state.screen, Screen::Quit);
    }
}
