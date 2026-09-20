// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_session::{SessionProjection, TranscriptItem};
use ratatui::layout::Rect;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub start: (u16, u16),
    pub end: (u16, u16),
}

pub fn selected_text(
    projection: &SessionProjection,
    rect: Rect,
    selection: Selection,
    scroll: usize,
    hide_tools: bool,
) -> String {
    if !rect.contains(selection.start.into()) && !rect.contains(selection.end.into()) {
        return String::new();
    }
    let lines = transcript_lines(projection, hide_tools);
    let height = rect.height as usize;
    if height == 0 || lines.is_empty() {
        return String::new();
    }
    let start_row = selection.start.1.min(selection.end.1);
    let end_row = selection.start.1.max(selection.end.1);
    let first = start_row.saturating_sub(rect.y) as usize;
    let last = end_row.saturating_sub(rect.y) as usize;
    let total = lines.len();
    let from_bottom = scroll.min(total.saturating_sub(height));
    let window_start = total.saturating_sub(height).saturating_sub(from_bottom);
    let first = window_start.saturating_add(first);
    let last = window_start
        .saturating_add(last)
        .min(total.saturating_sub(1));
    let mut out = Vec::new();
    for (index, line) in lines
        .iter()
        .enumerate()
        .skip(first)
        .take(last.saturating_sub(first) + 1)
    {
        let mut text = line.clone();
        if index == first {
            let column = if selection.start.1 <= selection.end.1 {
                selection.start.0
            } else {
                selection.end.0
            };
            text = text
                .chars()
                .skip(column.saturating_sub(rect.x) as usize)
                .collect();
        }
        if index == last {
            let column = if selection.start.1 <= selection.end.1 {
                selection.end.0
            } else {
                selection.start.0
            };
            text = text
                .chars()
                .take(column.saturating_sub(rect.x) as usize)
                .collect();
        }
        out.push(text.trim_end().to_owned());
    }
    out.join("\n").trim().to_owned()
}

fn transcript_lines(projection: &SessionProjection, hide_tools: bool) -> Vec<String> {
    projection
        .transcript
        .iter()
        .filter(|item| !(hide_tools && matches!(item, TranscriptItem::Tool { .. })))
        .flat_map(|item| match item {
            TranscriptItem::User { text, .. }
            | TranscriptItem::Assistant { text, .. }
            | TranscriptItem::System { summary: text, .. } => {
                text.lines().map(str::to_owned).collect::<Vec<_>>()
            }
            TranscriptItem::Tool { name, status, .. } => vec![format!("{name} [{status}]")],
            TranscriptItem::Routing {
                to_provider,
                to_model,
                ..
            } => {
                vec![format!("切换到 {to_provider} / {to_model}")]
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_events::{EventEnvelope, KnownPayload, NewEvent, SessionCreated, UserInput};
    use blora_types::{Mode, SessionId};

    #[test]
    fn extracts_selected_rows() {
        let id = SessionId::generate();
        let events = vec![
            EventEnvelope::from_new(
                NewEvent::new(
                    id.clone(),
                    KnownPayload::SessionCreated(SessionCreated {
                        title: None,
                        workspace_path: ".".to_owned(),
                        mode: Mode::Code,
                        parent_session_id: None,
                    }),
                ),
                1,
                &blora_types::SystemClock,
            )
            .unwrap(),
            EventEnvelope::from_new(
                NewEvent::new(
                    id,
                    KnownPayload::UserInput(UserInput {
                        text: "hello\nworld".to_owned(),
                    }),
                ),
                2,
                &blora_types::SystemClock,
            )
            .unwrap(),
        ];
        let projection = blora_session::rebuild(&events).unwrap();
        let text = selected_text(
            &projection,
            Rect::new(0, 0, 20, 4),
            Selection {
                start: (0, 0),
                end: (5, 1),
            },
            0,
            false,
        );
        assert_eq!(text, "hello\nworld");
    }
}
