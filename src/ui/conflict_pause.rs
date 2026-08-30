use crate::app::{AppState, ExecutionOutcome, PauseReason, Screen};
use crate::git::{self, CherryPickOutcome, GitError};
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, Paragraph};

/// Mark the currently-paused-on commit's result entry as Done and advance.
fn mark_current_done_and_advance(state: &mut AppState) {
    if let Some(r) = state.execution_results.get_mut(state.execution_index) {
        r.outcome = ExecutionOutcome::Done;
    }
    state.conflict_message = None;
    state.pause_reason = None;
    state.execution_index += 1;
    state.screen = Screen::Execution;
}

/// Undo already-applied commits' selection so a retry doesn't re-queue them,
/// then return to the commit list.
fn abort_and_return_to_commit_list(state: &mut AppState) {
    for idx in state.execution_queue[..state.execution_index].to_vec() {
        state.selected.remove(&idx);
    }
    state.conflict_message = None;
    state.pause_reason = None;
    state.execution_queue.truncate(state.execution_index);
    state.screen = Screen::CommitList;
}

pub fn handle_key_conflict_pause(state: &mut AppState, key: KeyCode) -> Result<(), GitError> {
    let reason = state.pause_reason.unwrap_or(PauseReason::CherryPickConflict);
    match key {
        KeyCode::Char('q') | KeyCode::Esc => {
            state.screen = Screen::Quit;
        }
        KeyCode::Char('a') => match reason {
            PauseReason::CherryPickConflict => {
                git::cherry_pick_abort(&state.cwd)?;
                abort_and_return_to_commit_list(state);
            }
            PauseReason::AmendFailure => {
                // The cherry-pick already succeeded and landed as a real commit;
                // there is no sequencer to abort — undo the commit instead.
                git::reset_hard_head_minus_one(&state.cwd)?;
                abort_and_return_to_commit_list(state);
            }
            PauseReason::StepError => {
                // Nothing was applied for this commit — just go back.
                abort_and_return_to_commit_list(state);
            }
        },
        KeyCode::Char('c') => match reason {
            PauseReason::CherryPickConflict => match git::cherry_pick_continue(&state.cwd)? {
                CherryPickOutcome::Success => {
                    let commit_idx = state.execution_queue[state.execution_index];
                    let commit = state.commits[commit_idx].clone();
                    match git::amend_reauthor(&state.cwd, &commit.date_rfc2822) {
                        Ok(()) => mark_current_done_and_advance(state),
                        Err(e) => {
                            state.conflict_message = Some(e.to_string());
                            state.pause_reason = Some(PauseReason::AmendFailure);
                        }
                    }
                }
                CherryPickOutcome::Conflict(msg) => {
                    state.conflict_message = Some(msg);
                }
            },
            PauseReason::AmendFailure => {
                // The pick already succeeded — retry the amend directly, no
                // cherry-pick --continue (there's nothing to continue).
                let commit_idx = state.execution_queue[state.execution_index];
                let commit = state.commits[commit_idx].clone();
                match git::amend_reauthor(&state.cwd, &commit.date_rfc2822) {
                    Ok(()) => mark_current_done_and_advance(state),
                    Err(e) => {
                        state.conflict_message = Some(e.to_string());
                    }
                }
            }
            PauseReason::StepError => {
                // Nothing to continue — retrying means re-running execution
                // from the same index on the next Execution tick.
                state.conflict_message = None;
                state.pause_reason = None;
                state.screen = Screen::Execution;
            }
        },
        _ => {}
    }
    Ok(())
}

pub fn draw_conflict_pause(frame: &mut Frame, area: Rect, state: &AppState, status: &str) {
    let text = format!(
        "{}\n\n{}\n\n[c] continue   [a] abort",
        state.conflict_message.clone().unwrap_or_default(),
        status
    );
    let widget = Paragraph::new(text).block(Block::default().title("Conflict").borders(Borders::ALL));
    frame.render_widget(widget, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, ExecutionOutcome, ExecutionResult, PauseReason, Screen};
    use crate::git;
    use crossterm::event::KeyCode;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.email", "t@example.com"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir.path()).status().unwrap();
        dir
    }

    fn conflicted_state() -> (TempDir, AppState) {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "line1").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "base"]).current_dir(dir.path()).status().unwrap();
        let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "line1-feature").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "feature change"]).current_dir(dir.path()).status().unwrap();
        let feature_commits = git::list_commits(dir.path(), &base_sha, "feature").unwrap();

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "line1-base").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "base change"]).current_dir(dir.path()).status().unwrap();

        let mut state = AppState::new(dir.path().to_path_buf(), base_sha, vec![]);
        state.load_commits(feature_commits);
        state.selected.insert(0);
        state.execution_queue = vec![0];
        state.execution_index = 0;
        state.execution_results = vec![ExecutionResult {
            commit: state.commits[0].clone(),
            outcome: ExecutionOutcome::Pending,
        }];

        // simulate step_execution() having already hit the conflict
        git::cherry_pick(dir.path(), &state.commits[0].sha).ok();
        state.conflict_message = Some("conflict in a.txt".to_string());
        state.pause_reason = Some(PauseReason::CherryPickConflict);
        state.screen = Screen::ConflictPause;

        (dir, state)
    }

    #[test]
    fn abort_clears_conflict_and_returns_to_commit_list() {
        let (_dir, mut state) = conflicted_state();
        handle_key_conflict_pause(&mut state, KeyCode::Char('a')).unwrap();
        assert!(state.conflict_message.is_none());
        assert_eq!(state.screen, Screen::CommitList);
        assert!(state.execution_queue.is_empty());
    }

    #[test]
    fn abort_removes_already_applied_commits_from_selected() {
        let (_dir, mut state) = conflicted_state();
        // pretend index 0 already succeeded and we're now paused on index 1
        state.selected.insert(0);
        state.selected.insert(1);
        state.execution_queue = vec![0, 1];
        state.execution_index = 1;

        handle_key_conflict_pause(&mut state, KeyCode::Char('a')).unwrap();

        // commit 0 already applied before the abort -> must not be re-queued on retry
        assert!(!state.selected.contains(&0));
        // commit 1 is the one we aborted on and was never applied -> stays selected
        assert!(state.selected.contains(&1));
    }

    #[test]
    fn continue_after_resolving_advances_and_resumes_execution() {
        let (dir, mut state) = conflicted_state();
        // user resolves the conflict and stages it, as they would outside the TUI
        std::fs::write(dir.path().join("a.txt"), "line1-resolved").unwrap();
        Command::new("git").args(["add", "a.txt"]).current_dir(dir.path()).status().unwrap();

        handle_key_conflict_pause(&mut state, KeyCode::Char('c')).unwrap();

        assert!(state.conflict_message.is_none());
        assert_eq!(state.execution_index, 1);
        assert_eq!(state.screen, Screen::Execution);
    }

    #[test]
    fn continue_after_resolving_marks_result_done_not_failed() {
        let (dir, mut state) = conflicted_state();
        std::fs::write(dir.path().join("a.txt"), "line1-resolved").unwrap();
        Command::new("git").args(["add", "a.txt"]).current_dir(dir.path()).status().unwrap();

        handle_key_conflict_pause(&mut state, KeyCode::Char('c')).unwrap();

        assert!(matches!(state.execution_results[0].outcome, ExecutionOutcome::Done));
    }

    #[test]
    fn q_quits_from_conflict_pause() {
        let (_dir, mut state) = conflicted_state();
        handle_key_conflict_pause(&mut state, KeyCode::Char('q')).unwrap();
        assert_eq!(state.screen, Screen::Quit);
    }

    #[test]
    fn esc_quits_from_conflict_pause() {
        let (_dir, mut state) = conflicted_state();
        handle_key_conflict_pause(&mut state, KeyCode::Esc).unwrap();
        assert_eq!(state.screen, Screen::Quit);
    }

    #[test]
    fn amend_failure_continue_retries_amend_directly_without_cherry_pick_continue() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "line1").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "base"]).current_dir(dir.path()).status().unwrap();
        let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "feature commit"]).current_dir(dir.path()).status().unwrap();
        let feature_commits = git::list_commits(dir.path(), &base_sha, "feature").unwrap();

        // Cherry-pick already succeeded (as if the amend afterward had failed);
        // now retry the amend from the ConflictPause screen.
        let mut state = AppState::new(dir.path().to_path_buf(), base_sha, vec![]);
        state.load_commits(feature_commits);
        state.selected.insert(0);
        state.execution_queue = vec![0];
        state.execution_index = 0;
        state.execution_results = vec![ExecutionResult {
            commit: state.commits[0].clone(),
            outcome: ExecutionOutcome::Pending,
        }];
        state.conflict_message = Some("amend failed".to_string());
        state.pause_reason = Some(PauseReason::AmendFailure);
        state.screen = Screen::ConflictPause;

        handle_key_conflict_pause(&mut state, KeyCode::Char('c')).unwrap();

        assert!(state.conflict_message.is_none());
        assert_eq!(state.execution_index, 1);
        assert_eq!(state.screen, Screen::Execution);
        assert!(matches!(state.execution_results[0].outcome, ExecutionOutcome::Done));
        let log = git::run_git(dir.path(), &["log", "-1", "--format=%B"]).unwrap();
        assert!(log.contains("Signed-off-by"));
    }

    #[test]
    fn amend_failure_abort_resets_hard_to_undo_the_landed_commit() {
        let dir = init_repo();
        std::fs::write(dir.path().join("a.txt"), "line1").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "base"]).current_dir(dir.path()).status().unwrap();
        let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("b.txt"), "b").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "feature commit"]).current_dir(dir.path()).status().unwrap();
        let feature_commits = git::list_commits(dir.path(), &base_sha, "feature").unwrap();

        let mut state = AppState::new(dir.path().to_path_buf(), base_sha.clone(), vec![]);
        state.load_commits(feature_commits);
        state.selected.insert(0);
        state.execution_queue = vec![0];
        state.execution_index = 0;
        state.conflict_message = Some("amend failed".to_string());
        state.pause_reason = Some(PauseReason::AmendFailure);
        state.screen = Screen::ConflictPause;

        handle_key_conflict_pause(&mut state, KeyCode::Char('a')).unwrap();

        assert_eq!(state.screen, Screen::CommitList);
        let head = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_eq!(head, base_sha);
        // the reset --hard undid the landed commit entirely, so it is not
        // "already applied" and should remain selected for a future retry
        assert!(state.selected.contains(&0));
    }
}
