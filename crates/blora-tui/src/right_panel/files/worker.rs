// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! File-pane background reader and bounded file/directory IO.

use super::FileEntry;
use super::editor::Editor;
use crate::i18n;
use blora_runtime::CancelToken;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

pub(in crate::right_panel) struct FileResult {
    pub(in crate::right_panel) generation: u64,
    pub(in crate::right_panel) result: Result<FileData, String>,
}
pub(in crate::right_panel) enum FileData {
    List(Vec<FileEntry>),
    Preview(String),
    Document {
        preview: String,
        editor: Option<Editor>,
        reason: String,
    },
}
pub(in crate::right_panel) struct FileRequest {
    pub(in crate::right_panel) generation: u64,
    pub(in crate::right_panel) root: PathBuf,
    pub(in crate::right_panel) path: PathBuf,
    pub(in crate::right_panel) preview: bool,
    pub(in crate::right_panel) cancel: CancelToken,
}
#[derive(Default)]
pub(in crate::right_panel) struct FileWork {
    pub(in crate::right_panel) pending: Option<FileRequest>,
    pub(in crate::right_panel) result: Option<FileResult>,
    pub(in crate::right_panel) stop: bool,
}
#[derive(Default)]
pub(in crate::right_panel) struct FileWorker {
    pub(in crate::right_panel) work: std::sync::Mutex<FileWork>,
    pub(in crate::right_panel) ready: std::sync::Condvar,
}
impl FileWorker {
    pub(in crate::right_panel) fn run(&self) {
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
#[cfg(test)]
pub(in crate::right_panel) fn read_file_data(
    root: &Path,
    path: &Path,
    preview: bool,
) -> Result<FileData, String> {
    match read_file_data_cancelled(root, path, preview, &CancelToken::new())? {
        FileData::Document { reason, .. }
            if reason == i18n::tr("right.file_binary", "只读 · 二进制文件") =>
        {
            Err(reason)
        }
        FileData::Document { preview, .. } => Ok(FileData::Preview(preview)),
        data => Ok(data),
    }
}
pub(in crate::right_panel) fn file_cancelled(cancel: &CancelToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("file request cancelled".to_owned())
    } else {
        Ok(())
    }
}
pub(in crate::right_panel) fn file_read_error(path: &Path, error: std::io::Error) -> String {
    let heading = if error.kind() == std::io::ErrorKind::PermissionDenied {
        i18n::tr("right.file_access_denied", "没有权限读取此路径")
    } else {
        i18n::tr("right.file_read_failed", "无法读取此路径")
    };
    let mut message = format!("{heading}\n{}\n{error}", path.display());
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        message.push_str(&format!(
            "\n{}",
            i18n::tr(
                "right.file_access_help",
                "请检查目录权限及终端应用的文件访问授权，恢复访问后点击刷新（r）。",
            )
        ));
    }
    message
}

pub(in crate::right_panel) fn read_file_data_cancelled(
    root: &Path,
    path: &Path,
    preview: bool,
    cancel: &CancelToken,
) -> Result<FileData, String> {
    file_cancelled(cancel)?;
    let root = root.canonicalize().map_err(|e| file_read_error(root, e))?;
    file_cancelled(cancel)?;
    let requested_path = path;
    let path = path.canonicalize().map_err(|e| file_read_error(path, e))?;
    file_cancelled(cancel)?;
    if !path.starts_with(&root) {
        return Err(i18n::tr("right.boundary", "路径超出项目边界"));
    }
    if preview {
        if !path.is_file() {
            return Err(i18n::tr("right.special", "仅支持预览普通文件"));
        }
        file_cancelled(cancel)?;
        let mut file = fs::File::open(&path).map_err(|e| file_read_error(&path, e))?;
        let metadata = file.metadata().map_err(|e| file_read_error(&path, e))?;
        let truncated = metadata.len() > 65536;
        let mut bytes = Vec::new();
        let mut chunk = [0; 8192];
        while bytes.len() < 65536 {
            file_cancelled(cancel)?;
            let remaining = (65536 - bytes.len()).min(chunk.len());
            let count = file
                .read(&mut chunk[..remaining])
                .map_err(|e| file_read_error(&path, e))?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
        }
        file_cancelled(cancel)?;
        let valid = std::str::from_utf8(&bytes).is_ok();
        let binary = bytes
            .iter()
            .any(|b| *b == 0 || (*b < 32 && !matches!(*b, 9 | 10 | 13)));
        let reason = if truncated {
            i18n::tr("right.file_truncated", "只读 · 仅显示前 64 KiB")
        } else if binary {
            i18n::tr("right.file_binary", "只读 · 二进制文件")
        } else if !valid {
            i18n::tr("right.file_invalid_utf8", "只读 · 无效 UTF-8")
        } else {
            String::new()
        };
        let mut text = String::from_utf8_lossy(&bytes).into_owned();
        if binary {
            text = text
                .chars()
                .map(|c| {
                    if c.is_control() && !matches!(c, '\n' | '\r' | '\t') {
                        '�'
                    } else {
                        c
                    }
                })
                .collect();
        }
        let editor = if reason.is_empty() {
            Some(Editor::new(requested_path, bytes, metadata)?)
        } else {
            None
        };
        if truncated {
            text.push_str(&format!(
                "\n{}",
                i18n::tr("right.file_truncated_note", "仅显示前 64 KiB")
            ));
        }
        if text.is_empty() {
            text = i18n::tr("right.empty_file", "空文件");
        }
        let preview = text;
        Ok(FileData::Document {
            preview,
            editor,
            reason: reason.into(),
        })
    } else {
        let mut entries = Vec::new();
        file_cancelled(cancel)?;
        let mut directory = fs::read_dir(&path).map_err(|e| file_read_error(&path, e))?;
        for _ in 0..1001 {
            file_cancelled(cancel)?;
            let Some(entry) = directory.next() else {
                break;
            };
            file_cancelled(cancel)?;
            let entry = entry.map_err(|e| file_read_error(&path, e))?;
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

pub(in crate::right_panel) fn comparison_lines(draft: &str, disk: &str) -> Vec<String> {
    let identical = draft == disk;
    let draft: Vec<_> = draft.lines().collect();
    let disk: Vec<_> = disk.lines().collect();
    let prefix = draft.iter().zip(&disk).take_while(|(a, b)| a == b).count();
    let suffix = draft[prefix..]
        .iter()
        .rev()
        .zip(disk[prefix..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let mut lines = vec!["--- 编辑草稿".into(), "+++ 磁盘内容".into()];
    if identical {
        lines.push("内容相同；文件身份或元数据可能已改变".into());
    } else {
        lines.push(format!(
            "@@ 草稿 {} 行 / 磁盘 {} 行 @@",
            draft.len(),
            disk.len()
        ));
        if draft == disk {
            lines.push("换行格式或末尾换行已变化".into());
        }
        lines.extend(
            draft[..prefix]
                .iter()
                .rev()
                .take(3)
                .rev()
                .map(|line| format!(" {line}")),
        );
        lines.extend(
            draft[prefix..draft.len() - suffix]
                .iter()
                .map(|line| format!("-{line}")),
        );
        lines.extend(
            disk[prefix..disk.len() - suffix]
                .iter()
                .map(|line| format!("+{line}")),
        );
        lines.extend(
            disk[disk.len() - suffix..]
                .iter()
                .take(3)
                .map(|line| format!(" {line}")),
        );
    }
    lines
}
