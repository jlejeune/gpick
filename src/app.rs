use crate::git;
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum Screen {
    BranchList,
    CommitList,
    Execution,
    ConflictPause,
    Quit,
}

#[derive(Debug)]
pub enum ExecutionOutcome {
    Pending,
    Done,
    Failed(String),
}

#[derive(Debug)]
pub struct ExecutionResult {
    pub commit: git::Commit,
    pub outcome: ExecutionOutcome,
}

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum PauseReason {
    /// A cherry-pick sequencer is actively in progress (conflict mid-pick).
    CherryPickConflict,
    /// The cherry-pick itself succeeded, but the follow-up amend failed.
    /// There is no sequencer to continue/abort here.
    AmendFailure,
    /// A hard error occurred while stepping execution (e.g. git unspawnable).
    StepError,
}

pub struct AppState {
    pub cwd: PathBuf,
    pub base: String,
    pub screen: Screen,
    pub branches: Vec<git::Branch>,
    pub branch_cursor: usize,
    pub branch_filter: String,
    pub selected_branch: Option<String>,
    pub commits: Vec<git::Commit>,
    pub commit_cursor: usize,
    pub selected: HashSet<usize>,
    pub execution_queue: Vec<usize>,
    pub execution_index: usize,
    pub execution_results: Vec<ExecutionResult>,
    pub conflict_message: Option<String>,
    pub pause_reason: Option<PauseReason>,
    pub last_error: Option<String>,
    pub pending_delete: Option<String>,
}

impl AppState {
    pub fn new(cwd: PathBuf, base: String, branches: Vec<git::Branch>) -> Self {
        Self {
            cwd,
            base,
            screen: Screen::BranchList,
            branches,
            branch_cursor: 0,
            branch_filter: String::new(),
            selected_branch: None,
            commits: Vec::new(),
            commit_cursor: 0,
            selected: HashSet::new(),
            execution_queue: Vec::new(),
            execution_index: 0,
            execution_results: Vec::new(),
            conflict_message: None,
            pause_reason: None,
            last_error: None,
            pending_delete: None,
        }
    }

    pub fn load_commits(&mut self, commits: Vec<git::Commit>) {
        self.commits = commits;
        self.commit_cursor = 0;
        self.selected.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::Branch;

    #[test]
    fn new_starts_on_branch_list_with_no_selection() {
        let state = AppState::new(
            "/tmp".into(),
            "main".to_string(),
            vec![Branch { name: "feature".into(), last_commit_epoch: 0, is_local: true }],
        );
        assert!(matches!(state.screen, Screen::BranchList));
        assert_eq!(state.branch_cursor, 0);
        assert!(state.selected_branch.is_none());
        assert!(state.selected.is_empty());
    }
}
