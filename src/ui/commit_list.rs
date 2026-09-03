use crate::app::{AppState, ExecutionOutcome, ExecutionResult, Screen};
use crate::ui::theme;
use crossterm::event::KeyCode;
use ratatui::prelude::*;
use ratatui::widgets::{List, ListItem, ListState, Paragraph, Wrap};

pub fn handle_key_commit_list(state: &mut AppState, key: KeyCode) {
    match key {
        KeyCode::Down => {
            if state.commit_cursor + 1 < state.commits.len() {
                state.commit_cursor += 1;
            }
        }
        KeyCode::Up => {
            state.commit_cursor = state.commit_cursor.saturating_sub(1);
        }
        KeyCode::Char(' ') => {
            if !state.selected.remove(&state.commit_cursor) {
                state.selected.insert(state.commit_cursor);
            }
        }
        KeyCode::Enter => {
            if state.selected.is_empty() {
                return;
            }
            let mut queue: Vec<usize> = state.selected.iter().copied().collect();
            queue.sort_unstable();
            state.execution_results = queue
                .iter()
                .filter_map(|&idx| {
                    state.commits.get(idx).map(|c| ExecutionResult {
                        commit: c.clone(),
                        outcome: ExecutionOutcome::Pending,
                    })
                })
                .collect();
            state.execution_queue = queue;
            state.execution_index = 0;
            state.screen = Screen::Execution;
        }
        KeyCode::Esc | KeyCode::Char('q') => state.screen = Screen::BranchList,
        _ => {}
    }
}

pub fn draw_commit_list(frame: &mut Frame, area: Rect, state: &AppState, preview: &str) {
    let is_empty = state.commits.is_empty();
    let items: Vec<ListItem> = if is_empty {
        let msg = state
            .last_error
            .clone()
            .unwrap_or_else(|| format!("No commits ahead of {}", state.base));
        vec![ListItem::new(Span::styled(msg, Style::default().fg(theme::ERROR)))]
    } else {
        state
            .commits
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let (mark, mark_color) =
                    if state.selected.contains(&i) { ("[x]", theme::SUCCESS) } else { ("[ ]", theme::MUTED) };
                let line = Line::from(vec![
                    Span::styled(mark, Style::default().fg(mark_color).add_modifier(Modifier::BOLD)),
                    Span::raw(" "),
                    Span::styled(c.short_sha.clone(), Style::default().fg(theme::MUTED)),
                    Span::raw(" "),
                    Span::raw(c.message.clone()),
                ]);
                ListItem::new(line)
            })
            .collect()
    };
    let list = List::new(items)
        .block(theme::titled_block("Commits"))
        .highlight_style(theme::highlight_style())
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    if !is_empty {
        list_state.select(Some(state.commit_cursor));
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    frame.render_stateful_widget(list, chunks[0], &mut list_state);
    if !is_empty {
        theme::draw_scrollbar(frame, chunks[0], state.commits.len(), state.commit_cursor);
    }
    draw_preview_panel(frame, chunks[1], state, preview);
}

fn draw_preview_panel(frame: &mut Frame, area: Rect, state: &AppState, preview: &str) {
    let block = theme::titled_block("Preview");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let header_line = match state.commits.get(state.commit_cursor) {
        Some(c) => format!("{} · {}", c.author, c.date_rfc2822),
        None => String::new(),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    let header = Paragraph::new(header_line).style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));
    frame.render_widget(header, rows[0]);

    let diff = Paragraph::new(preview).wrap(Wrap { trim: false });
    frame.render_widget(diff, rows[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{AppState, Screen};
    use crate::git::Commit;
    use crossterm::event::KeyCode;

    fn commit(msg: &str) -> Commit {
        Commit {
            sha: format!("sha-{msg}"),
            short_sha: msg.to_string(),
            message: msg.to_string(),
            author: "Test".into(),
            date_rfc2822: "Mon, 1 Jan 2024 00:00:00 +0000".into(),
        }
    }

    fn state_with_commits() -> AppState {
        let mut state = AppState::new("/tmp".into(), "main".into(), vec![]);
        state.load_commits(vec![commit("first"), commit("second"), commit("third")]);
        state
    }

    #[test]
    fn space_toggles_selection_of_hovered_commit() {
        let mut state = state_with_commits();
        handle_key_commit_list(&mut state, KeyCode::Char(' '));
        assert!(state.selected.contains(&0));
        handle_key_commit_list(&mut state, KeyCode::Char(' '));
        assert!(!state.selected.contains(&0));
    }

    #[test]
    fn hover_moves_independently_of_selection() {
        let mut state = state_with_commits();
        handle_key_commit_list(&mut state, KeyCode::Char(' ')); // select 0
        handle_key_commit_list(&mut state, KeyCode::Down); // hover -> 1
        assert_eq!(state.commit_cursor, 1);
        assert!(state.selected.contains(&0));
        assert!(!state.selected.contains(&1));
    }

    #[test]
    fn enter_builds_ascending_execution_queue_and_moves_to_execution() {
        let mut state = state_with_commits();
        handle_key_commit_list(&mut state, KeyCode::Down); // hover 1
        handle_key_commit_list(&mut state, KeyCode::Char(' ')); // select 1
        handle_key_commit_list(&mut state, KeyCode::Up); // hover 0
        handle_key_commit_list(&mut state, KeyCode::Char(' ')); // select 0
        handle_key_commit_list(&mut state, KeyCode::Enter);
        assert_eq!(state.execution_queue, vec![0, 1]);
        assert_eq!(state.screen, Screen::Execution);
    }

    #[test]
    fn enter_with_no_selection_stays_on_commit_list() {
        let mut state = state_with_commits();
        state.screen = Screen::CommitList;
        handle_key_commit_list(&mut state, KeyCode::Enter);
        assert_eq!(state.screen, Screen::CommitList);
        assert!(state.execution_queue.is_empty());
    }

    #[test]
    fn esc_returns_to_branch_list() {
        let mut state = state_with_commits();
        handle_key_commit_list(&mut state, KeyCode::Esc);
        assert_eq!(state.screen, Screen::BranchList);
    }

    #[test]
    fn q_returns_to_branch_list() {
        let mut state = state_with_commits();
        handle_key_commit_list(&mut state, KeyCode::Char('q'));
        assert_eq!(state.screen, Screen::BranchList);
    }

    #[test]
    fn enter_prepopulates_pending_execution_results() {
        let mut state = state_with_commits();
        handle_key_commit_list(&mut state, KeyCode::Char(' ')); // select 0
        handle_key_commit_list(&mut state, KeyCode::Down); // hover 1
        handle_key_commit_list(&mut state, KeyCode::Char(' ')); // select 1
        handle_key_commit_list(&mut state, KeyCode::Enter);
        assert_eq!(state.execution_results.len(), 2);
        assert!(state
            .execution_results
            .iter()
            .all(|r| matches!(r.outcome, ExecutionOutcome::Pending)));
    }
}
