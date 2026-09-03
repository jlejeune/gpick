use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("not a git repository")]
    NotAGitRepo,
    #[error("git command failed: {0}")]
    CommandFailed(String),
    #[error("could not resolve a base branch")]
    NoBaseFound,
}

pub fn run_git(cwd: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| GitError::CommandFailed(e.to_string()))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        Err(GitError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

pub fn check_is_repo(cwd: &Path) -> Result<(), GitError> {
    match run_git(cwd, &["rev-parse", "--is-inside-work-tree"]) {
        Ok(out) if out == "true" => Ok(()),
        _ => Err(GitError::NotAGitRepo),
    }
}

#[derive(Debug, Clone)]
pub struct Branch {
    pub name: String,
    pub last_commit_epoch: i64,
    /// Human-readable relative date of the branch's last commit (e.g. "3
    /// days ago"), straight from git's own `committerdate:relative` — no
    /// date math needed on the Rust side.
    pub last_commit_relative: String,
    pub is_local: bool,
}

/// Refreshes remote-tracking refs (`git fetch --prune`) so branch listings
/// and cherry-pick sources reflect what's actually on the remote — without
/// this, a branch that was force-updated upstream (e.g. by Renovate)
/// leaves gpick cherry-picking a stale, no-longer-current commit.
pub fn fetch_all(cwd: &Path) -> Result<(), GitError> {
    run_git(cwd, &["fetch", "--prune", "--quiet"]).map(|_| ())
}

pub fn list_branches(cwd: &Path) -> Result<Vec<Branch>, GitError> {
    let current = run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_default();

    let raw = run_git(
        cwd,
        &[
            "for-each-ref",
            "--format=%(refname)|%(refname:short)|%(committerdate:unix)|%(committerdate:relative)",
            "refs/heads",
            "refs/remotes",
        ],
    )?;

    let mut branches: Vec<Branch> = raw
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(4, '|');
            let full_ref = parts.next()?;
            let name = parts.next()?;
            let epoch = parts.next()?;
            let relative = parts.next()?;
            if name == current || name.ends_with("/HEAD") {
                return None;
            }
            Some(Branch {
                name: name.to_string(),
                last_commit_epoch: epoch.parse().unwrap_or(0),
                last_commit_relative: relative.to_string(),
                is_local: full_ref.starts_with("refs/heads/"),
            })
        })
        .collect();

    branches.sort_by(|a, b| b.last_commit_epoch.cmp(&a.last_commit_epoch));
    Ok(branches)
}

/// Force-deletes a local branch (`git branch -D`). Never call this on a
/// remote-tracking ref (`origin/...`) — deleting that locally would not
/// touch the branch on the remote and would just be confusing.
pub fn delete_branch(cwd: &Path, name: &str) -> Result<(), GitError> {
    run_git(cwd, &["branch", "-D", name]).map(|_| ())
}

/// Deletes the real branch on a remote (`git push <remote> --delete <branch>`).
/// This affects the shared remote, not just the local repo.
pub fn delete_remote_branch(cwd: &Path, remote: &str, branch: &str) -> Result<(), GitError> {
    run_git(cwd, &["push", remote, "--delete", branch]).map(|_| ())
}

/// Removes a stale local remote-tracking ref (e.g. `origin/foo`) without
/// touching the actual remote — used when the branch was already deleted
/// there and the local cache just hasn't caught up.
pub fn prune_remote_tracking_ref(cwd: &Path, name: &str) -> Result<(), GitError> {
    run_git(cwd, &["update-ref", "-d", &format!("refs/remotes/{name}")]).map(|_| ())
}

/// Removes a worktree. Never forces — if the worktree has uncommitted
/// changes, git refuses and this returns that error verbatim.
pub fn remove_worktree(cwd: &Path, path: &str) -> Result<(), GitError> {
    run_git(cwd, &["worktree", "remove", path]).map(|_| ())
}

/// Pushes `base` (a local branch, ref, or commit-ish) onto `master` on the
/// `origin` remote. This affects the shared remote.
pub fn push_to_master(cwd: &Path, base: &str) -> Result<(), GitError> {
    run_git(cwd, &["push", "origin", &format!("{base}:master")]).map(|_| ())
}

/// How many commits `base` is ahead of `origin/master` — how many would
/// land on the remote if `push_to_master` runs right now.
pub fn commits_ahead_of_remote_master(cwd: &Path, base: &str) -> Result<usize, GitError> {
    let range = format!("origin/master..{base}");
    let out = run_git(cwd, &["rev-list", "--count", "--end-of-options", &range])?;
    out.parse().map_err(|_| GitError::CommandFailed(format!("unexpected rev-list output: {out}")))
}

pub fn detect_base(cwd: &Path, override_ref: Option<&str>) -> Result<String, GitError> {
    if let Some(r) = override_ref {
        return Ok(r.to_string());
    }

    if let Ok(sym) = run_git(cwd, &["symbolic-ref", "refs/remotes/origin/HEAD"]) {
        if let Some(short) = sym.strip_prefix("refs/remotes/") {
            return Ok(short.to_string());
        }
    }

    for candidate in ["main", "master"] {
        if run_git(cwd, &["rev-parse", "--verify", "--quiet", candidate]).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    Err(GitError::NoBaseFound)
}

#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    pub short_sha: String,
    pub message: String,
    pub author: String,
    pub date_rfc2822: String,
}

const COMMIT_FIELD_SEP: &str = "\u{1f}"; // unit separator, won't collide with commit messages

pub fn list_commits(cwd: &Path, base: &str, branch: &str) -> Result<Vec<Commit>, GitError> {
    let range = format!("{base}..{branch}");
    let format = format!("--format=%H{COMMIT_FIELD_SEP}%h{COMMIT_FIELD_SEP}%s{COMMIT_FIELD_SEP}%an{COMMIT_FIELD_SEP}%aD");
    let raw = run_git(cwd, &["log", "--reverse", &format, "--end-of-options", &range])?;

    if raw.is_empty() {
        return Ok(Vec::new());
    }

    Ok(raw
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split(COMMIT_FIELD_SEP).collect();
            if parts.len() != 5 {
                return None;
            }
            Some(Commit {
                sha: parts[0].to_string(),
                short_sha: parts[1].to_string(),
                message: parts[2].to_string(),
                author: parts[3].to_string(),
                date_rfc2822: parts[4].to_string(),
            })
        })
        .collect())
}

pub fn show_commit(cwd: &Path, sha: &str) -> Result<String, GitError> {
    run_git(cwd, &["show", "--end-of-options", sha])
}

/// Full SHAs of commits on `branch` (ahead of `base`) whose patch is
/// already equivalent to a commit on `base` — cherry-picking them would
/// produce an empty commit. Uses `git cherry`, which compares patch-ids
/// rather than requiring the commit to actually be applied.
pub fn empty_pick_shas(cwd: &Path, base: &str, branch: &str) -> Result<std::collections::HashSet<String>, GitError> {
    let out = run_git(cwd, &["cherry", "--end-of-options", base, branch])?;
    Ok(out
        .lines()
        .filter_map(|line| {
            let (sign, sha) = line.split_once(' ')?;
            (sign == "-").then(|| sha.trim().to_string())
        })
        .collect())
}

/// True if there is nothing worth cherry-picking from `branch`: either it
/// has no commits ahead of `base` at all (e.g. a worktree branch that never
/// diverged), or every commit it does have would cherry-pick as empty
/// because it's already integrated on base.
pub fn is_fully_picked(cwd: &Path, base: &str, branch: &str) -> Result<bool, GitError> {
    let commits = list_commits(cwd, base, branch)?;
    if commits.is_empty() {
        return Ok(true);
    }
    let empty = empty_pick_shas(cwd, base, branch)?;
    Ok(commits.iter().all(|c| empty.contains(&c.sha)))
}

/// Names of branches from `branches` with nothing worth cherry-picking (see
/// `is_fully_picked`). A branch whose check errors is treated as pickable —
/// showing it is a safer default than hiding it on a git error.
pub fn fully_picked_branches(cwd: &Path, base: &str, branches: &[Branch]) -> std::collections::HashSet<String> {
    branches
        .iter()
        .filter(|b| matches!(is_fully_picked(cwd, base, &b.name), Ok(true)))
        .map(|b| b.name.clone())
        .collect()
}

#[derive(Debug)]
pub enum CherryPickOutcome {
    Success,
    Conflict(String),
}

fn cherry_pick_result(cwd: &Path, args: &[&str]) -> Result<CherryPickOutcome, GitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|e| GitError::CommandFailed(e.to_string()))?;

    if output.status.success() {
        Ok(CherryPickOutcome::Success)
    } else {
        Ok(CherryPickOutcome::Conflict(
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

pub fn cherry_pick(cwd: &Path, sha: &str) -> Result<CherryPickOutcome, GitError> {
    cherry_pick_result(cwd, &["cherry-pick", sha])
}

pub fn cherry_pick_continue(cwd: &Path) -> Result<CherryPickOutcome, GitError> {
    cherry_pick_result(cwd, &["cherry-pick", "--continue"])
}

pub fn cherry_pick_abort(cwd: &Path) -> Result<(), GitError> {
    run_git(cwd, &["cherry-pick", "--abort"]).map(|_| ())
}

/// True if a cherry-pick failure message is git's refusal to create an
/// empty commit (the resolved diff has nothing left to apply — the change
/// is already present on base) rather than an actual merge conflict.
pub fn is_empty_cherry_pick_message(msg: &str) -> bool {
    msg.contains("previous cherry-pick is now empty") || msg.contains("allow an empty commit")
}

/// Finishes a cherry-pick whose resolved diff is empty by committing that
/// empty commit directly — `git cherry-pick --continue` refuses to, even
/// with `--allow-empty` passed through to it.
pub fn commit_allow_empty(cwd: &Path) -> Result<(), GitError> {
    run_git(cwd, &["commit", "--allow-empty", "--no-edit"]).map(|_| ())
}

pub fn amend_reauthor(cwd: &Path, date_rfc2822: &str) -> Result<(), GitError> {
    let date_arg = format!("--date={date_rfc2822}");
    // --allow-empty is a no-op on a normal non-empty commit, but without it
    // amending a commit created via commit_allow_empty() (an intentionally
    // empty cherry-pick) fails outright.
    run_git(
        cwd,
        &["commit", "--amend", "--allow-empty", "--reset-author", "-s", "--no-edit", &date_arg],
    )
    .map(|_| ())
}

pub fn status_summary(cwd: &Path) -> Result<String, GitError> {
    run_git(cwd, &["status", "--short"])
}

/// Undo the most recent commit (used to unwind a cherry-pick that succeeded
/// but whose follow-up amend failed and is being aborted from ConflictPause).
pub fn reset_hard_head_minus_one(cwd: &Path) -> Result<(), GitError> {
    run_git(cwd, &["reset", "--hard", "HEAD~1"]).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.email", "t@example.com"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir.path()).status().unwrap();
        dir
    }

    #[test]
    fn run_git_returns_trimmed_stdout_on_success() {
        let dir = init_repo();
        let out = run_git(dir.path(), &["rev-parse", "--is-inside-work-tree"]).unwrap();
        assert_eq!(out, "true");
    }

    #[test]
    fn run_git_returns_command_failed_on_nonzero_exit() {
        let dir = init_repo();
        let err = run_git(dir.path(), &["show", "nonexistent-ref"]).unwrap_err();
        assert!(matches!(err, GitError::CommandFailed(_)));
    }

    #[test]
    fn check_is_repo_ok_for_git_repo() {
        let dir = init_repo();
        assert!(check_is_repo(dir.path()).is_ok());
    }

    #[test]
    fn check_is_repo_errs_for_non_repo() {
        let dir = TempDir::new().unwrap();
        assert!(matches!(check_is_repo(dir.path()), Err(GitError::NotAGitRepo)));
    }

    fn commit_file(dir: &TempDir, name: &str, content: &str) {
        std::fs::write(dir.path().join(name), content).unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", name]).current_dir(dir.path()).status().unwrap();
    }

    #[test]
    fn list_branches_excludes_current_and_sorts_by_recency() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        Command::new("git").args(["branch", "old"]).current_dir(dir.path()).status().unwrap();
        // make `old`'s tip older than a second new branch
        std::thread::sleep(std::time::Duration::from_secs(1));
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "b.txt", "b");
        Command::new("git").args(["checkout", "-q", "-"]).current_dir(dir.path()).status().unwrap(); // back to master/main

        let branches = list_branches(dir.path()).unwrap();
        let names: Vec<&str> = branches.iter().map(|b| b.name.as_str()).collect();
        assert!(names.contains(&"feature"));
        assert!(names.contains(&"old"));
        assert!(!names.iter().any(|n| n.contains("HEAD")));
        // feature has a newer commit than old, so it must sort first
        let feature_pos = names.iter().position(|n| *n == "feature").unwrap();
        let old_pos = names.iter().position(|n| *n == "old").unwrap();
        assert!(feature_pos < old_pos);

        let old = branches.iter().find(|b| b.name == "old").unwrap();
        assert!(old.is_local);
    }

    #[test]
    fn list_branches_includes_a_human_readable_relative_date() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        Command::new("git").args(["branch", "feature"]).current_dir(dir.path()).status().unwrap();

        let branches = list_branches(dir.path()).unwrap();
        let feature = branches.iter().find(|b| b.name == "feature").unwrap();
        assert!(feature.last_commit_relative.contains("ago"), "got: {:?}", feature.last_commit_relative);
    }

    #[test]
    fn fetch_all_updates_a_stale_remote_tracking_ref() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let stale = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        let remote_dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git").args(["push", "-q", "origin", "HEAD:refs/heads/feature"]).current_dir(dir.path()).status().unwrap();
        assert_eq!(run_git(dir.path(), &["rev-parse", "origin/feature"]).unwrap(), stale);

        // advance the branch on the bare "remote" directly (not via a local
        // push), simulating someone else (e.g. Renovate) force-updating it
        // independently of this clone — the local origin/feature ref stays
        // stale until an explicit fetch.
        commit_file(&dir, "b.txt", "b");
        let new_tip = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git")
            .args(["push", "-q", "origin", "HEAD:refs/heads/tmp-carrier"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git")
            .args(["update-ref", "refs/heads/feature", &new_tip])
            .current_dir(remote_dir.path())
            .status()
            .unwrap();

        assert_eq!(run_git(dir.path(), &["rev-parse", "origin/feature"]).unwrap(), stale, "test setup bug: local ref moved early");

        fetch_all(dir.path()).unwrap();

        assert_eq!(run_git(dir.path(), &["rev-parse", "origin/feature"]).unwrap(), new_tip);
    }

    #[test]
    fn list_branches_marks_remote_tracking_refs_as_not_local() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let remote_dir = TempDir::new().unwrap();
        Command::new("git")
            .args(["clone", "-q", dir.path().to_str().unwrap(), remote_dir.path().to_str().unwrap()])
            .status()
            .unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git").args(["fetch", "-q", "origin"]).current_dir(dir.path()).status().unwrap();

        let branches = list_branches(dir.path()).unwrap();
        let remote = branches.iter().find(|b| b.name.starts_with("origin/")).unwrap();
        assert!(!remote.is_local);
    }

    #[test]
    fn delete_branch_removes_a_local_branch() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        Command::new("git").args(["branch", "throwaway"]).current_dir(dir.path()).status().unwrap();

        delete_branch(dir.path(), "throwaway").unwrap();

        let branches = list_branches(dir.path()).unwrap();
        assert!(!branches.iter().any(|b| b.name == "throwaway"));
    }

    #[test]
    fn prune_remote_tracking_ref_removes_the_local_ref_without_touching_the_remote() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        // simulate a stale remote-tracking ref: no actual "origin" remote needed,
        // just a ref under refs/remotes/ as list_branches would have created.
        run_git(dir.path(), &["update-ref", "refs/remotes/origin/ghost", &sha]).unwrap();
        assert!(!run_git(dir.path(), &["for-each-ref", "refs/remotes/origin/ghost"]).unwrap().is_empty());

        prune_remote_tracking_ref(dir.path(), "origin/ghost").unwrap();

        assert!(run_git(dir.path(), &["for-each-ref", "refs/remotes/origin/ghost"]).unwrap().is_empty());
    }

    #[test]
    fn delete_remote_branch_removes_it_from_the_remote() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        // a bare repo acts as a stand-in "remote" we can push --delete against
        let remote_dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git").args(["branch", "throwaway"]).current_dir(dir.path()).status().unwrap();
        Command::new("git")
            .args(["push", "-q", "origin", "throwaway"])
            .current_dir(dir.path())
            .status()
            .unwrap();

        delete_remote_branch(dir.path(), "origin", "throwaway").unwrap();

        let remote_refs = run_git(remote_dir.path(), &["branch", "--list", "throwaway"]).unwrap();
        assert!(remote_refs.is_empty());
    }

    #[test]
    fn push_to_master_pushes_base_onto_the_remote_master_branch() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let head_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        let remote_dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(dir.path())
            .status()
            .unwrap();

        push_to_master(dir.path(), "HEAD").unwrap();

        let remote_master = run_git(remote_dir.path(), &["rev-parse", "master"]).unwrap();
        assert_eq!(remote_master, head_sha);
    }

    #[test]
    fn commits_ahead_of_remote_master_counts_unpushed_commits() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let remote_dir = TempDir::new().unwrap();
        Command::new("git").args(["init", "-q", "--bare"]).current_dir(remote_dir.path()).status().unwrap();
        Command::new("git")
            .args(["remote", "add", "origin", remote_dir.path().to_str().unwrap()])
            .current_dir(dir.path())
            .status()
            .unwrap();
        Command::new("git").args(["push", "-q", "origin", "HEAD:master"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["fetch", "-q", "origin"]).current_dir(dir.path()).status().unwrap();

        assert_eq!(commits_ahead_of_remote_master(dir.path(), "HEAD").unwrap(), 0);

        commit_file(&dir, "b.txt", "b");
        commit_file(&dir, "c.txt", "c");

        assert_eq!(commits_ahead_of_remote_master(dir.path(), "HEAD").unwrap(), 2);
    }

    #[test]
    fn remove_worktree_removes_a_clean_worktree() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        Command::new("git").args(["branch", "feature"]).current_dir(dir.path()).status().unwrap();
        let worktree_dir = TempDir::new().unwrap();
        std::fs::remove_dir(worktree_dir.path()).unwrap(); // git worktree add needs the path to not exist
        Command::new("git")
            .args(["worktree", "add", "-q", worktree_dir.path().to_str().unwrap(), "feature"])
            .current_dir(dir.path())
            .status()
            .unwrap();

        remove_worktree(dir.path(), worktree_dir.path().to_str().unwrap()).unwrap();

        assert!(!worktree_dir.path().exists());
    }

    #[test]
    fn remove_worktree_refuses_a_dirty_worktree() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        Command::new("git").args(["branch", "feature"]).current_dir(dir.path()).status().unwrap();
        let worktree_dir = TempDir::new().unwrap();
        std::fs::remove_dir(worktree_dir.path()).unwrap();
        Command::new("git")
            .args(["worktree", "add", "-q", worktree_dir.path().to_str().unwrap(), "feature"])
            .current_dir(dir.path())
            .status()
            .unwrap();
        std::fs::write(worktree_dir.path().join("untracked.txt"), "dirty").unwrap();
        Command::new("git")
            .args(["add", "untracked.txt"])
            .current_dir(worktree_dir.path())
            .status()
            .unwrap();

        let err = remove_worktree(dir.path(), worktree_dir.path().to_str().unwrap()).unwrap_err();
        assert!(matches!(err, GitError::CommandFailed(_)));
        assert!(worktree_dir.path().exists());
    }

    #[test]
    fn detect_base_uses_override_verbatim() {
        let dir = init_repo();
        let base = detect_base(dir.path(), Some("some-ref")).unwrap();
        assert_eq!(base, "some-ref");
    }

    #[test]
    fn detect_base_falls_back_to_main_when_no_origin_head() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        Command::new("git").args(["branch", "-m", "main"]).current_dir(dir.path()).status().unwrap();
        let base = detect_base(dir.path(), None).unwrap();
        assert_eq!(base, "main");
    }

    #[test]
    fn detect_base_errs_when_nothing_resolves() {
        let dir = init_repo(); // no commits, no main/master, no origin
        let err = detect_base(dir.path(), None).unwrap_err();
        assert!(matches!(err, GitError::NoBaseFound));
    }

    #[test]
    fn list_commits_returns_oldest_first_with_fields() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a"); // base
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "b.txt", "b"); // first ahead commit
        commit_file(&dir, "c.txt", "c"); // second ahead commit

        let commits = list_commits(dir.path(), &base_sha, "feature").unwrap();
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].message, "b.txt");
        assert_eq!(commits[1].message, "c.txt");
        assert!(!commits[0].short_sha.is_empty());
        assert_eq!(commits[0].author, "Test");
    }

    #[test]
    fn show_commit_returns_diff_output() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        let out = show_commit(dir.path(), &sha).unwrap();
        assert!(out.contains("a.txt"));
    }

    #[test]
    fn empty_pick_shas_flags_a_commit_already_equivalent_on_base() {
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change");
        commit_file(&dir, "unique.txt", "unique change");
        let shared_sha = run_git(dir.path(), &["rev-parse", "HEAD~1"]).unwrap();
        let unique_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        // apply the same change directly on base with a different commit
        // message, so its patch-id matches `shared_sha` (same diff) even
        // though it's a genuinely different commit object
        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("shared.txt"), "shared change").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "applied directly on base"]).current_dir(dir.path()).status().unwrap();
        let new_base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        assert_ne!(new_base_sha, shared_sha, "test setup bug: commits should differ");

        let empty = empty_pick_shas(dir.path(), &new_base_sha, "feature").unwrap();
        assert!(empty.contains(&shared_sha), "expected {shared_sha} to be flagged as empty, got {empty:?}");
        assert!(!empty.contains(&unique_sha));
    }

    #[test]
    fn is_fully_picked_true_when_branch_has_no_commits_ahead() {
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();

        assert!(is_fully_picked(dir.path(), &base_sha, "feature").unwrap());
    }

    #[test]
    fn is_fully_picked_false_when_some_commits_are_still_unique() {
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "unique.txt", "unique change");

        assert!(!is_fully_picked(dir.path(), &base_sha, "feature").unwrap());
    }

    #[test]
    fn is_fully_picked_true_when_every_commit_is_already_applied_on_base() {
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change");

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("shared.txt"), "shared change").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "applied directly on base"]).current_dir(dir.path()).status().unwrap();
        let new_base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        assert!(is_fully_picked(dir.path(), &new_base_sha, "feature").unwrap());
    }

    #[test]
    fn fully_picked_branches_returns_only_the_fully_integrated_ones() {
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", "-b", "picked"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change");
        Command::new("git").args(["checkout", "-q", "-b", "unpicked", &base_sha]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "other.txt", "other change");

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("shared.txt"), "shared change").unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", "applied directly on base"]).current_dir(dir.path()).status().unwrap();
        let new_base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        let branches = vec![
            Branch { name: "picked".into(), last_commit_epoch: 0, last_commit_relative: String::new(), is_local: true },
            Branch { name: "unpicked".into(), last_commit_epoch: 0, last_commit_relative: String::new(), is_local: true },
        ];
        let result = fully_picked_branches(dir.path(), &new_base_sha, &branches);

        assert!(result.contains("picked"));
        assert!(!result.contains("unpicked"));
    }

    #[test]
    fn cherry_pick_succeeds_on_clean_apply() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "b.txt", "b");
        let feature_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();

        let outcome = cherry_pick(dir.path(), &feature_sha).unwrap();
        assert!(matches!(outcome, CherryPickOutcome::Success));
    }

    #[test]
    fn cherry_pick_reports_conflict_on_overlapping_change() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "line1");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "line1-feature").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "feature change"]).current_dir(dir.path()).status().unwrap();
        let feature_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "line1-base").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "base change"]).current_dir(dir.path()).status().unwrap();

        let outcome = cherry_pick(dir.path(), &feature_sha).unwrap();
        assert!(matches!(outcome, CherryPickOutcome::Conflict(_)));
        cherry_pick_abort(dir.path()).unwrap();
    }

    #[test]
    fn is_empty_cherry_pick_message_recognizes_gits_wording() {
        assert!(is_empty_cherry_pick_message(
            "The previous cherry-pick is now empty, possibly due to conflict resolution."
        ));
        assert!(!is_empty_cherry_pick_message("CONFLICT (content): Merge conflict in a.txt"));
    }

    #[test]
    fn commit_allow_empty_finishes_a_stuck_empty_cherry_pick() {
        let dir = init_repo();
        commit_file(&dir, "f.txt", "base");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same-change").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "feature"]).current_dir(dir.path()).status().unwrap();
        let feature_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("f.txt"), "same-change").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "same change, different message"]).current_dir(dir.path()).status().unwrap();

        // this cherry-pick resolves to an empty diff — content already matches
        let outcome = cherry_pick(dir.path(), &feature_sha).unwrap();
        let CherryPickOutcome::Conflict(msg) = outcome else { panic!("expected the empty-pick refusal") };
        assert!(is_empty_cherry_pick_message(&msg), "unexpected message: {msg}");

        commit_allow_empty(dir.path()).unwrap();

        let status = run_git(dir.path(), &["status", "--short"]).unwrap();
        assert!(status.is_empty());
    }

    #[test]
    fn amend_reauthor_updates_author_and_signoff() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "a");
        amend_reauthor(dir.path(), "Mon, 1 Jan 2024 00:00:00 +0000").unwrap();
        let log = run_git(dir.path(), &["log", "-1", "--format=%an|%ad|%B"]).unwrap();
        assert!(log.contains("Test"));
        assert!(log.contains("Signed-off-by"));
    }

    #[test]
    fn status_summary_lists_unmerged_paths_on_conflict() {
        let dir = init_repo();
        commit_file(&dir, "a.txt", "line1");
        let base_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "line1-feature").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "feature change"]).current_dir(dir.path()).status().unwrap();
        let feature_sha = run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        std::fs::write(dir.path().join("a.txt"), "line1-base").unwrap();
        Command::new("git").args(["commit", "-q", "-am", "base change"]).current_dir(dir.path()).status().unwrap();

        cherry_pick(dir.path(), &feature_sha).unwrap();
        let status = status_summary(dir.path()).unwrap();
        assert!(status.contains("a.txt"));
        cherry_pick_abort(dir.path()).unwrap();
    }
}
