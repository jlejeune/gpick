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
    /// The cherry-pick (initial, or after resolving a real conflict)
    /// resolved to an empty diff — everything it would change is already
    /// present on base. Git refuses to commit it via `--continue`; offer to
    /// keep it as an empty commit or give up on it.
    EmptyAfterResolve,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PendingDelete {
    /// Confirm force-deleting a local branch (`git branch -D`).
    Local(String),
    /// Confirm deleting a remote-tracking branch's real branch on the
    /// remote (`git push <remote> --delete <branch>`).
    Remote(String),
    /// The local delete above failed because the branch is checked out in
    /// another worktree; confirm removing that worktree (only if it has no
    /// uncommitted changes — gpick never force-removes) and retrying.
    RemoveWorktree { branch: String, path: String },
    /// Confirm deleting several branches at once (multi-selected with
    /// Space), local and remote alike.
    Bulk(Vec<String>),
}

/// Tracks an in-progress bulk delete so the run loop can process one branch
/// per tick (redrawing between each) instead of blocking on the whole
/// batch — that's what makes the footer's progress spinner actually move.
#[derive(Debug, Clone, PartialEq)]
pub struct BulkDeleteState {
    pub names: Vec<String>,
    pub index: usize,
    pub errors: Vec<String>,
}

pub struct AppState {
    pub cwd: PathBuf,
    pub base: String,
    pub screen: Screen,
    pub branches: Vec<git::Branch>,
    pub branch_cursor: usize,
    pub branch_filter: String,
    /// True while the branch list's search field is being edited (entered
    /// with `/`) — while active, character keys type into `branch_filter`
    /// instead of triggering branch-list shortcuts.
    pub search_active: bool,
    /// Names of branches whose every ahead-of-base commit would cherry-pick
    /// as empty — hidden from the branch list unless `show_all_branches`.
    pub fully_picked: HashSet<String>,
    /// Toggled with `a` to reveal branches hidden via `fully_picked`.
    pub show_all_branches: bool,
    /// Set with `p` on the branch list; confirmed with y/n like a delete.
    pub pending_push: bool,
    /// Names of branches multi-selected with Space on the branch list, for
    /// a bulk-delete via Delete.
    pub selected_branches: HashSet<String>,
    /// Cursor index where a Shift+Up/Down range selection started; reset
    /// to `None` by any other key so a later range starts fresh.
    pub range_anchor: Option<usize>,
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
    pub pending_delete: Option<PendingDelete>,
    pub bulk_delete: Option<BulkDeleteState>,
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
            search_active: false,
            fully_picked: HashSet::new(),
            show_all_branches: false,
            pending_push: false,
            selected_branches: HashSet::new(),
            range_anchor: None,
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
            bulk_delete: None,
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
