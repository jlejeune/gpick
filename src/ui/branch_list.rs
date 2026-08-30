use crate::app::{AppState, Screen};
use crate::git::{self, Branch};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState};

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
        KeyCode::Char('d') => {
            let local_branch = visible_branches(state)
                .get(state.branch_cursor)
                .filter(|b| b.is_local)
                .map(|b| b.name.clone());
            if let Some(name) = local_branch {
                state.pending_delete = Some(name);
            }
        }
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

/// Handles a key press while a delete confirmation is pending. Returns
/// `true` if the key was consumed here (the caller must not dispatch it
/// to `handle_key_branch_list`). While a confirmation is pending, every
/// key is swallowed so the user can't navigate mid-prompt.
pub fn handle_key_delete_confirm(state: &mut AppState, key: KeyCode) -> bool {
    let Some(name) = state.pending_delete.clone() else {
        return false;
    };

    match key {
        KeyCode::Char('y') => {
            match git::delete_branch(&state.cwd, &name) {
                Ok(()) => {
                    state.branches.retain(|b| b.name != name);
                    let visible_len = visible_branches(state).len();
                    if state.branch_cursor >= visible_len {
                        state.branch_cursor = visible_len.saturating_sub(1);
                    }
                    state.last_error = None;
                }
                Err(e) => {
                    state.last_error = Some(e.to_string());
                }
            }
            state.pending_delete = None;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            state.pending_delete = None;
        }
        _ => {}
    }
    true
}

pub fn draw_branch_list(frame: &mut Frame, area: Rect, state: &AppState) {
    let visible = visible_branches(state);
    let is_empty = visible.is_empty();
    let items: Vec<ListItem> = if is_empty {
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
    let list = List::new(items)
        .block(Block::default().title(title).borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    if !is_empty {
        list_state.select(Some(state.branch_cursor));
    }
    frame.render_stateful_widget(list, area, &mut list_state);
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
            names
                .iter()
                .map(|n| Branch { name: n.to_string(), last_commit_epoch: 0, is_local: true })
                .collect(),
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

    #[test]
    fn d_on_local_branch_sets_pending_delete() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Char('d'));
        assert_eq!(state.pending_delete, Some("a".to_string()));
    }

    #[test]
    fn d_on_remote_branch_does_nothing() {
        let mut state = AppState::new(
            "/tmp".into(),
            "main".into(),
            vec![Branch { name: "origin/feature".into(), last_commit_epoch: 0, is_local: false }],
        );
        handle_key_branch_list(&mut state, KeyCode::Char('d'));
        assert_eq!(state.pending_delete, None);
    }

    #[test]
    fn other_keys_are_swallowed_while_delete_is_pending() {
        let mut state = state_with_branches(&["a", "b"]);
        state.pending_delete = Some("a".to_string());
        let consumed = handle_key_delete_confirm(&mut state, KeyCode::Down);
        assert!(consumed);
        assert_eq!(state.branch_cursor, 0);
        assert_eq!(state.pending_delete, Some("a".to_string()));
    }

    #[test]
    fn no_pending_delete_is_not_consumed() {
        let mut state = state_with_branches(&["a"]);
        let consumed = handle_key_delete_confirm(&mut state, KeyCode::Char('y'));
        assert!(!consumed);
    }

    #[test]
    fn n_cancels_pending_delete_without_deleting() {
        let mut state = state_with_branches(&["a"]);
        state.pending_delete = Some("a".to_string());
        handle_key_delete_confirm(&mut state, KeyCode::Char('n'));
        assert_eq!(state.pending_delete, None);
        assert!(state.branches.iter().any(|b| b.name == "a"));
    }

    #[test]
    fn y_deletes_the_local_branch_and_clears_pending_delete() {
        use std::process::Command;
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.email", "t@example.com"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "a"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["branch", "throwaway"]).current_dir(dir.path()).status().unwrap();

        let mut state = AppState::new(
            dir.path().to_path_buf(),
            "main".into(),
            vec![Branch { name: "throwaway".into(), last_commit_epoch: 0, is_local: true }],
        );
        state.pending_delete = Some("throwaway".to_string());

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(!state.branches.iter().any(|b| b.name == "throwaway"));
        let remaining = git::run_git(dir.path(), &["branch", "--list", "throwaway"]).unwrap();
        assert!(remaining.is_empty());
    }
}
