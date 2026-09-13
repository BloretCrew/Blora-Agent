// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Terminal UI. Renders session projections, never provider payloads.

use std::io::{self, stdout};
use std::path::Path;
use std::thread;
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
    let mut sessions = runtime.list_sessions()?;
    if sessions.is_empty() {
        let id = runtime.create_session(CreateSession {
            title: Some("tui".to_owned()),
            workspace_path: workspace.display().to_string(),
            mode: Mode::Code,
            parent_session_id: None,
        })?;
        sessions = runtime.list_sessions()?;
        let _ = id;
    }
    let mut index = 0usize;
    enable_raw_mode().map_err(blora_types::BloraError::exec)?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen).map_err(blora_types::BloraError::exec)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(blora_types::BloraError::exec)?;
    let mut input = String::new();
    let mut status =
        "Enter send  /help  [ ] session  Ctrl+N new  y/n approve  Ctrl+C quit".to_owned();
    let mut auto_approve = false;
    let mut scroll = 0usize;
    let mut search: Option<String> = None;
    let mut hide_tools = false;
    let mut cancel = CancelToken::new();
    let result = thread::scope(|scope| -> Result<()> {
        let mut job: Option<thread::ScopedJoinHandle<'_, Result<blora_types::RunId>>> = None;
        loop {
            refresh_sessions(runtime, &mut sessions, &mut index);
            let session_id = sessions.get(index).map(|item| item.id.clone());
            let projection = session_id
                .as_ref()
                .and_then(|id| runtime.show_session(id).ok());
            let pending = session_id
                .as_ref()
                .and_then(|id| runtime.pending_approvals(id).ok())
                .unwrap_or_default();
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
                        .map(|s| {
                            format!(
                                "Blora Agent  {}/{}  {}  tasks:{}  agents:{}",
                                index + 1,
                                sessions.len().max(1),
                                s.id,
                                projection.as_ref().map(|p| p.tasks.len()).unwrap_or(0),
                                projection.as_ref().map(|p| p.subagents.len()).unwrap_or(0)
                            )
                        })
                        .unwrap_or_else(|| "Blora Agent".to_owned());
                    let running_tasks = projection
                        .as_ref()
                        .map(|item| {
                            item.tasks
                                .iter()
                                .filter(|task| task.status.as_str() == "running")
                                .count()
                        })
                        .unwrap_or(0);
                    let banner = if let Some(first) = pending.first() {
                        format!("APPROVE {}  y/n  {}", first.id, first.summary)
                    } else if running_tasks > 0 {
                        format!("task running ({running_tasks})  {status}")
                    } else {
                        status.clone()
                    };
                    frame.render_widget(
                        Paragraph::new(banner).block(
                            Block::default()
                                .title(title)
                                .borders(Borders::ALL)
                                .border_style(Style::default().fg(Color::Rgb(159, 89, 100))),
                        ),
                        chunks[0],
                    );
                    let lines = projection
                        .as_ref()
                        .map(|projection| {
                            render_transcript(projection, scroll, search.as_deref(), hide_tools)
                        })
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

            if let Some(handle) =
                job.take_if(|handle: &mut thread::ScopedJoinHandle<'_, _>| handle.is_finished())
            {
                match handle.join() {
                    Ok(Ok(_)) => status = "completed".to_owned(),
                    Ok(Err(err)) => status = err.to_string(),
                    Err(_) => status = "run thread panicked".to_owned(),
                }
                cancel = CancelToken::new();
            }

            if event::poll(Duration::from_millis(200)).map_err(blora_types::BloraError::exec)? {
                match event::read().map_err(blora_types::BloraError::exec)? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && (key.code == KeyCode::Char('c') || key.code == KeyCode::Char('q'))
                        {
                            cancel.cancel();
                            break Ok(());
                        }
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('n')
                        {
                            match create_session(runtime, workspace) {
                                Ok(id) => {
                                    refresh_sessions(runtime, &mut sessions, &mut index);
                                    if let Some(found) =
                                        sessions.iter().position(|item| item.id == id)
                                    {
                                        index = found;
                                    }
                                    status = format!("session {id}");
                                }
                                Err(err) => status = err.to_string(),
                            }
                            continue;
                        }
                        match key.code {
                            KeyCode::Left | KeyCode::Char('[') if input.is_empty() => {
                                index = index.saturating_sub(1);
                            }
                            KeyCode::Right | KeyCode::Char(']') if input.is_empty() => {
                                if index + 1 < sessions.len() {
                                    index += 1;
                                }
                            }
                            KeyCode::PageUp if input.is_empty() => {
                                scroll = scroll.saturating_add(8);
                            }
                            KeyCode::PageDown if input.is_empty() => {
                                scroll = scroll.saturating_sub(8);
                            }
                            KeyCode::Home if input.is_empty() => scroll = usize::MAX / 4,
                            KeyCode::End if input.is_empty() => scroll = 0,
                            KeyCode::Char('y') if input.is_empty() && !pending.is_empty() => {
                                let _ = runtime.resolve_approval(&pending[0].id, true);
                            }
                            KeyCode::Char('n') if input.is_empty() && !pending.is_empty() => {
                                let _ = runtime.resolve_approval(&pending[0].id, false);
                            }
                            KeyCode::Enter => {
                                if input.starts_with('/') {
                                    let command = input.clone();
                                    input.clear();
                                    if let Some(id) = session_id.as_ref() {
                                        status = slash(
                                            runtime,
                                            &command,
                                            id,
                                            &mut auto_approve,
                                            &mut sessions,
                                            &mut index,
                                            workspace,
                                            &cancel,
                                            &mut search,
                                            &mut hide_tools,
                                        );
                                    }
                                } else if !input.trim().is_empty() {
                                    if job.is_some() {
                                        status = "a run is already in progress".to_owned();
                                        continue;
                                    }
                                    let Some(id) = session_id.clone() else {
                                        continue;
                                    };
                                    let prompt = input.clone();
                                    input.clear();
                                    status = "running…".to_owned();
                                    let options = RunOptions {
                                        mock: std::env::var("BLORA_API_KEY").is_err()
                                            && std::env::var("OPENAI_API_KEY").is_err(),
                                        auto_approve,
                                        interactive: true,
                                        ..RunOptions::default()
                                    };
                                    let cancel_clone = cancel.clone();
                                    job = Some(scope.spawn(move || {
                                        runtime.run(&id, &prompt, &cancel_clone, &options)
                                    }));
                                }
                            }
                            KeyCode::Backspace => {
                                input.pop();
                            }
                            KeyCode::Char(ch) => input.push(ch),
                            KeyCode::Esc => {
                                cancel.cancel();
                                break Ok(());
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    disable_raw_mode().ok();
    execute!(io::stdout(), LeaveAlternateScreen).ok();
    result
}

fn refresh_sessions(
    runtime: &Runtime,
    sessions: &mut Vec<blora_storage::SessionSummary>,
    index: &mut usize,
) {
    if let Ok(list) = runtime.list_sessions() {
        *sessions = list;
        if sessions.is_empty() {
            *index = 0;
        } else if *index >= sessions.len() {
            *index = sessions.len() - 1;
        }
    }
}

fn create_session(runtime: &Runtime, workspace: &Path) -> Result<SessionId> {
    runtime.create_session(CreateSession {
        title: Some("tui".to_owned()),
        workspace_path: workspace.display().to_string(),
        mode: Mode::Code,
        parent_session_id: None,
    })
}

#[allow(clippy::too_many_arguments)]
fn slash(
    runtime: &Runtime,
    command: &str,
    session_id: &SessionId,
    auto_approve: &mut bool,
    sessions: &mut Vec<blora_storage::SessionSummary>,
    index: &mut usize,
    workspace: &Path,
    cancel: &CancelToken,
    search: &mut Option<String>,
    hide_tools: &mut bool,
) -> String {
    let cmd = command.trim();
    if let Some(query) = cmd.strip_prefix("/search") {
        let query = query.trim();
        if query.is_empty() {
            *search = None;
            return "search cleared".to_owned();
        }
        *search = Some(query.to_owned());
        return format!("search {query}");
    }
    match cmd {
        "/help" => {
            "/new /compact /tasks /yes /model /cancel /fork /resume /export /permissions /mode /search /tools"
                .to_owned()
        }
        "/tools" => {
            *hide_tools = !*hide_tools;
            format!("hide-tools={hide_tools}")
        }
        "/new" => match create_session(runtime, workspace) {
            Ok(id) => {
                refresh_sessions(runtime, sessions, index);
                if let Some(found) = sessions.iter().position(|item| item.id == id) {
                    *index = found;
                }
                format!("created {id}")
            }
            Err(err) => err.to_string(),
        },
        "/compact" => runtime
            .compact(session_id)
            .map(|_| "compacted".to_owned())
            .unwrap_or_else(|err| err.to_string()),
        "/tasks" => runtime
            .list_tasks(Some(session_id))
            .map(|tasks| {
                if tasks.is_empty() {
                    "no tasks".to_owned()
                } else {
                    tasks
                        .into_iter()
                        .map(|task| format!("{} {}", task.status.as_str(), task.title))
                        .collect::<Vec<_>>()
                        .join(" · ")
                }
            })
            .unwrap_or_else(|err| err.to_string()),
        "/yes" => {
            *auto_approve = !*auto_approve;
            format!("auto-approve={}", *auto_approve)
        }
        "/model" => {
            std::env::var("BLORA_MODEL").unwrap_or_else(|_| "mock or gpt-4o-mini".to_owned())
        }
        "/cancel" => {
            cancel.cancel();
            "cancel requested".to_owned()
        }
        "/fork" => runtime
            .fork_session(session_id)
            .map(|id| {
                refresh_sessions(runtime, sessions, index);
                if let Some(found) = sessions.iter().position(|item| item.id == id) {
                    *index = found;
                }
                format!("forked {id}")
            })
            .unwrap_or_else(|err| err.to_string()),
        "/resume" => runtime
            .resume_session(session_id)
            .map(|()| "resumed".to_owned())
            .unwrap_or_else(|err| err.to_string()),
        "/export" => runtime
            .export_session(session_id)
            .map(|value| {
                format!(
                    "exported {} events",
                    value.as_array().map(Vec::len).unwrap_or(0)
                )
            })
            .unwrap_or_else(|err| err.to_string()),
        "/permissions" => format!("auto-approve={auto_approve}"),
        "/mode" => runtime
            .show_session(session_id)
            .ok()
            .and_then(|projection| projection.session)
            .map(|session| session.mode.as_str().to_owned())
            .unwrap_or_else(|| "unknown".to_owned()),
        _ => format!("unknown command {cmd}"),
    }
}

fn render_transcript(
    projection: &blora_session::SessionProjection,
    scroll: usize,
    search: Option<&str>,
    hide_tools: bool,
) -> Vec<Line<'static>> {
    let needle = search.map(str::to_ascii_lowercase);
    let filtered: Vec<_> = projection
        .transcript
        .iter()
        .filter(|item| {
            if hide_tools {
                !matches!(item, TranscriptItem::Tool { .. })
            } else {
                true
            }
        })
        .filter(|item| {
            let Some(needle) = needle.as_deref() else {
                return true;
            };
            match item {
                TranscriptItem::User { text, .. } | TranscriptItem::Assistant { text, .. } => {
                    text.to_ascii_lowercase().contains(needle)
                }
                TranscriptItem::Tool { name, .. } => name.to_ascii_lowercase().contains(needle),
                TranscriptItem::System { summary, .. } => {
                    summary.to_ascii_lowercase().contains(needle)
                }
            }
        })
        .collect();
    let total = filtered.len();
    let skip = scroll.min(total.saturating_sub(1));
    filtered
        .into_iter()
        .rev()
        .skip(skip)
        .take(80)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
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
