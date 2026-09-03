use crate::app::{AppState, PendingDelete, Screen};
use crate::git::{self, Branch};
use crate::ui::theme;
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, ListState};

pub fn visible_branches(state: &AppState) -> Vec<&Branch> {
    let filter = state.branch_filter.to_lowercase();
    state
        .branches
        .iter()
        .filter(|b| b.name.to_lowercase().contains(&filter))
        .filter(|b| state.show_all_branches || !state.fully_picked.contains(&b.name))
        .collect()
}

pub fn handle_key_branch_list(state: &mut AppState, key: KeyCode) {
    if state.search_active {
        handle_key_search(state, key);
        return;
    }

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
        KeyCode::Char('/') => {
            state.search_active = true;
        }
        KeyCode::Char('a') => {
            state.show_all_branches = !state.show_all_branches;
            state.branch_cursor = 0;
        }
        KeyCode::Char('p') => {
            state.pending_push = true;
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

/// Handles input while the search field (entered with `/`) is active.
/// Character keys type into the filter instead of triggering shortcuts;
/// Enter keeps the filter and exits search mode, Esc clears it and exits.
fn handle_key_search(state: &mut AppState, key: KeyCode) {
    match key {
        KeyCode::Char(c) => {
            state.branch_filter.push(c);
            state.branch_cursor = 0;
        }
        KeyCode::Backspace => {
            state.branch_filter.pop();
            state.branch_cursor = 0;
        }
        KeyCode::Enter => {
            state.search_active = false;
        }
        KeyCode::Esc => {
            state.branch_filter.clear();
            state.search_active = false;
            state.branch_cursor = 0;
        }
        _ => {}
    }
}

/// Prompt text for the footer while a push confirmation is pending.
pub fn push_confirm_prompt(state: &AppState) -> String {
    format!("Push '{}' to origin/master? y/n", state.base)
}

/// Handles a key press while a push confirmation is pending. Mirrors
/// `handle_key_delete_confirm`: returns `true` if the key was consumed here.
pub fn handle_key_push_confirm(state: &mut AppState, key: KeyCode) -> bool {
    if !state.pending_push {
        return false;
    }

    match key {
        KeyCode::Char('y') => {
            match git::push_to_master(&state.cwd, &state.base) {
                Ok(()) => state.last_error = None,
                Err(e) => state.last_error = Some(e.to_string()),
            }
            state.pending_push = false;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            state.pending_push = false;
        }
        _ => {}
    }
    true
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

/// Title for the branch list panel, reflecting active search text and
/// whether fully-picked branches are currently shown.
fn branch_list_title(state: &AppState) -> String {
    let mut title = format!("Branches (base: {})", state.base);
    if state.search_active {
        title.push_str(&format!(", search: {}█", state.branch_filter));
    } else if !state.branch_filter.is_empty() {
        title.push_str(&format!(", search: {}", state.branch_filter));
    }
    if state.show_all_branches {
        title.push_str(", showing all");
    }
    title
}

pub fn draw_branch_list(frame: &mut Frame, area: Rect, state: &AppState) {
    let visible = visible_branches(state);
    let is_empty = visible.is_empty();
    let items: Vec<ListItem> = if is_empty {
        let msg = state
            .last_error
            .clone()
            .unwrap_or_else(|| "No branches found".to_string());
        vec![ListItem::new(Span::styled(msg, Style::default().fg(theme::ERROR)))]
    } else {
        visible
            .iter()
            .map(|b| {
                let (marker, color) = if b.is_local { ("[L]", theme::LOCAL) } else { ("[R]", theme::REMOTE) };
                let line = Line::from(vec![
                    Span::styled(marker, Style::default().fg(color).add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::raw(b.name.clone()),
                ]);
                ListItem::new(line)
            })
            .collect()
    };
    let title = branch_list_title(state);
    let list = List::new(items)
        .block(theme::titled_block(&title))
        .highlight_style(theme::highlight_style())
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    if !is_empty {
        list_state.select(Some(state.branch_cursor));
    }
    frame.render_stateful_widget(list, area, &mut list_state);
    if !is_empty {
        theme::draw_scrollbar(frame, area, visible.len(), state.branch_cursor);
    }
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
    fn slash_enters_search_mode_and_typing_filters_visible_branches() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('/'));
        assert!(state.search_active);
        handle_key_branch_list(&mut state, KeyCode::Char('f'));
        handle_key_branch_list(&mut state, KeyCode::Char('e'));
        let visible = visible_branches(&state);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "feature-x");
    }

    #[test]
    fn typing_without_search_mode_does_not_filter() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('f'));
        assert_eq!(state.branch_filter, "");
        assert_eq!(visible_branches(&state).len(), 2);
    }

    #[test]
    fn enter_exits_search_mode_and_keeps_the_filter() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('/'));
        handle_key_branch_list(&mut state, KeyCode::Char('f'));
        handle_key_branch_list(&mut state, KeyCode::Enter);
        assert!(!state.search_active);
        assert_eq!(state.branch_filter, "f");
    }

    #[test]
    fn esc_exits_search_mode_and_clears_the_filter() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('/'));
        handle_key_branch_list(&mut state, KeyCode::Char('f'));
        handle_key_branch_list(&mut state, KeyCode::Esc);
        assert!(!state.search_active);
        assert_eq!(state.branch_filter, "");
        // Esc while NOT in search mode still quits, but while in search mode
        // it must not have quit the app instead of cancelling the search.
        assert_eq!(state.screen, Screen::BranchList);
    }

    #[test]
    fn a_toggles_show_all_branches() {
        let mut state = state_with_branches(&["a"]);
        assert!(!state.show_all_branches);
        handle_key_branch_list(&mut state, KeyCode::Char('a'));
        assert!(state.show_all_branches);
        handle_key_branch_list(&mut state, KeyCode::Char('a'));
        assert!(!state.show_all_branches);
    }

    #[test]
    fn fully_picked_branches_are_hidden_unless_show_all_is_on() {
        let mut state = state_with_branches(&["picked", "unpicked"]);
        state.fully_picked.insert("picked".to_string());

        let visible = visible_branches(&state);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "unpicked");

        state.show_all_branches = true;
        assert_eq!(visible_branches(&state).len(), 2);
    }

    #[test]
    fn p_sets_pending_push() {
        let mut state = state_with_branches(&["a"]);
        handle_key_branch_list(&mut state, KeyCode::Char('p'));
        assert!(state.pending_push);
    }

    #[test]
    fn n_cancels_pending_push_without_pushing() {
        let mut state = state_with_branches(&["a"]);
        state.pending_push = true;
        let consumed = handle_key_push_confirm(&mut state, KeyCode::Char('n'));
        assert!(consumed);
        assert!(!state.pending_push);
    }

    #[test]
    fn no_pending_push_is_not_consumed() {
        let mut state = state_with_branches(&["a"]);
        assert!(!handle_key_push_confirm(&mut state, KeyCode::Char('y')));
    }

    #[test]
    fn y_pushes_base_to_origin_master() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        let remote_dir = tempfile::TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(&cwd)
            .status()
            .unwrap();
        let head_sha = git::run_git(&cwd, &["rev-parse", "HEAD"]).unwrap();

        let mut state = AppState::new(cwd.clone(), "HEAD".into(), vec![]);
        state.pending_push = true;

        handle_key_push_confirm(&mut state, KeyCode::Char('y'));

        assert!(!state.pending_push);
        assert!(state.last_error.is_none());
        let remote_master = git::run_git(remote_dir.path(), &["rev-parse", "master"]).unwrap();
        assert_eq!(remote_master, head_sha);
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
    fn char_d_outside_search_mode_does_nothing() {
        let mut state = state_with_branches(&["dev", "feature"]);
        handle_key_branch_list(&mut state, KeyCode::Char('d'));
        assert_eq!(state.branch_filter, "");
        assert_eq!(state.pending_delete, None);
        assert_eq!(visible_branches(&state).len(), 2);
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
