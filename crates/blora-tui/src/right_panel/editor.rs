// SPDX-License-Identifier: GPL-3.0-or-later
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone)]
struct Snapshot {
    text: String,
    cursor: usize,
}

pub(super) struct Editor {
    pub(super) text: String,
    pub(super) cursor: usize,
    path: PathBuf,
    canonical: PathBuf,
    original: Vec<u8>,
    metadata: fs::Metadata,
    newline: &'static str,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    preferred_column: Option<usize>,
}
impl Editor {
    pub(super) fn new(path: &Path, bytes: Vec<u8>, metadata: fs::Metadata) -> Result<Self, String> {
        let text = String::from_utf8(bytes.clone()).map_err(|e| e.to_string())?;
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
        Ok(Self {
            text,
            cursor: 0,
            path: path.into(),
            canonical: path.canonicalize().map_err(|e| e.to_string())?,
            original: bytes,
            metadata,
            newline,
            undo: vec![],
            redo: vec![],
            preferred_column: None,
        })
    }
    pub(super) fn dirty(&self) -> bool {
        self.text.as_bytes() != self.original
    }
    pub(super) fn position(&self) -> (usize, usize) {
        let prefix = &self.text[..self.cursor];
        (
            prefix.bytes().filter(|b| *b == b'\n').count(),
            prefix.rsplit('\n').next().unwrap_or("").chars().count(),
        )
    }
    fn start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }
    fn end(&self) -> usize {
        let end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |i| self.cursor + i);
        if end > 0 && self.text.as_bytes()[end - 1] == b'\r' {
            end - 1
        } else {
            end
        }
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            text: self.text.clone(),
            cursor: self.cursor,
        }
    }
    fn checkpoint(&mut self) {
        self.undo.push(self.snapshot());
        if self.undo.len() > 128 {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.preferred_column = None;
    }
    pub(super) fn insert(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.checkpoint();
        let normalized = text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .replace('\n', self.newline);
        self.text.insert_str(self.cursor, &normalized);
        self.cursor += normalized.len();
    }
    fn previous(&self) -> usize {
        if self.text[..self.cursor].ends_with("\r\n") {
            self.cursor - 2
        } else {
            self.text[..self.cursor]
                .char_indices()
                .next_back()
                .map_or(0, |(i, _)| i)
        }
    }
    fn next(&self) -> usize {
        if self.text[self.cursor..].starts_with("\r\n") {
            self.cursor + 2
        } else {
            self.cursor
                + self.text[self.cursor..]
                    .chars()
                    .next()
                    .map_or(0, char::len_utf8)
        }
    }
    fn vertical(&mut self, delta: isize) {
        let (row, column) = self.position();
        let wanted = *self.preferred_column.get_or_insert(column);
        let target = row.saturating_add_signed(delta);
        let mut start = 0;
        for (index, line) in self.text.split('\n').enumerate() {
            if index == target {
                let line = line.strip_suffix('\r').unwrap_or(line);
                self.cursor = start
                    + line
                        .char_indices()
                        .nth(wanted)
                        .map_or(line.len(), |(i, _)| i);
                return;
            }
            start += line.len() + 1;
        }
    }
    pub(super) fn key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            let redo = key.code == KeyCode::Char('y')
                || (key.code == KeyCode::Char('z') && key.modifiers.contains(KeyModifiers::SHIFT));
            if redo {
                if let Some(snapshot) = self.redo.pop() {
                    self.undo.push(self.snapshot());
                    self.text = snapshot.text;
                    self.cursor = snapshot.cursor;
                }
            } else if key.code == KeyCode::Char('z') {
                if let Some(snapshot) = self.undo.pop() {
                    self.redo.push(self.snapshot());
                    self.text = snapshot.text;
                    self.cursor = snapshot.cursor;
                }
            }
            self.preferred_column = None;
            return;
        }
        match key.code {
            KeyCode::Up => self.vertical(-1),
            KeyCode::Down => self.vertical(1),
            KeyCode::Left => {
                self.cursor = self.previous();
                self.preferred_column = None;
            }
            KeyCode::Right => {
                self.cursor = self.next();
                self.preferred_column = None;
            }
            KeyCode::Home => {
                self.cursor = self.start();
                self.preferred_column = None;
            }
            KeyCode::End => {
                self.cursor = self.end();
                self.preferred_column = None;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.previous();
                self.checkpoint();
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
            }
            KeyCode::Delete if self.cursor < self.text.len() => {
                let next = self.next();
                self.checkpoint();
                self.text.replace_range(self.cursor..next, "");
            }
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Tab => self.insert("\t"),
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::SUPER) =>
            {
                self.insert(&c.to_string())
            }
            _ => {}
        }
    }
    fn check(&self, root: &Path) -> Result<(), String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let path = self.path.canonicalize().map_err(|e| e.to_string())?;
        if path != self.canonical || !path.starts_with(root) {
            return Err("路径已改变或超出项目边界，未保存".into());
        }
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.dev() != self.metadata.dev() || metadata.ino() != self.metadata.ino() {
                return Err("文件已被外部替换，未保存".into());
            }
        }
        if metadata.modified().ok() != self.metadata.modified().ok()
            || metadata.permissions() != self.metadata.permissions()
            || fs::read(&path).map_err(|e| e.to_string())? != self.original
        {
            return Err("文件已被外部修改，未保存；请保留草稿后重新打开".into());
        }
        Ok(())
    }
    pub(super) fn save(&mut self, root: &Path) -> Result<(), String> {
        self.check(root)?;
        if !self.dirty() {
            return Ok(());
        }
        let parent = self.canonical.parent().ok_or("文件没有父目录")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        temp.write_all(self.text.as_bytes())
            .map_err(|e| e.to_string())?;
        temp.as_file()
            .set_permissions(self.metadata.permissions())
            .map_err(|e| e.to_string())?;
        temp.as_file().sync_all().map_err(|e| e.to_string())?;
        self.check(root)?;
        temp.persist(&self.canonical).map_err(|e| e.to_string())?;
        self.original = self.text.as_bytes().to_vec();
        self.metadata = fs::metadata(&self.canonical).map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editor(text: &str) -> (tempfile::TempDir, Editor) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        fs::write(&path, text).unwrap();
        let editor = Editor::new(
            &path,
            text.as_bytes().to_vec(),
            fs::metadata(&path).unwrap(),
        )
        .unwrap();
        (dir, editor)
    }
    fn key(editor: &mut Editor, code: KeyCode) {
        editor.key(KeyEvent::new(code, KeyModifiers::NONE));
    }
    #[test]
    fn unicode_crlf_paste_and_history() {
        let (dir, mut e) = editor("中🙂\r\nlast");
        key(&mut e, KeyCode::End);
        key(&mut e, KeyCode::Backspace);
        assert_eq!(e.text, "中\r\nlast");
        e.insert("é\n新");
        assert_eq!(e.text, "中é\r\n新\r\nlast");
        e.key(KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL));
        assert_eq!(e.text, "中\r\nlast");
        e.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
        e.save(dir.path()).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("file")).unwrap(), e.text);
        assert!(!e.dirty());
    }
    #[test]
    fn vertical_home_end_and_delete() {
        let (_, mut e) = editor("中文\na\nthree");
        key(&mut e, KeyCode::End);
        key(&mut e, KeyCode::Down);
        assert_eq!(e.position(), (1, 1));
        key(&mut e, KeyCode::Down);
        assert_eq!(e.position(), (2, 2));
        key(&mut e, KeyCode::Home);
        key(&mut e, KeyCode::Delete);
        assert!(e.text.ends_with("hree"));
    }
    #[test]
    fn conflict_and_boundary_never_overwrite() {
        let (dir, mut e) = editor("original");
        e.insert("draft");
        let other = tempfile::tempdir().unwrap();
        assert!(e.save(other.path()).is_err());
        fs::write(dir.path().join("file"), "external").unwrap();
        assert!(e.save(dir.path()).is_err());
        assert!(e.dirty());
        assert_eq!(
            fs::read_to_string(dir.path().join("file")).unwrap(),
            "external"
        );
    }
    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_permissions_and_symlink_target() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (dir, _) = editor("original");
        let path = dir.path().join("file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        let mut e = Editor::new(
            &link,
            fs::read(&path).unwrap(),
            fs::metadata(&path).unwrap(),
        )
        .unwrap();
        e.insert("new");
        e.save(dir.path()).unwrap();
        assert!(fs::symlink_metadata(link).unwrap().is_symlink());
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
}
