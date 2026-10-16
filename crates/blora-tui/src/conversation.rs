// SPDX-License-Identifier: GPL-3.0-or-later
use crate::{
    theme::Theme,
    view::{self, FrameModel, TranscriptCache},
};
use blora_types::Mode;
use ratatui::{
    Frame,
    layout::Rect,
    text::{Line, Span},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthChar;
use view::{
    highlight_selection, spinner_line, tool_summary_hit_rows, transcript_window, welcome_lines,
};

pub(crate) fn render_transcript(
    frame: &mut Frame<'_>,
    inner: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    cache: &mut TranscriptCache,
) -> Vec<(Rect, Vec<usize>)> {
    let width = inner.width.saturating_sub(2).max(8) as usize;
    let session_id = model
        .projection
        .and_then(|projection| projection.session.as_ref())
        .map(|session| session.id.to_string())
        .unwrap_or_default();
    if let Some(projection) = model.projection {
        cache.ensure(
            projection,
            &session_id,
            model.search,
            model.hide_tools,
            model.message_roles,
            width,
            model.user_label,
            theme,
        );
    }
    let show_cached = model.projection.is_some() && !cache.lines.is_empty();
    let spinner = show_cached && cache.live_visible && model.running;
    let line_count = if show_cached {
        cache.lines.len() + usize::from(spinner)
    } else {
        0
    };
    let visible = if show_cached {
        transcript_window(
            &cache.lines,
            spinner.then(|| spinner_line(model.tick, theme)),
            inner.height as usize,
            model.scroll,
        )
    } else {
        welcome_lines(inner.width as usize, inner.height as usize, theme)
    };
    let visible = highlight_selection(visible, inner, model.text_selection, theme);
    frame.render_widget(Paragraph::new(visible).style(theme.base()), inner);
    if show_cached {
        tool_summary_hit_rows(&cache.marks, line_count, inner, model.scroll)
    } else {
        Vec::new()
    }
}

pub(crate) fn render_composer(
    frame: &mut Frame<'_>,
    prompt: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
) -> u16 {
    render_editor(frame, prompt, model, theme, None).1
}

pub(crate) fn render_editor(
    frame: &mut Frame<'_>,
    prompt: Rect,
    model: &FrameModel<'_>,
    theme: &Theme,
    cursor: Option<usize>,
) -> (u16, u16) {
    let available = prompt.width.saturating_sub(3) as usize;
    let (visible, cursor_row, cursor_col) = if let Some(cursor) = cursor {
        let (lines, (row, col)) = draft_lines(model.input, cursor, available.max(1));
        let offset = row.saturating_sub(prompt.height.saturating_sub(1) as usize);
        (
            lines
                .into_iter()
                .skip(offset)
                .take(prompt.height as usize)
                .collect::<Vec<_>>(),
            (row - offset) as u16,
            col as u16,
        )
    } else {
        let (line, col) = view::visible_input(model.input, available);
        (vec![line], 0, col)
    };
    let imagine = model
        .projection
        .and_then(|p| p.session.as_ref())
        .is_some_and(|s| s.mode == Mode::Imagine);
    let empty = model.input.is_empty();
    let lines = visible
        .into_iter()
        .enumerate()
        .map(|(row, text)| {
            Line::from(vec![
                Span::styled(if row == 0 { "❯ " } else { "  " }, theme.rose_bold()),
                if empty {
                    Span::styled(
                        view::composer_placeholder(imagine, model.running, available),
                        theme.mute(),
                    )
                } else {
                    Span::styled(text, theme.base())
                },
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).style(theme.base()), prompt);
    (cursor_row, cursor_col)
}

pub(crate) fn browse_input_history(
    history: &[String],
    index: &mut Option<usize>,
    draft: &mut String,
    input: &mut String,
    older: bool,
) {
    if history.is_empty() {
        return;
    }
    if older {
        if index.is_none() {
            draft.clone_from(input);
        }
        let next = index.map_or(history.len() - 1, |position| position.saturating_sub(1));
        *index = Some(next);
        input.clone_from(&history[next]);
    } else if let Some(position) = *index {
        if position + 1 < history.len() {
            *index = Some(position + 1);
            input.clone_from(&history[position + 1]);
        } else {
            *index = None;
            input.clone_from(draft);
        }
    }
}

pub(crate) fn insert_input(input: &mut String, cursor: &mut usize, text: &str) {
    input.insert_str(*cursor, text);
    *cursor += text.len();
}

pub(crate) fn edit_input(input: &mut String, cursor: &mut usize, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    let prev = input[..*cursor]
        .char_indices()
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0);
    let next = input[*cursor..]
        .chars()
        .next()
        .map(|c| *cursor + c.len_utf8())
        .unwrap_or(*cursor);
    match key.code {
        KeyCode::Left => *cursor = prev,
        KeyCode::Right => *cursor = next,
        KeyCode::Home => *cursor = input[..*cursor].rfind('\n').map(|i| i + 1).unwrap_or(0),
        KeyCode::End => {
            *cursor = input[*cursor..]
                .find('\n')
                .map(|i| *cursor + i)
                .unwrap_or(input.len())
        }
        KeyCode::Backspace if *cursor > 0 => {
            input.replace_range(prev..*cursor, "");
            *cursor = prev;
        }
        KeyCode::Delete => {
            input.replace_range(*cursor..next, "");
        }
        _ => {}
    }
}

pub(crate) fn draft_lines(
    text: &str,
    cursor: usize,
    width: usize,
) -> (Vec<String>, (usize, usize)) {
    let width = width.max(1);
    let mut lines = vec![String::new()];
    let (mut row, mut col) = (0, 0);
    let mut position = (0, 0);
    for (offset, c) in text.char_indices() {
        let w = c.width().unwrap_or(0).min(width);
        if c != '\n' && col + w > width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        if offset == cursor {
            position = (row, col);
        }
        if c == '\n' {
            lines.push(String::new());
            row += 1;
            col = 0;
        } else {
            lines[row].push(c);
            col += w;
        }
    }
    if cursor == text.len() {
        if col == width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        position = (row, col);
    }
    (lines, position)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn shared_editor_preserves_unicode_cursor_and_history_draft() {
        let mut input = "中文".to_owned();
        let mut cursor = input.len();
        edit_input(
            &mut input,
            &mut cursor,
            KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        );
        assert_eq!(input, "中");
        insert_input(&mut input, &mut cursor, "a\n你");
        edit_input(
            &mut input,
            &mut cursor,
            KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        );
        edit_input(
            &mut input,
            &mut cursor,
            KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
        );
        assert_eq!(input, "中a\n");
        let history = vec!["旧消息".to_owned()];
        let mut index = None;
        let mut draft = String::new();
        browse_input_history(&history, &mut index, &mut draft, &mut input, true);
        assert_eq!(input, "旧消息");
        browse_input_history(&history, &mut index, &mut draft, &mut input, false);
        assert_eq!(input, "中a\n");
        assert_eq!(index, None);
    }
}

pub(crate) fn render_mentions(
    frame: &mut Frame<'_>,
    area: Rect,
    mentions: &crate::session_mentions::SessionMentions,
    theme: &Theme,
) {
    mentions.render(frame, area, theme);
}
