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
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(frame.area());
            let (content, footer) = (chunks[0], chunks[1]);

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
            if let Some(name) = &state.pending_delete {
                ui::help::draw_footer_text(frame, footer, &format!("Delete branch '{name}'? y/n"));
            } else if let Some(err) = &state.last_error {
                ui::help::draw_footer_text(frame, footer, &format!("Error: {err}"));
            } else {
                ui::help::draw_footer(frame, footer, &state.screen);
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
}
