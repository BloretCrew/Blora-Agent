// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_events::{
    CheckpointCreated, ContextCompactionCompleted, ContextCompactionStarted, KnownPayload,
};
use blora_session::TranscriptItem;
use blora_types::{Result, SessionId};
use chrono::Utc;

use crate::Runtime;

impl Runtime {
    pub fn compact(&self, session_id: &SessionId) -> Result<String> {
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ContextCompactionStarted(ContextCompactionStarted {
                reason: Some("manual".to_owned()),
            }),
        )?;
        let projection = self.show_session(session_id)?;
        let mut lines = Vec::new();
        for item in projection.transcript.iter().rev().take(24).rev() {
            let line = match item {
                TranscriptItem::User { text, .. } => format!("user: {}", truncate(text, 240)),
                TranscriptItem::Assistant { text, .. } => {
                    format!("assistant: {}", truncate(text, 240))
                }
                TranscriptItem::Tool { name, status, .. } => format!("tool {name} ({status})"),
                TranscriptItem::System { summary, .. } => format!("system: {summary}"),
            };
            lines.push(line);
        }
        let summary = if lines.is_empty() {
            "(empty session)".to_owned()
        } else {
            lines.join("\n")
        };
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ContextCompactionCompleted(ContextCompactionCompleted {
                summary: summary.clone(),
            }),
        )?;
        self.checkpoint(session_id, None, Some("compaction"))?;
        Ok(summary)
    }

    pub fn checkpoint(
        &self,
        session_id: &SessionId,
        run_id: Option<&blora_types::RunId>,
        note: Option<&str>,
    ) -> Result<()> {
        let projection = self.show_session(session_id)?;
        let sequence = projection.last_sequence;
        self.store
            .insert_checkpoint(session_id, run_id, sequence, note, Utc::now())?;
        self.emit(
            session_id,
            run_id,
            None,
            KnownPayload::CheckpointCreated(CheckpointCreated {
                sequence,
                note: note.map(ToOwned::to_owned),
            }),
        )?;
        Ok(())
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}
