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

        // refresh preview cache when hovering a different commit on the commit list screen
        if state.screen == Screen::CommitList {
            if let Some(c) = state.commits.get(state.commit_cursor) {
                preview = git::show_commit(&state.cwd, &c.sha).unwrap_or_default();
            }
        }

        terminal.draw(|frame| {
            enum FooterKind {
                Confirm(String),
                Error(String),
                Hints(String),
            }
            let footer_kind = if let Some(pending) = &state.pending_delete {
                FooterKind::Confirm(ui::branch_list::confirm_prompt(pending))
            } else if let Some(err) = &state.last_error {
                FooterKind::Error(format!("Error: {err}"))
            } else {
                FooterKind::Hints(ui::help::footer_text(&state.screen).to_string())
            };
            let footer_str = match &footer_kind {
                FooterKind::Confirm(s) | FooterKind::Error(s) | FooterKind::Hints(s) => s.clone(),
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

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                if is_ctrl_c(&key) {
                    state.screen = Screen::Quit;
                    continue;
                }

                if ui::branch_list::handle_key_delete_confirm(state, key.code) {
                    continue;
                }

                match state.screen {
                    Screen::BranchList => {
                        ui::branch_list::handle_key_branch_list(state, key.code);
                        if state.screen == Screen::CommitList {
                            if let Some(branch) = state.selected_branch.clone() {
                                match git::list_commits(&state.cwd, &state.base, &branch) {
                                    Ok(commits) => {
                                        state.last_error = None;
                                        state.load_commits(commits);
                                    }
                                    Err(e) => {
                                        state.last_error = Some(e.to_string());
                                        state.load_commits(Vec::new());
                                    }
                                }
                            }
                        }
                    }
                    Screen::CommitList => ui::commit_list::handle_key_commit_list(state, key.code),
                    Screen::Execution => {
                        if key.code == KeyCode::Char('q') {
                            state.screen = Screen::Quit;
                        }
                    }
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
}
