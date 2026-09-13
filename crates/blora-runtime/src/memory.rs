// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_session::TranscriptItem;
use blora_types::{Result, SessionId};
use chrono::Utc;

use crate::Runtime;

impl Runtime {
    pub fn list_memories(&self, workspace: &str) -> Result<Vec<blora_storage::MemoryRecord>> {
        self.store.list_memories(workspace)
    }

    pub fn remember(&self, workspace: &str, key: &str, value: &str) -> Result<()> {
        self.store.upsert_memory(workspace, key, value, Utc::now())
    }

    pub fn recall(
        &self,
        workspace: &str,
        key: &str,
    ) -> Result<Option<blora_storage::MemoryRecord>> {
        self.store.get_memory(workspace, key)
    }

    pub fn forget(&self, workspace: &str, key: &str) -> Result<bool> {
        self.store.delete_memory(workspace, key)
    }

    /// Extract durable memories from a session and write `.blora/memory.md`.
    pub fn distill_memories(&self, session_id: &SessionId) -> Result<u32> {
        let projection = self.show_session(session_id)?;
        let workspace = projection
            .session
            .as_ref()
            .map(|session| session.workspace_path.clone())
            .unwrap_or_default();
        if workspace.is_empty() {
            return Ok(0);
        }
        let mut stored = 0u32;
        if let Some(assistant) = projection.transcript.iter().rev().find_map(|item| {
            if let TranscriptItem::Assistant { text, .. } = item {
                Some(text.as_str())
            } else {
                None
            }
        }) {
            self.store.upsert_memory(
                &workspace,
                &format!("session:{session_id}:summary"),
                assistant,
                Utc::now(),
            )?;
            stored += 1;
            for line in assistant.lines() {
                let line = line.trim();
                let value = line
                    .strip_prefix("Remember:")
                    .or_else(|| line.strip_prefix("MEMORY:"))
                    .or_else(|| line.strip_prefix("记住："))
                    .or_else(|| line.strip_prefix("记忆："));
                if let Some(value) = value {
                    let key = format!("note:{}", stored);
                    self.store
                        .upsert_memory(&workspace, &key, value.trim(), Utc::now())?;
                    stored += 1;
                }
            }
        }
        let rows = self.store.list_memories(&workspace)?;
        let body = rows
            .iter()
            .map(|row| format!("- {}: {}", row.key, row.value))
            .collect::<Vec<_>>()
            .join("\n");
        let dir = std::path::Path::new(&workspace).join(".blora");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("memory.md"), body);
        Ok(stored)
    }
}
