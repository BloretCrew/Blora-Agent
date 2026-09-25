// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Terminal UI. Renders session projections, never provider payloads.

mod i18n;
mod markdown;
mod selection;
mod slash;
mod theme;
mod view;

use std::io::{self, Write, stdout};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::{CreateSession, CreateTask};
use blora_types::{Mode, Result, SessionId, TaskId};
use crossterm::cursor::{Hide, Show};
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

fn begin_passport_login(
    passport_url: &mut Option<String>,
    passport_receiver: &mut Option<std::sync::mpsc::Receiver<blora_auth::PassportUser>>,
    passport_browser_opened: &mut bool,
    passport_dialog: &mut Option<view::PassportDialog>,
) -> String {
    match start_passport_login() {
        Ok(Some((device, receiver))) => {
            *passport_url = Some(device.verification_uri.clone());
            *passport_receiver = Some(receiver);
            *passport_browser_opened = false;
            *passport_dialog = Some(view::PassportDialog {
                user_code: device.user_code.clone(),
                verification_uri: device.verification_uri.clone(),
                opened_browser: false,
            });
            format!("{}\n设备码：{}", device.verification_uri, device.user_code)
        }
        Ok(None) => "PassPort 登录未配置".to_owned(),
        Err(err) => err.to_string(),
    }
}

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
    let _ = i18n::init();
    let mut passport_url = None;
    let mut passport_receiver: Option<std::sync::mpsc::Receiver<blora_auth::PassportUser>> = None;
    let mut passport_browser_opened = false;
    // Pending device-flow login, shown as a centered dialog; hidden with Esc.
    let mut passport_dialog: Option<view::PassportDialog> = None;
    // Model-provider switch dialog opened via `/provider`.
    let mut provider_dialog: Option<view::ProviderDialog> = None;
    let mut add_provider_dialog: Option<view::AddProviderDialog> = None;
    let mut theme_dialog: Option<view::ThemeDialog> = None;
    let mut context_dialog: Option<view::ContextDialog> = None;
    let mut tool_dialog: Option<view::ToolDialog> = None;
    let mut tool_detail_dialog: Option<view::ToolDetailDialog> = None;
    let mut git_dialog: Option<view::GitDialog> = None;
    let mut mode_menu: Option<view::ModeMenu> = None;
    let mut project_picker: Option<view::ProjectPicker> = None;
    let mut folder_picker: Option<view::FolderPicker> = None;
    let mut session_picker: Option<view::SessionPicker> = None;
    // PassPort user token of the logged-in user; drives the default provider.
    let mut passport_user_token: Option<String> = None;
    let mut passport_username = String::from("you");
    let mut passport_needs_login = false;
    if let Ok(users) = runtime.list_users() {
        for user in &users {
            if let Some(token) = usable_passport_token(runtime, user)? {
                passport_user_token = Some(token);
                break;
            }
        }
        if let Some(name) = users.iter().find_map(passport_display_name) {
            passport_username = name;
        }
        // Do not start an interactive login on every TUI launch. An expired
        // access token is reported as unavailable; the user can run /login.
        if passport_user_token.is_none() {
            passport_needs_login = true;
        }
    }
    let mut sessions = runtime.list_sessions()?;
    if sessions.is_empty() {
        let id = runtime.create_session(CreateSession {
            title: None,
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
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture, Hide)
        .map_err(blora_types::BloraError::exec)?;
    if std::env::var_os("NO_COLOR").is_none() {
        let _ = write!(stdout, "{}", theme::Theme::current().terminal_osc());
        let _ = stdout.flush();
    }
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).map_err(blora_types::BloraError::exec)?;
    let mut input = String::new();
    let mut input_history: Vec<String> = Vec::new();
    let mut history_index: Option<usize> = None;
    let mut paste_preview: Option<String> = None;
    let mut status = if passport_needs_login {
        "PassPort 未登录或令牌已过期，请执行 /login".to_owned()
    } else {
        String::new()
    };
    let mut auto_approve = false;
    let mut scroll = 0usize;
    let mut search: Option<String> = None;
    let mut hide_tools = false;
    let mut slash_selected = 0usize;
    let mut last_slash_token = String::new();
    let mut model_override = String::new();
    let mut provider_override = String::new();
    let mut git_status: Option<view::GitStatusInfo> = None;
    let mut notice: Option<String> = None;
    let mut tick = 0u64;
    let mut last_window_title = String::new();
    let mut last_routing_session: Option<SessionId> = None;
    let mut pointer: Option<(u16, u16)> = None;
    let mut text_selection: Option<selection::Selection> = None;
    let mut selecting_text = false;
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
                    user.refresh_token.as_deref(),
                    Some(chrono::Utc::now()
                        + chrono::Duration::seconds(user.expires_in.unwrap_or(3600) as i64)),
                )?;
                passport_user_token = user
                    .apptoken
                    .clone()
                    .filter(|token| !token.trim().is_empty());
                passport_receiver = None;
                passport_dialog = None;
                passport_needs_login = false;
                passport_username = user
                    .nickname
                    .as_deref()
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .unwrap_or("you")
                    .to_owned();
                status = format!("PassPort 登录成功：{}", user.display_name());
                if provider_dialog.is_some() {
                    provider_dialog = Some(open_provider_dialog(
                        &provider_override,
                        &model_override,
                        passport_user_token.is_some(),
                    ));
                }
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
            if session_id.as_ref() != last_routing_session.as_ref() {
                last_routing_session = session_id.clone();
                if let Some(projection) = projection {
                    provider_override = projection.provider.clone().unwrap_or_default();
                    model_override = projection.model.clone().unwrap_or_default();
                } else {
                    provider_override.clear();
                    model_override.clear();
                }
            }
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
            if tick % 20 == 0 || git_status.is_none() {
                git_status = runtime
                    .workspace_info(workspace)
                    .ok()
                    .map(|info| parse_git_status(&info));
            }
            let logged_in = passport_user_token.is_some();
            let provider_option_list = provider_options(&provider_override, logged_in);
            let model = if model_override.is_empty() {
                env_model.as_str()
            } else {
                model_override.as_str()
            };
            let provider = if provider_override.is_empty() {
                if env_provider.is_empty() {
                    "Bloret PassPort"
                } else {
                    env_provider.as_str()
                }
            } else {
                provider_override.as_str()
            };
            let running = job.is_some();
            let mode = projection
                .and_then(|projection| projection.session.as_ref())
                .map(|session| session.mode.as_str())
                .or_else(|| sessions.get(index).map(|session| session.mode.as_str()))
                .unwrap_or("code");
            let session_title = projection
                .and_then(|projection| projection.session.as_ref())
                .and_then(|session| session.title.as_deref())
                .or_else(|| {
                    sessions
                        .get(index)
                        .and_then(|session| session.title.as_deref())
                });
            let label = view::title_label(projection, session_title);
            let title = view::window_title(running, tick, mode, &label);
            if title != last_window_title {
                last_window_title.clone_from(&title);
                let _ = write!(io::stdout(), "{}", theme::title_osc(&title));
                let _ = execute!(io::stdout(), Hide);
            }
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
                            paste_preview: paste_preview.as_deref(),
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
                            add_provider_dialog: add_provider_dialog.as_ref(),
                            theme_dialog: theme_dialog.as_ref(),
                            context_dialog: context_dialog.as_ref(),
                            tool_dialog: tool_dialog.as_ref(),
                            tool_detail_dialog: tool_detail_dialog.as_ref(),
                            git_dialog: git_dialog.as_ref(),
                            git_status: git_status.as_ref(),
                            slash_hits: &slash_hits,
                            slash_selected,
                            search: search.as_deref(),
                            hide_tools,
                            scroll,
                            auto_approve,
                            model,
                            provider,
                            mode_menu: mode_menu.as_ref(),
                            project_picker: project_picker.as_ref(),
                            folder_picker: folder_picker.as_ref(),
                            session_picker: session_picker.as_ref(),
                            user_label: &passport_username,
                            running: job.is_some(),
                            tick,
                            pointer,
                            text_selection,
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

            let wait_ms = if job.is_some() { 50 } else { 200 };
            if event::poll(Duration::from_millis(wait_ms)).map_err(blora_types::BloraError::exec)? {
                match event::read().map_err(blora_types::BloraError::exec)? {
                    Event::Paste(text) => {
                        if text.is_empty() {
                            paste_clipboard(&mut input, &mut paste_preview);
                        } else {
                            insert_pasted_text(&mut input, &mut paste_preview, &text);
                        }
                        history_index = None;
                    }
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
                        if folder_picker.is_some()
                            && matches!(key.code, KeyCode::Left | KeyCode::Right)
                        {
                            handle_traffic_light(
                                Some(if key.code == KeyCode::Left {
                                    view::Hit::TrafficClose
                                } else {
                                    view::Hit::TrafficMinimize
                                }),
                                &mut folder_picker,
                                &mut project_picker,
                                &mut session_picker,
                                &mut mode_menu,
                                &mut tool_dialog,
                                &mut tool_detail_dialog,
                                &mut git_dialog,
                                &mut theme_dialog,
                                &mut add_provider_dialog,
                                &mut provider_dialog,
                                &mut context_dialog,
                                &mut passport_dialog,
                                &mut passport_browser_opened,
                                passport_url.as_deref(),
                                &mut cancel,
                                &mut status,
                            );
                            continue;
                        }
                        if folder_picker.is_some() {
                            match key.code {
                                KeyCode::Up => {
                                    if let Some(picker) = folder_picker.as_mut() {
                                        picker.selected = picker.selected.saturating_sub(1);
                                        if picker.selected < picker.scroll {
                                            picker.scroll = picker.selected;
                                        }
                                    }
                                }
                                KeyCode::Down => {
                                    if let Some(picker) = folder_picker.as_mut() {
                                        picker.selected = (picker.selected + 1)
                                            .min(picker.entries.len().saturating_sub(1));
                                        let page = 9usize;
                                        if picker.selected >= picker.scroll + page {
                                            picker.scroll = picker.selected + 1 - page;
                                        }
                                    }
                                }
                                KeyCode::Esc => folder_picker = None,
                                KeyCode::Enter => {
                                    if let Some(picker) = folder_picker.as_mut() {
                                        if picker.selected == 0 {
                                            if let Some(parent) = picker.path.parent() {
                                                picker.path = parent.to_path_buf();
                                                picker.entries = folder_entries(&picker.path);
                                                picker.selected = 0;
                                                picker.scroll = 0;
                                            }
                                        } else if let Some(path) =
                                            picker.entries.get(picker.selected).cloned()
                                        {
                                            picker.path = path;
                                            picker.entries = folder_entries(&picker.path);
                                            picker.selected = 0;
                                            picker.scroll = 0;
                                        }
                                    }
                                }
                                KeyCode::Char('o' | 'O') => {
                                    if let Some(picker) = folder_picker.take() {
                                        match create_session(
                                            runtime,
                                            &picker.path,
                                            Mode::Code,
                                            None,
                                        ) {
                                            Ok(id) => {
                                                refresh_sessions(
                                                    runtime,
                                                    &mut sessions,
                                                    &mut index,
                                                );
                                                index = sessions
                                                    .iter()
                                                    .position(|item| item.id == id)
                                                    .unwrap_or(0);
                                                cached = None;
                                                status = format!(
                                                    "已打开项目：{}",
                                                    picker.path.display()
                                                );
                                            }
                                            Err(err) => status = err.to_string(),
                                        }
                                    }
                                }
                                _ => {}
                            }
                            continue;
                        }
                        match key.code {
                            KeyCode::Char('v') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                                paste_clipboard(&mut input, &mut paste_preview);
                                history_index = None;
                            }
                            KeyCode::Esc if text_selection.is_some() => {
                                text_selection = None;
                                selecting_text = false;
                            }
                            KeyCode::Esc if add_provider_dialog.is_some() => {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    dialog.error = None;
                                    match dialog.step {
                                        view::AddProviderStep::Catalog => {
                                            add_provider_dialog = None;
                                        }
                                        view::AddProviderStep::CustomId => {
                                            dialog.step = view::AddProviderStep::Catalog;
                                        }
                                        view::AddProviderStep::CustomBase => {
                                            dialog.step = if dialog.is_custom() {
                                                view::AddProviderStep::CustomId
                                            } else {
                                                view::AddProviderStep::Catalog
                                            };
                                        }
                                        view::AddProviderStep::ApiKey => {
                                            dialog.step = if dialog.needs_base_step() {
                                                view::AddProviderStep::CustomBase
                                            } else {
                                                view::AddProviderStep::Catalog
                                            };
                                        }
                                        view::AddProviderStep::Review => {
                                            dialog.step = view::AddProviderStep::ApiKey;
                                        }
                                    }
                                }
                            }
                            KeyCode::Up if add_provider_dialog.is_some() => {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    if dialog.step == view::AddProviderStep::Catalog {
                                        dialog.selected = dialog.selected.saturating_sub(1);
                                    }
                                }
                            }
                            KeyCode::Down if add_provider_dialog.is_some() => {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    if dialog.step == view::AddProviderStep::Catalog {
                                        let max = dialog.catalog_len().saturating_sub(1);
                                        dialog.selected = (dialog.selected + 1).min(max);
                                    }
                                }
                            }
                            KeyCode::Left
                                if add_provider_dialog.as_ref().is_some_and(|dialog| {
                                    dialog.step == view::AddProviderStep::Review
                                }) =>
                            {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    dialog.format = cycle_format(dialog.format, -1);
                                    refresh_add_preview(dialog);
                                }
                            }
                            KeyCode::Right
                                if add_provider_dialog.as_ref().is_some_and(|dialog| {
                                    dialog.step == view::AddProviderStep::Review
                                }) =>
                            {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    dialog.format = cycle_format(dialog.format, 1);
                                    refresh_add_preview(dialog);
                                }
                            }
                            KeyCode::Enter if add_provider_dialog.is_some() => {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    match advance_add_provider(dialog) {
                                        AddAdvance::Stay => {}
                                        AddAdvance::Saved(saved) => {
                                            status = accept_saved_provider(
                                                saved,
                                                &mut provider_override,
                                                &mut model_override,
                                                &mut provider_dialog,
                                                logged_in,
                                                runtime,
                                                session_id.as_ref(),
                                            );
                                            add_provider_dialog = None;
                                        }
                                        AddAdvance::Fail(err) => {
                                            dialog.error = Some(err);
                                        }
                                    }
                                }
                            }
                            KeyCode::Backspace
                                if add_provider_dialog.as_ref().is_some_and(|dialog| {
                                    matches!(
                                        dialog.step,
                                        view::AddProviderStep::Catalog
                                            | view::AddProviderStep::CustomId
                                            | view::AddProviderStep::CustomBase
                                            | view::AddProviderStep::ApiKey
                                    )
                                }) =>
                            {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    add_step_buffer(dialog).pop();
                                    if dialog.step == view::AddProviderStep::Catalog {
                                        dialog.clamp_catalog_selected();
                                    }
                                }
                            }
                            KeyCode::Char(ch)
                                if add_provider_dialog.as_ref().is_some_and(|dialog| {
                                    matches!(
                                        dialog.step,
                                        view::AddProviderStep::Catalog
                                            | view::AddProviderStep::CustomId
                                            | view::AddProviderStep::CustomBase
                                            | view::AddProviderStep::ApiKey
                                    )
                                }) && !key.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                if let Some(dialog) = add_provider_dialog.as_mut() {
                                    add_step_buffer(dialog).push(ch);
                                    if dialog.step == view::AddProviderStep::Catalog {
                                        dialog.clamp_catalog_selected();
                                    }
                                }
                            }
                            KeyCode::Left if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut() {
                                    dialog.pane = view::ProviderPane::Providers;
                                }
                            }
                            KeyCode::Right if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut()
                                    && !dialog.is_add()
                                    && !dialog.current_models().is_empty()
                                {
                                    dialog.pane = view::ProviderPane::Models;
                                }
                            }
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
                            KeyCode::Left if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.as_mut() {
                                    dialog.scheme = cycle_scheme(dialog.scheme, -1);
                                    preview_theme_dialog(dialog);
                                }
                            }
                            KeyCode::Right if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.as_mut() {
                                    dialog.scheme = cycle_scheme(dialog.scheme, 1);
                                    preview_theme_dialog(dialog);
                                }
                            }
                            KeyCode::Up if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.as_mut() {
                                    dialog.selected = dialog.selected.saturating_sub(1);
                                    preview_theme_dialog(dialog);
                                }
                            }
                            KeyCode::Down if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.as_mut() {
                                    dialog.selected =
                                        (dialog.selected + 1).min(dialog.options.len() - 1);
                                    preview_theme_dialog(dialog);
                                }
                            }
                            KeyCode::Enter if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.take() {
                                    status = commit_theme_dialog(&dialog);
                                }
                            }
                            KeyCode::Up if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut() {
                                    match dialog.pane {
                                        view::ProviderPane::Providers => {
                                            dialog.selected = dialog.selected.saturating_sub(1);
                                            dialog.model_selected = 0;
                                        }
                                        view::ProviderPane::Models => {
                                            dialog.model_selected =
                                                dialog.model_selected.saturating_sub(1);
                                        }
                                    }
                                }
                            }
                            KeyCode::Down if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut() {
                                    match dialog.pane {
                                        view::ProviderPane::Providers => {
                                            dialog.selected = (dialog.selected + 1)
                                                .min(dialog.options.len().saturating_sub(1));
                                            dialog.model_selected = 0;
                                        }
                                        view::ProviderPane::Models => {
                                            let max =
                                                dialog.current_models().len().saturating_sub(1);
                                            dialog.model_selected =
                                                (dialog.model_selected + 1).min(max);
                                        }
                                    }
                                }
                            }
                            KeyCode::Enter if provider_dialog.is_some() => {
                                if let Some(dialog) = provider_dialog.as_mut() {
                                    if dialog.is_add() {
                                        add_provider_dialog = Some(open_add_provider_dialog());
                                    } else if dialog
                                        .current()
                                        .is_some_and(|option| !option.available)
                                    {
                                        status = begin_passport_login(
                                            &mut passport_url,
                                            &mut passport_receiver,
                                            &mut passport_browser_opened,
                                            &mut passport_dialog,
                                        );
                                    } else if dialog.pane == view::ProviderPane::Providers
                                        && !dialog.current_models().is_empty()
                                    {
                                        dialog.pane = view::ProviderPane::Models;
                                    } else if let Some(status_text) = commit_provider_dialog(
                                        dialog,
                                        &mut provider_override,
                                        &mut model_override,
                                        runtime,
                                        session_id.as_ref(),
                                    ) {
                                        provider_dialog = None;
                                        status = status_text;
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
                            KeyCode::Left if git_dialog.is_some() => {
                                if let Some(dialog) = git_dialog.as_mut() {
                                    dialog.page = dialog.page.saturating_sub(1);
                                }
                            }
                            KeyCode::Esc if git_dialog.is_some() => {
                                git_dialog = None;
                            }
                            KeyCode::Right if git_dialog.is_some() => {
                                if let Some(dialog) = git_dialog.as_mut() {
                                    dialog.page = (dialog.page + 1).min(2);
                                }
                            }
                            KeyCode::Enter if git_dialog.is_some() => {
                                if let Some(dialog) = git_dialog.as_mut() {
                                    if dialog.page == 0 {
                                        dialog.info = git_status
                                            .clone()
                                            .unwrap_or_else(|| dialog.info.clone());
                                    }
                                }
                            }
                            KeyCode::Up
                                if project_picker.is_none()
                                    && session_picker.is_none()
                                    && mode_menu.is_none()
                                    && theme_dialog.is_none()
                                    && add_provider_dialog.is_none()
                                    && provider_dialog.is_none()
                                    && context_dialog.is_none()
                                    && tool_dialog.is_none()
                                    && tool_detail_dialog.is_none()
                                    && passport_dialog.is_none() =>
                            {
                                if !input_history.is_empty() {
                                    let next = history_index
                                        .map_or(input_history.len().saturating_sub(1), |index| {
                                            index.saturating_sub(1)
                                        });
                                    history_index = Some(next);
                                    input = input_history[next].clone();
                                    slash_selected = 0;
                                } else if input.is_empty() {
                                    scroll = scroll.saturating_add(1);
                                }
                            }
                            KeyCode::Down
                                if project_picker.is_none()
                                    && session_picker.is_none()
                                    && mode_menu.is_none()
                                    && theme_dialog.is_none()
                                    && add_provider_dialog.is_none()
                                    && provider_dialog.is_none()
                                    && context_dialog.is_none()
                                    && tool_dialog.is_none()
                                    && tool_detail_dialog.is_none()
                                    && passport_dialog.is_none() =>
                            {
                                if let Some(index) = history_index {
                                    if index + 1 < input_history.len() {
                                        let next = index + 1;
                                        history_index = Some(next);
                                        input = input_history[next].clone();
                                    } else {
                                        history_index = None;
                                        input.clear();
                                    }
                                    slash_selected = 0;
                                } else if input.is_empty() {
                                    scroll = scroll.saturating_sub(1);
                                }
                            }
                            KeyCode::Tab if slash::is_open(&input) => {
                                if let Some(cmd) = slash_hits.get(slash_selected) {
                                    input = slash::complete(cmd);
                                    slash_selected = 0;
                                }
                            }
                            KeyCode::Enter if mode_menu.is_none() => {
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
                                            SlashOutcome::LoggedOut(text) => {
                                                notice = None;
                                                status = text;
                                                passport_user_token = None;
                                                passport_needs_login = true;
                                                passport_username = "you".to_owned();
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
                                                passport_needs_login = false;
                                            }
                                            SlashOutcome::ProviderDialog => {
                                                theme_dialog = None;
                                                add_provider_dialog = None;
                                                provider_dialog = Some(open_provider_dialog(
                                                    &provider_override,
                                                    &model_override,
                                                    logged_in,
                                                ));
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
                                    if input_history.last() != Some(&prompt) {
                                        input_history.push(prompt.clone());
                                    }
                                    history_index = None;
                                    input.clear();
                                    notice = None;
                                    status = "running…".to_owned();
                                    let options = tui_run_options(
                                        auto_approve,
                                        &model_override,
                                        &provider_override,
                                        passport_user_token.clone(),
                                    );
                                    let cancel_clone = cancel.clone();
                                    job = Some(scope.spawn(move || {
                                        runtime.run(&id, &prompt, &cancel_clone, &options)
                                    }));
                                }
                            }
                            KeyCode::Backspace => {
                                input.pop();
                                history_index = None;
                                paste_preview = None;
                            }
                            KeyCode::Char(ch) => {
                                input.push(ch);
                                history_index = None;
                                paste_preview = None;
                            }
                            KeyCode::Insert => {
                                if let Some(text) = read_clipboard_text() {
                                    insert_pasted_text(&mut input, &mut paste_preview, &text);
                                    history_index = None;
                                }
                            }
                            KeyCode::Esc if slash::is_open(&input) || input.starts_with('/') => {
                                input.clear();
                                paste_preview = None;
                                slash_selected = 0;
                            }
                            // The login dialog is only hidden: polling keeps
                            // running so completing authorization still lands.
                            KeyCode::Enter
                                if session_picker
                                    .as_ref()
                                    .is_some_and(|picker| picker.minimized) =>
                            {
                                if let Some(picker) = session_picker.as_mut() {
                                    picker.minimized = false;
                                }
                            }
                            KeyCode::Enter
                                if project_picker
                                    .as_ref()
                                    .is_some_and(|picker| picker.minimized) =>
                            {
                                if let Some(picker) = project_picker.as_mut() {
                                    picker.minimized = false;
                                }
                            }
                            KeyCode::Enter if project_picker.is_some() => {
                                if let Some(picker) = project_picker.as_ref() {
                                    if let Some(path) =
                                        project_paths(&sessions).get(picker.selected)
                                    {
                                        if let Some(found) = sessions
                                            .iter()
                                            .position(|session| &session.workspace_path == path)
                                        {
                                            index = found;
                                            cached = None;
                                            project_picker = None;
                                        }
                                    }
                                }
                            }
                            KeyCode::Up if project_picker.is_some() => {
                                if let Some(picker) = project_picker.as_mut() {
                                    picker.selected = picker.selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down if project_picker.is_some() => {
                                if let Some(picker) = project_picker.as_mut() {
                                    picker.selected = (picker.selected + 1)
                                        .min(project_count(&sessions).saturating_sub(1));
                                }
                            }
                            KeyCode::Esc if project_picker.is_some() => {
                                project_picker = None;
                            }
                            KeyCode::Enter if session_picker.is_some() => {
                                if let Some(picker) = session_picker.take() {
                                    select_mode_session(
                                        runtime,
                                        &sessions,
                                        &mut index,
                                        (0..sessions.len()).collect(),
                                        picker.selected,
                                    );
                                }
                            }
                            KeyCode::Up if session_picker.is_some() => {
                                if let Some(picker) = session_picker.as_mut() {
                                    picker.selected = picker.selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down if session_picker.is_some() => {
                                if let Some(picker) = session_picker.as_mut() {
                                    picker.selected =
                                        (picker.selected + 1).min(sessions.len().saturating_sub(1));
                                }
                            }
                            KeyCode::Esc if session_picker.is_some() => {
                                session_picker = None;
                            }
                            KeyCode::Esc if project_picker.is_some() => {
                                project_picker = None;
                            }
                            KeyCode::Enter
                                if mode_menu.as_ref().is_some_and(|menu| menu.minimized) =>
                            {
                                if let Some(menu) = mode_menu.as_mut() {
                                    menu.minimized = false;
                                }
                            }
                            KeyCode::Enter if mode_menu.is_some() => {
                                if let Some(menu) = mode_menu.take()
                                    && let Some(id) = session_id.as_ref()
                                {
                                    let option = view::MODE_OPTIONS
                                        [menu.selected.min(view::MODE_OPTIONS.len() - 1)];
                                    match runtime.set_session_mode(id, option.mode) {
                                        Ok(()) => {
                                            status = format!("已切换到 {} 模式", option.title)
                                        }
                                        Err(err) => status = err.to_string(),
                                    }
                                }
                            }
                            KeyCode::Up if mode_menu.is_some() => {
                                if let Some(menu) = mode_menu.as_mut() {
                                    menu.selected = menu.selected.saturating_sub(1);
                                }
                            }
                            KeyCode::Down if mode_menu.is_some() => {
                                if let Some(menu) = mode_menu.as_mut() {
                                    menu.selected =
                                        (menu.selected + 1).min(view::MODE_OPTIONS.len() - 1);
                                }
                            }
                            KeyCode::Esc if mode_menu.is_some() => {
                                mode_menu = None;
                            }
                            KeyCode::Esc if passport_dialog.is_some() => {
                                passport_dialog = None;
                            }
                            KeyCode::Esc if theme_dialog.is_some() => {
                                if let Some(dialog) = theme_dialog.take() {
                                    cancel_theme_dialog(&dialog);
                                    status = "theme picker cancelled".to_owned();
                                }
                            }
                            KeyCode::Esc if provider_dialog.is_some() => {
                                provider_dialog = None;
                            }
                            KeyCode::Esc if context_dialog.is_some() => {
                                context_dialog = None;
                            }
                            KeyCode::Up if context_dialog.is_some() => {
                                if let Some(dialog) = context_dialog.as_mut() {
                                    dialog.scroll = dialog.scroll.saturating_sub(1);
                                }
                            }
                            KeyCode::Down if tool_detail_dialog.is_some() => {
                                if let Some(dialog) = tool_detail_dialog.as_mut() {
                                    dialog.scroll = dialog.scroll.saturating_add(1);
                                }
                            }
                            KeyCode::Up if tool_detail_dialog.is_some() => {
                                if let Some(dialog) = tool_detail_dialog.as_mut() {
                                    dialog.scroll = dialog.scroll.saturating_sub(1);
                                }
                            }
                            KeyCode::Esc if tool_detail_dialog.is_some() => {
                                tool_detail_dialog = None;
                            }
                            KeyCode::Down if tool_dialog.is_some() => {
                                if let Some(dialog) = tool_dialog.as_mut() {
                                    dialog.scroll = dialog.scroll.saturating_add(1);
                                }
                            }
                            KeyCode::Up if tool_dialog.is_some() => {
                                if let Some(dialog) = tool_dialog.as_mut() {
                                    dialog.scroll = dialog.scroll.saturating_sub(1);
                                }
                            }
                            KeyCode::Esc if tool_dialog.is_some() => {
                                tool_dialog = None;
                            }
                            KeyCode::Down if context_dialog.is_some() => {
                                if let Some(dialog) = context_dialog.as_mut() {
                                    dialog.scroll = dialog.scroll.saturating_add(1);
                                }
                            }
                            KeyCode::Esc if notice.is_some() => {
                                notice = None;
                            }
                            KeyCode::Esc => {
                                input.clear();
                                notice = None;
                                slash_selected = 0;
                            }
                            _ => {}
                        }
                    }
                    Event::Mouse(mouse) => {
                        pointer = Some((mouse.column, mouse.row));
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                if let Some(picker) = folder_picker.as_mut() {
                                    picker.scroll = picker.scroll.saturating_sub(1);
                                    picker.selected =
                                        picker.selected.saturating_sub(1).max(picker.scroll);
                                    continue;
                                }
                                if git_dialog.is_some() {
                                    if let Some(dialog) = git_dialog.as_mut() {
                                        dialog.selected = dialog.selected.saturating_sub(1);
                                    }
                                    continue;
                                }
                                if tool_detail_dialog.is_some() {
                                    if let Some(dialog) = tool_detail_dialog.as_mut() {
                                        dialog.scroll = dialog.scroll.saturating_sub(3);
                                    }
                                } else if tool_dialog.is_some() {
                                    if let Some(dialog) = tool_dialog.as_mut() {
                                        dialog.scroll = dialog.scroll.saturating_sub(3);
                                    }
                                } else if hits.over_slash(mouse.column, mouse.row) {
                                    slash_selected = slash_selected.saturating_sub(1);
                                } else {
                                    scroll = scroll.saturating_add(3);
                                }
                            }
                            MouseEventKind::ScrollDown => {
                                if let Some(picker) = folder_picker.as_mut() {
                                    let visible = 8usize;
                                    picker.scroll = picker
                                        .scroll
                                        .saturating_add(1)
                                        .min(picker.entries.len().saturating_sub(visible));
                                    picker.selected =
                                        picker.selected.saturating_add(1).max(picker.scroll);
                                    continue;
                                }
                                if git_dialog.is_some() {
                                    if let Some(dialog) = git_dialog.as_mut() {
                                        dialog.selected = dialog.selected.saturating_add(1);
                                    }
                                    continue;
                                }
                                if tool_detail_dialog.is_some() {
                                    if let Some(dialog) = tool_detail_dialog.as_mut() {
                                        dialog.scroll = dialog.scroll.saturating_add(3);
                                    }
                                } else if tool_dialog.is_some() {
                                    if let Some(dialog) = tool_dialog.as_mut() {
                                        dialog.scroll = dialog.scroll.saturating_add(3);
                                    }
                                } else if hits.over_slash(mouse.column, mouse.row) {
                                    if !slash_hits.is_empty() {
                                        slash_selected =
                                            (slash_selected + 1).min(slash_hits.len() - 1);
                                    }
                                } else {
                                    scroll = scroll.saturating_sub(3);
                                }
                            }
                            MouseEventKind::Moved | MouseEventKind::Drag(MouseButton::Left) => {
                                if selecting_text && let Some(selection) = text_selection.as_mut() {
                                    selection.end = (mouse.column, mouse.row);
                                }
                                match hits.hit(mouse.column, mouse.row) {
                                    Some(view::Hit::Slash(idx)) => {
                                        slash_selected = idx;
                                    }
                                    Some(view::Hit::ProviderRow(idx)) => {
                                        if let Some(dialog) = provider_dialog.as_mut() {
                                            dialog.selected = idx;
                                            dialog.model_selected = 0;
                                            dialog.pane = view::ProviderPane::Providers;
                                        }
                                    }
                                    Some(view::Hit::ProviderModelRow(idx)) => {
                                        if let Some(dialog) = provider_dialog.as_mut() {
                                            dialog.model_selected = idx;
                                            dialog.pane = view::ProviderPane::Models;
                                        }
                                    }
                                    Some(view::Hit::AddProviderRow(idx)) => {
                                        if let Some(dialog) = add_provider_dialog.as_mut() {
                                            match dialog.step {
                                                view::AddProviderStep::Catalog => {
                                                    dialog.selected = idx
                                                }
                                                view::AddProviderStep::Review => {
                                                    dialog.format = cycle_format(dialog.format, 1);
                                                    refresh_add_preview(dialog);
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                    Some(view::Hit::ThemeTab(idx)) => {
                                        if let Some(dialog) = theme_dialog.as_mut() {
                                            dialog.scheme = scheme_from_tab(idx);
                                            preview_theme_dialog(dialog);
                                        }
                                    }
                                    Some(view::Hit::ThemeRow(idx)) => {
                                        if let Some(dialog) = theme_dialog.as_mut() {
                                            dialog.selected = idx;
                                            preview_theme_dialog(dialog);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                if let Some(selection) = text_selection
                                    && selecting_text
                                {
                                    if let Some(projection) = projection {
                                        let text = selection::selected_text(
                                            projection,
                                            hits.transcript,
                                            selection,
                                            scroll,
                                            hide_tools,
                                        );
                                        if !text.is_empty() {
                                            status = copy_text_to_clipboard(&text);
                                        }
                                    }
                                }
                                selecting_text = false;
                            }
                            MouseEventKind::Down(MouseButton::Left) => {
                                let hit = hits.hit(mouse.column, mouse.row);
                                if matches!(
                                    hit,
                                    Some(
                                        view::Hit::TrafficClose
                                            | view::Hit::TrafficMinimize
                                            | view::Hit::TrafficOpenBrowser
                                    )
                                ) {
                                    handle_traffic_light(
                                        hit,
                                        &mut folder_picker,
                                        &mut project_picker,
                                        &mut session_picker,
                                        &mut mode_menu,
                                        &mut tool_dialog,
                                        &mut tool_detail_dialog,
                                        &mut git_dialog,
                                        &mut theme_dialog,
                                        &mut add_provider_dialog,
                                        &mut provider_dialog,
                                        &mut context_dialog,
                                        &mut passport_dialog,
                                        &mut passport_browser_opened,
                                        passport_url.as_deref(),
                                        &mut cancel,
                                        &mut status,
                                    );
                                    continue;
                                }
                                if let Some(view::Hit::ToolSummary(indices)) = hit.clone() {
                                    selecting_text = false;
                                    text_selection = None;
                                    let tools = projection
                                        .map(|projection| {
                                            indices
                                                .iter()
                                                .filter_map(|index| {
                                                    let item = projection.transcript.get(*index)?;
                                                    if let blora_session::TranscriptItem::Tool {
                                                        name,
                                                        status,
                                                        arguments,
                                                        output,
                                                        ..
                                                    } = item
                                                    {
                                                        Some(view::ToolCallDetail {
                                                            name: name.clone(),
                                                            status: status.clone(),
                                                            arguments: arguments.clone(),
                                                            output: output.clone(),
                                                        })
                                                    } else {
                                                        None
                                                    }
                                                })
                                                .collect()
                                        })
                                        .unwrap_or_default();
                                    tool_dialog = Some(view::ToolDialog {
                                        tools,
                                        scroll: 0,
                                        fullscreen: false,
                                        minimized: false,
                                    });
                                    continue;
                                }
                                if tool_detail_dialog.is_some() {
                                    continue;
                                }
                                if let Some(view::Hit::ToolDetailRow(index)) = hit.clone()
                                    && let Some(dialog) = tool_dialog.as_ref()
                                    && let Some(tool) = dialog.tools.get(index).cloned()
                                {
                                    tool_detail_dialog = Some(view::ToolDetailDialog {
                                        tool,
                                        scroll: 0,
                                        fullscreen: false,
                                        minimized: false,
                                    });
                                    continue;
                                }
                                if tool_dialog.is_some() {
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::GitStatus)) {
                                    if let Some(info) = git_status.clone() {
                                        git_dialog = Some(view::GitDialog {
                                            info,
                                            page: 0,
                                            selected: 0,
                                            fullscreen: false,
                                            minimized: false,
                                        });
                                    }
                                    continue;
                                }
                                if git_dialog.is_some() {
                                    if let Some(dialog) = git_dialog.as_mut() {
                                        match hit {
                                            Some(view::Hit::GitRow(index))
                                            | Some(view::Hit::GitAction(index)) => {
                                                dialog.selected = index
                                            }
                                            _ => {}
                                        }
                                    }
                                    continue;
                                }
                                let modal_open = folder_picker.is_some()
                                    || project_picker.is_some()
                                    || session_picker.is_some()
                                    || mode_menu.is_some()
                                    || theme_dialog.is_some()
                                    || add_provider_dialog.is_some()
                                    || provider_dialog.is_some()
                                    || context_dialog.is_some()
                                    || tool_dialog.is_some()
                                    || tool_detail_dialog.is_some()
                                    || git_dialog.is_some()
                                    || passport_dialog.is_some();
                                if matches!(
                                    hit,
                                    Some(
                                        view::Hit::TrafficClose
                                            | view::Hit::TrafficMinimize
                                            | view::Hit::TrafficOpenBrowser,
                                    )
                                ) {
                                    handle_traffic_light(
                                        hit,
                                        &mut folder_picker,
                                        &mut project_picker,
                                        &mut session_picker,
                                        &mut mode_menu,
                                        &mut tool_dialog,
                                        &mut tool_detail_dialog,
                                        &mut git_dialog,
                                        &mut theme_dialog,
                                        &mut add_provider_dialog,
                                        &mut provider_dialog,
                                        &mut context_dialog,
                                        &mut passport_dialog,
                                        &mut passport_browser_opened,
                                        passport_url.as_deref(),
                                        &mut cancel,
                                        &mut status,
                                    );
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::ContextUsage)) {
                                    context_dialog = Some(view::ContextDialog {
                                        scroll: 0,
                                        fullscreen: false,
                                        minimized: false,
                                    });
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::ProviderTarget)) {
                                    provider_dialog = Some(open_provider_dialog(
                                        &provider_override,
                                        &model_override,
                                        logged_in,
                                    ));
                                    continue;
                                }
                                if !modal_open
                                    && hits.transcript.contains((mouse.column, mouse.row).into())
                                {
                                    text_selection = Some(selection::Selection {
                                        start: (mouse.column, mouse.row),
                                        end: (mouse.column, mouse.row),
                                    });
                                    selecting_text = true;
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::FolderPickerRow(_)))
                                    || matches!(
                                        hit,
                                        Some(
                                            view::Hit::FolderPickerParent
                                                | view::Hit::FolderPickerOpen
                                        )
                                    )
                                {
                                    selecting_text = false;
                                    text_selection = None;
                                }
                                if matches!(hit, Some(view::Hit::FolderPickerParent)) {
                                    if let Some(picker) = folder_picker.as_mut()
                                        && let Some(parent) = picker.path.parent()
                                    {
                                        picker.path = parent.to_path_buf();
                                        picker.entries = folder_entries(&picker.path);
                                        picker.selected = 0;
                                    }
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::FolderPickerOpen)) {
                                    if let Some(picker) = folder_picker.take() {
                                        match create_session(
                                            runtime,
                                            &picker.path,
                                            Mode::Code,
                                            None,
                                        ) {
                                            Ok(id) => {
                                                refresh_sessions(
                                                    runtime,
                                                    &mut sessions,
                                                    &mut index,
                                                );
                                                index = sessions
                                                    .iter()
                                                    .position(|item| item.id == id)
                                                    .unwrap_or(0);
                                                cached = None;
                                                status = format!(
                                                    "已打开项目：{}",
                                                    picker.path.display()
                                                );
                                            }
                                            Err(err) => status = err.to_string(),
                                        }
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::FolderPickerRow(selected)) = hit {
                                    selecting_text = false;
                                    text_selection = None;
                                    if let Some(picker) = folder_picker.as_mut() {
                                        if selected == 0 {
                                            if let Some(parent) = picker.path.parent() {
                                                picker.path = parent.to_path_buf();
                                                picker.entries = folder_entries(&picker.path);
                                                picker.selected = 0;
                                                picker.scroll = 0;
                                            }
                                        } else if let Some(path) =
                                            picker.entries.get(selected).cloned()
                                        {
                                            picker.path = path;
                                            picker.entries = folder_entries(&picker.path);
                                            picker.selected = 0;
                                            picker.scroll = 0;
                                        }
                                    }
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::OpenNewProject)) {
                                    folder_picker = Some(open_folder_picker(workspace));
                                    project_picker = None;
                                    continue;
                                }
                                if let Some(view::Hit::ProjectPickerRow(project_index)) = hit {
                                    selecting_text = false;
                                    text_selection = None;
                                    if let Some(picker) = project_picker.as_mut() {
                                        picker.selected = project_index;
                                    }
                                    if let Some(path) = project_paths(&sessions).get(project_index)
                                        && let Some(found) = sessions
                                            .iter()
                                            .position(|session| &session.workspace_path == path)
                                    {
                                        index = found;
                                        cached = None;
                                    }
                                    project_picker = None;
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::ProjectPicker)) {
                                    project_picker = Some(view::ProjectPicker {
                                        selected: 0,
                                        fullscreen: false,
                                        minimized: false,
                                    });
                                    continue;
                                }
                                if let Some(view::Hit::SessionPickerRow(session_index)) = hit {
                                    index = session_index;
                                    session_picker = None;
                                    cached = None;
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::SessionPicker)) {
                                    let selected = index;
                                    session_picker = Some(view::SessionPicker {
                                        selected,
                                        fullscreen: false,
                                        minimized: false,
                                    });
                                    continue;
                                }
                                if let Some(view::Hit::ModeRow(index)) = hit {
                                    if let Some(id) = session_id.as_ref() {
                                        let option = view::MODE_OPTIONS
                                            [index.min(view::MODE_OPTIONS.len() - 1)];
                                        match runtime.set_session_mode(id, option.mode) {
                                            Ok(()) => {
                                                status = format!("已切换到 {} 模式", option.title)
                                            }
                                            Err(err) => status = err.to_string(),
                                        }
                                    }
                                    mode_menu = None;
                                    continue;
                                }
                                if matches!(hit, Some(view::Hit::Mode)) {
                                    let current = projection
                                        .and_then(|projection| projection.session.as_ref())
                                        .map(|session| session.mode)
                                        .unwrap_or(Mode::Code);
                                    let selected = view::MODE_OPTIONS
                                        .iter()
                                        .position(|option| option.mode == current)
                                        .unwrap_or(0);
                                    mode_menu = Some(view::ModeMenu {
                                        selected,
                                        fullscreen: false,
                                        minimized: false,
                                    });
                                    continue;
                                }
                                // Provider dialog rows: click selects and confirms.
                                if let Some(view::Hit::ThemeTab(idx)) = hit {
                                    if let Some(dialog) = theme_dialog.as_mut() {
                                        dialog.scheme = scheme_from_tab(idx);
                                        preview_theme_dialog(dialog);
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::ThemeRow(idx)) = hit {
                                    if let Some(dialog) = theme_dialog.take() {
                                        let mut dialog = dialog;
                                        dialog.selected = idx;
                                        status = commit_theme_dialog(&dialog);
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::ProviderRow(idx)) = hit {
                                    if let Some(dialog) = provider_dialog.as_mut() {
                                        dialog.selected = idx;
                                        dialog.model_selected = 0;
                                        if dialog.is_add() {
                                            add_provider_dialog = Some(open_add_provider_dialog());
                                        } else if dialog
                                            .current()
                                            .is_some_and(|option| !option.available)
                                        {
                                            status = begin_passport_login(
                                                &mut passport_url,
                                                &mut passport_receiver,
                                                &mut passport_browser_opened,
                                                &mut passport_dialog,
                                            );
                                        } else {
                                            dialog.pane = view::ProviderPane::Models;
                                        }
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::ProviderModelRow(idx)) = hit {
                                    if let Some(dialog) = provider_dialog.as_mut() {
                                        dialog.model_selected = idx;
                                        dialog.pane = view::ProviderPane::Models;
                                        if let Some(status_text) = commit_provider_dialog(
                                            dialog,
                                            &mut provider_override,
                                            &mut model_override,
                                            runtime,
                                            session_id.as_ref(),
                                        ) {
                                            provider_dialog = None;
                                            status = status_text;
                                        }
                                    }
                                    continue;
                                }
                                if let Some(view::Hit::AddProviderRow(idx)) = hit {
                                    if let Some(dialog) = add_provider_dialog.as_mut() {
                                        match dialog.step {
                                            view::AddProviderStep::Catalog => {
                                                dialog.selected = idx;
                                                match advance_add_provider(dialog) {
                                                    AddAdvance::Stay => {}
                                                    AddAdvance::Saved(saved) => {
                                                        status = accept_saved_provider(
                                                            saved,
                                                            &mut provider_override,
                                                            &mut model_override,
                                                            &mut provider_dialog,
                                                            logged_in,
                                                            runtime,
                                                            session_id.as_ref(),
                                                        );
                                                        add_provider_dialog = None;
                                                    }
                                                    AddAdvance::Fail(err) => {
                                                        dialog.error = Some(err);
                                                    }
                                                }
                                            }
                                            view::AddProviderStep::Review => {
                                                dialog.format = cycle_format(dialog.format, 1);
                                                refresh_add_preview(dialog);
                                            }
                                            _ => {}
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
                                                &mut passport_user_token,
                                                &mut passport_username,
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
                                    if let Some(dialog) = theme_dialog.take() {
                                        cancel_theme_dialog(&dialog);
                                        status = "theme picker closed".to_owned();
                                    } else if add_provider_dialog.take().is_some() {
                                        status = "add provider cancelled".to_owned();
                                    } else if provider_dialog.take().is_some() {
                                        status = "provider picker closed".to_owned();
                                    } else if context_dialog.take().is_some() {
                                        status = "context dialog closed".to_owned();
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
                                    } else if let Some(dialog) = add_provider_dialog.as_mut() {
                                        dialog.minimized = true;
                                        dialog.fullscreen = false;
                                    } else if let Some(dialog) = provider_dialog.as_mut() {
                                        dialog.minimized = true;
                                        dialog.fullscreen = false;
                                    } else if let Some(dialog) = context_dialog.as_mut() {
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
                                    } else if let Some(dialog) = add_provider_dialog.as_mut() {
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
                                    } else if let Some(dialog) = context_dialog.as_mut() {
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
                                                &mut passport_user_token,
                                                &mut passport_username,
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
                                        let options = tui_run_options(
                                            auto_approve,
                                            &model_override,
                                            &provider_override,
                                            passport_user_token.clone(),
                                        );
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
    execute!(
        io::stdout(),
        DisableMouseCapture,
        LeaveAlternateScreen,
        Show
    )
    .ok();
    if std::env::var_os("NO_COLOR").is_none() {
        let _ = write!(io::stdout(), "{}", theme::TERMINAL_RESET);
        let _ = io::stdout().flush();
    }
    result
}

fn folder_entries(path: &Path) -> Vec<PathBuf> {
    let mut entries = vec![PathBuf::from("..")];
    if let Ok(read_dir) = std::fs::read_dir(path) {
        let mut children: Vec<PathBuf> = read_dir
            .filter_map(std::result::Result::ok)
            .filter_map(|entry| {
                let path = entry.path();
                path.is_dir().then_some(path)
            })
            .collect();
        children.sort_by_key(|path| path.file_name().map(|name| name.to_os_string()));
        entries.extend(children);
    }
    entries
}

fn open_folder_picker(workspace: &Path) -> view::FolderPicker {
    let path = std::env::current_dir().unwrap_or_else(|_| workspace.to_path_buf());
    view::FolderPicker {
        entries: folder_entries(&path),
        path,
        selected: 0,
        scroll: 0,
        fullscreen: false,
        minimized: false,
    }
}

fn refresh_sessions(
    runtime: &Runtime,
    sessions: &mut Vec<blora_storage::SessionSummary>,
    index: &mut usize,
) {
    let selected_id = sessions.get(*index).map(|session| session.id.clone());
    if let Ok(list) = runtime.list_sessions() {
        *sessions = list;
        if let Some(selected_id) = selected_id
            && let Some(selected_index) = sessions
                .iter()
                .position(|session| session.id == selected_id)
        {
            *index = selected_index;
        } else if sessions.is_empty() {
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
        title: title.map(str::to_owned),
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
    LoggedOut(String),
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
    passport_user_token: &mut Option<String>,
    passport_username: &mut String,
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
                pane: view::ProviderPane::Providers,
                model_selected: 0,
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
        SlashOutcome::LoggedOut(text) => {
            *notice = None;
            *status = text;
            *passport_user_token = None;
            *passport_username = "you".to_owned();
            false
        }
    }
}

/// Built-in list is Bloret PassPort plus any saved vendors. Extra vendors
/// come from the add-provider wizard.
fn provider_options(provider_override: &str, logged_in: bool) -> Vec<view::ProviderOption> {
    let override_id = provider_override.trim().to_owned();
    let current = |id: &str| {
        if override_id.is_empty() {
            id.eq_ignore_ascii_case("blora")
        } else {
            override_id.eq_ignore_ascii_case(id)
        }
    };
    let mut entries: Vec<view::ProviderOption> = vec![view::ProviderOption {
        id: "blora".to_owned(),
        display: "Bloret PassPort".to_owned(),
        hint: if logged_in {
            "默认 · 200 次/天".to_owned()
        } else {
            "未登录 · /login 或回车登录".to_owned()
        },
        available: logged_in,
        models: models_for_provider("blora"),
    }];
    for mut saved in blora_catalog::load_saved() {
        if entries
            .iter()
            .any(|option| option.id.eq_ignore_ascii_case(&saved.id))
        {
            continue;
        }
        if saved.models.is_empty() {
            let entry = blora_catalog::CatalogEntry {
                id: saved.id.clone(),
                name: saved.name.clone(),
                api: Some(saved.base_url.clone()),
                env: Vec::new(),
                npm: None,
                models: Vec::new(),
            };
            if let Ok(models) =
                blora_catalog::resolve_models(&entry, saved.format, &saved.api_key, &saved.base_url)
            {
                saved.models = models;
                if matches!(
                    saved.format,
                    blora_catalog::MessageFormat::Openai | blora_catalog::MessageFormat::Responses
                ) {
                    if let Some(base) = blora_catalog::openai_base_candidates(&saved.base_url)
                        .into_iter()
                        .find(|candidate| candidate.ends_with("/v1"))
                    {
                        saved.base_url = base;
                    }
                }
                let _ = blora_catalog::save_provider(saved.clone());
            }
        }
        entries.push(view::ProviderOption {
            id: saved.id.clone(),
            display: saved.name.clone(),
            hint: saved.format.label().to_owned(),
            available: !saved.api_key.is_empty(),
            models: saved
                .models
                .into_iter()
                .map(|model| view::ModelOption {
                    id: model.id,
                    display: model.name,
                    hint: String::new(),
                })
                .collect(),
        });
    }
    entries.push(view::ProviderOption {
        id: blora_catalog::ADD_PROVIDER_ID.to_owned(),
        display: "+ 添加供应商".to_owned(),
        hint: "从 models.dev 接入".to_owned(),
        available: true,
        models: Vec::new(),
    });
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

fn models_for_provider(id: &str) -> Vec<view::ModelOption> {
    let catalog_id = if id.is_empty() { "openai" } else { id };
    let from_catalog = blora_catalog::cached_catalog()
        .into_iter()
        .find(|entry| entry.id.eq_ignore_ascii_case(catalog_id))
        .map(|entry| {
            entry
                .models
                .into_iter()
                .map(|model| view::ModelOption {
                    id: model.id,
                    display: model.name,
                    hint: String::new(),
                })
                .collect::<Vec<_>>()
        })
        .filter(|models: &Vec<_>| !models.is_empty());
    if let Some(models) = from_catalog {
        return models;
    }
    match catalog_id {
        "blora" => vec![view::ModelOption {
            id: blora_model::PASSPORT_MODEL_NAME.to_owned(),
            display: blora_model::PASSPORT_PROVIDER_DISPLAY_NAME.to_owned(),
            hint: String::new(),
        }],
        "anthropic" => vec![view::ModelOption {
            id: "claude-3-5-sonnet-latest".to_owned(),
            display: "claude-3-5-sonnet-latest".to_owned(),
            hint: String::new(),
        }],
        "gemini" => vec![view::ModelOption {
            id: "gemini-2.0-flash".to_owned(),
            display: "gemini-2.0-flash".to_owned(),
            hint: String::new(),
        }],
        _ => vec![
            view::ModelOption {
                id: "gpt-4o".to_owned(),
                display: "gpt-4o".to_owned(),
                hint: String::new(),
            },
            view::ModelOption {
                id: "gpt-4o-mini".to_owned(),
                display: "gpt-4o-mini".to_owned(),
                hint: String::new(),
            },
        ],
    }
}

fn open_provider_dialog(
    provider_override: &str,
    model_override: &str,
    logged_in: bool,
) -> view::ProviderDialog {
    let options = provider_options(provider_override, logged_in);
    let selected = options
        .iter()
        .position(|option| {
            if provider_override.is_empty() {
                option.id.eq_ignore_ascii_case("blora")
            } else {
                option.id.eq_ignore_ascii_case(provider_override)
            }
        })
        .or_else(|| {
            options
                .iter()
                .position(|option| option.id != blora_catalog::ADD_PROVIDER_ID)
        })
        .unwrap_or(0);
    let model_selected = options
        .get(selected)
        .map(|option| {
            option
                .models
                .iter()
                .position(|model| model.id == model_override)
                .unwrap_or(0)
        })
        .unwrap_or(0);
    view::ProviderDialog {
        options,
        selected,
        pane: view::ProviderPane::Providers,
        model_selected,
        fullscreen: false,
        minimized: false,
    }
}

fn commit_provider_dialog(
    dialog: &view::ProviderDialog,
    provider_override: &mut String,
    model_override: &mut String,
    runtime: &Runtime,
    session_id: Option<&SessionId>,
) -> Option<String> {
    let option = dialog.current()?;
    if option.id == blora_catalog::ADD_PROVIDER_ID {
        return None;
    }
    if !option.available {
        return None;
    }
    *provider_override = option.id.clone();
    if let Some(model) = option.models.get(dialog.model_selected) {
        *model_override = model.id.clone();
    }
    if let Some(session_id) = session_id {
        if let Err(err) =
            runtime.set_session_routing(session_id, option.id.clone(), model_override.clone())
        {
            return Some(err.to_string());
        }
    }
    Some(if model_override.is_empty() {
        format!("provider={}", option.display)
    } else {
        format!("provider={}  model={model_override}", option.display)
    })
}

fn usable_passport_token(
    runtime: &Runtime,
    user: &blora_storage::UserRecord,
) -> Result<Option<String>> {
    let Some(access_token) = user
        .passport_app_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    else {
        return Ok(None);
    };
    let refresh_soon = user
        .passport_token_expires_at
        .is_some_and(|expires| expires <= chrono::Utc::now() + chrono::Duration::minutes(2));
    if !refresh_soon {
        return Ok(Some(access_token.to_owned()));
    }
    let Some(refresh_token) = user
        .passport_refresh_token
        .as_deref()
        .map(str::trim)
        .filter(|token| !token.is_empty())
    else {
        return Ok(None);
    };
    let config = blora_auth::PassportConfig::from_env()
        .map_err(|err| blora_types::BloraError::Other(err.to_string()))?;
    match config.refresh_access_token(refresh_token) {
        Ok(refreshed) => {
            let expires_at = refreshed
                .expires_in
                .map(|seconds| chrono::Utc::now() + chrono::Duration::seconds(seconds as i64));
            runtime.refresh_passport_token(
                user.passport_username.as_deref().unwrap_or_default(),
                &refreshed.access_token,
                refreshed.refresh_token.as_deref(),
                expires_at,
            )?;
            Ok(Some(refreshed.access_token))
        }
        Err(err) => {
            eprintln!(
                "PassPort 令牌刷新失败（{}）：{}；请执行 /login 重新登录",
                user.passport_username.as_deref().unwrap_or_default(),
                err
            );
            Ok(None)
        }
    }
}

fn project_paths(sessions: &[blora_storage::SessionSummary]) -> Vec<String> {
    let mut paths: Vec<String> = sessions
        .iter()
        .map(|session| session.workspace_path.clone())
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

fn project_count(sessions: &[blora_storage::SessionSummary]) -> usize {
    project_paths(sessions).len().max(1)
}

fn select_mode_session(
    _runtime: &Runtime,
    _sessions: &[blora_storage::SessionSummary],
    index: &mut usize,
    indices: Vec<usize>,
    selected: usize,
) {
    if let Some(session_index) = indices.get(selected) {
        *index = *session_index;
    }
}

fn passport_display_name(user: &blora_storage::UserRecord) -> Option<String> {
    user.passport_nickname
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(ToOwned::to_owned)
}

fn tui_run_options(
    auto_approve: bool,
    model: &str,
    provider: &str,
    passport_user_token: Option<String>,
) -> RunOptions {
    RunOptions {
        mock: blora_model::should_auto_mock(provider, passport_user_token.as_deref()),
        auto_approve,
        interactive: true,
        model: model.to_owned(),
        provider: provider.to_owned(),
        passport_user_token,
        ..RunOptions::default()
    }
}

fn open_add_provider_dialog() -> view::AddProviderDialog {
    let (catalog, error) = match blora_catalog::fetch_catalog() {
        Ok(list) => (list, None),
        Err(err) => (Vec::new(), Some(err.to_string())),
    };
    view::AddProviderDialog {
        step: view::AddProviderStep::Catalog,
        catalog,
        selected: 0,
        filter: String::new(),
        custom_id: String::new(),
        custom_base: String::new(),
        api_key: String::new(),
        format: blora_catalog::MessageFormat::Openai,
        preview_models: Vec::new(),
        resolved_base: String::new(),
        error,
        fullscreen: false,
        minimized: false,
    }
}

enum AddAdvance {
    Stay,
    Saved(blora_catalog::SavedProvider),
    Fail(String),
}

fn add_step_buffer(dialog: &mut view::AddProviderDialog) -> &mut String {
    match dialog.step {
        view::AddProviderStep::Catalog => &mut dialog.filter,
        view::AddProviderStep::CustomId => &mut dialog.custom_id,
        view::AddProviderStep::CustomBase => &mut dialog.custom_base,
        view::AddProviderStep::ApiKey => &mut dialog.api_key,
        view::AddProviderStep::Review => &mut dialog.custom_id,
    }
}

fn cycle_format(current: blora_catalog::MessageFormat, delta: i8) -> blora_catalog::MessageFormat {
    let all = blora_catalog::MessageFormat::ALL;
    let idx = all.iter().position(|item| *item == current).unwrap_or(0) as i8;
    let next = (idx + delta).rem_euclid(all.len() as i8) as usize;
    all[next]
}

fn accept_saved_provider(
    saved: blora_catalog::SavedProvider,
    provider_override: &mut String,
    model_override: &mut String,
    provider_dialog: &mut Option<view::ProviderDialog>,
    logged_in: bool,
    runtime: &Runtime,
    session_id: Option<&SessionId>,
) -> String {
    *provider_override = saved.id.clone();
    if let Some(model) = saved.models.first() {
        *model_override = model.id.clone();
    }
    if let Some(session_id) = session_id {
        let _ = runtime.set_session_routing(
            session_id,
            provider_override.clone(),
            model_override.clone(),
        );
    }
    *provider_dialog = Some(open_provider_dialog(
        provider_override,
        model_override,
        logged_in,
    ));
    format!(
        "已添加供应商 {}（{} 个模型）",
        saved.name,
        saved.models.len()
    )
}

fn advance_add_provider(dialog: &mut view::AddProviderDialog) -> AddAdvance {
    dialog.error = None;
    match dialog.step {
        view::AddProviderStep::Catalog => {
            if dialog.is_custom() {
                dialog.step = view::AddProviderStep::CustomId;
                AddAdvance::Stay
            } else if let Some(entry) = dialog.catalog_entry().cloned() {
                dialog.format = entry.format();
                dialog.custom_id = entry.id.clone();
                if let Some(api) = entry.api.clone().filter(|api| !api.trim().is_empty()) {
                    dialog.custom_base = api;
                } else {
                    dialog.custom_base.clear();
                }
                dialog.step = if dialog.needs_base_step() {
                    view::AddProviderStep::CustomBase
                } else {
                    view::AddProviderStep::ApiKey
                };
                AddAdvance::Stay
            } else {
                AddAdvance::Fail("目录为空，可选手动添加".to_owned())
            }
        }
        view::AddProviderStep::CustomId => {
            let id = dialog.custom_id.trim().to_ascii_lowercase();
            if !id
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
                || id.is_empty()
            {
                return AddAdvance::Fail("id 只能用小写字母、数字、连字符或下划线".to_owned());
            }
            dialog.custom_id = id;
            dialog.step = view::AddProviderStep::CustomBase;
            AddAdvance::Stay
        }
        view::AddProviderStep::CustomBase => {
            let base = dialog.custom_base.trim();
            if base.is_empty() {
                return AddAdvance::Fail("需要 API 基址".to_owned());
            }
            if !(base.starts_with("https://") || base.starts_with("http://")) {
                return AddAdvance::Fail("基址需要以 http:// 或 https:// 开头".to_owned());
            }
            dialog.custom_base = base.trim_end_matches('/').to_owned();
            dialog.step = view::AddProviderStep::ApiKey;
            AddAdvance::Stay
        }
        view::AddProviderStep::ApiKey => {
            if dialog.api_key.trim().is_empty() {
                return AddAdvance::Fail("需要 API key".to_owned());
            }
            dialog.step = view::AddProviderStep::Review;
            refresh_add_preview(dialog);
            AddAdvance::Stay
        }
        view::AddProviderStep::Review => finish_add_provider(dialog),
    }
}

fn add_provider_draft(
    dialog: &view::AddProviderDialog,
) -> std::result::Result<(String, String, blora_catalog::CatalogEntry, String), String> {
    let (id, name, entry) = if dialog.is_custom() {
        let id = dialog.custom_id.trim().to_owned();
        if id.is_empty() {
            return Err("未填写供应商 id".to_owned());
        }
        (
            id.clone(),
            id.clone(),
            blora_catalog::CatalogEntry {
                id: id.clone(),
                name: id,
                api: Some(dialog.custom_base.clone()),
                env: Vec::new(),
                npm: None,
                models: Vec::new(),
            },
        )
    } else if let Some(entry) = dialog.catalog_entry().cloned() {
        (entry.id.clone(), entry.name.clone(), entry)
    } else {
        return Err("未选择供应商".to_owned());
    };
    let mut base = dialog.custom_base.trim().trim_end_matches('/').to_owned();
    if matches!(
        dialog.format,
        blora_catalog::MessageFormat::Openai | blora_catalog::MessageFormat::Responses
    ) {
        if let Some(with_v1) = blora_catalog::openai_base_candidates(&base)
            .into_iter()
            .find(|candidate| candidate.ends_with("/v1"))
        {
            if !base.ends_with("/v1") {
                base = with_v1;
            }
        }
    }
    Ok((id, name, entry, base))
}

fn refresh_add_preview(dialog: &mut view::AddProviderDialog) {
    match add_provider_draft(dialog) {
        Ok((_, _, entry, base)) => {
            dialog.resolved_base = base.clone();
            match blora_catalog::resolve_models(&entry, dialog.format, &dialog.api_key, &base) {
                Ok(models) => {
                    dialog.preview_models = models;
                    if dialog.preview_models.is_empty() {
                        dialog.error =
                            Some("没有列出模型。可按 ←→ 切换格式，或返回修改基址。".to_owned());
                    } else {
                        dialog.error = None;
                    }
                }
                Err(err) => {
                    dialog.preview_models.clear();
                    dialog.error = Some(format!("拉取模型失败：{err}"));
                }
            }
        }
        Err(err) => {
            dialog.preview_models.clear();
            dialog.error = Some(err);
        }
    }
}

fn finish_add_provider(dialog: &view::AddProviderDialog) -> AddAdvance {
    let (id, name, mut entry, base) = match add_provider_draft(dialog) {
        Ok(draft) => draft,
        Err(err) => return AddAdvance::Fail(err),
    };
    let models = if dialog.preview_models.is_empty() {
        match blora_catalog::resolve_models(&entry, dialog.format, &dialog.api_key, &base) {
            Ok(models) => models,
            Err(err) => return AddAdvance::Fail(format!("拉取模型失败：{err}")),
        }
    } else {
        dialog.preview_models.clone()
    };
    if models.is_empty()
        && matches!(
            dialog.format,
            blora_catalog::MessageFormat::Openai | blora_catalog::MessageFormat::Responses
        )
    {
        return AddAdvance::Fail(
            "没有可保存的模型。请确认基址（通常以 /v1 结尾）和密钥。".to_owned(),
        );
    }
    entry.models = models.clone();
    let saved = blora_catalog::SavedProvider {
        id,
        name,
        api_key: dialog.api_key.clone(),
        base_url: base,
        format: dialog.format,
        models,
    };
    if let Err(err) = blora_catalog::save_provider(saved.clone()) {
        return AddAdvance::Fail(err.to_string());
    }
    AddAdvance::Saved(saved)
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
            "usage: /theme [coral|indigo|graphite|mono|circuit|dusk|dark|light|auto]".to_owned(),
        ),
    }
}

fn open_theme_dialog() -> view::ThemeDialog {
    let current_palette = theme::palette();
    let current_scheme = match theme::scheme() {
        theme::Scheme::Plain => theme::Scheme::Auto,
        other => other,
    };
    let original = theme::ThemePref {
        palette: current_palette,
        scheme: theme::scheme(),
    };
    let options = theme::Palette::ALL
        .into_iter()
        .map(|palette| {
            let hint = if palette == current_palette {
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
        .position(|option| option.id == current_palette.as_str())
        .unwrap_or(0);
    view::ThemeDialog {
        options,
        selected,
        scheme: current_scheme,
        original,
        fullscreen: false,
        minimized: false,
    }
}

fn scheme_from_tab(index: usize) -> theme::Scheme {
    match index {
        1 => theme::Scheme::Light,
        2 => theme::Scheme::Dark,
        _ => theme::Scheme::Auto,
    }
}

fn cycle_scheme(current: theme::Scheme, delta: i8) -> theme::Scheme {
    let tabs = [
        theme::Scheme::Auto,
        theme::Scheme::Light,
        theme::Scheme::Dark,
    ];
    let idx = tabs
        .iter()
        .position(|scheme| *scheme == current)
        .unwrap_or(0) as i8;
    let next = (idx + delta).rem_euclid(3) as usize;
    tabs[next]
}

fn preview_theme_dialog(dialog: &view::ThemeDialog) {
    let palette = dialog
        .options
        .get(dialog.selected)
        .and_then(|option| theme::Palette::parse(&option.id))
        .unwrap_or(theme::Palette::Coral);
    theme::set_pref(theme::ThemePref {
        palette,
        scheme: dialog.scheme,
    });
    write_cursor();
}

fn commit_theme_dialog(dialog: &view::ThemeDialog) -> String {
    preview_theme_dialog(dialog);
    apply_theme_pref(theme::ThemePref {
        palette: dialog
            .options
            .get(dialog.selected)
            .and_then(|option| theme::Palette::parse(&option.id))
            .unwrap_or(theme::Palette::Coral),
        scheme: dialog.scheme,
    })
}

fn cancel_theme_dialog(dialog: &view::ThemeDialog) {
    theme::set_pref(dialog.original);
    write_cursor();
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
    let _ = write!(io::stdout(), "{}", theme::Theme::current().terminal_osc());
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
        "logout" => SlashOutcome::LoggedOut(
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

fn parse_git_status(info: &blora_runtime::WorkspaceInfo) -> view::GitStatusInfo {
    let mut branch = info.git_branch.trim().to_owned();
    if branch.is_empty() {
        branch = info
            .git_status
            .lines()
            .next()
            .unwrap_or("未检测到 Git")
            .to_owned();
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    for line in info.git_diff.lines() {
        for token in line.split_whitespace() {
            if let Some(value) = token.strip_prefix('+') {
                added += value.parse::<usize>().unwrap_or(0);
            }
            if let Some(value) = token.strip_prefix('-') {
                removed += value.parse::<usize>().unwrap_or(0);
            }
        }
    }
    let status_line = info.git_status.lines().next().unwrap_or("");
    let ahead = git_tracking_count(status_line, "ahead ");
    let behind = git_tracking_count(status_line, "behind ");
    let stashes = info
        .git_stashes
        .lines()
        .filter(|line| line.starts_with("stash@{"))
        .count();
    let clean =
        status_line.starts_with("## ") && info.git_status.lines().skip(1).all(str::is_empty);
    view::GitStatusInfo {
        branch,
        added,
        removed,
        ahead,
        behind,
        stashes,
        clean,
        raw: info.git_status.clone(),
        diff: info.git_diff.clone(),
        log: info.git_log.clone(),
    }
}

#[cfg(test)]
mod git_status_tests {
    use super::*;

    #[test]
    fn parses_clean_tracking_and_stash_counts() {
        let info = blora_runtime::WorkspaceInfo {
            git_status: "## main...origin/main [ahead 5, behind 2]".into(),
            git_stashes: "stash@{0}\nstash@{1}".into(),
            ..Default::default()
        };
        let status = parse_git_status(&info);
        assert!(status.clean);
        assert_eq!((status.ahead, status.behind, status.stashes), (5, 2, 2));
        let dirty = blora_runtime::WorkspaceInfo {
            git_status: "## main...origin/main [ahead 5]\n M src/lib.rs".into(),
            ..Default::default()
        };
        assert!(!parse_git_status(&dirty).clean);
    }
}

fn git_tracking_count(status: &str, label: &str) -> usize {
    status
        .split_once(label)
        .and_then(|(_, rest)| rest.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|number| number.parse().ok())
        .unwrap_or(0)
}

fn handle_traffic_light(
    hit: Option<view::Hit>,
    folder_picker: &mut Option<view::FolderPicker>,
    project_picker: &mut Option<view::ProjectPicker>,
    session_picker: &mut Option<view::SessionPicker>,
    mode_menu: &mut Option<view::ModeMenu>,
    tool_dialog: &mut Option<view::ToolDialog>,
    tool_detail_dialog: &mut Option<view::ToolDetailDialog>,
    git_dialog: &mut Option<view::GitDialog>,
    theme_dialog: &mut Option<view::ThemeDialog>,
    add_provider_dialog: &mut Option<view::AddProviderDialog>,
    provider_dialog: &mut Option<view::ProviderDialog>,
    context_dialog: &mut Option<view::ContextDialog>,
    passport_dialog: &mut Option<view::PassportDialog>,
    passport_browser_opened: &mut bool,
    passport_url: Option<&str>,
    cancel: &mut CancelToken,
    status: &mut String,
) {
    match hit {
        Some(view::Hit::TrafficClose) => {
            if folder_picker.take().is_some() {
                *status = "文件夹选择器已关闭".to_owned();
            } else if project_picker.take().is_some() {
                *status = "项目选择器已关闭".to_owned();
            } else if session_picker.take().is_some() {
                *status = "会话选择器已关闭".to_owned();
            } else if mode_menu.take().is_some() {
                *status = "运行模式选择器已关闭".to_owned();
            } else if git_dialog.take().is_some() {
                *status = "Git 状态已关闭".to_owned();
            } else if tool_detail_dialog.take().is_some() {
                *status = "工具详细信息已关闭".to_owned();
            } else if tool_dialog.take().is_some() {
                *status = "工具调用列表已关闭".to_owned();
            } else if let Some(dialog) = theme_dialog.take() {
                cancel_theme_dialog(&dialog);
                *status = "theme picker closed".to_owned();
            } else if add_provider_dialog.take().is_some() {
                *status = "add provider cancelled".to_owned();
            } else if provider_dialog.take().is_some() {
                *status = "provider picker closed".to_owned();
            } else if context_dialog.take().is_some() {
                *status = "context dialog closed".to_owned();
            } else if passport_dialog.take().is_some() {
                *status = "login dialog closed".to_owned();
            } else {
                cancel.cancel();
            }
        }
        Some(view::Hit::TrafficMinimize) => {
            if let Some(dialog) = folder_picker.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = project_picker.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = session_picker.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = mode_menu.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = tool_detail_dialog.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = tool_dialog.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = theme_dialog.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = add_provider_dialog.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = provider_dialog.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if let Some(dialog) = context_dialog.as_mut() {
                dialog.minimized = true;
                dialog.fullscreen = false;
            } else if passport_dialog.is_some() {
                *passport_dialog = None;
            }
        }
        Some(view::Hit::TrafficOpenBrowser) => {
            if let Some(dialog) = folder_picker.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = project_picker.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = session_picker.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = mode_menu.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = tool_detail_dialog.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = tool_dialog.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = theme_dialog.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(dialog) = add_provider_dialog.as_mut() {
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
            } else if let Some(dialog) = context_dialog.as_mut() {
                if dialog.minimized {
                    dialog.minimized = false;
                } else {
                    dialog.fullscreen = !dialog.fullscreen;
                }
            } else if let Some(url) = passport_url {
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                let _ = std::process::Command::new("open").arg(url).spawn();
                *passport_browser_opened = true;
                if let Some(dialog) = passport_dialog.as_mut() {
                    dialog.opened_browser = true;
                }
                *status = format!("已打开 {url}");
            }
        }
        _ => {}
    }
}

fn paste_clipboard(input: &mut String, preview: &mut Option<String>) {
    if let Some((image_preview, bytes)) = read_clipboard_image_preview() {
        input.push_str(&format!("[图片 {}]", format_bytes(bytes)));
        *preview = Some(format!(
            "剪贴板图片（{}）：\n{}",
            format_bytes(bytes),
            image_preview
        ));
    } else if let Some(text) = read_clipboard_text() {
        insert_pasted_text(input, preview, &text);
    } else {
        *preview = Some("无法读取剪贴板，请使用终端的 Ctrl+Shift+V 粘贴。".to_owned());
    }
}

fn insert_pasted_text(input: &mut String, preview: &mut Option<String>, text: &str) {
    if let Some((image_preview, bytes)) = read_clipboard_image_preview() {
        input.push_str(&format!("[图片 {}]", format_bytes(bytes)));
        *preview = Some(format!(
            "剪贴板图片（{}）：\n{}",
            format_bytes(bytes),
            image_preview
        ));
        return;
    }
    const LARGE_PASTE_BYTES: usize = 512;
    if text.len() < LARGE_PASTE_BYTES {
        input.push_str(text);
        *preview = None;
        return;
    }
    let size = format_bytes(text.len());
    let first_lines = text.lines().take(6).collect::<Vec<_>>().join("\n");
    input.push_str(&format!("[粘贴 {size} 内容]"));
    *preview = Some(format!(
        "剪贴板内容（预览）：\n{}",
        truncate_preview(&first_lines, 480)
    ));
}

fn format_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

fn truncate_preview(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if out.len() + ch.len_utf8() > max_bytes {
            break;
        }
        out.push(ch);
    }
    if out.len() < text.len() {
        out.push_str("…");
    }
    out
}

fn read_clipboard_image_preview() -> Option<(String, usize)> {
    let output = std::process::Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.is_empty() {
        return None;
    }
    let bytes = output.stdout;
    let temp = std::env::temp_dir().join(format!("blora-paste-{}.png", std::process::id()));
    std::fs::write(&temp, &bytes).ok()?;
    let preview = std::process::Command::new("convert")
        .arg(&temp)
        .args([
            "-resize",
            "48x18!",
            "-colorspace",
            "Gray",
            "-contrast-stretch",
            "2%x2%",
            "txt:-",
        ])
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
        })
        .map(|text| ascii_image_preview(&text))
        .unwrap_or_else(|| "[终端不支持图像预览，请使用外部查看器打开图片]".to_owned());
    let _ = std::fs::remove_file(temp);
    Some((preview, bytes.len()))
}

fn ascii_image_preview(text: &str) -> String {
    let shades: Vec<char> = "@%#*+=-:. ".chars().collect();
    let mut pixels = Vec::new();
    let mut width = 0usize;
    let mut height = 0usize;
    for line in text.lines().skip(1) {
        let Some((coordinate, rgba)) = line.split_once(": (") else {
            continue;
        };
        let mut coordinates = coordinate.split(',');
        let Some(x) = coordinates
            .next()
            .and_then(|value| value.parse::<usize>().ok())
        else {
            continue;
        };
        let Some(y) = coordinates
            .next()
            .and_then(|value| value.parse::<usize>().ok())
        else {
            continue;
        };
        let Some(rgb) = rgba.split(')').next() else {
            continue;
        };
        let channels: Vec<u8> = rgb
            .split(',')
            .take(3)
            .filter_map(|part| {
                let value = part.trim().trim_end_matches('%').parse::<f32>().ok()?;
                Some(if part.trim().ends_with('%') {
                    (value * 2.55).round() as u8
                } else {
                    value.clamp(0.0, 255.0) as u8
                })
            })
            .collect();
        if channels.len() >= 1 {
            let gray = channels[0];
            let pixel = if channels.len() >= 3 {
                let luminance = (u32::from(channels[0]) * 299
                    + u32::from(channels[1]) * 587
                    + u32::from(channels[2]) * 114)
                    / 1000;
                luminance as u8
            } else {
                gray
            };
            width = width.max(x + 1);
            height = height.max(y + 1);
            let index =
                (255usize.saturating_sub(u32::from(pixel) as usize)) * (shades.len() - 1) / 255;
            pixels.push((x, y, shades[index]));
        }
    }
    if pixels.is_empty() || width == 0 || height == 0 {
        return "[无法生成图像预览]".to_owned();
    }
    let mut canvas = vec![vec![' '; width]; height];
    for (x, y, glyph) in pixels {
        canvas[y][x] = glyph;
    }
    canvas
        .into_iter()
        .map(|row| row.into_iter().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn read_clipboard_text() -> Option<String> {
    for (program, args) in [
        ("wl-paste", vec!["--no-newline"]),
        ("xclip", vec!["-selection", "clipboard", "-o"]),
        ("xsel", vec!["--clipboard", "--output"]),
        ("pbpaste", Vec::new()),
    ] {
        let Ok(output) = std::process::Command::new(program).args(args).output() else {
            continue;
        };
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }
    None
}

fn copy_text_to_clipboard(text: &str) -> String {
    for (program, args) in [
        ("wl-copy", &[] as &[&str]),
        ("xclip", &["-selection", "clipboard"] as &[&str]),
        ("xsel", &["--clipboard", "--input"] as &[&str]),
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
    format!("no clipboard tool; selected {} chars", text.chars().count())
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
        ("xsel", &["--clipboard", "--input"] as &[&str]),
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
