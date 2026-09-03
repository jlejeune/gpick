use crate::app::{AppState, PendingDelete, Screen};
use crate::git::{self, Branch};
use crate::ui::theme;
use crossterm::event::{KeyCode, KeyModifiers};
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

pub fn handle_key_branch_list(state: &mut AppState, key: KeyCode, modifiers: KeyModifiers) {
    if state.search_active {
        handle_key_search(state, key);
        return;
    }

    let shift_arrow = modifiers.contains(KeyModifiers::SHIFT) && matches!(key, KeyCode::Down | KeyCode::Up);
    if !shift_arrow {
        state.range_anchor = None;
    }

    let visible_len = visible_branches(state).len();
    match key {
        KeyCode::Down if shift_arrow => {
            let anchor = state.range_anchor.unwrap_or(state.branch_cursor);
            state.range_anchor = Some(anchor);
            if visible_len > 0 && state.branch_cursor + 1 < visible_len {
                state.branch_cursor += 1;
            }
            select_range(state, anchor, state.branch_cursor);
        }
        KeyCode::Up if shift_arrow => {
            let anchor = state.range_anchor.unwrap_or(state.branch_cursor);
            state.range_anchor = Some(anchor);
            state.branch_cursor = state.branch_cursor.saturating_sub(1);
            select_range(state, anchor, state.branch_cursor);
        }
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
            if !state.selected_branches.is_empty() {
                let mut names: Vec<String> = state.selected_branches.iter().cloned().collect();
                names.sort();
                state.pending_delete = Some(PendingDelete::Bulk(names));
                state.last_error = None;
            } else if let Some(b) = visible_branches(state).get(state.branch_cursor) {
                let pending = if b.is_local {
                    PendingDelete::Local(b.name.clone())
                } else {
                    PendingDelete::Remote(b.name.clone())
                };
                state.pending_delete = Some(pending);
                state.last_error = None;
            }
        }
        KeyCode::Char(' ') => {
            if let Some(b) = visible_branches(state).get(state.branch_cursor) {
                let name = b.name.clone();
                if !state.selected_branches.remove(&name) {
                    state.selected_branches.insert(name);
                }
            }
            if visible_len > 0 && state.branch_cursor + 1 < visible_len {
                state.branch_cursor += 1;
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

/// Adds every visible branch between indices `from` and `to` (inclusive,
/// either order) to the multi-selection — used by Shift+Up/Down to extend
/// a contiguous range from the anchor to the current cursor.
fn select_range(state: &mut AppState, from: usize, to: usize) {
    let (lo, hi) = if from <= to { (from, to) } else { (to, from) };
    let names: Vec<String> = visible_branches(state)
        .into_iter()
        .enumerate()
        .filter(|(i, _)| *i >= lo && *i <= hi)
        .map(|(_, b)| b.name.clone())
        .collect();
    state.selected_branches.extend(names);
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
        PendingDelete::Bulk(names) => {
            format!("Delete {} branches ({})? y/n", names.len(), names.join(", "))
        }
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

/// Deletes remote-tracking branch `name` (e.g. `origin/foo`) on its actual
/// remote. If the remote already lost that branch — the local cache just
/// hasn't caught up, surfaced by git as "remote ref does not exist" — this
/// prunes the stale local ref and reports success instead of an error, so
/// the user isn't stuck on a branch gpick can never delete "successfully".
fn delete_remote_with_stale_fallback(state: &AppState, name: &str) -> Result<(), String> {
    let (remote, branch) = name.split_once('/').unwrap_or(("origin", name));
    match git::delete_remote_branch(&state.cwd, remote, branch) {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("remote ref does not exist") {
                let _ = git::prune_remote_tracking_ref(&state.cwd, name);
                Ok(())
            } else {
                Err(msg)
            }
        }
    }
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
                match delete_remote_with_stale_fallback(state, &name) {
                    Ok(()) => remove_deleted_branch(state, &name),
                    Err(msg) => state.last_error = Some(msg),
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
            PendingDelete::Bulk(names) => {
                let is_local: std::collections::HashMap<String, bool> =
                    state.branches.iter().map(|b| (b.name.clone(), b.is_local)).collect();
                let mut errors = Vec::new();
                for name in &names {
                    let result = if is_local.get(name).copied().unwrap_or(true) {
                        git::delete_branch(&state.cwd, name).map_err(|e| e.to_string())
                    } else {
                        delete_remote_with_stale_fallback(state, name)
                    };
                    match result {
                        Ok(()) => {
                            remove_deleted_branch(state, name);
                            state.selected_branches.remove(name);
                        }
                        Err(e) => errors.push(format!("{name}: {e}")),
                    }
                }
                if !errors.is_empty() {
                    state.last_error = Some(errors.join("; "));
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
                let (check, check_color) =
                    if state.selected_branches.contains(&b.name) { ("[x]", theme::SUCCESS) } else { ("[ ]", theme::MUTED) };
                let line = Line::from(vec![
                    Span::styled(check, Style::default().fg(check_color).add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
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
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(state.branch_cursor, 1);
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(state.branch_cursor, 1); // clamped
    }

    #[test]
    fn up_clamps_at_zero() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(state.branch_cursor, 0);
    }

    #[test]
    fn shift_down_extends_a_contiguous_range_selection() {
        let mut state = state_with_branches(&["a", "b", "c", "d"]);
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::SHIFT);
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::SHIFT);
        assert_eq!(state.branch_cursor, 2);
        assert_eq!(
            state.selected_branches,
            ["a", "b", "c"].iter().map(|s| s.to_string()).collect()
        );
    }

    #[test]
    fn shift_up_extends_the_range_upward_from_the_anchor() {
        let mut state = state_with_branches(&["a", "b", "c", "d"]);
        state.branch_cursor = 3;
        handle_key_branch_list(&mut state, KeyCode::Up, KeyModifiers::SHIFT);
        handle_key_branch_list(&mut state, KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(state.branch_cursor, 1);
        assert_eq!(
            state.selected_branches,
            ["b", "c", "d"].iter().map(|s| s.to_string()).collect()
        );
    }

    #[test]
    fn plain_arrow_after_shift_range_resets_the_anchor() {
        let mut state = state_with_branches(&["a", "b", "c"]);
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::SHIFT);
        assert!(state.range_anchor.is_some());
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::NONE);
        assert!(state.range_anchor.is_none());
    }

    #[test]
    fn a_new_shift_range_starts_fresh_from_the_current_cursor() {
        let mut state = state_with_branches(&["a", "b", "c", "d"]);
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::SHIFT); // selects a,b
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::NONE); // plain move, resets anchor, cursor -> 2 (c)
        state.selected_branches.clear();
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::SHIFT); // new range c,d

        assert_eq!(state.selected_branches, ["c", "d"].iter().map(|s| s.to_string()).collect());
    }

    #[test]
    fn slash_enters_search_mode_and_typing_filters_visible_branches() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(state.search_active);
        handle_key_branch_list(&mut state, KeyCode::Char('f'), KeyModifiers::NONE);
        handle_key_branch_list(&mut state, KeyCode::Char('e'), KeyModifiers::NONE);
        let visible = visible_branches(&state);
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "feature-x");
    }

    #[test]
    fn typing_without_search_mode_does_not_filter() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('f'), KeyModifiers::NONE);
        assert_eq!(state.branch_filter, "");
        assert_eq!(visible_branches(&state).len(), 2);
    }

    #[test]
    fn enter_exits_search_mode_and_keeps_the_filter() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('/'), KeyModifiers::NONE);
        handle_key_branch_list(&mut state, KeyCode::Char('f'), KeyModifiers::NONE);
        handle_key_branch_list(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert!(!state.search_active);
        assert_eq!(state.branch_filter, "f");
    }

    #[test]
    fn esc_exits_search_mode_and_clears_the_filter() {
        let mut state = state_with_branches(&["feature-x", "bugfix-y"]);
        handle_key_branch_list(&mut state, KeyCode::Char('/'), KeyModifiers::NONE);
        handle_key_branch_list(&mut state, KeyCode::Char('f'), KeyModifiers::NONE);
        handle_key_branch_list(&mut state, KeyCode::Esc, KeyModifiers::NONE);
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
        handle_key_branch_list(&mut state, KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(state.show_all_branches);
        handle_key_branch_list(&mut state, KeyCode::Char('a'), KeyModifiers::NONE);
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
        handle_key_branch_list(&mut state, KeyCode::Char('p'), KeyModifiers::NONE);
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
    fn space_selects_the_hovered_branch_and_advances_to_the_next_one() {
        let mut state = state_with_branches(&["a", "b", "c"]);
        handle_key_branch_list(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(state.selected_branches.contains("a"));
        assert_eq!(state.branch_cursor, 1);
    }

    #[test]
    fn space_does_not_advance_past_the_last_branch() {
        let mut state = state_with_branches(&["a", "b"]);
        state.branch_cursor = 1;
        handle_key_branch_list(&mut state, KeyCode::Char(' '), KeyModifiers::NONE);
        assert!(state.selected_branches.contains("b"));
        assert_eq!(state.branch_cursor, 1);
    }

    #[test]
    fn space_toggles_off_when_pressed_again_on_the_same_branch() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Char(' '), KeyModifiers::NONE); // select a, cursor -> b
        handle_key_branch_list(&mut state, KeyCode::Up, KeyModifiers::NONE); // back to a
        handle_key_branch_list(&mut state, KeyCode::Char(' '), KeyModifiers::NONE); // toggle a off
        assert!(!state.selected_branches.contains("a"));
    }

    #[test]
    fn delete_with_multi_selection_sets_pending_bulk_delete() {
        let mut state = state_with_branches(&["a", "b", "c"]);
        handle_key_branch_list(&mut state, KeyCode::Char(' '), KeyModifiers::NONE); // select a, cursor -> b
        handle_key_branch_list(&mut state, KeyCode::Char(' '), KeyModifiers::NONE); // select b, cursor -> c
        handle_key_branch_list(&mut state, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(state.pending_delete, Some(PendingDelete::Bulk(vec!["a".to_string(), "b".to_string()])));
    }

    #[test]
    fn delete_without_multi_selection_still_targets_the_hovered_branch() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(state.pending_delete, Some(PendingDelete::Local("a".to_string())));
    }

    #[test]
    fn y_bulk_deletes_all_selected_local_branches() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        Command::new("git").args(["branch", "one"]).current_dir(&cwd).status().unwrap();
        Command::new("git").args(["branch", "two"]).current_dir(&cwd).status().unwrap();

        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![
                Branch { name: "one".into(), last_commit_epoch: 0, is_local: true },
                Branch { name: "two".into(), last_commit_epoch: 0, is_local: true },
            ],
        );
        state.selected_branches.insert("one".to_string());
        state.selected_branches.insert("two".to_string());
        state.pending_delete = Some(PendingDelete::Bulk(vec!["one".to_string(), "two".to_string()]));

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(state.branches.is_empty());
        assert!(state.selected_branches.is_empty());
        assert!(git::run_git(&cwd, &["branch", "--list", "one"]).unwrap().is_empty());
        assert!(git::run_git(&cwd, &["branch", "--list", "two"]).unwrap().is_empty());
    }

    #[test]
    fn y_bulk_delete_reports_errors_for_branches_that_fail() {
        let (_dir, cwd) = init_repo();
        // "missing" doesn't actually exist locally, so its delete will fail
        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![Branch { name: "missing".into(), last_commit_epoch: 0, is_local: true }],
        );
        state.pending_delete = Some(PendingDelete::Bulk(vec!["missing".to_string()]));

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(state.last_error.is_some());
    }

    #[test]
    fn enter_selects_branch_and_moves_to_commit_list() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Down, KeyModifiers::NONE);
        handle_key_branch_list(&mut state, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(state.selected_branch, Some("b".to_string()));
        assert_eq!(state.screen, Screen::CommitList);
    }

    #[test]
    fn q_quits() {
        let mut state = state_with_branches(&["a"]);
        handle_key_branch_list(&mut state, KeyCode::Char('q'), KeyModifiers::NONE);
        assert_eq!(state.screen, Screen::Quit);
    }

    #[test]
    fn delete_key_on_local_branch_sets_pending_local_delete() {
        let mut state = state_with_branches(&["a", "b"]);
        handle_key_branch_list(&mut state, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(state.pending_delete, Some(PendingDelete::Local("a".to_string())));
    }

    #[test]
    fn delete_key_on_remote_branch_sets_pending_remote_delete() {
        let mut state = AppState::new(
            "/tmp".into(),
            "main".into(),
            vec![Branch { name: "origin/feature".into(), last_commit_epoch: 0, is_local: false }],
        );
        handle_key_branch_list(&mut state, KeyCode::Delete, KeyModifiers::NONE);
        assert_eq!(state.pending_delete, Some(PendingDelete::Remote("origin/feature".to_string())));
    }

    #[test]
    fn char_d_outside_search_mode_does_nothing() {
        let mut state = state_with_branches(&["dev", "feature"]);
        handle_key_branch_list(&mut state, KeyCode::Char('d'), KeyModifiers::NONE);
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
    fn y_deleting_an_already_gone_remote_branch_prunes_the_stale_ref_without_erroring() {
        use std::process::Command;
        let (_dir, cwd) = init_repo();
        let remote_dir = tempfile::TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(&cwd)
            .status()
            .unwrap();
        // simulate a remote-tracking ref whose branch was already deleted on
        // the actual remote (e.g. by someone else, or a prior run) — the
        // local cache just hasn't been pruned, matching a real report where
        // "git push --delete" failed with "remote ref does not exist".
        let head_sha = git::run_git(&cwd, &["rev-parse", "HEAD"]).unwrap();
        git::run_git(&cwd, &["update-ref", "refs/remotes/origin/ghost", &head_sha]).unwrap();

        let mut state = AppState::new(
            cwd.clone(),
            "main".into(),
            vec![Branch { name: "origin/ghost".into(), last_commit_epoch: 0, is_local: false }],
        );
        state.pending_delete = Some(PendingDelete::Remote("origin/ghost".to_string()));

        handle_key_delete_confirm(&mut state, KeyCode::Char('y'));

        assert_eq!(state.pending_delete, None);
        assert!(state.last_error.is_none(), "expected no error, got {:?}", state.last_error);
        assert!(!state.branches.iter().any(|b| b.name == "origin/ghost"));
        let local_ref = git::run_git(&cwd, &["for-each-ref", "refs/remotes/origin/ghost"]).unwrap();
        assert!(local_ref.is_empty());
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
