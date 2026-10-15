// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! State for the ask_user dialog. Rendering stays in `view`.

use std::path::Path;

use blora_events::AskedQuestion;
use blora_runtime::format_question_answers;
use blora_session::PendingInput;
use crossterm::event::KeyCode;

/// What a key or click did to the questionnaire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuestionAction {
    None,
    Dismiss,
    Submit(String),
}

#[derive(Clone, Debug)]
pub struct QuestionForm {
    pub request_id: String,
    pub owner_label: Option<String>,
    pub questions: Vec<AskedQuestion>,
    pub focus: usize,
    pub cursor: Vec<usize>,
    pub choice: Vec<Option<usize>>,
    pub checks: Vec<Vec<bool>>,
    pub other_text: Vec<String>,
    pub text: Vec<String>,
    pub picked: Vec<Option<String>>,
    pub browse: Vec<String>,
    pub entries: Vec<Vec<String>>,
    pub scroll: Vec<usize>,
    pub fullscreen: bool,
    pub minimized: bool,
}

impl QuestionForm {
    pub fn open(pending: &PendingInput, workspace: &Path) -> Self {
        let questions = if pending.questions.is_empty() {
            vec![AskedQuestion::from_legacy(
                &pending.question,
                &pending.options,
            )]
        } else {
            pending.questions.clone()
        };
        let count = questions.len();
        let mut form = Self {
            request_id: pending.request_id.clone(),
            owner_label: None,
            questions,
            focus: 0,
            cursor: vec![0; count],
            choice: vec![None; count],
            checks: Vec::with_capacity(count),
            other_text: vec![String::new(); count],
            text: vec![String::new(); count],
            picked: vec![None; count],
            browse: vec![String::new(); count],
            entries: vec![Vec::new(); count],
            scroll: vec![0; count],
            fullscreen: false,
            minimized: false,
        };
        for index in 0..count {
            form.checks
                .push(vec![false; form.questions[index].options.len()]);
            if matches!(form.questions[index].kind.as_str(), "folder" | "file") {
                form.reload(workspace, index);
            }
        }
        form
    }

    pub fn on_key(&mut self, code: KeyCode, workspace: &Path) -> QuestionAction {
        match code {
            KeyCode::Esc => QuestionAction::Dismiss,
            KeyCode::Tab => {
                self.shift_focus(1);
                QuestionAction::None
            }
            KeyCode::BackTab => {
                self.shift_focus(-1);
                QuestionAction::None
            }
            KeyCode::Up => {
                if self.should_switch_question(-1) {
                    self.focus_neighbor(-1);
                } else {
                    self.move_cursor(-1);
                }
                QuestionAction::None
            }
            KeyCode::Down => {
                if self.should_switch_question(1) {
                    self.focus_neighbor(1);
                } else {
                    self.move_cursor(1);
                }
                QuestionAction::None
            }
            KeyCode::Left => {
                if self.picker_blocks_left() {
                    self.picker_parent(workspace);
                } else {
                    self.shift_focus(-1);
                }
                QuestionAction::None
            }
            KeyCode::Right => {
                if self.focused_row_is_directory() {
                    self.enter_directory(workspace);
                } else {
                    self.shift_focus(1);
                }
                QuestionAction::None
            }
            KeyCode::Backspace => {
                self.backspace();
                QuestionAction::None
            }
            KeyCode::Char(ch) => {
                if ch == ' ' && !self.accepts_text() {
                    self.select_current(workspace);
                } else if self.accepts_text() {
                    self.push_char(ch);
                }
                QuestionAction::None
            }
            KeyCode::Enter => {
                if let Some(answer) = self.formatted() {
                    QuestionAction::Submit(answer)
                } else if self.focused_row_is_directory() {
                    self.enter_directory(workspace);
                    QuestionAction::None
                } else {
                    self.select_current(workspace);
                    QuestionAction::None
                }
            }
            _ => QuestionAction::None,
        }
    }

    pub fn click_option(&mut self, question: usize, option: usize) {
        if question >= self.questions.len() {
            return;
        }
        self.focus = question;
        self.cursor[question] = option;
        self.select_current(Path::new("."));
    }

    pub fn click_row(&mut self, row: usize, workspace: &Path) {
        let index = self.focus;
        if self.entries.get(index).is_some_and(|rows| row < rows.len()) {
            self.cursor[index] = row;
            self.select_current(workspace);
        }
    }

    pub fn click_tab(&mut self, index: usize) {
        if index < self.questions.len() {
            self.focus = index;
        }
    }

    /// Move within the current list. Wheel scrolling uses this so it does not change questions.
    pub fn scroll_rows(&mut self, delta: isize) {
        self.move_cursor(delta);
    }

    pub fn is_answered(&self, index: usize) -> bool {
        self.value_of(index).is_some()
    }

    pub fn paste(&mut self, text: &str) {
        if !self.accepts_text() {
            return;
        }
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                self.push_char(' ');
            } else {
                self.push_char(ch);
            }
        }
    }

    pub fn formatted(&self) -> Option<String> {
        let mut values = Vec::with_capacity(self.questions.len());
        for index in 0..self.questions.len() {
            values.push(self.value_of(index)?);
        }
        format_question_answers(&self.questions, &values).ok()
    }

    pub fn accepts_text(&self) -> bool {
        let Some(question) = self.questions.get(self.focus) else {
            return false;
        };
        match question.kind.as_str() {
            "text" => true,
            "single" | "boolean" => self.choice[self.focus].is_some_and(|index| {
                question
                    .options
                    .get(index)
                    .is_some_and(|choice| choice.other)
            }),
            "multi" => question
                .options
                .iter()
                .zip(self.checks[self.focus].iter())
                .any(|(choice, on)| choice.other && *on),
            _ => false,
        }
    }

    pub fn field_text(&self) -> &str {
        let Some(question) = self.questions.get(self.focus) else {
            return "";
        };
        if question.kind == "text" {
            &self.text[self.focus]
        } else {
            &self.other_text[self.focus]
        }
    }

    fn value_of(&self, index: usize) -> Option<String> {
        let question = &self.questions[index];
        match question.kind.as_str() {
            "single" | "boolean" => {
                let choice = self.choice[index]?;
                let option = question.options.get(choice)?;
                if option.other {
                    nonempty(&self.other_text[index])
                } else {
                    Some(option.label.clone())
                }
            }
            "multi" => {
                let mut parts = Vec::new();
                for (option, on) in question.options.iter().zip(self.checks[index].iter()) {
                    if !on {
                        continue;
                    }
                    if option.other {
                        parts.push(nonempty(&self.other_text[index])?);
                    } else {
                        parts.push(option.label.clone());
                    }
                }
                if parts.is_empty() {
                    None
                } else {
                    Some(parts.join(", "))
                }
            }
            "text" => nonempty(&self.text[index]),
            "folder" | "file" => self.picked[index].clone(),
            _ => None,
        }
    }

    fn shift_focus(&mut self, delta: isize) {
        let len = self.questions.len() as isize;
        if len == 0 {
            return;
        }
        self.focus = (self.focus as isize + delta).rem_euclid(len) as usize;
    }

    fn focus_neighbor(&mut self, delta: isize) {
        self.shift_focus(delta);
        let len = self.row_len();
        if len == 0 {
            return;
        }
        self.cursor[self.focus] = if delta < 0 { len - 1 } else { 0 };
        self.reveal_cursor();
    }

    /// Up/Down leave the question only from the ends, and only when another question exists.
    fn should_switch_question(&self, delta: isize) -> bool {
        if self.questions.len() <= 1 {
            return false;
        }
        let len = self.row_len();
        if len == 0 {
            return true;
        }
        let cursor = self.cursor[self.focus];
        if delta < 0 {
            cursor == 0
        } else {
            cursor + 1 >= len
        }
    }

    fn picker_blocks_left(&self) -> bool {
        let Some(question) = self.questions.get(self.focus) else {
            return false;
        };
        matches!(question.kind.as_str(), "folder" | "file") && !self.browse[self.focus].is_empty()
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.row_len();
        if len == 0 {
            return;
        }
        let current = self.cursor[self.focus] as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.cursor[self.focus] = next;
        self.reveal_cursor();
    }

    fn reveal_cursor(&mut self) {
        let next = self.cursor[self.focus];
        const WINDOW: usize = 8;
        if next < self.scroll[self.focus] {
            self.scroll[self.focus] = next;
        } else if next >= self.scroll[self.focus].saturating_add(WINDOW) {
            self.scroll[self.focus] = next + 1 - WINDOW;
        }
    }

    fn row_len(&self) -> usize {
        let Some(question) = self.questions.get(self.focus) else {
            return 0;
        };
        match question.kind.as_str() {
            "single" | "multi" | "boolean" => question.options.len(),
            "folder" | "file" => self.entries[self.focus].len(),
            _ => 0,
        }
    }

    fn select_current(&mut self, workspace: &Path) {
        let index = self.focus;
        let kind = self.questions[index].kind.clone();
        match kind.as_str() {
            "single" | "boolean" => {
                let cursor =
                    self.cursor[index].min(self.questions[index].options.len().saturating_sub(1));
                self.choice[index] = Some(cursor);
            }
            "multi" => {
                let cursor = self.cursor[index];
                if let Some(flag) = self.checks[index].get_mut(cursor) {
                    *flag = !*flag;
                }
            }
            "folder" | "file" => self.activate_row(workspace),
            _ => {}
        }
    }

    fn activate_row(&mut self, workspace: &Path) {
        let index = self.focus;
        let Some(row) = self.entries[index].get(self.cursor[index]).cloned() else {
            return;
        };
        if row == "*" {
            let path = if self.browse[index].is_empty() {
                ".".to_owned()
            } else {
                self.browse[index].clone()
            };
            self.picked[index] = Some(path);
            return;
        }
        if row == ".." {
            self.browse[index] = parent_rel(&self.browse[index]);
            self.reload(workspace, index);
            return;
        }
        if let Some(name) = row.strip_suffix('/') {
            self.browse[index] = join_rel(&self.browse[index], name);
            self.reload(workspace, index);
            return;
        }
        if self.questions[index].kind == "file" {
            self.picked[index] = Some(join_rel(&self.browse[index], &row));
        }
    }

    fn focused_row_is_directory(&self) -> bool {
        let Some(question) = self.questions.get(self.focus) else {
            return false;
        };
        if !matches!(question.kind.as_str(), "folder" | "file") {
            return false;
        }
        self.entries[self.focus]
            .get(self.cursor[self.focus])
            .is_some_and(|row| row == ".." || row.ends_with('/'))
    }

    fn enter_directory(&mut self, workspace: &Path) {
        if self.focused_row_is_directory() {
            self.activate_row(workspace);
        }
    }

    fn picker_parent(&mut self, workspace: &Path) {
        let index = self.focus;
        if !matches!(self.questions[index].kind.as_str(), "folder" | "file") {
            return;
        }
        if self.browse[index].is_empty() {
            return;
        }
        self.browse[index] = parent_rel(&self.browse[index]);
        self.reload(workspace, index);
    }

    fn reload(&mut self, workspace: &Path, index: usize) {
        let files = self.questions[index].kind == "file";
        let mut entries = list_entries(workspace, &self.browse[index], files);
        if self.questions[index].kind == "folder" {
            entries.insert(0, "*".to_owned());
        }
        self.entries[index] = entries;
        self.cursor[index] = 0;
        self.scroll[index] = 0;
    }

    fn accepts_field(&self) -> Option<usize> {
        if self.accepts_text() {
            Some(self.focus)
        } else {
            None
        }
    }

    fn backspace(&mut self) {
        let Some(index) = self.accepts_field() else {
            return;
        };
        let field = if self.questions[index].kind == "text" {
            &mut self.text[index]
        } else {
            &mut self.other_text[index]
        };
        field.pop();
    }

    fn push_char(&mut self, ch: char) {
        let Some(index) = self.accepts_field() else {
            return;
        };
        let field = if self.questions[index].kind == "text" {
            &mut self.text[index]
        } else {
            &mut self.other_text[index]
        };
        if field.chars().count() < 2_000 {
            field.push(ch);
        }
    }
}

/// Open or replace the form when the pending question changes.
/// Returns true when a new questionnaire just opened.
pub fn sync(
    form: &mut Option<QuestionForm>,
    pending: Option<&PendingInput>,
    workspace: &Path,
) -> bool {
    match pending {
        None => {
            *form = None;
            false
        }
        Some(pending)
            if form
                .as_ref()
                .is_some_and(|current| current.request_id == pending.request_id) =>
        {
            false
        }
        Some(pending) => {
            *form = Some(QuestionForm::open(pending, workspace));
            true
        }
    }
}

fn nonempty(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_owned())
    }
}

fn parent_rel(relative: &str) -> String {
    match relative.rsplit_once('/') {
        Some((parent, _)) => parent.to_owned(),
        None => String::new(),
    }
}

fn join_rel(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

fn list_entries(workspace: &Path, relative: &str, files: bool) -> Vec<String> {
    let root = workspace
        .canonicalize()
        .unwrap_or_else(|_| workspace.to_path_buf());
    let dir = if relative.is_empty() {
        root.clone()
    } else {
        root.join(relative)
    };
    let dir = dir.canonicalize().unwrap_or(dir);
    if !dir.starts_with(&root) {
        return vec!["..".to_owned()];
    }
    let mut names = Vec::new();
    if !relative.is_empty() {
        names.push("..".to_owned());
    }
    let Ok(read) = std::fs::read_dir(&dir) else {
        return names;
    };
    let mut children = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "." || name == ".." {
            continue;
        }
        if entry.path().is_dir() {
            children.push(format!("{name}/"));
        } else if files {
            children.push(name);
        }
    }
    children.sort();
    names.extend(children);
    names
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::QuestionChoice;

    fn choice(label: &str, other: bool) -> QuestionChoice {
        QuestionChoice {
            label: label.to_owned(),
            description: None,
            other,
        }
    }

    fn pending(questions: Vec<AskedQuestion>) -> PendingInput {
        PendingInput {
            request_id: "ask_1".to_owned(),
            question: questions[0].question.clone(),
            options: Vec::new(),
            questions,
        }
    }

    #[test]
    fn other_selection_shows_input_and_replaces_the_label() {
        let dir = std::env::temp_dir();
        let form_questions = vec![AskedQuestion {
            id: "q1".into(),
            kind: "single".into(),
            question: "用哪个？".into(),
            header: Some("数据库".into()),
            options: vec![
                choice("SQLite", false),
                choice("Postgres", false),
                choice("其他", true),
            ],
        }];
        let mut form = QuestionForm::open(&pending(form_questions), &dir);
        assert!(!form.accepts_text());
        form.on_key(KeyCode::Down, &dir);
        form.on_key(KeyCode::Down, &dir);
        form.on_key(KeyCode::Char(' '), &dir);
        assert!(form.accepts_text());
        assert!(form.formatted().is_none());
        form.on_key(KeyCode::Char('T'), &dir);
        form.on_key(KeyCode::Char('u'), &dir);
        assert_eq!(form.formatted().as_deref(), Some("1. 数据库：Tu"));
    }

    #[test]
    fn file_question_picks_a_workspace_file_and_blocks_parent_escape() {
        let dir = std::env::temp_dir().join(format!("blora-ask-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("note.txt"), "hi").unwrap();
        std::fs::write(dir.join("sub").join("inner.txt"), "x").unwrap();
        let questions = vec![
            AskedQuestion {
                id: "q1".into(),
                kind: "boolean".into(),
                question: "继续？".into(),
                header: None,
                options: vec![
                    choice("是", false),
                    choice("否", false),
                    choice("其他", true),
                ],
            },
            AskedQuestion {
                id: "q2".into(),
                kind: "file".into(),
                question: "哪个文件？".into(),
                header: Some("文件".into()),
                options: Vec::new(),
            },
        ];
        let mut form = QuestionForm::open(&pending(questions), &dir);
        form.on_key(KeyCode::Char(' '), &dir);
        form.on_key(KeyCode::Tab, &dir);
        assert!(form.entries[1].iter().any(|row| row == "note.txt"));
        assert!(form.entries[1].iter().any(|row| row == "sub/"));
        let file_row = form.entries[1]
            .iter()
            .position(|row| row == "note.txt")
            .unwrap();
        form.cursor[1] = file_row;
        form.on_key(KeyCode::Char(' '), &dir);
        assert_eq!(form.picked[1].as_deref(), Some("note.txt"));
        let sub = form.entries[1]
            .iter()
            .position(|row| row == "sub/")
            .unwrap();
        form.cursor[1] = sub;
        form.on_key(KeyCode::Right, &dir);
        assert_eq!(form.browse[1], "sub");
        assert!(form.entries[1].iter().any(|row| row == ".."));
        form.on_key(KeyCode::Left, &dir);
        assert!(form.browse[1].is_empty());
        let answer = form.formatted().unwrap();
        assert_eq!(answer, "1. 继续？：是\n2. 文件：note.txt");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn arrows_and_clicks_switch_questions_without_leaving_a_folder() {
        let dir = std::env::temp_dir().join(format!("blora-ask-tabs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("note.txt"), "hi").unwrap();
        let questions = vec![
            AskedQuestion {
                id: "q1".into(),
                kind: "single".into(),
                question: "甲？".into(),
                header: Some("甲".into()),
                options: vec![choice("A", false), choice("B", false)],
            },
            AskedQuestion {
                id: "q2".into(),
                kind: "text".into(),
                question: "乙？".into(),
                header: Some("乙".into()),
                options: Vec::new(),
            },
            AskedQuestion {
                id: "q3".into(),
                kind: "file".into(),
                question: "丙？".into(),
                header: Some("丙".into()),
                options: Vec::new(),
            },
        ];
        let mut form = QuestionForm::open(&pending(questions), &dir);
        form.on_key(KeyCode::Right, &dir);
        assert_eq!(form.focus, 1);
        form.on_key(KeyCode::Down, &dir);
        assert_eq!(form.focus, 2);
        form.on_key(KeyCode::Up, &dir);
        assert_eq!(form.focus, 1);
        form.on_key(KeyCode::Left, &dir);
        assert_eq!(form.focus, 0);
        form.on_key(KeyCode::Down, &dir);
        form.on_key(KeyCode::Down, &dir);
        assert_eq!(form.focus, 1);
        form.click_tab(2);
        assert_eq!(form.focus, 2);
        let sub = form.entries[2]
            .iter()
            .position(|row| row == "sub/")
            .unwrap();
        form.cursor[2] = sub;
        form.on_key(KeyCode::Right, &dir);
        assert_eq!(form.focus, 2);
        assert_eq!(form.browse[2], "sub");
        form.on_key(KeyCode::Left, &dir);
        assert_eq!(form.focus, 2);
        assert!(form.browse[2].is_empty());
        form.on_key(KeyCode::Left, &dir);
        assert_eq!(form.focus, 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
