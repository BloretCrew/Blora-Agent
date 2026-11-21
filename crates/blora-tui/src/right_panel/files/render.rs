// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! File-pane rendering: tabs, explorer, editor/preview viewport, search bar.

use crate::right_panel::editor::Editor;
use crate::right_panel::{Action, RightPanel, layout};
use crate::{i18n, theme::Theme};
use ratatui::{
    Frame,
    layout::Rect,
    style::Modifier,
    widgets::{Block, Borders, Paragraph, Wrap},
};

fn draw_search_field(frame: &mut Frame<'_>, area: Rect, label: &str, value: &str, focused: bool) {
    use unicode_width::UnicodeWidthStr;
    if area.width == 0 || area.height == 0 {
        return;
    }
    let theme = Theme::current();
    let label_width = label.width().min(area.width as usize);
    let available = (area.width as usize).saturating_sub(label_width + 1);
    let display = value.replace(['\n', '\r', '\t'], " ");
    let mut start = display.len();
    for (byte, _) in display.char_indices().rev() {
        if display[byte..].width() > available {
            break;
        }
        start = byte;
    }
    let visible = &display[start..];
    frame.render_widget(
        Paragraph::new(format!("{label}{visible}")).style(theme.fg(theme.rose).bg(theme.hairline)),
        area,
    );
    if focused {
        let column = (label_width + visible.width()).min(area.width.saturating_sub(1) as usize);
        frame.set_cursor_position((area.x + column as u16, area.y));
    }
}

impl RightPanel {
    pub(in crate::right_panel) fn draw_files(
        &self,
        frame: &mut Frame<'_>,
        pointer: Option<(u16, u16)>,
        mut body: Rect,
        hits: &mut Vec<(Rect, Action)>,
        editor_focus: bool,
    ) -> Option<Rect> {
        let theme = Theme::current();
        if body.height == 0 || body.width == 0 {
            return None;
        }
        let files = &self.files;
        self.explorer_area.set(None);
        if editor_focus && body.width >= 72 && files.preview_path.is_some() {
            let explorer = Rect::new(body.x, body.y, (body.width / 4).clamp(18, 28), body.height);
            self.explorer_area.set(Some(explorer));
            frame.render_widget(
                Block::default()
                    .borders(Borders::RIGHT)
                    .border_style(theme.fg(theme.hairline)),
                explorer,
            );
            frame.render_widget(
                Paragraph::new(if self.explorer_focused {
                    "● 文件 / F6编辑"
                } else {
                    "文件 / F6聚焦"
                })
                .style(theme.fg(theme.rose)),
                Rect::new(explorer.x, explorer.y, explorer.width.saturating_sub(1), 1),
            );
            let count = explorer.height.saturating_sub(1) as usize;
            let start = files.selected.saturating_sub(count.saturating_sub(1));
            for (row, (index, entry)) in files
                .entries
                .iter()
                .enumerate()
                .skip(start)
                .take(count)
                .enumerate()
            {
                let rect = Rect::new(
                    explorer.x,
                    explorer.y + row as u16 + 1,
                    explorer.width.saturating_sub(1),
                    1,
                );
                let name = entry.path.file_name().unwrap_or_default().to_string_lossy();
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {}{} {}",
                        if index == files.selected { "›" } else { " " },
                        "  ".repeat(
                            entry
                                .path
                                .strip_prefix(&files.directory)
                                .unwrap_or(&entry.path)
                                .components()
                                .count()
                                .saturating_sub(1)
                        ),
                        if entry.directory {
                            if files.expanded.contains(&entry.path) {
                                "▾"
                            } else {
                                "▸"
                            }
                        } else {
                            " "
                        },
                        name,
                    ))
                    .style(
                        if files.preview_path.as_ref() == Some(&entry.path) {
                            theme.fg(theme.rose).bg(theme.hairline)
                        } else {
                            theme.fg(theme.text)
                        },
                    ),
                    rect,
                );
                hits.insert(0, (rect, Action::File(index)));
            }
            body.x += explorer.width;
            body.width = body.width.saturating_sub(explorer.width);
        }
        let tabs_y = body.y;
        if !files.documents.is_empty() && body.height >= 4 {
            let arrows =
                files.documents.len() > (body.width / 12).max(1) as usize && body.width > 4;
            let tabs_width = body.width.saturating_sub(if arrows { 4 } else { 0 });
            if arrows {
                for (offset, label, backwards) in [(0, "‹", true), (2, "›", false)] {
                    let rect = Rect::new(body.right() - 4 + offset, tabs_y, 2, 1);
                    frame.render_widget(Paragraph::new(label).style(theme.fg(theme.rose)), rect);
                    hits.insert(0, (rect, Action::DocumentCycle(backwards)));
                }
            }
            let capacity = (tabs_width / 12).max(1) as usize;
            let count = files.documents.len().min(capacity);
            let start = files
                .active_document
                .unwrap_or(0)
                .saturating_sub(count.saturating_sub(1));
            let tab_width = (tabs_width / count as u16).max(1);
            for (visible, (index, document)) in files
                .documents
                .iter()
                .enumerate()
                .skip(start)
                .take(count)
                .enumerate()
            {
                let x = body.x + visible as u16 * tab_width;
                let width = if visible + 1 == count {
                    (body.x + tabs_width).saturating_sub(x)
                } else {
                    tab_width.min(body.right().saturating_sub(x))
                };
                let rect = Rect::new(x, tabs_y, width, 1);
                let name = document
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                let dirty = document.editor.as_ref().is_some_and(Editor::dirty)
                    || files.active_document == Some(index) && files.dirty();
                let selected = files.active_document == Some(index);
                let close_rect = Rect::new(
                    rect.right().saturating_sub(1),
                    rect.y,
                    u16::from(width > 1),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(layout::clip(
                        &format!(
                            "{}{}{}{}",
                            if visible == 0 && start > 0 {
                                "‹ "
                            } else {
                                ""
                            },
                            name,
                            if dirty { " •" } else { "" },
                            if visible + 1 == count && start + count < files.documents.len() {
                                " ›"
                            } else {
                                ""
                            }
                        ),
                        width.saturating_sub(2) as usize,
                    ))
                    .style(if selected {
                        theme.fg(theme.rose).bg(theme.hairline)
                    } else {
                        theme.fg(theme.text_dim)
                    }),
                    rect,
                );
                hits.insert(0, (rect, Action::Document(document.path.clone())));
                if width > 1 {
                    frame.render_widget(
                        Paragraph::new("×").style(theme.fg(theme.text_dim)),
                        close_rect,
                    );
                    hits.insert(
                        0,
                        (close_rect, Action::CloseDocument(document.path.clone())),
                    );
                }
            }
        }
        let top = if files.documents.is_empty() || body.height < 4 {
            body.y
        } else {
            body.y + 1
        };
        let save_width = body.width.min(9);
        let back = Rect::new(body.x, top, body.width.saturating_sub(save_width), 1);
        let path = files.preview_path.as_ref().unwrap_or(&files.directory);
        let relative = path.strip_prefix(&files.root).unwrap_or(path);
        let label = if relative.as_os_str().is_empty() {
            files.root.display().to_string()
        } else {
            format!("/{}", relative.display())
        };
        frame.render_widget(
            Paragraph::new(format!(
                "‹ {}{}",
                label,
                if files.dirty() { " *" } else { "" }
            ))
            .style(theme.fg(theme.rose)),
            back,
        );
        hits.insert(
            0,
            (
                back,
                if files.preview_path.is_some() {
                    Action::Explorer
                } else {
                    Action::Back
                },
            ),
        );
        let button = Rect::new(back.right(), top, save_width, 1);
        let editing = files.editor.is_some();
        frame.render_widget(
            Paragraph::new(if editing {
                i18n::tr("right.file_save", "[保存 ^S]")
            } else {
                i18n::tr("right.file_refresh", "[刷新 r]")
            })
            .style(theme.fg(theme.rose)),
            button,
        );
        hits.insert(
            0,
            (
                button,
                if editing {
                    Action::FileSave
                } else {
                    Action::Refresh
                },
            ),
        );
        let mut content = Rect::new(
            body.x,
            top + 1,
            body.width,
            body.bottom().saturating_sub(top + 2),
        );
        let find_view = files.editor.as_ref().and_then(Editor::find_view);
        let goto_view = files.editor.as_ref().and_then(Editor::goto_view);
        let search_bar = if editing && (find_view.is_some() || goto_view.is_some()) {
            let rows = if find_view.as_ref().is_some_and(|view| view.replace) {
                2
            } else {
                1
            };
            let bar = Rect::new(
                content.x,
                content.y,
                content.width,
                rows.min(content.height),
            );
            content.y += bar.height;
            content.height = content.height.saturating_sub(bar.height);
            Some(bar)
        } else {
            None
        };
        let mut position;
        if editing {
            let editor = files.editor.as_ref().expect("checked editor");
            let dark = theme.is_dark();
            let total = editor.line_count();
            let max = total.saturating_sub(content.height as usize);
            files.max_scroll.set(max);
            let offset = files.scroll.min(max);
            let digits = total.max(1).to_string().len();
            let gutter = (digits as u16 + 1).min(content.width);
            let width = content.width.saturating_sub(gutter);
            let max_width = editor.max_width(dark);
            files
                .max_horizontal
                .set(max_width.saturating_sub(width as usize));
            let horizontal = files.horizontal.min(files.max_horizontal.get() + 1);
            editor.ensure_viewport(offset, content.height.max(1) as usize, dark);
            for row in 0..content.height as usize {
                let index = offset + row;
                if index >= total {
                    break;
                }
                let number = Rect::new(
                    content.x,
                    content.y + row as u16,
                    gutter.saturating_sub(1),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(format!("{:>digits$}", index + 1))
                        .style(theme.fg(theme.text_dim)),
                    number,
                );
                let text = Rect::new(content.x + gutter, number.y, width, 1);
                frame.render_widget(
                    Paragraph::new(editor.render_line(
                        index,
                        width as usize,
                        horizontal,
                        dark,
                        &theme,
                    )),
                    text,
                );
            }
            if let Some(bar) = search_bar {
                if let Some(goto) = goto_view {
                    draw_search_field(
                        frame,
                        Rect::new(bar.x, bar.y, bar.width, 1),
                        "行: ",
                        goto,
                        self.focused && files.pending_leave.is_none(),
                    );
                } else if let Some(view) = find_view.as_ref() {
                    let replacement_row = view.replace && view.focus_replace;
                    for row in 0..bar.height {
                        let replacing = row == 1 || (bar.height == 1 && replacement_row);
                        let active = replacing == replacement_row;
                        let label = if replacing { "替: " } else { "找: " };
                        let value = if replacing {
                            view.replacement
                        } else {
                            view.query
                        };
                        draw_search_field(
                            frame,
                            Rect::new(bar.x, bar.y + row, bar.width, 1),
                            label,
                            value,
                            self.focused && active && files.pending_leave.is_none(),
                        );
                    }
                }
            }
            let (row, column) = editor.position();
            let display_column = editor.display_column(editor.cursor());
            position = format!(
                "{}{} {}:{} · {}",
                editor.language(),
                if editor.dirty() { " *" } else { "" },
                row + 1,
                column + 1,
                "^S保存 ^F查找 ^H替换 F6文件 F11专注"
            );
            if goto_view.is_some() {
                position = "Enter跳转 Esc关闭".into();
            } else if let Some(view) = find_view.as_ref() {
                let current = view.current.map_or(0, |index| index + 1);
                position = if view.replace {
                    format!(
                        "{current}/{} Tab切换 Enter替换 ^Enter全部 Esc关闭",
                        view.matches
                    )
                } else {
                    format!(
                        "{current}/{} Enter下一处 Shift+Enter上一处 Esc关闭",
                        view.matches
                    )
                };
            }
            if self.focused
                && !self.explorer_focused
                && find_view.is_none()
                && goto_view.is_none()
                && files.comparison.is_none()
                && files.pending_leave.is_none()
                && row >= offset
                && row - offset < content.height as usize
                && display_column >= horizontal
                && display_column - horizontal < width as usize
            {
                frame.set_cursor_position((
                    content.x + gutter + (display_column - horizontal) as u16,
                    content.y + (row - offset) as u16,
                ));
            }
            if !files.message.is_empty() {
                position = format!("{} · {position}", files.message);
            }
        } else if let Some(preview_text) = files.preview.as_deref() {
            let dark = theme.is_dark();
            let preview_path = files.preview_path.as_deref().unwrap_or(&files.directory);
            let preview = super::editor::Preview::new(preview_path, preview_text.to_owned(), dark);
            let total = preview.line_count();
            let max = total.saturating_sub(content.height as usize);
            files.max_scroll.set(max);
            let offset = files.scroll.min(max);
            let digits = total.max(1).to_string().len();
            let gutter = (digits as u16 + 1).min(content.width);
            let width = content.width.saturating_sub(gutter);
            let max_width = preview.max_width(dark);
            files
                .max_horizontal
                .set(max_width.saturating_sub(width as usize));
            let horizontal = files.horizontal.min(files.max_horizontal.get());
            preview.ensure_viewport(offset, content.height.max(1) as usize, dark);
            for row in 0..content.height as usize {
                let index = offset + row;
                if index >= total {
                    break;
                }
                let number = Rect::new(
                    content.x,
                    content.y + row as u16,
                    gutter.saturating_sub(1),
                    1,
                );
                frame.render_widget(
                    Paragraph::new(format!("{:>digits$}", index + 1))
                        .style(theme.fg(theme.text_dim)),
                    number,
                );
                let text = Rect::new(content.x + gutter, number.y, width, 1);
                frame.render_widget(
                    Paragraph::new(preview.render_line(index, width as usize, horizontal, &theme)),
                    text,
                );
            }
            position = format!("{}  {}/{}", preview.language(), offset + 1, total);
            if !files.message.is_empty() {
                position = format!("{} · {position}", files.message);
            }
        } else if !files.message.is_empty() {
            frame.render_widget(
                Paragraph::new(files.message.as_str()).wrap(Wrap { trim: false }),
                content,
            );
            position = String::new();
        } else {
            let start = files
                .selected
                .saturating_sub(content.height.saturating_sub(1) as usize);
            for (row, (index, entry)) in files
                .entries
                .iter()
                .enumerate()
                .skip(start)
                .take(content.height as usize)
                .enumerate()
            {
                let rect = Rect::new(content.x, content.y + row as u16, content.width, 1);
                let label = format!(
                    "{} {}{}",
                    if index == files.selected { "›" } else { " " },
                    entry.path.file_name().unwrap_or_default().to_string_lossy(),
                    if entry.directory { "/" } else { "" }
                );
                let style = if index == files.selected || layout::hovered(rect, pointer) {
                    theme.fg(theme.rose).bg(theme.hairline)
                } else {
                    theme.fg(theme.text)
                };
                frame.render_widget(Paragraph::new(label).style(style), rect);
                hits.insert(0, (rect, Action::File(index)));
            }
            position = format!(
                "{} / {}",
                if files.entries.is_empty() {
                    0
                } else {
                    files.selected + 1
                },
                files.entries.len()
            );
        }
        if body.height > 1 {
            frame.render_widget(
                Paragraph::new(position).style(theme.fg(theme.text_dim)),
                Rect::new(body.x, body.bottom() - 1, body.width, 1),
            );
        }
        if let Some(comparison) = &files.comparison {
            hits.clear();
            frame.render_widget(ratatui::widgets::Clear, body);
            let header = Rect::new(body.x, body.y, body.width, body.height.min(2));
            frame.render_widget(
                Paragraph::new("磁盘差异 · Esc保留草稿\nR重新加载并丢弃草稿 · ↑↓滚动")
                    .style(theme.fg(theme.rose)),
                header,
            );
            if header.height > 0 {
                hits.push((
                    Rect::new(header.x, header.y, header.width, 1),
                    Action::CompareKeep,
                ));
            }
            if header.height > 1 {
                hits.push((
                    Rect::new(header.x, header.y + 1, header.width, 1),
                    Action::CompareReload,
                ));
            }
            let diff = Rect::new(
                body.x,
                header.bottom(),
                body.width,
                body.height.saturating_sub(header.height),
            );
            for (row, line) in comparison
                .lines
                .iter()
                .skip(comparison.scroll)
                .take(diff.height as usize)
                .enumerate()
            {
                let style = if line.starts_with('+') {
                    theme.fg(theme.rose).bg(theme.hairline)
                } else if line.starts_with('-') {
                    theme.fg(theme.text_dim).add_modifier(Modifier::CROSSED_OUT)
                } else {
                    theme.fg(theme.text)
                };
                frame.render_widget(
                    Paragraph::new(line.as_str()).style(style),
                    Rect::new(diff.x, diff.y + row as u16, diff.width, 1),
                );
            }
            return None;
        }
        if files.pending_leave.is_some() {
            frame.render_widget(ratatui::widgets::Clear, content);
            frame.render_widget(
                Paragraph::new(i18n::tr(
                    "right.file_unsaved",
                    "文件有未保存修改。请选择：\n保存后离开 / 放弃修改 / 取消离开",
                ))
                .wrap(Wrap { trim: false })
                .style(theme.fg(theme.rose)),
                content,
            );
            let labels = [
                (
                    i18n::tr("right.file_confirm_save", "[S 保存]"),
                    Action::FileConfirm(true),
                ),
                (
                    i18n::tr("right.file_confirm_discard", "[D 放弃]"),
                    Action::FileConfirm(false),
                ),
                (
                    i18n::tr("right.file_confirm_cancel", "[C 取消]"),
                    Action::FileCancel,
                ),
            ];
            for (index, (label, action)) in labels.into_iter().enumerate() {
                if index + 3 >= content.height as usize {
                    break;
                }
                let rect = Rect::new(content.x, content.y + index as u16 + 3, content.width, 1);
                frame.render_widget(Paragraph::new(label).style(theme.fg(theme.rose)), rect);
                hits.insert(0, (rect, action));
            }
            if !files.message.is_empty() && content.height > 7 {
                frame.render_widget(
                    Paragraph::new(files.message.as_str()).wrap(Wrap { trim: false }),
                    Rect::new(content.x, content.y + 7, content.width, content.height - 7),
                );
            }
        }
        files.editor.as_ref().map(|_| content)
    }
}
