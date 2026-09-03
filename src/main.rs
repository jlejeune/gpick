use gpick::{app, git, ui};

use app::{AppState, Screen};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::{Constraint, Direction, Layout};
use ratatui::Terminal;
use std::io;

/// Ctrl+C arrives as a plain key event once raw mode disables ISIG — it must
/// be treated as an immediate, universal quit rather than being dispatched to
/// whichever screen happens to be active (which would otherwise, e.g., be
/// misread as "continue" by the conflict-pause screen).
fn is_ctrl_c(key: &KeyEvent) -> bool {
    key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Loads the commits ahead of base for `branch` into `state`, filtering out
/// any commit whose patch is already equivalent to one on base — cherry-
/// picking those would land as an empty, useless commit. If the equivalence
/// check itself fails, commits are shown unfiltered rather than blocking
/// the user on an unrelated git error.
fn load_commits_for_branch(state: &mut AppState, branch: &str) {
    match git::list_commits(&state.cwd, &state.base, branch) {
        Ok(commits) => {
            state.last_error = None;
            let commits = match git::empty_pick_shas(&state.cwd, &state.base, branch) {
                Ok(empty) => commits.into_iter().filter(|c| !empty.contains(&c.sha)).collect(),
                Err(_) => commits,
            };
            state.load_commits(commits);
        }
        Err(e) => {
            state.last_error = Some(e.to_string());
            state.load_commits(Vec::new());
        }
    }
}

/// Resets per-pick state and returns to the branch list so the user can
/// chain another cherry-pick without restarting the program.
fn return_to_branch_list_after_execution(state: &mut AppState) {
    if let Some(branch) = &state.selected_branch {
        if let Ok(true) = git::is_fully_picked(&state.cwd, &state.base, branch) {
            state.fully_picked.insert(branch.clone());
        }
    }
    state.commits.clear();
    state.commit_cursor = 0;
    state.selected.clear();
    state.execution_queue.clear();
    state.execution_index = 0;
    state.execution_results.clear();
    state.selected_branch = None;
    state.screen = Screen::BranchList;
}

/// Breadcrumb segments for the header bar, reflecting where the user is in
/// the branch -> commits -> execution flow.
fn breadcrumb_segments(state: &AppState) -> Vec<&str> {
    match state.screen {
        Screen::BranchList => vec!["Branches"],
        Screen::CommitList => match &state.selected_branch {
            Some(b) => vec!["Branches", b.as_str(), "Commits"],
            None => vec!["Commits"],
        },
        Screen::Execution => vec!["Execution"],
        Screen::ConflictPause => vec!["Execution", "Conflict"],
        Screen::Quit => vec![],
    }
}

fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        default_hook(info);
    }));
}

#[derive(Parser)]
#[command(name = "gpick", about = "Interactive cherry-pick TUI")]
struct Cli {
    /// Override the auto-detected base ref (defaults to origin/HEAD, then main, then master)
    #[arg(long)]
    base: Option<String>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let cwd = std::env::current_dir()?;

    if let Err(e) = git::check_is_repo(&cwd) {
        eprintln!("gpick: {e}");
        std::process::exit(1);
    }

    // Best-effort: a branch that was force-updated upstream (e.g. by
    // Renovate) otherwise leaves gpick cherry-picking a stale commit.
    // Don't block startup on network trouble (offline, auth) — just warn.
    if let Err(e) = git::fetch_all(&cwd) {
        eprintln!("gpick: warning: git fetch failed ({e}) — branch list may be stale");
    }

    let base = match git::detect_base(&cwd, cli.base.as_deref()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("gpick: {e} — pass --base <ref>, or run `git remote set-head origin -a`");
            std::process::exit(1);
        }
    };

    let branches = match git::list_branches(&cwd) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("gpick: {e}");
            std::process::exit(1);
        }
    };

    let mut state = AppState::new(cwd, base, branches);
    state.fully_picked = git::fully_picked_branches(&state.cwd, &state.base, &state.branches);

    install_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, state: &mut AppState) -> io::Result<()> {
    let mut preview = String::new();

    loop {
        if state.screen == Screen::Quit {
            return Ok(());
        }

        // the execution queue just finished (whether by stepping through it
        // or by resuming from a resolved conflict) — chain straight back to
        // the branch list instead of dead-ending on an "all done" screen.
        if state.screen == Screen::Execution
            && !state.execution_queue.is_empty()
            && state.execution_index >= state.execution_queue.len()
        {
            return_to_branch_list_after_execution(state);
            continue;
        }

        // a bulk delete just finished processing every branch — finalize
        // it (report any errors) before the next draw.
        if let Some(bulk) = &state.bulk_delete {
            if bulk.index >= bulk.names.len() {
                ui::branch_list::finish_bulk_delete(state);
                continue;
            }
        }

        // refresh preview cache when hovering a different commit on the commit list screen
        if state.screen == Screen::CommitList {
            if let Some(c) = state.commits.get(state.commit_cursor) {
                preview = git::show_commit(&state.cwd, &c.sha).unwrap_or_default();
            }
        }

        terminal.draw(|frame| {
            enum FooterKind {
                Progress(String),
                Confirm(String),
                Error(String),
                Hints(String),
            }
            let footer_kind = if let Some(bulk) = &state.bulk_delete {
                FooterKind::Progress(ui::branch_list::bulk_delete_progress_text(bulk))
            } else if let Some(pending) = &state.pending_delete {
                FooterKind::Confirm(ui::branch_list::confirm_prompt(pending))
            } else if state.pending_push {
                FooterKind::Confirm(ui::branch_list::push_confirm_prompt(state))
            } else if let Some(err) = &state.last_error {
                FooterKind::Error(format!("Error: {err}"))
            } else {
                FooterKind::Hints(ui::help::footer_text(&state.screen).to_string())
            };
            let footer_str = match &footer_kind {
                FooterKind::Progress(s) | FooterKind::Confirm(s) | FooterKind::Error(s) | FooterKind::Hints(s) => s.clone(),
            };
            let footer_height = ui::help::footer_height(&footer_str, frame.area().width);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(1),
                    Constraint::Min(0),
                    Constraint::Length(footer_height),
                ])
                .split(frame.area());
            let (header, content, footer) = (chunks[0], chunks[1], chunks[2]);

            let breadcrumb_segments = breadcrumb_segments(state);
            ui::theme::draw_header(frame, header, &breadcrumb_segments);

            match state.screen {
                Screen::BranchList => ui::branch_list::draw_branch_list(frame, content, state),
                Screen::CommitList => ui::commit_list::draw_commit_list(frame, content, state, &preview),
                Screen::Execution => ui::execution::draw_execution(frame, content, state),
                Screen::ConflictPause => {
                    let status = git::status_summary(&state.cwd).unwrap_or_default();
                    ui::conflict_pause::draw_conflict_pause(frame, content, state, &status);
                }
                Screen::Quit => {}
            }
            match footer_kind {
                FooterKind::Progress(s) => ui::help::draw_message_text(frame, footer, &s, ui::theme::PENDING),
                FooterKind::Confirm(s) => ui::help::draw_message_text(frame, footer, &s, ui::theme::PENDING),
                FooterKind::Error(s) => ui::help::draw_message_text(frame, footer, &s, ui::theme::ERROR),
                FooterKind::Hints(s) => ui::help::draw_footer_text(frame, footer, &s),
            }
        })?;

        // drive execution forward automatically while on the Execution screen
        if state.screen == Screen::Execution && state.execution_index < state.execution_queue.len() {
            ui::execution::step_execution(state);
            continue;
        }

        // process one branch of an in-progress bulk delete per tick, so the
        // footer's progress spinner is redrawn between each git call
        if let Some(bulk) = &state.bulk_delete {
            if bulk.index < bulk.names.len() {
                ui::branch_list::step_bulk_delete(state);
                continue;
            }
        }

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if is_ctrl_c(&key) {
                    state.screen = Screen::Quit;
                    continue;
                }

                if ui::branch_list::handle_key_delete_confirm(state, key.code) {
                    continue;
                }

                if ui::branch_list::handle_key_push_confirm(state, key.code) {
                    continue;
                }

                match state.screen {
                    Screen::BranchList => {
                        ui::branch_list::handle_key_branch_list(state, key.code, key.modifiers);
                        if state.screen == Screen::CommitList {
                            if let Some(branch) = state.selected_branch.clone() {
                                load_commits_for_branch(state, &branch);
                            }
                        }
                    }
                    Screen::CommitList => ui::commit_list::handle_key_commit_list(state, key.code),
                    // Execution is never polled for input: the loop either
                    // steps it forward or, once the queue finishes, chains
                    // straight back to the branch list (see the top-of-loop
                    // check above) before ever reaching event::poll here.
                    Screen::Execution => {}
                    Screen::ConflictPause => {
                        if let Err(e) = ui::conflict_pause::handle_key_conflict_pause(state, key.code) {
                            state.conflict_message = Some(e.to_string());
                        }
                    }
                    Screen::Quit => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_ctrl_c_matches_control_modified_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(is_ctrl_c(&key));
    }

    #[test]
    fn is_ctrl_c_does_not_match_plain_c() {
        let key = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE);
        assert!(!is_ctrl_c(&key));
    }

    #[test]
    fn is_ctrl_c_does_not_match_other_control_keys() {
        let key = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        assert!(!is_ctrl_c(&key));
    }

    #[test]
    fn breadcrumb_reflects_branch_list_screen() {
        let state = AppState::new("/tmp".into(), "main".into(), vec![]);
        assert_eq!(breadcrumb_segments(&state), vec!["Branches"]);
    }

    #[test]
    fn breadcrumb_includes_selected_branch_on_commit_list() {
        let mut state = AppState::new("/tmp".into(), "main".into(), vec![]);
        state.screen = Screen::CommitList;
        state.selected_branch = Some("feature-x".to_string());
        assert_eq!(breadcrumb_segments(&state), vec!["Branches", "feature-x", "Commits"]);
    }

    #[test]
    fn breadcrumb_is_empty_on_quit() {
        let mut state = AppState::new("/tmp".into(), "main".into(), vec![]);
        state.screen = Screen::Quit;
        assert!(breadcrumb_segments(&state).is_empty());
    }

    fn init_repo() -> tempfile::TempDir {
        use std::process::Command;
        let dir = tempfile::TempDir::new().unwrap();
        Command::new("git").args(["init", "-q"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.email", "t@example.com"]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["config", "user.name", "Test"]).current_dir(dir.path()).status().unwrap();
        dir
    }

    fn commit_file(dir: &tempfile::TempDir, name: &str, content: &str, message: &str) {
        use std::process::Command;
        std::fs::write(dir.path().join(name), content).unwrap();
        Command::new("git").args(["add", "."]).current_dir(dir.path()).status().unwrap();
        Command::new("git").args(["commit", "-q", "-m", message]).current_dir(dir.path()).status().unwrap();
    }

    #[test]
    fn load_commits_for_branch_filters_out_already_applied_patches() {
        use std::process::Command;
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base", "base");
        let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change", "shared");
        commit_file(&dir, "unique.txt", "unique change", "unique");
        let unique_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change", "applied directly on base");
        let new_base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        let mut state = AppState::new(dir.path().to_path_buf(), new_base_sha, vec![]);
        load_commits_for_branch(&mut state, "feature");

        assert_eq!(state.commits.len(), 1);
        assert_eq!(state.commits[0].sha, unique_sha);
    }

    #[test]
    fn return_to_branch_list_after_execution_resets_pick_state() {
        let mut state = AppState::new("/tmp".into(), "main".into(), vec![]);
        state.selected_branch = Some("feature".to_string());
        state.commits = vec![crate::git::Commit {
            sha: "deadbeef".into(),
            short_sha: "deadbee".into(),
            message: "msg".into(),
            author: "Test".into(),
            date_rfc2822: "Mon, 1 Jan 2024 00:00:00 +0000".into(),
        }];
        state.selected.insert(0);
        state.execution_queue = vec![0];
        state.execution_index = 1;
        state.screen = Screen::Execution;

        return_to_branch_list_after_execution(&mut state);

        assert_eq!(state.screen, Screen::BranchList);
        assert!(state.selected_branch.is_none());
        assert!(state.commits.is_empty());
        assert!(state.selected.is_empty());
        assert!(state.execution_queue.is_empty());
        assert_eq!(state.execution_index, 0);
    }

    #[test]
    fn return_to_branch_list_after_execution_marks_the_branch_fully_picked_if_it_now_is() {
        use std::process::Command;
        let dir = init_repo();
        commit_file(&dir, "base.txt", "base", "base");
        let base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        Command::new("git").args(["checkout", "-q", "-b", "feature"]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change", "shared");

        Command::new("git").args(["checkout", "-q", &base_sha]).current_dir(dir.path()).status().unwrap();
        commit_file(&dir, "shared.txt", "shared change", "picked via cherry-pick");
        let new_base_sha = git::run_git(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        let mut state = AppState::new(dir.path().to_path_buf(), new_base_sha, vec![]);
        state.selected_branch = Some("feature".to_string());
        state.screen = Screen::Execution;
        state.execution_queue = vec![0];
        state.execution_index = 1;

        return_to_branch_list_after_execution(&mut state);

        assert!(state.fully_picked.contains("feature"));
    }
}
