// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Terminal UI. Renders session projections, never provider payloads.

use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;

use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::CreateSession;
use blora_types::{Mode, Result, SessionId};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

pub fn run(runtime: &Runtime, workspace: &Path) -> Result<()> {
    let session_id = ensure_session(runtime, workspace)?;
    enable_raw_mode().map_err(blora_types::BloraError::exec)?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).map_err(blora_types::BloraError::exec)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(blora_types::BloraError::exec)?;
    let mut input = String::new();
    let mut status = "Enter send  Ctrl+N new session  Ctrl+C quit".to_owned();
    let result = loop {
        let projection = runtime.show_session(&session_id).ok();
        terminal
            .draw(|frame| {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(3),
                        Constraint::Min(4),
                        Constraint::Length(3),
                    ])
                    .split(frame.area());
                let title = projection
                    .as_ref()
                    .and_then(|p| p.session.as_ref())
                    .map(|s| format!("Blora Agent  {}  {}", s.id, s.workspace_path))
                    .unwrap_or_else(|| "Blora Agent".to_owned());
                frame.render_widget(
                    Paragraph::new(status.clone()).block(
                        Block::default()
                            .title(title)
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Rgb(159, 89, 100))),
                    ),
                    chunks[0],
                );
                let lines = projection
                    .as_ref()
                    .map(render_transcript)
                    .unwrap_or_default();
                frame.render_widget(
                    Paragraph::new(lines)
                        .wrap(Wrap { trim: false })
                        .block(Block::default().title("session").borders(Borders::ALL)),
                    chunks[1],
                );
                frame.render_widget(
                    Paragraph::new(input.as_str()).block(
                        Block::default()
                            .title("prompt")
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Rgb(159, 89, 100))),
                    ),
                    chunks[2],
                );
            })
            .map_err(blora_types::BloraError::exec)?;

        if event::poll(Duration::from_millis(200)).map_err(blora_types::BloraError::exec)? {
            match event::read().map_err(blora_types::BloraError::exec)? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('q'))
                    {
                        break Ok(());
                    }
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('n')
                    {
                        status = "session already active; restart to create another".to_owned();
                        continue;
                    }
                    match key.code {
                        KeyCode::Enter => {
                            if !input.trim().is_empty() {
                                let prompt = input.clone();
                                input.clear();
                                match runtime.run(
                                    &session_id,
                                    &prompt,
                                    &CancelToken::new(),
                                    &RunOptions {
                                        mock: std::env::var("BLORA_API_KEY").is_err()
                                            && std::env::var("OPENAI_API_KEY").is_err(),
                                        auto_approve: false,
                                        ..RunOptions::default()
                                    },
                                ) {
                                    Ok(_) => status = "completed".to_owned(),
                                    Err(err) => status = err.to_string(),
                                }
                            }
                        }
                        KeyCode::Backspace => {
                            input.pop();
                        }
                        KeyCode::Char(ch) => input.push(ch),
                        KeyCode::Esc => break Ok(()),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    };

    disable_raw_mode().ok();
    execute!(io::stdout(), LeaveAlternateScreen).ok();
    result
}

fn ensure_session(runtime: &Runtime, workspace: &Path) -> Result<SessionId> {
    if let Some(existing) = runtime.list_sessions()?.into_iter().next() {
        return Ok(existing.id);
    }
    runtime.create_session(CreateSession {
        title: Some("tui".to_owned()),
        workspace_path: workspace.display().to_string(),
        mode: Mode::Code,
        parent_session_id: None,
    })
}

fn render_transcript(projection: &blora_session::SessionProjection) -> Vec<Line<'static>> {
    projection
        .transcript
        .iter()
        .map(|item| match item {
            TranscriptItem::User { text, .. } => Line::from(vec![
                Span::styled(
                    "you  ",
                    Style::default()
                        .fg(Color::Rgb(159, 89, 100))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(text.clone()),
            ]),
            TranscriptItem::Assistant { text, .. } => Line::from(vec![
                Span::styled(
                    "blora  ",
                    Style::default()
                        .fg(Color::Rgb(91, 117, 107))
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(text.clone()),
            ]),
            TranscriptItem::Tool { name, status, .. } => Line::from(Span::styled(
                format!("tool {name} ({status})"),
                Style::default().fg(Color::DarkGray),
            )),
            TranscriptItem::System { summary, .. } => Line::from(Span::styled(
                summary.clone(),
                Style::default().fg(Color::DarkGray),
            )),
        })
        .collect()
}
