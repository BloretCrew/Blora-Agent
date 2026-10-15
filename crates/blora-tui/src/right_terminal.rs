// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! An independent PTY shell and its VT100 screen, without UI layout policy.

use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize};

enum Output {
    Bytes(Vec<u8>),
    Error(String),
}

pub struct TerminalPane {
    parser: vt100::Parser,
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    output: mpsc::Receiver<Output>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    exit_status: Option<ExitStatus>,
    error: Option<String>,
    pid: Option<u32>,
    pending_input: Vec<u8>,
    input_offset: usize,
}

impl TerminalPane {
    /// Starts $SHELL (or /bin/sh) interactively in a new controlling PTY.
    #[cfg(unix)]
    pub fn spawn(cwd: &Path, cols: u16, rows: u16) -> Result<Self, String> {
        use nix::fcntl::{FcntlArg, OFlag, fcntl};

        if !cwd.is_dir() {
            return Err(format!(
                "terminal working directory does not exist: {}",
                cwd.display()
            ));
        }
        let pair = portable_pty::native_pty_system()
            .openpty(size(cols, rows))
            .map_err(|e| format!("open terminal PTY: {e}"))?;
        let fd = pair
            .master
            .as_raw_fd()
            .ok_or("PTY has no file descriptor")?;
        let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(|e| e.to_string())?;
        fcntl(
            fd,
            FcntlArg::F_SETFL(OFlag::from_bits_truncate(flags) | OFlag::O_NONBLOCK),
        )
        .map_err(|e| format!("configure terminal reader: {e}"))?;
        let reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
        let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
        let shell = std::env::var_os("SHELL")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "/bin/sh".into());
        let mut command = CommandBuilder::new(shell);
        command.arg("-i");
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| format!("start terminal shell: {e}"))?;
        drop(pair.slave);
        let pid = child.process_id();
        let (tx, output) = mpsc::sync_channel(64);
        let stop = Arc::new(AtomicBool::new(false));
        let mut pane = Self {
            parser: vt100::Parser::new(rows.max(1), cols.max(1), 2000),
            master: Some(pair.master),
            writer: Some(writer),
            child,
            output,
            stop: stop.clone(),
            reader: None,
            exit_status: None,
            error: None,
            pid,
            pending_input: Vec::new(),
            input_offset: 0,
        };
        pane.reader = Some(
            thread::Builder::new()
                .name("blora-right-terminal".into())
                .spawn(move || read_output(reader, tx, stop))
                .map_err(|e| format!("start terminal reader: {e}"))?,
        );
        Ok(pane)
    }

    #[cfg(not(unix))]
    pub fn spawn(_cwd: &Path, _cols: u16, _rows: u16) -> Result<Self, String> {
        Err("the right terminal currently requires a Unix PTY".into())
    }

    /// Drains a bounded amount of output and checks shell status without waiting.
    pub fn poll(&mut self) {
        self.drain_output();
        if self.writer.is_some() {
            if let Err(error) = self.flush_input() {
                self.error = Some(error);
            }
        }
        if self.exit_status.is_none() {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    self.exit_status = Some(status);
                    self.shutdown();
                    self.drain_output();
                }
                Ok(None) => {}
                Err(error) => {
                    self.error = Some(format!("wait for terminal shell: {error}"));
                    self.shutdown();
                }
            }
        }
    }

    fn drain_output(&mut self) {
        for _ in 0..128 {
            match self.output.try_recv() {
                Ok(Output::Bytes(bytes)) => self.parser.process(&bytes),
                Ok(Output::Error(error)) => self.error = Some(error),
                Err(_) => break,
            }
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }
    pub fn exit_status(&self) -> Option<&ExitStatus> {
        self.exit_status.as_ref()
    }
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
    pub fn process_id(&self) -> Option<u32> {
        self.pid
    }

    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        let result = self
            .master
            .as_ref()
            .ok_or("terminal is closed")?
            .resize(size(cols, rows))
            .map_err(|e| format!("resize terminal: {e}"));
        if result.is_ok() {
            self.parser.set_size(rows.max(1), cols.max(1));
        }
        self.record(result)
    }

    /// Sends literal bytes, including control characters such as ETX (Ctrl+C).
    pub fn input(&mut self, bytes: &[u8]) -> Result<(), String> {
        let result = if self.writer.is_none() {
            Err("terminal is closed".into())
        } else if self.pending_input.len() - self.input_offset + bytes.len() > 4 * 1024 * 1024 {
            Err("terminal input buffer is full".into())
        } else {
            self.pending_input.drain(..self.input_offset);
            self.input_offset = 0;
            self.pending_input.extend_from_slice(bytes);
            self.flush_input()
        };
        self.record(result)
    }

    fn flush_input(&mut self) -> Result<(), String> {
        let writer = self.writer.as_mut().ok_or("terminal is closed")?;
        while self.input_offset < self.pending_input.len() {
            match writer.write(&self.pending_input[self.input_offset..]) {
                Ok(0) => return Err("terminal input closed".into()),
                Ok(count) => self.input_offset += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
                Err(error) => return Err(format!("write terminal input: {error}")),
            }
        }
        self.pending_input.clear();
        self.input_offset = 0;
        Ok(())
    }

    /// Wraps paste only when the application has requested bracketed paste.
    pub fn paste(&mut self, text: &str) -> Result<(), String> {
        let text = text.replace("\r\n", "\n").replace('\n', "\r");
        if self.screen().bracketed_paste() {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.input(&bytes)
        } else {
            self.input(text.as_bytes())
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Result<(), String> {
        self.input(&key_to_bytes(key, self.screen().application_cursor()))
    }

    fn record(&mut self, result: Result<(), String>) -> Result<(), String> {
        if let Err(error) = &result {
            self.error = Some(error.clone());
        }
        result
    }

    fn shutdown(&mut self) {
        if self.master.is_none() && self.reader.is_none() {
            return;
        }
        self.stop.store(true, Ordering::Release);
        #[cfg(unix)]
        {
            use nix::{
                sys::signal::{Signal, killpg},
                unistd::Pid,
            };
            // Foreground jobs may have a different process group from the shell.
            if let Some(group) = self
                .master
                .as_ref()
                .and_then(|m| m.process_group_leader())
                .filter(|group| *group > 0)
            {
                let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
            }
            if let Some(pid) = self.pid {
                kill_session(pid);
                let _ = killpg(Pid::from_raw(pid as i32), Signal::SIGKILL);
            }
        }
        if self.exit_status.is_none() {
            let _ = self.child.kill();
            match self.child.wait() {
                Ok(status) => self.exit_status = Some(status),
                Err(error) => self.error = Some(format!("reap terminal shell: {error}")),
            }
        }
        self.writer.take();
        self.pending_input.clear();
        self.input_offset = 0;
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        self.master.take();
    }
}

impl Drop for TerminalPane {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn size(cols: u16, rows: u16) -> PtySize {
    PtySize {
        rows: rows.max(1),
        cols: cols.max(1),
        pixel_width: 0,
        pixel_height: 0,
    }
}

#[cfg(unix)]
fn kill_session(shell: u32) {
    // Linux job-control shells give background jobs their own process groups.
    #[cfg(target_os = "linux")]
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
                continue;
            };
            let Some((_, fields)) = stat.rsplit_once(") ") else {
                continue;
            };
            let fields: Vec<_> = fields.split_whitespace().collect();
            if fields.get(3).and_then(|s| s.parse::<u32>().ok()) == Some(shell) {
                if let Some(group) = fields.get(2).and_then(|s| s.parse::<i32>().ok()) {
                    let _ = nix::sys::signal::killpg(
                        nix::unistd::Pid::from_raw(group),
                        nix::sys::signal::Signal::SIGKILL,
                    );
                }
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = shell;
}

fn read_output(
    mut reader: Box<dyn Read + Send>,
    tx: mpsc::SyncSender<Output>,
    stop: Arc<AtomicBool>,
) {
    let mut buffer = [0; 8192];
    loop {
        let stopping = stop.load(Ordering::Acquire);
        let message = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => Output::Bytes(buffer[..count].to_vec()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stopping {
                    break;
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            // Linux PTY masters return EIO when the last slave is closed.
            Err(error) if error.raw_os_error() == Some(5) => break,
            Err(error) => {
                let _ = tx.try_send(Output::Error(format!("read terminal output: {error}")));
                break;
            }
        };
        let mut message = message;
        loop {
            match tx.try_send(message) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Disconnected(_)) => return,
                Err(mpsc::TrySendError::Full(pending)) => {
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    message = pending;
                    thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
}

/// Converts crossterm press/repeat events into xterm-compatible input bytes.
/// `application_cursor` comes from `pane.screen().application_cursor()`.
pub fn key_to_bytes(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    if key.kind == KeyEventKind::Release {
        return Vec::new();
    }
    let modifiers = key.modifiers;
    let control = modifiers.contains(KeyModifiers::CONTROL);
    let alt = modifiers.contains(KeyModifiers::ALT);
    let shift = modifiers.contains(KeyModifiers::SHIFT);
    let modifier = 1 + u8::from(shift) + 2 * u8::from(alt) + 4 * u8::from(control);
    let mut bytes = match key.code {
        KeyCode::Char(c) if control => match c.to_ascii_lowercase() {
            'a'..='z' => vec![c.to_ascii_lowercase() as u8 - b'a' + 1],
            ' ' | '@' | '2' => vec![0],
            '[' | '3' => vec![27],
            '\\' | '4' => vec![28],
            ']' | '5' => vec![29],
            '^' | '6' => vec![30],
            '_' | '7' | '/' => vec![31],
            '?' | '8' => vec![127],
            _ => c.to_string().into_bytes(),
        },
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Tab if shift => b"\x1b[Z".to_vec(),
        KeyCode::Tab => vec![b'\t'],
        KeyCode::BackTab => b"\x1b[Z".to_vec(),
        KeyCode::Backspace => vec![if control { 8 } else { 127 }],
        KeyCode::Esc => vec![27],
        KeyCode::Up
        | KeyCode::Down
        | KeyCode::Right
        | KeyCode::Left
        | KeyCode::Home
        | KeyCode::End => {
            let final_char = match key.code {
                KeyCode::Up => 'A',
                KeyCode::Down => 'B',
                KeyCode::Right => 'C',
                KeyCode::Left => 'D',
                KeyCode::Home => 'H',
                _ => 'F',
            };
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{final_char}").into_bytes()
            } else {
                format!(
                    "\x1b{}{final_char}",
                    if application_cursor { 'O' } else { '[' }
                )
                .into_bytes()
            };
        }
        KeyCode::Insert
        | KeyCode::Delete
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::F(5..=12) => {
            let number = match key.code {
                KeyCode::Insert => 2,
                KeyCode::Delete => 3,
                KeyCode::PageUp => 5,
                KeyCode::PageDown => 6,
                KeyCode::F(n) => [15, 17, 18, 19, 20, 21, 23, 24][usize::from(n - 5)],
                _ => unreachable!(),
            };
            return if modifier > 1 {
                format!("\x1b[{number};{modifier}~").into_bytes()
            } else {
                format!("\x1b[{number}~").into_bytes()
            };
        }
        KeyCode::F(n @ 1..=4) => {
            let suffix = char::from(b'P' + n - 1);
            return if modifier > 1 {
                format!("\x1b[1;{modifier}{suffix}").into_bytes()
            } else {
                format!("\x1bO{suffix}").into_bytes()
            };
        }
        _ => return Vec::new(),
    };
    if alt {
        bytes.insert(0, 27);
    }
    bytes
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    fn wait_for(pane: &mut TerminalPane, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            pane.poll();
            if pane.screen().contents().contains(text) {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "missing {text:?}; screen: {:?}; error: {:?}",
                pane.screen().contents(),
                pane.error()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn key_encoding() {
        let key = |code, modifiers| KeyEvent::new(code, modifiers);
        assert_eq!(
            key_to_bytes(key(KeyCode::Char('c'), KeyModifiers::CONTROL), false),
            b"\x03"
        );
        assert_eq!(
            key_to_bytes(key(KeyCode::Char('界'), KeyModifiers::NONE), false),
            "界".as_bytes()
        );
        assert_eq!(
            key_to_bytes(key(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
        assert_eq!(
            key_to_bytes(key(KeyCode::Left, KeyModifiers::CONTROL), false),
            b"\x1b[1;5D"
        );
        assert_eq!(
            key_to_bytes(key(KeyCode::Char('x'), KeyModifiers::ALT), false),
            b"\x1bx"
        );
        assert!(
            key_to_bytes(
                KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::NONE, KeyEventKind::Release),
                false
            )
            .is_empty()
        );
    }

    #[test]
    fn pty_io_color_resize_paste_interrupt_exit() {
        let mut pane = TerminalPane::spawn(Path::new("/tmp"), 90, 30).unwrap();
        pane.input(b"stty -echo; printf '\\033[2J\\033[H\\033[31mPTY_%s\\033[0m\\n' RED\r")
            .unwrap();
        wait_for(&mut pane, "PTY_RED");
        assert_eq!(
            pane.screen().cell(0, 0).unwrap().fgcolor(),
            vt100::Color::Idx(1)
        );
        pane.resize(101, 37).unwrap();
        assert_eq!(pane.screen().size(), (37, 101));
        pane.input(b"stty size\r").unwrap();
        wait_for(&mut pane, "37 101");
        pane.paste("printf 'PASTE_OK\\n'").unwrap();
        pane.input(b"\r").unwrap();
        wait_for(&mut pane, "PASTE_OK");
        pane.input(b"sleep 30; printf 'INTERRUPTED_OK\\n'\r")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pane.master.as_ref().unwrap().process_group_leader() == pane.pid.map(|pid| pid as i32)
        {
            pane.poll();
            assert!(Instant::now() < deadline, "foreground sleep did not start");
            thread::sleep(Duration::from_millis(10));
        }
        pane.input(b"\x03").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pane.master.as_ref().unwrap().process_group_leader() != pane.pid.map(|pid| pid as i32)
        {
            pane.poll();
            assert!(Instant::now() < deadline, "shell did not regain terminal");
            thread::sleep(Duration::from_millis(10));
        }
        pane.input(b"printf 'SHELL_ALIVE\\n'\r").unwrap();
        wait_for(&mut pane, "SHELL_ALIVE");
        assert!(!pane.screen().contents().contains("INTERRUPTED_OK"));
        pane.input(b"exit 7\r").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pane.exit_status().is_none() && Instant::now() < deadline {
            pane.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(pane.exit_status().unwrap().exit_code(), 7);
        assert!(pane.reader.is_none());
        assert!(pane.input(b"x").is_err());
    }

    #[test]
    fn invalid_cwd_and_zero_size() {
        assert!(TerminalPane::spawn(Path::new("/nonexistent/blora-terminal"), 80, 24).is_err());
        let mut pane = TerminalPane::spawn(Path::new("/tmp"), 0, 0).unwrap();
        assert_eq!(pane.screen().size(), (1, 1));
        pane.resize(0, 0).unwrap();
        assert_eq!(pane.screen().size(), (1, 1));
    }

    #[test]
    fn cwd_and_final_output_are_preserved() {
        let mut pane = TerminalPane::spawn(Path::new("/tmp"), 100, 30).unwrap();
        pane.input(b"stty -echo; pwd; printf 'FINAL_%s\\n' OUTPUT; exit 0\r")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while pane.exit_status().is_none() && Instant::now() < deadline {
            pane.poll();
            thread::sleep(Duration::from_millis(10));
        }
        assert!(pane.exit_status().unwrap().success());
        assert!(pane.screen().contents().contains("/tmp"));
        assert!(pane.screen().contents().contains("FINAL_OUTPUT"));
        assert!(pane.reader.is_none());
    }

    #[test]
    fn drop_does_not_wait_for_full_output_channel() {
        let mut pane = TerminalPane::spawn(Path::new("/tmp"), 80, 24).unwrap();
        pane.input(b"yes flood\r").unwrap();
        thread::sleep(Duration::from_millis(200));
        let start = Instant::now();
        drop(pane);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn drop_kills_foreground_background_jobs_and_reader() {
        let mut pane = TerminalPane::spawn(Path::new("/tmp"), 100, 30).unwrap();
        let pid = pane.process_id().unwrap();
        pane.input(b"stty -echo; sleep 60 & printf 'BACKGROUND=%s\\n' $!; sleep 60\r")
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let background = loop {
            pane.poll();
            let screen = pane.screen().contents();
            if let Some(pid) = screen.lines().find_map(|line| {
                line.split_once("BACKGROUND=")
                    .and_then(|(_, value)| value.trim().parse::<u32>().ok())
            }) {
                break pid;
            }
            assert!(
                Instant::now() < deadline,
                "background PID missing: {screen:?}"
            );
            thread::sleep(Duration::from_millis(10));
        };
        let start = Instant::now();
        drop(pane);
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let stat = std::fs::read_to_string(format!("/proc/{background}/stat"));
            if stat.is_err() || stat.unwrap().rsplit_once(") ").unwrap().1.starts_with('Z') {
                break;
            }
            assert!(Instant::now() < deadline, "background job still running");
            thread::sleep(Duration::from_millis(10));
        }
    }
}
