// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

#[derive(Debug)]
pub(super) struct FileEntry {
    pub(super) path: PathBuf,
    pub(super) directory: bool,
}
pub(super) struct FileResult {
    pub(super) generation: u64,
    pub(super) result: Result<FileData, String>,
}
pub(super) enum FileData {
    List(Vec<FileEntry>),
    Preview(String),
}
pub(super) struct FileRequest {
    pub(super) generation: u64,
    pub(super) root: PathBuf,
    pub(super) path: PathBuf,
    pub(super) preview: bool,
    pub(super) cancel: CancelToken,
}
#[derive(Default)]
pub(super) struct FileWork {
    pub(super) pending: Option<FileRequest>,
    pub(super) result: Option<FileResult>,
    pub(super) stop: bool,
}
#[derive(Default)]
pub(super) struct FileWorker {
    pub(super) work: std::sync::Mutex<FileWork>,
    pub(super) ready: std::sync::Condvar,
}
impl FileWorker {
    pub(super) fn run(&self) {
        loop {
            let request = {
                let mut work = self.work.lock().unwrap();
                while work.pending.is_none() && !work.stop {
                    work = self.ready.wait(work).unwrap();
                }
                if work.stop {
                    return;
                }
                work.pending.take().unwrap()
            };
            let result = read_file_data_cancelled(
                &request.root,
                &request.path,
                request.preview,
                &request.cancel,
            );
            let mut work = self.work.lock().unwrap();
            if !work.stop && !request.cancel.is_cancelled() {
                work.result = Some(FileResult {
                    generation: request.generation,
                    result,
                });
            }
        }
    }
}
pub(super) struct Files {
    pub(super) root: PathBuf,
    pub(super) directory: PathBuf,
    pub(super) generation: u64,
    pub(super) entries: Vec<FileEntry>,
    pub(super) selected: usize,
    pub(super) preview: Option<String>,
    pub(super) preview_path: Option<PathBuf>,
    pub(super) horizontal: usize,
    pub(super) max_scroll: std::cell::Cell<usize>,
    pub(super) max_horizontal: std::cell::Cell<usize>,
    positions: std::collections::HashMap<PathBuf, (usize, usize)>,
    restore: Option<PathBuf>,
    pub(super) message: String,
    pub(super) worker: std::sync::Arc<FileWorker>,
    pub(super) handle: Option<thread::JoinHandle<()>>,
    pub(super) cancel: CancelToken,
    pub(super) scroll: usize,
}
impl Drop for Files {
    fn drop(&mut self) {
        self.cancel();
        {
            let mut work = self.worker.work.lock().unwrap();
            work.stop = true;
        }
        self.worker.ready.notify_one();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
impl Files {
    pub(super) fn new() -> Self {
        let worker = std::sync::Arc::new(FileWorker::default());
        let background = worker.clone();
        let handle = thread::Builder::new()
            .name("blora-right-files".into())
            .spawn(move || background.run())
            .ok();
        Self {
            root: PathBuf::new(),
            directory: PathBuf::new(),
            generation: 0,
            entries: vec![],
            selected: 0,
            preview: None,
            preview_path: None,
            horizontal: 0,
            max_scroll: 0.into(),
            max_horizontal: 0.into(),
            positions: Default::default(),
            restore: None,
            message: String::new(),
            worker,
            handle,
            cancel: CancelToken::new(),
            scroll: 0,
        }
    }
    pub(super) fn load(&mut self, path: PathBuf, preview: bool) {
        self.cancel();
        self.message = i18n::tr("right.loading", "加载中…");
        self.preview = None;
        if !preview && !self.directory.as_os_str().is_empty() {
            self.positions
                .insert(self.directory.clone(), (self.selected, self.scroll));
        }
        self.scroll = 0;
        self.horizontal = 0;
        self.preview_path = preview.then(|| path.clone());
        if !preview {
            self.selected = 0;
            self.directory = path.clone();
            self.entries.clear();
            if let Some((selected, scroll)) = self.positions.get(&self.directory) {
                self.selected = *selected;
                self.scroll = *scroll;
            }
        }
        if self.handle.is_none() {
            self.message = "could not start file worker".to_owned();
            return;
        }
        self.cancel = CancelToken::new();
        let mut work = self.worker.work.lock().unwrap();
        work.pending = Some(FileRequest {
            generation: self.generation,
            root: self.root.clone(),
            path,
            preview,
            cancel: self.cancel.clone(),
        });
        self.worker.ready.notify_one();
    }
    pub(super) fn cancel(&mut self) {
        self.cancel.cancel();
        self.generation = self.generation.wrapping_add(1);
        let mut work = self.worker.work.lock().unwrap();
        work.pending = None;
        work.result = None;
    }
    pub(super) fn close(&mut self) {
        self.cancel();
        self.root.clear();
        self.directory.clear();
        self.entries.clear();
        self.preview = None;
        self.preview_path = None;
        self.positions.clear();
        self.restore = None;
        self.message.clear();
    }

    pub(super) fn sync(&mut self, root: &Path) {
        if self.root != root {
            self.positions.clear();
            self.restore = None;
            self.directory.clear();
            self.root = root.to_path_buf();
            self.load(root.to_path_buf(), false);
        }
        let update = self.worker.work.lock().unwrap().result.take();
        if let Some(update) = update {
            if update.generation != self.generation {
                return;
            }
            match update.result {
                Ok(FileData::List(entries)) => {
                    self.entries = entries;
                    if let Some(path) = self.restore.take() {
                        if let Some(index) =
                            self.entries.iter().position(|entry| entry.path == path)
                        {
                            self.selected = index;
                        }
                    }
                    self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                    self.message = if self.entries.is_empty() {
                        i18n::tr("right.empty", "空目录")
                    } else {
                        String::new()
                    };
                }
                Ok(FileData::Preview(text)) => {
                    self.preview = Some(text);
                    self.message = String::new();
                }
                Err(error) => {
                    self.preview = None;
                    self.entries.clear();
                    self.selected = 0;
                    self.message = error;
                }
            }
        }
    }
    pub(super) fn open(&mut self, index: usize) {
        if let Some(entry) = self.entries.get(index) {
            let path = entry.path.clone();
            let preview = !entry.directory;
            self.selected = index;
            self.load(path, preview);
        }
    }
    pub(super) fn refresh(&mut self) {
        let path = self
            .preview_path
            .clone()
            .unwrap_or_else(|| self.directory.clone());
        let preview = self.preview_path.is_some();
        self.restore = self
            .entries
            .get(self.selected)
            .map(|entry| entry.path.clone());
        self.load(path, preview);
    }
    pub(super) fn back(&mut self) {
        self.cancel();
        self.message.clear();
        if self.preview_path.take().is_some() {
            self.preview = None;
            self.scroll = 0;
            self.horizontal = 0;
            if self.entries.is_empty() {
                self.load(self.directory.clone(), false);
            }
        } else if self.directory != self.root {
            let child = self.directory.clone();
            if let Some(parent) = child.parent() {
                self.restore = Some(child.clone());
                self.load(parent.to_path_buf(), false);
            }
        }
    }
}
#[cfg(test)]
pub(super) fn read_file_data(root: &Path, path: &Path, preview: bool) -> Result<FileData, String> {
    read_file_data_cancelled(root, path, preview, &CancelToken::new())
}
pub(super) fn file_cancelled(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("file request cancelled".to_owned())
    } else {
        Ok(())
    }
}
pub(super) fn read_file_data_cancelled(
    root: &Path,
    path: &Path,
    preview: bool,
    cancel: &CancelToken,
) -> Result<FileData, String> {
    file_cancelled(cancel)?;
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    file_cancelled(cancel)?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    file_cancelled(cancel)?;
    if !path.starts_with(&root) {
        return Err(i18n::tr("right.boundary", "路径超出项目边界"));
    }
    if preview {
        if !path.is_file() {
            return Err(i18n::tr("right.special", "仅支持预览普通文件"));
        }
        file_cancelled(cancel)?;
        let mut file = fs::File::open(&path).map_err(|e| e.to_string())?;
        let truncated = file.metadata().map_err(|e| e.to_string())?.len() > 65536;
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        while bytes.len() < 65536 {
            file_cancelled(cancel)?;
            let remaining = (65536 - bytes.len()).min(chunk.len());
            let count = file
                .read(&mut chunk[..remaining])
                .map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        file_cancelled(cancel)?;
        if bytes.contains(&0)
            || std::str::from_utf8(&bytes)
                .is_err_and(|error| error.error_len().is_some() || !truncated)
        {
            return Err(i18n::tr("right.binary", "二进制文件，无法预览"));
        }
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if truncated {
            text.push_str(&format!(
                "\n{}",
                i18n::tr("right.truncated", "仅显示前 64 KiB")
            ));
        }
        if text.is_empty() {
            text = i18n::tr("right.empty_file", "空文件");
        }
        Ok(FileData::Preview(text))
    } else {
        let mut entries = Vec::new();
        file_cancelled(cancel)?;
        let mut directory = fs::read_dir(&path).map_err(|e| e.to_string())?;
        for _ in 0..1001 {
            file_cancelled(cancel)?;
            let Some(entry) = directory.next() else {
                break;
            };
            file_cancelled(cancel)?;
            let entry = entry.map_err(|e| e.to_string())?;
            let canonical = match entry.path().canonicalize() {
                Ok(p) if p.starts_with(&root) => p,
                _ => continue,
            };
            file_cancelled(cancel)?;
            if !canonical.is_dir() && !canonical.is_file() {
                continue;
            }
            entries.push(FileEntry {
                directory: canonical.is_dir(),
                path: entry.path(),
            });
        }
        if entries.len() > 1000 {
            return Err(i18n::tr(
                "right.too_many",
                "目录超过 1000 项，请选择更小的目录",
            ));
        }
        file_cancelled(cancel)?;
        entries.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.path.cmp(&b.path)));
        file_cancelled(cancel)?;
        Ok(FileData::List(entries))
    }
}
