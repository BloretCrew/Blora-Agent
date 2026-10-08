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

    /// Store explicit `Remember:` lines from the latest reply and rewrite
    /// `.blora/memory.md` from durable memories.
    ///
    /// Session summaries are not memories: they stay searchable through
    /// `session_search` and are kept out of the prompt. Legacy
    /// `session:*:summary` rows are left in the store but no longer rendered.
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
            for line in assistant.lines() {
                let line = line.trim();
                let value = line
                    .strip_prefix("Remember:")
                    .or_else(|| line.strip_prefix("MEMORY:"))
                    .or_else(|| line.strip_prefix("记住："))
                    .or_else(|| line.strip_prefix("记忆："))
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                if let Some(value) = value {
                    let key = format!("note:{}", short_hash(value));
                    self.store
                        .upsert_memory(&workspace, &key, value, Utc::now())?;
                    stored += 1;
                }
            }
        }
        self.write_memory_file(&workspace)?;
        Ok(stored)
    }

    /// Render durable memories into `.blora/memory.md` (read by the context
    /// compiler). Session summaries are excluded.
    pub(crate) fn write_memory_file(&self, workspace: &str) -> Result<()> {
        let rows = self.store.list_memories(workspace)?;
        let body = rows
            .iter()
            .filter(|row| !is_session_summary(&row.key))
            .map(|row| format!("- {}: {}", row.key, row.value))
            .collect::<Vec<_>>()
            .join("\n");
        let dir = std::path::Path::new(workspace).join(".blora");
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("memory.md"), body);
        Ok(())
    }
}

fn is_session_summary(key: &str) -> bool {
    key.starts_with("session:") && key.ends_with(":summary")
}

/// Short stable key so repeated `Remember:` lines update one row.
fn short_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.trim().to_lowercase().bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{:08x}", hash as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summaries_are_not_memories() {
        assert!(is_session_summary("session:ses_1:summary"));
        assert!(!is_session_summary("note:1"));
        assert_eq!(short_hash("Use pnpm"), short_hash(" use PNPM "));
    }
}
