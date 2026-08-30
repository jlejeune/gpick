use crate::app::{AppState, PendingDelete, Screen};
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
        KeyCode::Delete => {
            if let Some(b) = visible_branches(state).get(state.branch_cursor) {
                let pending = if b.is_local {
                    PendingDelete::Local(b.name.clone())
                } else {
                    PendingDelete::Remote(b.name.clone())
                };
                state.pending_delete = Some(pending);
                state.last_error = None;
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

/// Prompt text for the footer while a delete confirmation is pending.
pub fn confirm_prompt(pending: &PendingDelete) -> String {
    match pending {
        PendingDelete::Local(name) => format!("Delete local branch '{name}'? y/n"),
        PendingDelete::Remote(name) => {
            format!("Delete REMOTE branch '{name}' (affects origin)? y/n")
        }
        PendingDelete::RemoveWorktree { branch, path } => format!(
            "'{branch}' is checked out at '{path}'. Remove that worktree and retry delete? y/n"
        ),
    }
}

/// If a `git branch -D` failure was caused by the branch being checked out
/// in another worktree, extracts that worktree's path from git's error
/// text. Handles both known message shapes ("checked out at '<path>'" and
/// "used by worktree at '<path>'") by taking the last single-quoted
/// substring, since in both cases that's the path.
fn parse_worktree_path(err: &str) -> Option<String> {
    if !err.contains("worktree") && !err.contains("checked out") {
        return None;
    }
    let end = err.rfind('\'')?;
    let start = err[..end].rfind('\'')?;
    Some(err[start + 1..end].to_string())
}

fn remove_deleted_branch(state: &mut AppState, name: &str) {
    state.branches.retain(|b| b.name != name);
    let visible_len = visible_branches(state).len();
    if state.branch_cursor >= visible_len {
        state.branch_cursor = visible_len.saturating_sub(1);
    }
    state.last_error = None;
}

/// Handles a key press while a delete confirmation is pending. Returns
/// `true` if the key was consumed here (the caller must not dispatch it
/// to `handle_key_branch_list`). While a confirmation is pending, every
/// key is swallowed so the user can't navigate mid-prompt.
pub fn handle_key_delete_confirm(state: &mut AppState, key: KeyCode) -> bool {
    let Some(pending) = state.pending_delete.clone() else {
        return false;
    };

    match key {
        KeyCode::Char('y') => match pending {
            PendingDelete::Local(name) => match git::delete_branch(&state.cwd, &name) {
                Ok(()) => {
                    remove_deleted_branch(state, &name);
                    state.pending_delete = None;
                }
                Err(e) => {
                    let msg = e.to_string();
                    if let Some(path) = parse_worktree_path(&msg) {
                        state.pending_delete = Some(PendingDelete::RemoveWorktree { branch: name, path });
                    } else {
                        state.last_error = Some(msg);
                        state.pending_delete = None;
                    }
                }
            },
            PendingDelete::Remote(name) => {
                let (remote, branch) = name.split_once('/').unwrap_or(("origin", name.as_str()));
                match git::delete_remote_branch(&state.cwd, remote, branch) {
                    Ok(()) => {
                        remove_deleted_branch(state, &name);
                    }
                    Err(e) => {
                        state.last_error = Some(e.to_string());
                    }
                }
                state.pending_delete = None;
            }
            PendingDelete::RemoveWorktree { branch, path } => {
                match git::remove_worktree(&state.cwd, &path) {
                    Ok(()) => match git::delete_branch(&state.cwd, &branch) {
                        Ok(()) => {
                            remove_deleted_branch(state, &branch);
                        }
                        Err(e) => {
                            state.last_error = Some(e.to_string());
                        }
                    },
                    Err(e) => {
                        state.last_error = Some(e.to_string());
                    }
                }
                state.pending_delete = None;
            }
        },
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
        visible
            .iter()
            .map(|b| {
                let marker = if b.is_local { "[L]" } else { "[R]" };
                ListItem::new(format!("{marker} {}", b.name))
            })
            .collect()
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
    fn delete_key_on_local_branch_sets_pending_local_delete() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Delete);
        assert_eq!(state.pending_delete, Some(PendingDelete::Local("a".to_string())));
    }

    #[test]
    fn delete_key_on_remote_branch_sets_pending_remote_delete() {
        let mut state = AppState::new(
            "/tmp".into(),
            "main".into(),
            vec![Branch { name: "origin/feature".into(), last_commit_epoch: 0, is_local: false }],
        );
        handle_key_branch_list(&mut state, KeyCode::Delete);
        assert_eq!(state.pending_delete, Some(PendingDelete::Remote("origin/feature".to_string())));
    }

    #[test]
    fn char_d_filters_instead_of_deleting() {
        let mut state = state_with_branches(&["dev", "feature"]);
        handle_key_branch_list(&mut state, KeyCode::Char('d'));
        assert_eq!(state.branch_filter, "d");
        assert_eq!(state.pending_delete, None);
        let visible = visible_branches(&state);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "dev");
    }

    #[test]
    fn other_keys_are_swallowed_while_delete_is_pending() {
        let mut state = state_with_branches(&["a", "b"]);
        state.pending_delete = Some(PendingDelete::Local("a".to_string()));
        let consumed = handle_key_delete_confirm(&mut state, KeyCode::Down);
        assert!(consumed);
        assert_eq!(state.branch_cursor, 0);
        assert_eq!(state.pending_delete, Some(PendingDelete::Local("a".to_string())));
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
        state.pending_delete = Some(PendingDelete::Local("a".to_string()));
        handle_key_delete_confirm(&mut state, KeyCode::Char('n'));
        assert_eq!(state.pending_delete, None);
        assert!(state.branches.iter().any(|b| b.name == "a"));
    }

    fn init_repo() -> (tempfile::TempDir, std::path::PathBuf) {
        use std::process::Command;
        let dir = tempfile::TempDir::new().unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.email", "t@example.com"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "a"]).current_dir(dir.path()).status().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    #[test]
    fn y_deletes_the_local_branch_and_clears_pending_delete() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        Command::new("git").args(["branch", "throwaway"]).current_dir(&cwd).status().unwrap();

        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![Branch { name: "throwaway".into(), last_commit_epoch: 0, is_local: true }],
        );
        state.pending_delete = Some(PendingDelete::Local("throwaway".to_string()));

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(!state.branches.iter().any(|b| b.name == "throwaway"));
        let remaining = git::run_git(&cwd, &["branch", "--list", "throwaway"]).unwrap();
        assert!(remaining.is_empty());
    }

    #[test]
    fn y_deletes_the_remote_branch_via_push_delete() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        let remote_dir = tempfile::TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(&cwd)
            .status()
            .unwrap();
        Command::new("git").args(["branch", "throwaway"]).current_dir(&cwd).status().unwrap();
        Command::new("git").args(["push", "-q", "origin", "throwaway"]).current_dir(&cwd).status().unwrap();

        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![Branch { name: "origin/throwaway".into(), last_commit_epoch: 0, is_local: false }],
        );
        state.pending_delete = Some(PendingDelete::Remote("origin/throwaway".to_string()));

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(!state.branches.iter().any(|b| b.name == "origin/throwaway"));
        let remote_refs = git::run_git(remote_dir.path(), &["branch", "--list", "throwaway"]).unwrap();
        assert!(remote_refs.is_empty());
    }

    #[test]
    fn local_delete_checked_out_elsewhere_offers_worktree_removal() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        Command::new("git").args(["branch", "feature"]).current_dir(&cwd).status().unwrap();
        let worktree_dir = tempfile::TempDir::new().unwrap();
        std::fs::remove_dir(worktree_dir.path()).unwrap();
        Command::new("git")
            .args(["worktree", "add", "-q", worktree_dir.path().to_str().unwrap(), "feature"])
            .current_dir(&cwd)
            .status()
            .unwrap();

        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![Branch { name: "feature".into(), last_commit_epoch: 0, is_local: true }],
        );
        state.pending_delete = Some(PendingDelete::Local("feature".to_string()));

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        match state.pending_delete {
            Some(PendingDelete::RemoveWorktree { branch, path }) => {
                assert_eq!(branch, "feature");
                assert_eq!(path, worktree_dir.path().to_str().unwrap());
            }
            other => panic!("expected RemoveWorktree, got {other:?}"),
        }
        // branch still exists — nothing destructive happened yet
        let still_there = git::run_git(&cwd, &["branch", "--list", "feature"]).unwrap();
        assert!(!still_there.is_empty());
    }

    #[test]
    fn confirming_worktree_removal_removes_it_and_retries_delete() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        Command::new("git").args(["branch", "feature"]).current_dir(&cwd).status().unwrap();
        let worktree_dir = tempfile::TempDir::new().unwrap();
        let worktree_path = worktree_dir.path().to_path_buf();
        std::fs::remove_dir(&worktree_path).unwrap();
        Command::new("git")
            .args(["worktree", "add", "-q", worktree_path.to_str().unwrap(), "feature"])
            .current_dir(&cwd)
            .status()
            .unwrap();

        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![Branch { name: "feature".into(), last_commit_epoch: 0, is_local: true }],
        );
        state.pending_delete = Some(PendingDelete::RemoveWorktree {
            branch: "feature".to_string(),
            path: worktree_path.to_str().unwrap().to_string(),
        });

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(!worktree_path.exists());
        let branch_left = git::run_git(&cwd, &["branch", "--list", "feature"]).unwrap();
        assert!(branch_left.is_empty());
        assert!(!state.branches.iter().any(|b| b.name == "feature"));
    }

    #[test]
    fn parse_worktree_path_extracts_the_path_from_either_message_shape() {
        assert_eq!(
            parse_worktree_path("error: Cannot delete branch 'foo' checked out at '/tmp/wt'"),
            Some("/tmp/wt".to_string())
        );
        assert_eq!(
            parse_worktree_path("fatal: 'foo' is already used by worktree at '/tmp/wt2'"),
            Some("/tmp/wt2".to_string())
        );
        assert_eq!(parse_worktree_path("error: branch 'foo' not fully merged"), None);
    }
}
