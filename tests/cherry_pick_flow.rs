use gpick::app::{AppState, ExecutionOutcome, ExecutionResult};
use gpick::{git, ui};
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
fn full_cherry_pick_flow_reauthors_and_signs_off_commit() {
    let dir = init_repo();
    commit_file(&dir, "a.txt", "a");
    let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

    Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
    Command::new("git").args(["config", "user.name", "Feature Author"]).current_dir(dir.path()).status().unwrap();
    Command::new("git").args(["config", "user.email", "feature@example.com"]).current_dir(dir.path()).status().unwrap();
    commit_file(&dir, "b.txt", "b");

    let feature_commits = git::list_commits(dir.path(), &base_sha, "feature").unwrap();

    Command::new("git").args(["checkout", "-q", "-"]).current_dir(dir.path()).status().unwrap();
    Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
    Command::new("git").args(["config", "user.name", "Me"]).current_dir(dir.path()).status().unwrap();
    Command::new("git").args(["config", "user.email", "me@example.com"]).current_dir(dir.path()).status().unwrap();

    let mut state = AppState::new(dir.path().to_path_buf(), base_sha, vec![]);
    state.load_commits(feature_commits);
    state.execution_queue = vec![0];
    state.execution_index = 0;
    state.execution_results = vec![ExecutionResult {
        commit: state.commits[0].clone(),
        outcome: ExecutionOutcome::Pending,
    }];

    ui::execution::step_execution(&mut state);

    assert_eq!(state.execution_index, 1);
    assert!(matches!(state.execution_results[0].outcome, ExecutionOutcome::Done));

    let log = git::run_git(dir.path(), &["log", "-1", "--format=%an|%B"]).unwrap();
    assert!(log.starts_with("Me|"));
    assert!(log.contains("Signed-off-by: Me <me@example.com>"));
}
