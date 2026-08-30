use crate::app::{AppState, ExecutionOutcome, PauseReason, Screen};
use crate::git::{self, CherryPickOutcome};
use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

pub fn step_execution(state: &mut AppState) {
    if state.execution_index >= state.execution_queue.len() {
        return;
    }

    let commit_idx = state.execution_queue[state.execution_index];
    let commit = state.commits[commit_idx].clone();
    let index = state.execution_index;

    let pick = match git::cherry_pick(&state.cwd, &commit.sha) {
        Ok(outcome) => outcome,
        Err(e) => {
            state.conflict_message = Some(e.to_string());
            state.pause_reason = Some(PauseReason::StepError);
            state.screen = Screen::ConflictPause;
            return;
        }
    };

    match pick {
        CherryPickOutcome::Success => match git::amend_reauthor(&state.cwd, &commit.date_rfc2822) {
            Ok(()) => {
                if let Some(r) = state.execution_results.get_mut(index) {
                    r.outcome = ExecutionOutcome::Done;
                }
                state.execution_index += 1;
            }
            Err(e) => {
                let msg = e.to_string();
                if let Some(r) = state.execution_results.get_mut(index) {
                    r.outcome = ExecutionOutcome::Failed(msg.clone());
                }
                state.conflict_message = Some(msg);
                state.pause_reason = Some(PauseReason::AmendFailure);
                state.screen = Screen::ConflictPause;
            }
        },
        CherryPickOutcome::Conflict(msg) => {
            if let Some(r) = state.execution_results.get_mut(index) {
                r.outcome = ExecutionOutcome::Failed(msg.clone());
            }
            state.conflict_message = Some(msg);
            state.pause_reason = Some(PauseReason::CherryPickConflict);
            state.screen = Screen::ConflictPause;
        }
    }
}

pub fn draw_execution(frame: &mut Frame, area: Rect, state: &AppState) {
    if state.execution_index >= state.execution_queue.len() && !state.execution_queue.is_empty() {
        let widget = Paragraph::new("All done — press q to quit")
            .block(Block::default().title("Execution").borders(Borders::ALL));
        frame.render_widget(widget, area);
        return;
    }

    let items: Vec<ListItem> = state
        .execution_results
        .iter()
        .map(|r| {
            let status = match &r.outcome {
                ExecutionOutcome::Pending => "…",
                ExecutionOutcome::Done => "done",
                ExecutionOutcome::Failed(_) => "failed",
            };
            ListItem::new(format!("{} {} — {}", status, r.commit.short_sha, r.commit.message))
        })
        .collect();
    let list = List::new(items).block(Block::default().title("Execution").borders(Borders::ALL));
    frame.render_widget(list, area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, ExecutionOutcome, ExecutionResult, PauseReason, Screen};
    use crate::git;
    use std::path::PathBuf;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.email", "t@example.com"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir.path()).status().unwrap();
        dir
    }

    fn commit_file(dir: &TempDir, name: &str, content: &str) {
        std::fs::write(dir.path().join(name), content).unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", name]).current_dir(dir.path()).status().unwrap();
    }

    #[test]
    fn step_execution_applies_commit_and_advances_on_success() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "b.txt", "b");
        let feature_commits = git::list_commits(dir.path(), &base_sha, "feature").unwrap();
        Command::new("git").args(["checkout", "-q", "-"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();

        let mut state = AppState::new(dir.path().to_path_buf(), base_sha, vec![]);
        state.load_commits(feature_commits);
        state.execution_queue = vec![0];
        state.execution_index = 0;
        state.execution_results = vec![ExecutionResult {
            commit: state.commits[0].clone(),
            outcome: ExecutionOutcome::Pending,
        }];
        state.screen = Screen::Execution;

        step_execution(&mut state);

        assert_eq!(state.execution_index, 1);
        assert!(matches!(state.execution_results[0].outcome, ExecutionOutcome::Done));
        assert_eq!(state.screen, Screen::Execution);
    }

    #[test]
    fn step_execution_pauses_on_conflict() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "line1");
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
        state.execution_queue = vec![0];
        state.execution_index = 0;
        state.execution_results = vec![ExecutionResult {
            commit: state.commits[0].clone(),
            outcome: ExecutionOutcome::Pending,
        }];
        state.screen = Screen::Execution;

        step_execution(&mut state);

        assert_eq!(state.execution_index, 0); // did not advance
        assert_eq!(state.screen, Screen::ConflictPause);
        assert!(state.conflict_message.is_some());
        assert_eq!(state.pause_reason, Some(PauseReason::CherryPickConflict));
        assert!(matches!(state.execution_results[0].outcome, ExecutionOutcome::Failed(_)));
        git::cherry_pick_abort(dir.path()).unwrap();
    }

    #[test]
    fn step_execution_routes_spawn_failure_to_conflict_pause_with_step_error() {
        // A cwd that doesn't exist makes the git subprocess unspawnable in that dir,
        // which git::cherry_pick surfaces as Err — step_execution must not discard it.
        let mut state = AppState::new(
            PathBuf::from("/nonexistent/gpick-test-path-xyz"),
            "main".to_string(),
            vec![],
        );
        state.commits = vec![crate::git::Commit {
            sha: "deadbeef".into(),
            short_sha: "deadbee".into(),
            message: "msg".into(),
            author: "Test".into(),
            date_rfc2822: "Mon, 1 Jan 2024 00:00:00 +0000".into(),
        }];
        state.execution_queue = vec![0];
        state.execution_index = 0;
        state.execution_results = vec![ExecutionResult {
            commit: state.commits[0].clone(),
            outcome: ExecutionOutcome::Pending,
        }];
        state.screen = Screen::Execution;

        step_execution(&mut state);

        assert_eq!(state.execution_index, 0);
        assert_eq!(state.screen, Screen::ConflictPause);
        assert_eq!(state.pause_reason, Some(PauseReason::StepError));
        assert!(state.conflict_message.is_some());
    }
}
