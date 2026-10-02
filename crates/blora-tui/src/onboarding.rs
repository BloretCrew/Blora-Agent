// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};

use blora_auth::{DeviceCode, PassportConfig, PassportUser};
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
};

pub const VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Welcome,
    Account,
    Complete,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Next,
    Login,
    Skip,
    Back,
    Cancel,
    Browser,
    Copy,
    Finish,
    Close,
}

struct Attempt {
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Message>,
}
enum Message {
    Device(DeviceCode),
    User(PassportUser),
    Error(String),
}
impl Drop for Attempt {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub struct Onboarding {
    step: Step,
    selected: usize,
    feedback: String,
    device: Option<DeviceCode>,
    attempt: Option<Attempt>,
    hits: Vec<(Rect, Action)>,
    scroll: u16,
    fullscreen: bool,
    minimized: bool,
    pointer: Option<(u16, u16)>,
    controls: [Option<Rect>; 3],
}

#[derive(PartialEq, Eq, Debug)]
pub enum Outcome {
    Stay,
    Close,
    Finish,
    Quit,
}

pub fn needs_onboarding(version: u32, valid_account: bool) -> bool {
    version < VERSION && !valid_account
}

impl Onboarding {
    pub fn new(account_only: bool) -> Self {
        Self {
            step: if account_only {
                Step::Account
            } else {
                Step::Welcome
            },
            selected: 0,
            feedback: String::new(),
            device: None,
            attempt: None,
            hits: Vec::new(),
            scroll: 0,
            fullscreen: false,
            minimized: false,
            pointer: None,
            controls: [None; 3],
        }
    }

    fn actions(&self, logged_in: bool) -> Vec<(Action, &'static str)> {
        match self.step {
            Step::Welcome => vec![
                (Action::Next, "开始设置"),
                (Action::Close, "退出引导（不保存）"),
            ],
            Step::Account if self.attempt.is_some() => vec![
                (Action::Cancel, "取消登录"),
                (Action::Browser, "打开浏览器"),
                (Action::Copy, "复制授权地址和设备码"),
            ],
            Step::Account => {
                let mut actions = Vec::new();
                if logged_in {
                    actions.push((Action::Next, "使用当前账号继续"));
                }
                actions.extend([
                    (Action::Login, "登录 Bloret PassPort / 重试"),
                    (Action::Skip, "稍后设置（保留当前供应商）"),
                    (Action::Back, "返回欢迎"),
                    (Action::Close, "关闭（不保存）"),
                ]);
                actions
            }
            Step::Complete => vec![(Action::Finish, "开始使用"), (Action::Back, "返回账号")],
        }
    }

    fn start(&mut self) {
        self.attempt = None;
        self.device = None;
        self.feedback = "正在请求设备码…".to_owned();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = (|| {
                let config = PassportConfig::from_env()?;
                let device = config.request_device_code()?;
                if flag.load(Ordering::Relaxed) {
                    return Err(blora_auth::AuthError::Cancelled);
                }
                let _ = sender.send(Message::Device(device.clone()));
                config.poll_device_cancellable(&device, &flag)
            })();
            let message = match result {
                Ok(user) => Message::User(user),
                Err(err) => Message::Error(err.to_string()),
            };
            let _ = sender.send(message);
        });
        self.attempt = Some(Attempt { cancel, receiver });
        self.selected = 0;
    }

    pub fn poll(&mut self) -> Option<PassportUser> {
        loop {
            let message = self.attempt.as_ref()?.receiver.try_recv();
            match message {
                Ok(Message::Device(device)) => {
                    self.device = Some(device);
                    self.feedback = "等待浏览器授权；可取消后重试".to_owned();
                }
                Ok(Message::User(user)) => {
                    self.attempt = None;
                    self.device = None;
                    self.feedback = format!("登录成功：{}", user.display_name());
                    self.selected = 0;
                    return Some(user);
                }
                Ok(Message::Error(error)) => {
                    self.attempt = None;
                    self.device = None;
                    self.feedback = format!("登录失败：{error}。可重试或稍后设置。");
                    self.selected = 0;
                }
                Err(mpsc::TryRecvError::Empty) => return None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.attempt = None;
                    self.feedback = "登录线程已停止，请重试".to_owned();
                    return None;
                }
            }
        }
    }

    pub fn error(&mut self, text: String) {
        self.feedback = text;
    }

    fn activate(&mut self, action: Action) -> Outcome {
        match action {
            Action::Next | Action::Skip => {
                self.step = if self.step == Step::Welcome {
                    Step::Account
                } else {
                    Step::Complete
                };
                self.selected = 0;
                self.scroll = 0;
            }
            Action::Login => self.start(),
            Action::Back => {
                self.attempt = None;
                self.device = None;
                self.step = if self.step == Step::Complete {
                    Step::Account
                } else {
                    Step::Welcome
                };
                self.selected = 0;
            }
            Action::Cancel => {
                self.attempt = None;
                self.device = None;
                self.feedback = "登录已取消；可重试或稍后设置".to_owned();
                self.selected = 0;
            }
            Action::Browser => {
                if let Some(device) = &self.device {
                    blora_runtime::open_url(&device.verification_uri);
                }
            }
            Action::Copy => {
                if let Some(device) = &self.device {
                    self.feedback = super::copy_text_to_clipboard(&format!(
                        "{}\n设备码：{}",
                        device.verification_uri, device.user_code
                    ));
                }
            }
            Action::Finish => return Outcome::Finish,
            Action::Close => return Outcome::Close,
        }
        Outcome::Stay
    }

    pub fn event(&mut self, event: Event, logged_in: bool) -> Outcome {
        let actions = self.actions(logged_in);
        if let Event::Mouse(mouse) = event {
            self.pointer = Some((mouse.column, mouse.row));
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                if let Some(index) = self.controls.iter().position(|rect| {
                    rect.is_some_and(|rect| rect.contains((mouse.column, mouse.row).into()))
                }) {
                    match index {
                        0 => return Outcome::Close,
                        1 => self.minimized = !self.minimized,
                        _ => {
                            self.fullscreen = !self.fullscreen;
                            self.minimized = false;
                        }
                    }
                    return Outcome::Stay;
                }
            }
        }
        if self.minimized {
            match event {
                Event::Key(key) if key.code == KeyCode::Enter => self.minimized = false,
                Event::Key(key) if key.code == KeyCode::Esc => return Outcome::Close,
                Event::Key(key)
                    if matches!(key.code, KeyCode::Char('c' | 'q'))
                        && key.modifiers.contains(KeyModifiers::CONTROL) =>
                {
                    return Outcome::Quit;
                }
                _ => {}
            }
            return Outcome::Stay;
        }
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('c' | 'q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Outcome::Quit;
                }
                KeyCode::PageDown => self.scroll = self.scroll.saturating_add(1),
                KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(1),
                KeyCode::Esc => {
                    return if self.attempt.is_some() {
                        self.activate(Action::Cancel)
                    } else {
                        Outcome::Close
                    };
                }
                KeyCode::Tab | KeyCode::Down | KeyCode::Right => {
                    self.selected = (self.selected + 1) % actions.len()
                }
                KeyCode::BackTab | KeyCode::Up | KeyCode::Left => {
                    self.selected = (self.selected + actions.len() - 1) % actions.len()
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    return self.activate(actions[self.selected.min(actions.len() - 1)].0);
                }
                _ => {}
            },
            Event::Mouse(mouse) if mouse.kind == MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, action)) = self
                    .hits
                    .iter()
                    .find(|(rect, _)| rect.contains((mouse.column, mouse.row).into()))
                {
                    return self.activate(*action);
                }
            }
            _ => {}
        }
        Outcome::Stay
    }

    pub fn draw(&mut self, frame: &mut Frame, username: &str, logged_in: bool) {
        let area = frame.area();
        let theme = super::theme::Theme::current();
        frame.render_widget(
            Paragraph::new("").style(Style::default().bg(theme.bg).fg(theme.text)),
            area,
        );
        self.hits.clear();
        self.controls = [None; 3];
        if area.width >= 50 && area.height >= 18 {
            self.draw_dialog(frame, area, username, logged_in, &theme);
            return;
        }
        if area.width < 4 || area.height < 4 {
            frame.render_widget(Paragraph::new("请扩大终端；Esc 关闭"), area);
            return;
        }
        let actions = self.actions(logged_in);
        let visible = actions.len().min(area.height.saturating_sub(3) as usize);
        let body_height = area.height.saturating_sub(visible as u16 + 1);
        let title = match self.step {
            Step::Welcome => "欢迎使用 Blora · 1/3",
            Step::Account => "连接账号 · 2/3",
            Step::Complete => "设置完成 · 3/3",
        };
        let description = match self.step {
            Step::Welcome => {
                "Blora 帮助你编写代码、处理任务与生成图像。设置账号，或稍后使用已有供应商。"
                    .to_owned()
            }
            Step::Account if logged_in => format!("当前账号：{username}。继续不会修改供应商设置。"),
            Step::Account => {
                "按登录后才会申请设备码。已有自定义供应商或环境变量凭据可选择稍后设置。".to_owned()
            }
            Step::Complete if logged_in => {
                format!("账号：{username}。点击开始使用保存引导完成状态。")
            }
            Step::Complete => {
                "已选择稍后设置。供应商配置保持不变，PassPort 使用前仍需登录。".to_owned()
            }
        };
        let device = self
            .device
            .as_ref()
            .map(|d| format!("\n设备码：{}\n{}", d.user_code, d.verification_uri))
            .unwrap_or_default();
        frame.render_widget(
            Paragraph::new(format!("{title}{device}\n{}\n{description}", self.feedback))
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            Rect::new(area.x, area.y, area.width, body_height),
        );
        let start = if self.selected >= visible {
            self.selected + 1 - visible
        } else {
            0
        };
        for (row, (action, label)) in actions.iter().enumerate().skip(start).take(visible) {
            let rect = Rect::new(
                area.x,
                area.y + body_height + (row - start) as u16,
                area.width,
                1,
            );
            let style = if self.selected == row {
                Style::default().fg(theme.rose).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {label}",
                    if self.selected == row { "›" } else { " " }
                ))
                .style(style),
                rect,
            );
            self.hits.push((rect, *action));
        }
        frame.render_widget(
            Paragraph::new("Tab/方向键 · Enter确认 · PgUp/Dn滚动 · Esc取消 · Ctrl+Q退出"),
            Rect::new(area.x, area.y + area.height - 1, area.width, 1),
        );
    }
    fn draw_dialog(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        username: &str,
        logged_in: bool,
        theme: &super::theme::Theme,
    ) {
        let Some(modal) = super::view::dialog_outer(area, 82, 26, self.fullscreen, self.minimized)
        else {
            return;
        };
        let mut chrome = super::view::HitMap::default();
        let inner = super::view::paint_dialog_frame(
            frame,
            modal,
            "Blora · 首次使用",
            self.pointer,
            theme,
            &mut chrome,
        );
        self.controls = chrome.traffic_lights;
        if self.minimized {
            return;
        }
        let content = Rect::new(
            inner.x + 2,
            inner.y + 2,
            inner.width.saturating_sub(4),
            inner.height.saturating_sub(3),
        );
        let steps = ["欢迎", "账号", "完成"];
        let current = match self.step {
            Step::Welcome => 0,
            Step::Account => 1,
            Step::Complete => 2,
        };
        let mut tabs = Vec::new();
        for (index, label) in steps.iter().enumerate() {
            tabs.push(Span::styled(
                format!(" {} {} ", index + 1, label),
                if index == current {
                    theme.fg(theme.rose).add_modifier(Modifier::BOLD)
                } else {
                    theme.mute()
                },
            ));
            if index < 2 {
                tabs.push(Span::styled("  ·  ", theme.mute()));
            }
        }
        frame.render_widget(
            Paragraph::new(Line::from(tabs)).style(theme.base()),
            Rect::new(content.x, content.y, content.width, 1),
        );
        let actions = self.actions(logged_in);
        let action_height = actions.len() as u16;
        let body = Rect::new(
            content.x,
            content.y + 2,
            content.width,
            content.height.saturating_sub(action_height + 4),
        );
        let mut lines = match self.step {
            Step::Welcome => vec![
                Line::from(Span::styled(
                    "欢迎使用 Blora",
                    theme.fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(Span::styled(
                    "选择工作方式，在当前项目中开始。",
                    theme.fg(theme.text_dim),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled("Code     ", theme.fg(theme.rose)),
                    Span::raw("阅读项目、修改代码、运行测试"),
                ]),
                Line::from(vec![
                    Span::styled("Work     ", theme.fg(theme.rose)),
                    Span::raw("处理任务与后台工作"),
                ]),
                Line::from(vec![
                    Span::styled("Agent    ", theme.fg(theme.rose)),
                    Span::raw("分工执行与协作"),
                ]),
                Line::from(vec![
                    Span::styled("Imagine  ", theme.fg(theme.rose)),
                    Span::raw("描述画面，生成图像"),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "接下来设置账号。已有供应商配置会保留。",
                    theme.mute(),
                )),
            ],
            Step::Account => vec![
                Line::from(Span::styled(
                    "连接账号 · Bloret PassPort",
                    theme.fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(if logged_in {
                    format!("当前账号  {username}")
                } else {
                    "当前账号  尚未登录".to_owned()
                }),
                Line::from(Span::styled(
                    "在浏览器确认授权后，这里会自动更新。",
                    theme.mute(),
                )),
                Line::from(""),
            ],
            Step::Complete => vec![
                Line::from(Span::styled(
                    "设置完成",
                    theme.fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                Line::from(if logged_in {
                    format!("账号      {username}")
                } else {
                    "账号      稍后设置".to_owned()
                }),
                Line::from("供应商    保留当前配置"),
                Line::from("权限      沿用当前会话设置"),
                Line::from(""),
                Line::from(Span::styled(
                    "开始使用后保存引导记录，并返回原会话。",
                    theme.mute(),
                )),
                Line::from(Span::styled(
                    "/login 连接账号 · /onboarding 重开引导",
                    theme.mute(),
                )),
            ],
        };
        if let Some(device) = &self.device {
            lines.extend([
                Line::from(vec![
                    Span::styled("设备码    ", theme.mute()),
                    Span::styled(
                        device.user_code.clone(),
                        theme.fg(theme.rose).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(device.verification_uri.clone()),
                Line::from(Span::styled(
                    format!("有效期 {} 秒 · 等待授权", device.expires_in),
                    theme.mute(),
                )),
            ]);
        }
        if !self.feedback.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                self.feedback.clone(),
                theme.fg(theme.amber),
            )));
        }
        frame.render_widget(
            Paragraph::new(lines)
                .style(theme.base())
                .wrap(Wrap { trim: false })
                .scroll((self.scroll, 0)),
            body,
        );
        let actions_y = content.y + content.height.saturating_sub(action_height + 1);
        for (index, (action, label)) in actions.iter().enumerate() {
            let rect = Rect::new(content.x, actions_y + index as u16, content.width, 1);
            let selected = self.selected == index;
            let style = if selected {
                theme
                    .fg(theme.rose)
                    .bg(theme.bg_select)
                    .add_modifier(Modifier::BOLD)
            } else {
                theme.fg(theme.text_dim)
            };
            frame.render_widget(
                Paragraph::new(format!("{} {label}", if selected { "›" } else { " " }))
                    .style(style),
                rect,
            );
            self.hits.push((rect, *action));
        }
        frame.render_widget(
            Paragraph::new("Tab/↑↓ 选择 · Enter 确认 · PgUp/Dn 滚动 · Esc 关闭")
                .style(theme.mute()),
            Rect::new(content.x, content.y + content.height - 1, content.width, 1),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rendered(guide: &mut Onboarding, width: u16, height: u16, logged_in: bool) -> String {
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| guide.draw(frame, "测试账号", logged_in))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn dialog_uses_shared_chrome_and_step_content() {
        let mut guide = Onboarding::new(false);
        let welcome = rendered(&mut guide, 100, 32, false);
        assert!(welcome.contains("● ● ●"));
        assert!(welcome.replace(' ', "").contains("Blora·首次使用"));
        assert!(welcome.contains("Code"));
        assert!(welcome.contains("Imagine"));
        assert!(guide.controls.iter().all(Option::is_some));
        guide.step = Step::Complete;
        let complete = rendered(&mut guide, 100, 32, true);
        assert!(complete.replace(' ', "").contains("测试账号"));
        assert!(complete.replace(' ', "").contains("保留当前配置"));
    }

    #[test]
    fn authorization_details_and_errors_render_in_dialog() {
        let mut guide = Onboarding::new(true);
        guide.device = Some(DeviceCode::from_public(
            "private",
            "TEST-1234",
            "https://passport.example/authorize",
            600,
            5,
        ));
        guide.feedback = "等待授权".to_owned();
        let content = rendered(&mut guide, 100, 32, false);
        assert!(content.contains("TEST-1234"));
        assert!(content.contains("https://passport.example/authorize"));
        assert!(!content.contains("private"));
        guide.error("授权已过期，请重试".to_owned());
        assert!(
            rendered(&mut guide, 100, 32, false)
                .replace(' ', "")
                .contains("授权已过期")
        );
    }

    #[test]
    fn window_controls_minimize_restore_expand_and_close() {
        use crossterm::event::MouseEvent;
        let mut guide = Onboarding::new(false);
        rendered(&mut guide, 100, 32, false);
        let click = |rect: Rect| {
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: rect.x,
                row: rect.y,
                modifiers: KeyModifiers::NONE,
            })
        };
        assert_eq!(
            guide.event(click(guide.controls[1].unwrap()), false),
            Outcome::Stay
        );
        assert!(guide.minimized);
        rendered(&mut guide, 100, 32, false);
        assert!(guide.hits.is_empty());
        guide.event(
            Event::Key(crossterm::event::KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )),
            false,
        );
        assert!(!guide.minimized);
        rendered(&mut guide, 100, 32, false);
        guide.event(click(guide.controls[2].unwrap()), false);
        assert!(guide.fullscreen);
        rendered(&mut guide, 100, 32, false);
        assert_eq!(
            guide.event(click(guide.controls[0].unwrap()), false),
            Outcome::Close
        );
    }

    #[test]
    fn all_steps_fit_terminal_sizes_with_clickable_actions() {
        for (width, height) in [(4, 4), (20, 6), (49, 17), (50, 18), (80, 24), (120, 40)] {
            for step in [Step::Welcome, Step::Account, Step::Complete] {
                let mut guide = Onboarding::new(false);
                guide.step = step;
                rendered(&mut guide, width, height, true);
                assert!(!guide.hits.is_empty());
                for (rect, _) in &guide.hits {
                    assert!(rect.x + rect.width <= width);
                    assert!(rect.y + rect.height <= height);
                }
            }
        }
    }

    #[test]
    fn startup_and_version_policy() {
        assert!(needs_onboarding(0, false));
        assert!(!needs_onboarding(0, true));
        assert!(!needs_onboarding(VERSION, false));
        assert!(!needs_onboarding(VERSION + 1, false));
    }
    #[test]
    fn skip_requires_explicit_finish_and_does_not_request_login() {
        let mut guide = Onboarding::new(false);
        assert_eq!(guide.activate(Action::Next), Outcome::Stay);
        assert_eq!(guide.step, Step::Account);
        assert!(guide.attempt.is_none());
        assert_eq!(guide.activate(Action::Skip), Outcome::Stay);
        assert_eq!(guide.step, Step::Complete);
        assert_eq!(guide.activate(Action::Finish), Outcome::Finish);
    }
    #[test]
    fn cancellation_drops_old_results() {
        let mut guide = Onboarding::new(true);
        let flag = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        guide.attempt = Some(Attempt {
            cancel: flag.clone(),
            receiver,
        });
        assert_eq!(guide.activate(Action::Cancel), Outcome::Stay);
        assert!(flag.load(Ordering::Relaxed));
        assert!(sender.send(Message::Error("stale".into())).is_err());
        assert!(guide.feedback.contains("已取消"));
        assert!(guide.poll().is_none());
    }
    #[test]
    fn errors_allow_retry_and_small_terminal_actions_are_visible() {
        let mut guide = Onboarding::new(true);
        let (sender, receiver) = mpsc::channel();
        guide.attempt = Some(Attempt {
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
        });
        sender.send(Message::Error("expired_token".into())).unwrap();
        assert!(guide.poll().is_none());
        assert!(guide.feedback.contains("expired_token"));
        let backend = ratatui::backend::TestBackend::new(20, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        guide.selected = 3;
        terminal
            .draw(|frame| guide.draw(frame, "you", false))
            .unwrap();
        assert!(
            guide
                .hits
                .iter()
                .any(|(_, action)| *action == Action::Close)
        );
    }
    #[test]
    fn successful_account_waits_for_explicit_continue() {
        let mut guide = Onboarding::new(true);
        let (sender, receiver) = mpsc::channel();
        guide.attempt = Some(Attempt {
            cancel: Arc::new(AtomicBool::new(false)),
            receiver,
        });
        sender
            .send(Message::User(PassportUser {
                username: "test".into(),
                apptoken: Some("token".into()),
                ..Default::default()
            }))
            .unwrap();
        assert_eq!(guide.poll().unwrap().username, "test");
        assert_eq!(guide.step, Step::Account);
        assert!(
            guide
                .actions(true)
                .iter()
                .any(|(action, _)| *action == Action::Login)
        );
        assert_eq!(guide.activate(Action::Next), Outcome::Stay);
        assert_eq!(guide.step, Step::Complete);
    }

    #[test]
    fn keyboard_mouse_close_and_exit_do_not_finish() {
        use crossterm::event::{KeyEvent, MouseEvent};
        let mut guide = Onboarding::new(false);
        let backend = ratatui::backend::TestBackend::new(40, 10);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| guide.draw(frame, "you", false))
            .unwrap();
        let rect = guide.hits[0].0;
        assert_eq!(
            guide.event(
                Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: rect.x,
                    row: rect.y,
                    modifiers: KeyModifiers::NONE
                }),
                false
            ),
            Outcome::Stay
        );
        assert_eq!(guide.step, Step::Account);
        assert_eq!(
            guide.event(
                Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
                false
            ),
            Outcome::Close
        );
        assert_eq!(
            guide.event(
                Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL)),
                false
            ),
            Outcome::Quit
        );
    }

    #[test]
    fn dropping_guide_cancels_pending_attempt() {
        let mut guide = Onboarding::new(true);
        let flag = Arc::new(AtomicBool::new(false));
        let (_sender, receiver) = mpsc::channel();
        guide.attempt = Some(Attempt {
            cancel: flag.clone(),
            receiver,
        });
        drop(guide);
        assert!(flag.load(Ordering::Relaxed));
    }
}
