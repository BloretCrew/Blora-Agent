// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Terminal UI. Renders session projections, never provider payloads.

mod slash;
mod theme;
mod view;

use std::io::{self, Write, stdout};
use std::path::Path;
use std::thread;
use std::time::Duration;

use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::{CreateSession, CreateTask};
use blora_types::{Mode, Result, SessionId, TaskId};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

fn start_passport_login() -> Result<
    Option<(
        blora_auth::DeviceCode,
        std::sync::mpsc::Receiver<blora_auth::PassportUser>,
    )>,
> {
    let config = match blora_auth::PassportConfig::from_env() {
        Ok(config) => config,
        Err(_) => return Ok(None),
    };
    let device = config
        .request_device_code()
        .map_err(|err| blora_types::BloraError::Other(err.to_string()))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let polling_device = device.clone();
    std::thread::spawn(move || {
        if let Ok(user) = config.poll_device(&polling_device) {
            let _ = sender.send(user);
        }
    });
    Ok(Some((device, receiver)))
}

pub fn run(runtime: &Runtime, workspace: &Path) -> Result<()> {
    let mut passport_url = None;
    let mut passport_receiver = None;
    let mut passport_browser_opened = false;
    // Pending device-flow login, shown as a centered dialog; hidden with Esc.
    let mut passport_dialog: Option<view::PassportDialog> = None;
    // Model-provider switch dialog opened via `/provider`.
    let mut provider_dialog: Option<view::ProviderDialog> = None;
    let mut theme_dialog: Option<view::ThemeDialog> = None;
    // PassPort user token of the logged-in user; drives the default provider.
    let mut passport_user_token: Option<String> = None;
    if let Ok(users) = runtime.list_users() {
        passport_user_token = users
            .iter()
            .find_map(|user| user.passport_app_token.clone())
            .filter(|token| !token.trim().is_empty());
        if !users.iter().any(|user| user.passport_username.is_some()) {
            if let Some((device, receiver)) = start_passport_login()? {
                eprintln!("需要登录 Bloret PassPort");
                eprintln!("设备码：{}", device.user_code);
                eprintln!("登录链接：{}", device.verification_uri);
                passport_url = Some(device.verification_uri.clone());
                passport_receiver = Some(receiver);
                passport_dialog = Some(view::PassportDialog {
                    user_code: device.user_code.clone(),
                    verification_uri: device.verification_uri.clone(),
                    opened_browser: false,
                });
            }
        }
    }
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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
        .map_err(blora_types::BloraError::exec)?;
    if std::env::var_os("NO_COLOR").is_none() {
        let _ = write!(stdout, "{}", theme::Theme::current().cursor_osc());
        let _ = stdout.flush();
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(blora_types::BloraError::exec)?;
    let mut input = String::new();
    let mut status = String::new();
    let mut auto_approve = false;
    let mut scroll = 0usize;
    let mut search: Option<String> = None;
    let mut hide_tools = false;
    let mut slash_selected = 0usize;
    let mut last_slash_token = String::new();
    let mut model_override = String::new();
    let mut provider_override = String::new();
    let mut notice: Option<String> = None;
    let mut tick = 0u64;
    let mut pointer: Option<(u16, u16)> = None;
    let mut hits = view::HitMap::default();
    let mut cancel = CancelToken::new();
    let mut cached: Option<(SessionId, blora_session::SessionProjection)> = None;
    let result = thread::scope(|scope| -> Result<()> {
        let mut job: Option<thread::ScopedJoinHandle<'_, Result<blora_types::RunId>>> = None;
        loop {
            refresh_sessions(runtime, &mut sessions, &mut index);
            if let Some(receiver) = passport_receiver.as_ref()
                && let Ok(user) = receiver.try_recv()
            {
                runtime.upsert_passport_user(
                    &user.username,
                    user.nickname.as_deref(),
                    user.avatar.as_deref(),
                    user.email.as_deref(),
                    user.apptoken.as_deref(),
                )?;
                passport_user_token = user
                    .apptoken
                    .clone()
                    .filter(|token| !token.trim().is_empty());
                passport_receiver = None;
                passport_dialog = None;
                status = format!("PassPort 登录成功：{}", user.display_name());
            }
            let session_id = sessions.get(index).map(|item| item.id.clone());
            // Keep one projection per selected session and apply only new events.
            if let Some(id) = session_id.as_ref() {
                if cached.as_ref().map(|(cached_id, _)| cached_id) != Some(id) {
                    cached = Some((id.clone(), blora_session::SessionProjection::new()));
                }
                if let Some((_, projection)) = cached.as_mut()
                    && runtime.refresh_projection(id, projection).is_err()
                {
                    cached = None;
                }
            } else {
                cached = None;
            }
            let projection = cached.as_ref().map(|(_, projection)| projection);
            let pending = session_id
                .as_ref()
                .and_then(|id| runtime.pending_approvals(id).ok())
                .unwrap_or_default();
            let slash_hits = slash::matches(&input);
            if slash::command_token(&input).unwrap_or("") != last_slash_token {
                last_slash_token = slash::command_token(&input).unwrap_or("").to_owned();
                slash_selected = 0;
            }
            if !slash_hits.is_empty() {
                slash_selected = slash_selected.min(slash_hits.len() - 1);
            }
            tick = tick.wrapping_add(1);
            let env_model = std::env::var("BLORA_MODEL").unwrap_or_default();
            let env_provider = std::env::var("BLORA_PROVIDER").unwrap_or_default();
            let logged_in = passport_user_token.is_some();
            let provider_option_list = provider_options(&provider_override, logged_in);
            let model = if model_override.is_empty() {
                env_model.as_str()
            } else {
                model_override.as_str()
            };
            // Logged-in PassPort users run on the PassPort provider by default.
            let provider = if provider_override.is_empty() {
                if env_provider.is_empty() && passport_user_token.is_some() {
                    "Blora"
                } else {
                    env_provider.as_str()
                }
            } else {
                provider_override.as_str()
            };
            terminal
                .draw(|frame| {
                    hits = view::draw(
                        frame,
                        &view::FrameModel {
                            workspace,
                            sessions: &sessions,
                            index,
                            projection,
                            pending: &pending,
                            input: &input,
                            status: if let Some(url) = passport_url.as_deref()
                                && passport_receiver.is_some()
                            {
                                url
                            } else {
                                &status
                            },
                            notice: notice.as_deref(),
                            passport_dialog: passport_dialog.as_ref(),
                            provider_dialog: provider_dialog.as_ref(),
                            theme_dialog: theme_dialog.as_ref(),
                            slash_hits: &slash_hits,
                            slash_selected,
                            search: search.as_deref(),
                            hide_tools,
                            scroll,
                            auto_approve,
                            model,
                            provider,
                            running: job.is_some(),
                            tick,
                            pointer,
                        },
                    );
                })
                .map_err(blora_types::BloraError::exec)?;
            if !passport_browser_opened
                && passport_receiver.is_some()
                && let Some(url) = passport_url.as_deref()
            {
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                let _ = std::process::Command::new("open").arg(url).spawn();
                passport_browser_opened = true;
                if let Some(dialog) = passport_dialog.as_mut() {
                    dialog.opened_browser = true;
                }
            }

            if let Some(handle) =
                job.take_if(|handle: &mut thread::ScopedJoinHandle<'_, _>| handle.is_finished())
            {
                match handle.join() {
                    Ok(Ok(_)) => status = "done".to_owned(),
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
                            match create_session(runtime, workspace, blora_types::Mode::Code, None)
                            {
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
                        if key.modifiers.contains(KeyModifiers::CONTROL)
                            && key.code == KeyCode::Char('p')
                            && input.is_empty()
                        {
                            input.push('/');
                            slash_selected = 0;
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
                            KeyCode::Up if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.as_mut() {
                                    dialog.selected = dialog.selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.as_mut() {
                                    dialog.selected =
                                        (dialog.selected + 1).min(dialog.options.len() - 1);
                                }
                            }
                            KeyCode::Enter if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.take() {
                                    if let Some(option) = dialog.options.get(dialog.selected) {
                                        status = apply_theme_id(&option.id);
                                    }
                                }
                            }
                            KeyCode::Up if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut() {
                                    dialog.selected = dialog.selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut() {
                                    dialog.selected =
                                        (dialog.selected + 1).min(dialog.options.len() - 1);
                                }
                            }
                            KeyCode::Enter if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.take() {
                                    if let Some(option) = dialog.options.get(dialog.selected) {
                                        provider_override = option.id.clone();
                                        status = if option.id.is_empty() {
                                            format!("provider=默认（跟随登录与环境）")
                                        } else {
                                            format!("provider={}", option.display)
                                        };
                                    }
                                }
                            }
                            KeyCode::Up if slash::is_open(&input) => {
                                slash_selected = slash_selected.saturating_sub(1);
                            }
                            KeyCode::Down if slash::is_open(&input) => {
                                if !slash_hits.is_empty() {
                                    slash_selected = (slash_selected + 1).min(slash_hits.len() - 1);
                                }
                            }
                            KeyCode::Tab if slash::is_open(&input) => {
                                if let Some(cmd) = slash_hits.get(slash_selected) {
                                    input = slash::complete(cmd);
                                    slash_selected = 0;
                                }
                            }
                            KeyCode::Enter => {
                                if input.starts_with('/') {
                                    if slash::is_open(&input) {
                                        if let Some(cmd) = slash_hits.get(slash_selected) {
                                            if slash::needs_args(cmd) {
                                                input = slash::complete(cmd);
                                                slash_selected = 0;
                                                continue;
                                            }
                                        }
                                    }
                                    let mut command = input.clone();
                                    if slash::is_open(&command) {
                                        if let Some(cmd) = slash_hits.get(slash_selected) {
                                            command = format!("/{}", cmd.name);
                                        }
                                    }
                                    input.clear();
                                    slash_selected = 0;
                                    if let Some(id) = session_id.as_ref() {
                                        match slash(
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
                                            &mut model_override,
                                            &mut provider_override,
                                        ) {
                                            SlashOutcome::Quit => {
                                                cancel.cancel();
                                                break Ok(());
                                            }
                                            SlashOutcome::Status(text) => {
                                                notice = None;
                                                status = text;
                                            }
                                            SlashOutcome::Panel { status: text, body } => {
                                                status = text;
                                                notice = Some(body);
                                            }
                                            SlashOutcome::Login { url, receiver } => {
                                                // Surface the device code as a centered
                                                // dialog; the URL also stays in the status.
                                                passport_dialog =
                                                    url_parts(&url).map(|(uri, code)| {
                                                        view::PassportDialog {
                                                            user_code: code,
                                                            verification_uri: uri,
                                                            opened_browser: false,
                                                        }
                                                    });
                                                status = url;
                                                passport_receiver = Some(receiver);
                                                passport_browser_opened = false;
                                            }
                                            SlashOutcome::ProviderDialog => {
                                                theme_dialog = None;
                                                provider_dialog = Some(view::ProviderDialog {
                                                    options: provider_option_list.clone(),
                                                    selected: provider_option_list
                                                        .iter()
                                                        .position(|option| !option.id.is_empty())
                                                        .unwrap_or(0),
                                                    fullscreen: false,
                                                    minimized: false,
                                                });
                                            }
                                            SlashOutcome::ThemeDialog => {
                                                provider_dialog = None;
                                                theme_dialog = Some(open_theme_dialog());
                                            }
                                        }
                                    }
                                } else if !input.trim().is_empty() {
                                    let Some(id) = session_id.clone() else {
                                        continue;
                                    };
                                    if job.is_some() {
                                        let text = input.clone();
                                        input.clear();
                                        status = match runtime.queue_steer(&id, &text) {
                                            Ok(()) => "已排队插话，将在下一轮送达".to_owned(),
                                            Err(err) => err.to_string(),
                                        };
                                        continue;
                                    }
                                    let prompt = input.clone();
                                    input.clear();
                                    notice = None;
                                    status = "running…".to_owned();
                                    let options = RunOptions {
                                        mock: passport_user_token.is_none()
                                            && std::env::var("BLORA_API_KEY").is_err()
                                            && std::env::var("OPENAI_API_KEY").is_err()
                                            && std::env::var("GEMINI_API_KEY").is_err(),
                                        auto_approve,
                                        interactive: true,
                                        model: model_override.clone(),
                                        provider: provider_override.clone(),
                                        passport_user_token: passport_user_token.clone(),
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
                            KeyCode::Esc if slash::is_open(&input) || input.starts_with('/') => {
                                input.clear();
                                slash_selected = 0;
                            }
                            // The login dialog is only hidden: polling keeps
                            // running so completing authorization still lands.
                            KeyCode::Esc if passport_dialog.is_some() => {
                                passport_dialog = None;
                            }
                            KeyCode::Esc if theme_dialog.is_some() => {
                                theme_dialog = None;
                            }
                            KeyCode::Esc if provider_dialog.is_some() => {
                                provider_dialog = None;
                            }
                            KeyCode::Esc if notice.is_some() => {
                                notice = None;
                            }
                            KeyCode::Esc => {
                                cancel.cancel();
                                break Ok(());
                            }
                            _ => {}
                        }
                    }
                    Event::Mouse(mouse) => {
                        pointer = Some((mouse.column, mouse.row));
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                if hits.over_slash(mouse.column, mouse.row) {
                                    slash_selected = slash_selected.saturating_sub(1);
                                } else {
                                    scroll = scroll.saturating_add(3);
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                if hits.over_slash(mouse.column, mouse.row) {
                                    if !slash_hits.is_empty() {
                                        slash_selected =
                                            (slash_selected + 1).min(slash_hits.len() - 1);
                                    }
                                } else {
                                    scroll = scroll.saturating_sub(3);
                                }
                            }
                            MouseEventKind::Moved => match hits.hit(mouse.column, mouse.row) {
                                Some(view::Hit::Slash(idx)) => {
                                    slash_selected = idx;
                                }
                                Some(view::Hit::ProviderRow(idx)) => {
                                    if let Some(dialog) = provider_dialog.as_mut() {
                                        dialog.selected = idx;
                                    }
                                }
                                Some(view::Hit::ThemeRow(idx)) => {
                                    if let Some(dialog) = theme_dialog.as_mut() {
                                        dialog.selected = idx;
                                    }
                                }
                                _ => {}
                            },
                            MouseEventKind::Down(MouseButton::Left) => {
                                let hit = hits.hit(mouse.column, mouse.row);
                                // Provider dialog rows: click selects and confirms.
                                if let Some(view::Hit::ThemeRow(idx)) = hit {
                                    if let Some(dialog) = theme_dialog.take() {
                                        if let Some(option) = dialog.options.get(idx) {
                                            status = apply_theme_id(&option.id);
                                        }
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::ProviderRow(idx)) = hit {
                                    if let Some(dialog) = provider_dialog.take() {
                                        if let Some(option) = dialog.options.get(idx) {
                                            provider_override = option.id.clone();
                                            status = if option.id.is_empty() {
                                                format!("provider=默认（跟随登录与环境）")
                                            } else {
                                                format!("provider={}", option.display)
                                            };
                                        }
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::Slash(idx)) = hit {
                                    slash_selected = idx;
                                    if let Some(cmd) = slash_hits.get(idx) {
                                        if slash::needs_args(cmd) {
                                            input = slash::complete(cmd);
                                            slash_selected = 0;
                                        } else if let Some(id) = session_id.as_ref() {
                                            let command = format!("/{}", cmd.name);
                                            input.clear();
                                            slash_selected = 0;
                                            if apply_slash(
                                                slash(
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
                                                    &mut model_override,
                                                    &mut provider_override,
                                                ),
                                                &cancel,
                                                &mut notice,
                                                &mut status,
                                                &mut provider_dialog,
                                                &provider_option_list,
                                                &mut theme_dialog,
                                            ) {
                                                break Ok(());
                                            }
                                        }
                                    }
                                } else if matches!(
                                    hit,
                                    Some(
                                        view::Hit::Allow | view::Hit::Hint(view::HintAction::Allow)
                                    )
                                ) {
                                    if let Some(first) = pending.first() {
                                        let _ = runtime.resolve_approval(&first.id, true);
                                    }
                                } else if matches!(
                                    hit,
                                    Some(view::Hit::Deny | view::Hit::Hint(view::HintAction::Deny))
                                ) {
                                    if let Some(first) = pending.first() {
                                        let _ = runtime.resolve_approval(&first.id, false);
                                    }
                                } else if hit == Some(view::Hit::PrevSession) {
                                    index = index.saturating_sub(1);
                                } else if matches!(
                                    hit,
                                    Some(
                                        view::Hit::NextSession
                                            | view::Hit::Hint(view::HintAction::SessionNext)
                                    )
                                ) {
                                    if index + 1 < sessions.len() {
                                        index += 1;
                                    }
                                } else if hit == Some(view::Hit::CancelRun) {
                                    cancel.cancel();
                                    status = "cancel requested".to_owned();
                                } else if hit == Some(view::Hit::ToggleApprove) {
                                    auto_approve = !auto_approve;
                                    status = format!("auto-approve={auto_approve}");
                                } else if hit == Some(view::Hit::TrafficClose) {
                                    // Red: close the front dialog. Login still
                                    // quits the TUI (the dialog is the session).
                                    if theme_dialog.take().is_some() {
                                        status = "theme picker closed".to_owned();
                                    } else if provider_dialog.take().is_some() {
                                        status = "provider picker closed".to_owned();
                                    } else {
                                        cancel.cancel();
                                        break Ok(());
                                    }
                                } else if hit == Some(view::Hit::TrafficMinimize) {
                                    // Yellow: collapse pickers to a title bar;
                                    // hide the login dialog while polling continues.
                                    if let Some(dialog) = theme_dialog.as_mut() {
                                        dialog.minimized = true;
                                        dialog.fullscreen = false;
                                    } else if let Some(dialog) = provider_dialog.as_mut() {
                                        dialog.minimized = true;
                                        dialog.fullscreen = false;
                                    } else {
                                        passport_dialog = None;
                                    }
                                } else if hit == Some(view::Hit::TrafficOpenBrowser) {
                                    // Green: fullscreen the picker, or restore
                                    // from minimized. Login still opens the browser.
                                    if let Some(dialog) = theme_dialog.as_mut() {
                                        if dialog.minimized {
                                            dialog.minimized = false;
                                        } else {
                                            dialog.fullscreen = !dialog.fullscreen;
                                        }
                                    } else if let Some(dialog) = provider_dialog.as_mut() {
                                        if dialog.minimized {
                                            dialog.minimized = false;
                                        } else {
                                            dialog.fullscreen = !dialog.fullscreen;
                                        }
                                    } else if let Some(url) = passport_url.as_deref() {
                                        let _ =
                                            std::process::Command::new("xdg-open").arg(url).spawn();
                                        let _ = std::process::Command::new("open").arg(url).spawn();
                                        if let Some(dialog) = passport_dialog.as_mut() {
                                            dialog.opened_browser = true;
                                        }
                                        status = format!("已打开 {url}");
                                    }
                                } else if hit == Some(view::Hit::Notice)
                                    || hit == Some(view::Hit::Hint(view::HintAction::Close))
                                {
                                    notice = None;
                                    if slash::is_open(&input) {
                                        input.clear();
                                        slash_selected = 0;
                                    }
                                } else if hit == Some(view::Hit::Hint(view::HintAction::Commands)) {
                                    if input.is_empty() {
                                        input.push('/');
                                        slash_selected = 0;
                                    }
                                } else if hit == Some(view::Hit::Hint(view::HintAction::New)) {
                                    match create_session(
                                        runtime,
                                        workspace,
                                        blora_types::Mode::Code,
                                        None,
                                    ) {
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
                                } else if hit == Some(view::Hit::Hint(view::HintAction::Complete)) {
                                    if let Some(cmd) = slash_hits.get(slash_selected) {
                                        input = slash::complete(cmd);
                                        slash_selected = 0;
                                    }
                                } else if hit == Some(view::Hit::Hint(view::HintAction::Run))
                                    || hit == Some(view::Hit::Hint(view::HintAction::Send))
                                {
                                    // Reuse Enter: inject a synthetic submit by falling through
                                    // to the same slash/send paths via a small helper.
                                    if input.starts_with('/') {
                                        if slash::is_open(&input) {
                                            if let Some(cmd) = slash_hits.get(slash_selected) {
                                                if slash::needs_args(cmd) {
                                                    input = slash::complete(cmd);
                                                    slash_selected = 0;
                                                    continue;
                                                }
                                            }
                                        }
                                        let mut command = input.clone();
                                        if slash::is_open(&command) {
                                            if let Some(cmd) = slash_hits.get(slash_selected) {
                                                command = format!("/{}", cmd.name);
                                            }
                                        }
                                        input.clear();
                                        slash_selected = 0;
                                        if let Some(id) = session_id.as_ref() {
                                            if apply_slash(
                                                slash(
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
                                                    &mut model_override,
                                                    &mut provider_override,
                                                ),
                                                &cancel,
                                                &mut notice,
                                                &mut status,
                                                &mut provider_dialog,
                                                &provider_option_list,
                                                &mut theme_dialog,
                                            ) {
                                                break Ok(());
                                            }
                                        }
                                    } else if !input.trim().is_empty() {
                                        let Some(id) = session_id.clone() else {
                                            continue;
                                        };
                                        if job.is_some() {
                                            let text = input.clone();
                                            input.clear();
                                            status = match runtime.queue_steer(&id, &text) {
                                                Ok(()) => "已排队插话，将在下一轮送达".to_owned(),
                                                Err(err) => err.to_string(),
                                            };
                                            continue;
                                        }
                                        let prompt = input.clone();
                                        input.clear();
                                        notice = None;
                                        status = "running…".to_owned();
                                        let options = RunOptions {
                                            mock: passport_user_token.is_none()
                                                && std::env::var("BLORA_API_KEY").is_err()
                                                && std::env::var("OPENAI_API_KEY").is_err()
                                                && std::env::var("GEMINI_API_KEY").is_err(),
                                            auto_approve,
                                            interactive: true,
                                            model: model_override.clone(),
                                            provider: provider_override.clone(),
                                            passport_user_token: passport_user_token.clone(),
                                            ..RunOptions::default()
                                        };
                                        let cancel_clone = cancel.clone();
                                        job = Some(scope.spawn(move || {
                                            runtime.run(&id, &prompt, &cancel_clone, &options)
                                        }));
                                    }
                                } else if hit == Some(view::Hit::Hint(view::HintAction::Quit)) {
                                    cancel.cancel();
                                    break Ok(());
                                } else if slash::is_open(&input)
                                    && !matches!(hit, Some(view::Hit::Composer))
                                {
                                    input.clear();
                                    slash_selected = 0;
                                } else if notice.is_some()
                                    && !matches!(hit, Some(view::Hit::Composer))
                                {
                                    notice = None;
                                }
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
    execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen).ok();
    if std::env::var_os("NO_COLOR").is_none() {
        let _ = write!(io::stdout(), "{}", theme::CURSOR_RESET);
        let _ = io::stdout().flush();
    }
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

fn create_session(
    runtime: &Runtime,
    workspace: &Path,
    mode: Mode,
    title: Option<&str>,
) -> Result<SessionId> {
    runtime.create_session(CreateSession {
        title: Some(title.unwrap_or("tui").to_owned()),
        workspace_path: workspace.display().to_string(),
        mode,
        parent_session_id: None,
    })
}

enum SlashOutcome {
    Status(String),
    Panel {
        status: String,
        body: String,
    },
    Login {
        url: String,
        receiver: std::sync::mpsc::Receiver<blora_auth::PassportUser>,
    },
    /// Open the model-provider switch dialog.
    ProviderDialog,
    /// Open the color-scheme picker.
    ThemeDialog,
    Quit,
}

fn apply_slash(
    outcome: SlashOutcome,
    cancel: &CancelToken,
    notice: &mut Option<String>,
    status: &mut String,
    provider_dialog: &mut Option<view::ProviderDialog>,
    provider_options: &[view::ProviderOption],
    theme_dialog: &mut Option<view::ThemeDialog>,
) -> bool {
    match outcome {
        SlashOutcome::Quit => {
            cancel.cancel();
            true
        }
        SlashOutcome::Status(text) => {
            *notice = None;
            *status = text;
            false
        }
        SlashOutcome::Panel { status: text, body } => {
            *status = text;
            *notice = Some(body);
            false
        }
        SlashOutcome::Login { .. } => {
            *status = "PassPort 登录已启动".to_owned();
            false
        }
        SlashOutcome::ProviderDialog => {
            *theme_dialog = None;
            let selected = provider_options
                .iter()
                .position(|option| !option.id.is_empty());
            *provider_dialog = Some(view::ProviderDialog {
                options: provider_options.to_vec(),
                selected: selected.unwrap_or(0),
                fullscreen: false,
                minimized: false,
            });
            false
        }
        SlashOutcome::ThemeDialog => {
            *provider_dialog = None;
            *theme_dialog = Some(open_theme_dialog());
            false
        }
    }
}

/// Build the provider list for the switch dialog. `provider_override` and the
/// login/API-key state decide which entries are marked available.
fn provider_options(provider_override: &str, logged_in: bool) -> Vec<view::ProviderOption> {
    let env_provider = std::env::var("BLORA_PROVIDER").unwrap_or_default();
    let default_display = if logged_in {
        "Blora"
    } else if env_provider.is_empty() {
        "OpenAI"
    } else {
        env_provider.trim()
    };
    let override_id = provider_override.trim().to_owned();
    let has_key = std::env::var("BLORA_API_KEY").is_ok()
        || std::env::var("OPENAI_API_KEY").is_ok()
        || std::env::var("ANTHROPIC_API_KEY").is_ok()
        || std::env::var("GEMINI_API_KEY").is_ok();
    let current = |id: &str| {
        if override_id.is_empty() {
            id.is_empty()
        } else {
            override_id.eq_ignore_ascii_case(id)
        }
    };
    let entries: Vec<view::ProviderOption> = vec![
        view::ProviderOption {
            id: String::new(),
            display: format!("默认（{default_display}）"),
            hint: "跟随登录与环境配置".to_owned(),
            available: true,
        },
        view::ProviderOption {
            id: "blora".to_owned(),
            display: "Blora".to_owned(),
            hint: "Bloret PassPort · 200 次/天".to_owned(),
            available: logged_in,
        },
        view::ProviderOption {
            id: "openai".to_owned(),
            display: "OpenAI".to_owned(),
            hint: "BLORA_API_KEY / OPENAI_API_KEY".to_owned(),
            available: has_key,
        },
        view::ProviderOption {
            id: "anthropic".to_owned(),
            display: "Anthropic".to_owned(),
            hint: "ANTHROPIC_API_KEY".to_owned(),
            available: std::env::var("ANTHROPIC_API_KEY").is_ok(),
        },
        view::ProviderOption {
            id: "gemini".to_owned(),
            display: "Gemini".to_owned(),
            hint: "GEMINI_API_KEY".to_owned(),
            available: std::env::var("GEMINI_API_KEY").is_ok(),
        },
    ];
    entries
        .into_iter()
        .map(|mut option| {
            if current(&option.id) {
                option.hint = format!("当前 · {}", option.hint);
            }
            option
        })
        .collect()
}

/// Split the `/login` status text (`<url>\n设备码：<code>`) back into its URL
/// and device code so the dialog can be rebuilt from the slash outcome.
fn url_parts(text: &str) -> Option<(String, String)> {
    let mut lines = text.lines();
    let url = lines.next()?.trim().to_owned();
    if url.is_empty() {
        return None;
    }
    let code = lines
        .next()
        .and_then(|line| line.split_once('：'))
        .map(|(_, code)| code.trim().to_owned())
        .unwrap_or_default();
    Some((url, code))
}

fn clip_text(text: &str, max_lines: usize) -> String {
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        lines.push("…");
    }
    lines.join("\n")
}

fn panel(status: impl Into<String>, body: impl Into<String>) -> SlashOutcome {
    SlashOutcome::Panel {
        status: status.into(),
        body: clip_text(&body.into(), 40),
    }
}

fn workspace_key(workspace: &Path) -> String {
    workspace.display().to_string()
}

fn apply_theme(args: &str) -> SlashOutcome {
    if args.is_empty() {
        return SlashOutcome::ThemeDialog;
    }
    match theme::ThemePref::parse(args) {
        Some(pref) => SlashOutcome::Status(apply_theme_pref(pref)),
        None => SlashOutcome::Status(
            "usage: /theme [coral|indigo|graphite|mono|circuit|dusk|dark|light|auto]"
                .to_owned(),
        ),
    }
}

fn open_theme_dialog() -> view::ThemeDialog {
    let current = theme::palette();
    let options = theme::Palette::ALL
        .into_iter()
        .map(|palette| {
            let hint = if palette == current {
                format!("当前 · {}", palette.about_zh())
            } else {
                palette.about_zh().to_owned()
            };
            view::ThemeOption {
                id: palette.as_str().to_owned(),
                display: palette.name_zh().to_owned(),
                hint,
                swatches: palette.swatches().to_vec(),
            }
        })
        .collect::<Vec<_>>();
    let selected = options
        .iter()
        .position(|option| option.id == current.as_str())
        .unwrap_or(0);
    view::ThemeDialog {
        options,
        selected,
        fullscreen: false,
        minimized: false,
    }
}

fn apply_theme_id(id: &str) -> String {
    theme::ThemePref::parse(id)
        .map(apply_theme_pref)
        .unwrap_or_else(|| format!("unknown theme {id}"))
}

fn apply_theme_pref(pref: theme::ThemePref) -> String {
    theme::set_pref(pref);
    write_cursor();
    let resolved = if theme::Theme::current().is_dark() {
        "dark"
    } else if theme::scheme() == theme::Scheme::Plain {
        "plain"
    } else {
        "light"
    };
    format!(
        "theme={} scheme={} (resolved {resolved})",
        pref.palette.as_str(),
        pref.scheme.as_str()
    )
}

fn write_cursor() {
    let _ = write!(io::stdout(), "{}", theme::Theme::current().cursor_osc());
    let _ = io::stdout().flush();
}

fn split_slash(input: &str) -> (&str, &str) {
    let rest = input.trim().trim_start_matches('/');
    match rest.split_once(char::is_whitespace) {
        Some((name, args)) => (name, args.trim()),
        None => (rest, ""),
    }
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
    model: &mut String,
    provider: &mut String,
) -> SlashOutcome {
    let (raw, args) = split_slash(command);
    let Some(spec) = slash::resolve(raw) else {
        return SlashOutcome::Status(format!("unknown command /{raw}  try /help"));
    };
    match spec.name {
        "help" => panel("help", slash::help_text(args)),
        "login" => match start_passport_login() {
            Ok(Some((device, receiver))) => SlashOutcome::Login {
                url: format!("{}\n设备码：{}", device.verification_uri, device.user_code),
                receiver,
            },
            Ok(None) => SlashOutcome::Status("Passport 未配置".to_owned()),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "logout" => SlashOutcome::Status(
            runtime
                .clear_passport_users()
                .map(|count| format!("已退出 Passport（清除 {count} 个本地用户）"))
                .unwrap_or_else(|err| err.to_string()),
        ),
        "keymap" => panel("keymap", slash::keymap_text()),
        "theme" => apply_theme(args),
        "new" => open_session(runtime, workspace, sessions, index, Mode::Code, "tui"),
        "sessions" => panel("sessions", list_session_lines(sessions)),
        "goto" => goto_session(sessions, index, args),
        "status" => SlashOutcome::Status(session_status(runtime, session_id)),
        "id" => SlashOutcome::Status(session_id.to_string()),
        "context" => SlashOutcome::Status(
            runtime
                .show_session(session_id)
                .map(|projection| {
                    format!(
                        "seq={}  in={}  out={}  ws={}",
                        projection.last_sequence,
                        projection.input_tokens,
                        projection.output_tokens,
                        projection
                            .session
                            .as_ref()
                            .map(|session| session.workspace_path.as_str())
                            .unwrap_or("-")
                    )
                })
                .unwrap_or_else(|err| err.to_string()),
        ),
        "compact" => SlashOutcome::Status(
            runtime
                .compact(session_id)
                .map(|_| {
                    if args.is_empty() {
                        "compacted".to_owned()
                    } else {
                        format!("compacted ({args})")
                    }
                })
                .unwrap_or_else(|err| err.to_string()),
        ),
        "checkpoint" => SlashOutcome::Status(
            runtime
                .checkpoint(session_id, None, Some("manual"))
                .map(|()| "checkpointed".to_owned())
                .unwrap_or_else(|err| err.to_string()),
        ),
        "search" => {
            if args.is_empty() {
                *search = None;
                SlashOutcome::Status("search cleared".to_owned())
            } else {
                *search = Some(args.to_owned());
                SlashOutcome::Status(format!("search {args}"))
            }
        }
        "find" => {
            if args.is_empty() {
                SlashOutcome::Status("usage: /find <query>".to_owned())
            } else {
                match runtime.search_sessions(args) {
                    Ok(rows) if rows.is_empty() => {
                        SlashOutcome::Status(format!("no sessions matching {args}"))
                    }
                    Ok(rows) => panel("find", list_session_lines(&rows)),
                    Err(err) => SlashOutcome::Status(err.to_string()),
                }
            }
        }
        "clear" => {
            *search = None;
            SlashOutcome::Status("search cleared".to_owned())
        }
        "tools" => {
            *hide_tools = !*hide_tools;
            SlashOutcome::Status(format!("hide-tools={hide_tools}"))
        }
        "copy" => SlashOutcome::Status(copy_last_assistant(runtime, session_id)),
        "cancel" => {
            cancel.cancel();
            SlashOutcome::Status("cancel requested".to_owned())
        }
        "steer" => {
            if args.is_empty() {
                SlashOutcome::Status("usage: /steer <message>".to_owned())
            } else {
                SlashOutcome::Status(
                    runtime
                        .queue_steer(session_id, args)
                        .map(|()| "steer queued for the next turn".to_owned())
                        .unwrap_or_else(|err| err.to_string()),
                )
            }
        }
        "yes" => {
            *auto_approve = match args {
                "off" | "false" | "0" | "no" => false,
                "on" | "true" | "1" => true,
                _ => !*auto_approve,
            };
            SlashOutcome::Status(format!("auto-approve={auto_approve}"))
        }
        "no" => {
            *auto_approve = false;
            SlashOutcome::Status("auto-approve=false".to_owned())
        }
        "permissions" => SlashOutcome::Status(format!(
            "auto-approve={auto_approve}  network={}  exec={}  pty={}  worktree={}",
            env_flag("BLORA_NETWORK", "0"),
            env_flag("BLORA_EXEC", "local"),
            env_flag("BLORA_PTY", "0"),
            env_flag("BLORA_WORKTREE", "0"),
        )),
        "approvals" => match runtime.pending_approvals(session_id) {
            Ok(rows) if rows.is_empty() => SlashOutcome::Status("no pending approvals".to_owned()),
            Ok(rows) => panel(
                "approvals",
                rows.into_iter()
                    .map(|row| format!("{}  {}", row.id, row.summary))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "model" => {
            if !args.is_empty() {
                *model = args.to_owned();
            }
            SlashOutcome::Status(if model.is_empty() {
                env_flag("BLORA_MODEL", "mock or gpt-4o-mini")
            } else {
                model.clone()
            })
        }
        "provider" => {
            if args.is_empty() {
                SlashOutcome::ProviderDialog
            } else {
                *provider = args.to_owned();
                SlashOutcome::Status(format!("provider={}", provider.clone()))
            }
        }
        "mode" => {
            if args.is_empty() {
                SlashOutcome::Status(
                    runtime
                        .show_session(session_id)
                        .ok()
                        .and_then(|projection| projection.session)
                        .map(|session| session.mode.as_str().to_owned())
                        .unwrap_or_else(|| "unknown".to_owned()),
                )
            } else {
                match Mode::parse(args) {
                    Ok(mode) => {
                        open_session(runtime, workspace, sessions, index, mode, mode.as_str())
                    }
                    Err(err) => SlashOutcome::Status(err.to_string()),
                }
            }
        }
        "code" => open_session(runtime, workspace, sessions, index, Mode::Code, "code"),
        "work" => open_session(runtime, workspace, sessions, index, Mode::Work, "work"),
        "agent" => open_session(runtime, workspace, sessions, index, Mode::Agent, "agent"),
        "plan" => open_session(runtime, workspace, sessions, index, Mode::Agent, "plan"),
        "exec" => SlashOutcome::Status(env_flag("BLORA_EXEC", "local")),
        "worktree" => SlashOutcome::Status(format!(
            "BLORA_WORKTREE={}",
            env_flag("BLORA_WORKTREE", "0")
        )),
        "tasks" => match runtime.list_tasks(Some(session_id)) {
            Ok(tasks) if tasks.is_empty() => SlashOutcome::Status("no tasks".to_owned()),
            Ok(tasks) => panel(
                "tasks",
                tasks
                    .into_iter()
                    .map(|task| format!("{}  {}  {}", task.id, task.status.as_str(), task.title))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "task" => queue_task(runtime, session_id, auto_approve, args, None, None),
        "cron" => queue_cron(runtime, session_id, auto_approve, args),
        "loop" => queue_loop(runtime, session_id, auto_approve, args),
        "pause" => mutate_task(args, |id| runtime.pause_task(id).map(|()| "paused")),
        "unpause" => mutate_task(args, |id| runtime.resume_task(id).map(|()| "resumed")),
        "cancel-task" => mutate_task(args, |id| runtime.cancel_task(id).map(|()| "cancelled")),
        "pump" => SlashOutcome::Status(
            runtime
                .pump()
                .map(|ids| {
                    if ids.is_empty() {
                        "no due tasks".to_owned()
                    } else {
                        format!("pumped {}", ids.len())
                    }
                })
                .unwrap_or_else(|err| err.to_string()),
        ),
        "fork" => match runtime.fork_session(session_id) {
            Ok(id) => {
                select_session(runtime, sessions, index, &id);
                SlashOutcome::Status(format!("forked {id}"))
            }
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "resume" => SlashOutcome::Status(
            runtime
                .resume_session(session_id)
                .map(|()| "resumed".to_owned())
                .unwrap_or_else(|err| err.to_string()),
        ),
        "archive" => SlashOutcome::Status(
            runtime
                .archive_session(session_id)
                .map(|()| "archived".to_owned())
                .unwrap_or_else(|err| err.to_string()),
        ),
        "export" => SlashOutcome::Status(
            runtime
                .export_session(session_id)
                .map(|value| {
                    format!(
                        "exported {} events",
                        value.as_array().map(Vec::len).unwrap_or(0)
                    )
                })
                .unwrap_or_else(|err| err.to_string()),
        ),
        "timeline" => match runtime.events(session_id) {
            Ok(events) if events.is_empty() => SlashOutcome::Status("no events".to_owned()),
            Ok(events) => panel(
                "timeline",
                events
                    .iter()
                    .rev()
                    .take(24)
                    .map(|event| format!("{}  {}", event.sequence, event.event_type))
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "usage" => SlashOutcome::Status(
            runtime
                .usage(Some(session_id))
                .map(|totals| {
                    format!(
                        "in={} out={} cached={}",
                        totals.input_tokens, totals.output_tokens, totals.cached_tokens
                    )
                })
                .unwrap_or_else(|err| err.to_string()),
        ),
        "memory" => memory_panel(runtime, workspace),
        "remember" => remember_cmd(runtime, workspace, args),
        "recall" => recall_cmd(runtime, workspace, args),
        "forget" => {
            if args.is_empty() {
                SlashOutcome::Status("usage: /forget <key>".to_owned())
            } else {
                match runtime.forget(&workspace_key(workspace), args) {
                    Ok(true) => SlashOutcome::Status(format!("forgot {args}")),
                    Ok(false) => SlashOutcome::Status(format!("no memory {args}")),
                    Err(err) => SlashOutcome::Status(err.to_string()),
                }
            }
        }
        "distill" => SlashOutcome::Status(
            runtime
                .distill_memories(session_id)
                .map(|count| format!("stored {count} memories"))
                .unwrap_or_else(|err| err.to_string()),
        ),
        "plugins" => {
            let plugins = runtime.list_plugins(workspace);
            if plugins.is_empty() {
                SlashOutcome::Status("no plugins installed".to_owned())
            } else {
                panel(
                    "plugins",
                    plugins
                        .into_iter()
                        .map(|plugin| format!("{}  {}", plugin.name, plugin.description))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
        }
        "marketplace" => match blora_runtime::load_index() {
            Ok(market) if market.plugins.is_empty() => {
                SlashOutcome::Status("marketplace is empty".to_owned())
            }
            Ok(market) => panel(
                "marketplace",
                market
                    .plugins
                    .into_iter()
                    .map(|plugin| format!("{}  {}", plugin.name, plugin.description))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "install" => {
            if args.is_empty() {
                SlashOutcome::Status("usage: /install <name>".to_owned())
            } else {
                SlashOutcome::Status(
                    blora_runtime::install_plugin(workspace, args)
                        .map(|path| format!("installed {}", path.display()))
                        .unwrap_or_else(|err| err.to_string()),
                )
            }
        }
        "uninstall" => {
            if args.is_empty() {
                SlashOutcome::Status("usage: /uninstall <name>".to_owned())
            } else {
                SlashOutcome::Status(
                    blora_runtime::uninstall_plugin(workspace, args)
                        .map(|()| format!("removed {args}"))
                        .unwrap_or_else(|err| err.to_string()),
                )
            }
        }
        "skills" => panel("skills", list_skills(workspace)),
        "mcp" => SlashOutcome::Status(format!(
            "BLORA_MCP_COMMAND={}",
            env_flag("BLORA_MCP_COMMAND", "(unset)")
        )),
        "agents" => match runtime.list_subagents(session_id) {
            Ok(rows) if rows.is_empty() => SlashOutcome::Status("no subagents".to_owned()),
            Ok(rows) => panel(
                "agents",
                rows.into_iter()
                    .map(|row| format!("{}  {}  {}", row.id, row.role, row.status))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "git" => match runtime.workspace_info(workspace) {
            Ok(info) => panel("git", format!("{}\n{}", info.git_branch, info.git_status)),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "diff" => match runtime.workspace_info(workspace) {
            Ok(info) => panel("diff", info.git_diff),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "log" => match runtime.workspace_info(workspace) {
            Ok(info) => panel("log", info.git_log),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "files" => match runtime.workspace_info(workspace) {
            Ok(info) => panel("files", info.files),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "read" => {
            if args.is_empty() {
                SlashOutcome::Status("usage: /read <path>".to_owned())
            } else {
                match runtime.read_workspace_file(workspace, args) {
                    Ok(text) => panel(format!("read {args}"), text),
                    Err(err) => SlashOutcome::Status(err.to_string()),
                }
            }
        }
        "artifacts" => match runtime.list_artifacts(Some(session_id)) {
            Ok(rows) if rows.is_empty() => SlashOutcome::Status("no artifacts".to_owned()),
            Ok(rows) => panel(
                "artifacts",
                rows.into_iter()
                    .map(|row| format!("{}  {}", row.kind, row.id))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "pwd" => SlashOutcome::Status(workspace.display().to_string()),
        "doctor" => panel(
            "doctor",
            format!(
                "provider={} (override={})\nmodel={} (override={})\ntheme={}\nexec={}\nworktree={}\nnetwork={}\npty={}\nmcp={}\nhooks={}\nauto-approve={auto_approve}",
                env_flag("BLORA_PROVIDER", "openai"),
                if provider.is_empty() {
                    "-"
                } else {
                    provider.as_str()
                },
                env_flag("BLORA_MODEL", "-"),
                if model.is_empty() {
                    "-"
                } else {
                    model.as_str()
                },
                format!("{} {}", theme::palette().as_str(), theme::scheme().as_str()),
                env_flag("BLORA_EXEC", "local"),
                env_flag("BLORA_WORKTREE", "0"),
                env_flag("BLORA_NETWORK", "0"),
                env_flag("BLORA_PTY", "0"),
                env_flag("BLORA_MCP_COMMAND", "(unset)"),
                env_flag("BLORA_HOOKS_DIR", "(unset)"),
            ),
        ),
        "hooks" => SlashOutcome::Status(env_flag("BLORA_HOOKS_DIR", "(unset)")),
        "init" => SlashOutcome::Status(write_rules(workspace)),
        "users" => match runtime.list_users() {
            Ok(rows) if rows.is_empty() => SlashOutcome::Status("no users".to_owned()),
            Ok(rows) => panel(
                "users",
                rows.into_iter()
                    .map(|row| format!("{}  {}", row.id, row.name))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        "web" => SlashOutcome::Status(
            "open with: blora --web   or   blora web --bind 127.0.0.1:8787".to_owned(),
        ),
        "reload" => {
            refresh_sessions(runtime, sessions, index);
            SlashOutcome::Status(format!("{} sessions", sessions.len()))
        }
        "next" => {
            if *index + 1 < sessions.len() {
                *index += 1;
            }
            SlashOutcome::Status(
                sessions
                    .get(*index)
                    .map(|session| session.id.to_string())
                    .unwrap_or_else(|| "no session".to_owned()),
            )
        }
        "prev" => {
            *index = index.saturating_sub(1);
            SlashOutcome::Status(
                sessions
                    .get(*index)
                    .map(|session| session.id.to_string())
                    .unwrap_or_else(|| "no session".to_owned()),
            )
        }
        "quit" => SlashOutcome::Quit,
        other => SlashOutcome::Status(format!("unhandled /{other}")),
    }
}

fn env_flag(name: &str, fallback: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fallback.to_owned())
}

fn open_session(
    runtime: &Runtime,
    workspace: &Path,
    sessions: &mut Vec<blora_storage::SessionSummary>,
    index: &mut usize,
    mode: Mode,
    title: &str,
) -> SlashOutcome {
    match create_session(runtime, workspace, mode, Some(title)) {
        Ok(id) => {
            select_session(runtime, sessions, index, &id);
            SlashOutcome::Status(format!("created {id} ({})", mode.as_str()))
        }
        Err(err) => SlashOutcome::Status(err.to_string()),
    }
}

fn select_session(
    runtime: &Runtime,
    sessions: &mut Vec<blora_storage::SessionSummary>,
    index: &mut usize,
    id: &SessionId,
) {
    refresh_sessions(runtime, sessions, index);
    if let Some(found) = sessions.iter().position(|item| item.id == *id) {
        *index = found;
    }
}

fn list_session_lines(sessions: &[blora_storage::SessionSummary]) -> String {
    if sessions.is_empty() {
        return "no sessions".to_owned();
    }
    sessions
        .iter()
        .map(|session| {
            format!(
                "{}  {}  {}",
                session.id,
                session.mode.as_str(),
                session.title.as_deref().unwrap_or("-")
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn session_status(runtime: &Runtime, session_id: &SessionId) -> String {
    runtime
        .show_session(session_id)
        .ok()
        .and_then(|projection| projection.session)
        .map(|session| {
            format!(
                "{}  {}  {}",
                session.id,
                session.mode.as_str(),
                session.status.as_str()
            )
        })
        .unwrap_or_else(|| session_id.to_string())
}

fn goto_session(
    sessions: &[blora_storage::SessionSummary],
    index: &mut usize,
    query: &str,
) -> SlashOutcome {
    if query.is_empty() {
        return SlashOutcome::Status("usage: /goto <id|title>".to_owned());
    }
    let needle = query.to_ascii_lowercase();
    match sessions.iter().position(|session| {
        session.id.as_str().to_ascii_lowercase().contains(&needle)
            || session
                .title
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains(&needle)
    }) {
        Some(found) => {
            *index = found;
            SlashOutcome::Status(sessions[found].id.to_string())
        }
        None => SlashOutcome::Status(format!("no session matching {query}")),
    }
}

fn queue_task(
    runtime: &Runtime,
    session_id: &SessionId,
    auto_approve: &bool,
    prompt: &str,
    delay_until: Option<chrono::DateTime<chrono::Utc>>,
    cron: Option<String>,
) -> SlashOutcome {
    if prompt.is_empty() {
        return SlashOutcome::Status("usage: /task <prompt>".to_owned());
    }
    SlashOutcome::Status(
        runtime
            .create_task(CreateTask {
                session_id: session_id.clone(),
                title: prompt.chars().take(40).collect(),
                prompt: prompt.to_owned(),
                delay_until,
                max_attempts: 3,
                auto_approve: *auto_approve,
                mock: std::env::var("BLORA_API_KEY").is_err(),
                cron,
            })
            .map(|id| format!("queued {id}"))
            .unwrap_or_else(|err| err.to_string()),
    )
}

fn queue_cron(
    runtime: &Runtime,
    session_id: &SessionId,
    auto_approve: &bool,
    args: &str,
) -> SlashOutcome {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() < 6 {
        return SlashOutcome::Status("usage: /cron <m h dom mon dow> <prompt>".to_owned());
    }
    let expr = parts[..5].join(" ");
    let prompt = parts[5..].join(" ");
    match blora_runtime::next_cron(&expr, chrono::Utc::now()) {
        Ok(next) => queue_task(
            runtime,
            session_id,
            auto_approve,
            &prompt,
            Some(next),
            Some(expr),
        ),
        Err(err) => SlashOutcome::Status(err.to_string()),
    }
}

fn queue_loop(
    runtime: &Runtime,
    session_id: &SessionId,
    auto_approve: &bool,
    args: &str,
) -> SlashOutcome {
    let Some((spec, prompt)) = args.split_once(char::is_whitespace) else {
        return SlashOutcome::Status("usage: /loop <duration> <prompt>".to_owned());
    };
    match parse_delay(spec) {
        Some(delay) => queue_task(
            runtime,
            session_id,
            auto_approve,
            prompt.trim(),
            Some(chrono::Utc::now() + delay),
            None,
        ),
        None => SlashOutcome::Status("duration must look like 30s, 5m, 1h, or 2d".to_owned()),
    }
}

fn parse_delay(spec: &str) -> Option<chrono::TimeDelta> {
    let spec = spec.trim();
    let split = spec.len().checked_sub(1)?;
    let (digits, unit) = spec.split_at(split);
    let n: i64 = digits.parse().ok()?;
    if n <= 0 {
        return None;
    }
    match unit {
        "s" => Some(chrono::TimeDelta::seconds(n)),
        "m" => Some(chrono::TimeDelta::minutes(n)),
        "h" => Some(chrono::TimeDelta::hours(n)),
        "d" => Some(chrono::TimeDelta::days(n)),
        _ => None,
    }
}

fn mutate_task(args: &str, op: impl FnOnce(&TaskId) -> Result<&'static str>) -> SlashOutcome {
    if args.is_empty() {
        return SlashOutcome::Status("usage: /pause|/unpause|/cancel-task <task-id>".to_owned());
    }
    match TaskId::parse(args) {
        Ok(id) => match op(&id) {
            Ok(verb) => SlashOutcome::Status(format!("{verb} {id}")),
            Err(err) => SlashOutcome::Status(err.to_string()),
        },
        Err(err) => SlashOutcome::Status(err.to_string()),
    }
}

fn memory_panel(runtime: &Runtime, workspace: &Path) -> SlashOutcome {
    match runtime.list_memories(&workspace_key(workspace)) {
        Ok(rows) if rows.is_empty() => SlashOutcome::Status("no memories".to_owned()),
        Ok(rows) => panel(
            "memory",
            rows.into_iter()
                .map(|row| format!("{}={}", row.key, row.value))
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        Err(err) => SlashOutcome::Status(err.to_string()),
    }
}

fn remember_cmd(runtime: &Runtime, workspace: &Path, args: &str) -> SlashOutcome {
    let Some((key, value)) = args.split_once(char::is_whitespace) else {
        return SlashOutcome::Status("usage: /remember <key> <value>".to_owned());
    };
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() || value.is_empty() {
        return SlashOutcome::Status("usage: /remember <key> <value>".to_owned());
    }
    SlashOutcome::Status(
        runtime
            .remember(&workspace_key(workspace), key, value)
            .map(|()| format!("remembered {key}"))
            .unwrap_or_else(|err| err.to_string()),
    )
}

fn recall_cmd(runtime: &Runtime, workspace: &Path, args: &str) -> SlashOutcome {
    if args.is_empty() {
        return memory_panel(runtime, workspace);
    }
    match runtime.recall(&workspace_key(workspace), args) {
        Ok(Some(row)) => SlashOutcome::Status(format!("{}={}", row.key, row.value)),
        Ok(None) => SlashOutcome::Status(format!("no memory {args}")),
        Err(err) => SlashOutcome::Status(err.to_string()),
    }
}

fn list_skills(workspace: &Path) -> String {
    let dir = workspace.join(".blora/skills");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return "no skills in .blora/skills".to_owned();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().into_owned())
            } else {
                None
            }
        })
        .collect();
    names.sort();
    if names.is_empty() {
        "no skills in .blora/skills".to_owned()
    } else {
        names.join("\n")
    }
}

fn write_rules(workspace: &Path) -> String {
    let dir = workspace.join(".blora");
    let path = dir.join("rules.md");
    if path.exists() {
        return format!("exists {}", path.display());
    }
    if std::fs::create_dir_all(&dir).is_err() {
        return "could not create .blora".to_owned();
    }
    match std::fs::write(
        &path,
        "# Blora rules\n\nStay inside this workspace. Prefer tests before claiming a fix.\n",
    ) {
        Ok(()) => format!("wrote {}", path.display()),
        Err(err) => err.to_string(),
    }
}

fn copy_last_assistant(runtime: &Runtime, session_id: &SessionId) -> String {
    let text = runtime
        .show_session(session_id)
        .ok()
        .and_then(|projection| {
            projection.transcript.into_iter().rev().find_map(|item| {
                if let TranscriptItem::Assistant { text, .. } = item {
                    Some(text)
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();
    if text.is_empty() {
        return "nothing to copy".to_owned();
    }
    for (program, args) in [
        ("wl-copy", &[] as &[&str]),
        ("xclip", &["-selection", "clipboard"] as &[&str]),
        ("pbcopy", &[] as &[&str]),
    ] {
        if let Ok(mut child) = std::process::Command::new(program)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
            return format!("copied {} chars", text.chars().count());
        }
    }
    format!(
        "no clipboard tool; last reply is {} chars",
        text.chars().count()
    )
}
