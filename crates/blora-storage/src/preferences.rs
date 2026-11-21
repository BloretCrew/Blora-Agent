// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{BloraError, Result};
use rusqlite::OptionalExtension;

use crate::SqliteStore;

impl SqliteStore {
    pub fn tui_reduce_motion(&self) -> Result<bool> {
        self.lock()
            .query_row(
                "SELECT value FROM tui_preferences WHERE key = 'reduce-motion'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or(false))
            .map_err(BloraError::storage)
    }
    pub fn set_tui_reduce_motion(&self, enabled: bool) -> Result<()> {
        self.lock().execute("INSERT INTO tui_preferences (key, value) VALUES ('reduce-motion', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value", [enabled]).map_err(BloraError::storage)?;
        Ok(())
    }
    pub fn tui_message_roles(&self) -> Result<bool> {
        self.lock()
            .query_row(
                "SELECT value FROM tui_preferences WHERE key = 'message-roles'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map(|value| value.unwrap_or(true))
            .map_err(BloraError::storage)
    }

    pub fn set_tui_message_roles(&self, enabled: bool) -> Result<()> {
        self.lock()
            .execute(
                "INSERT INTO tui_preferences (key, value) VALUES ('message-roles', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [enabled],
            )
            .map_err(BloraError::storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_roles_default_to_visible_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let store = SqliteStore::open(&path).unwrap();
        assert!(store.tui_message_roles().unwrap());
        store.set_tui_message_roles(false).unwrap();
        drop(store);
        let store = SqliteStore::open(&path).unwrap();
        assert!(!store.tui_message_roles().unwrap());
        store.set_tui_message_roles(true).unwrap();
        assert!(store.tui_message_roles().unwrap());
    }
    #[test]
    fn reduced_editor_motion_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("motion.sqlite");
        let store = SqliteStore::open(&path).unwrap();
        assert!(!store.tui_reduce_motion().unwrap());
        store.set_tui_reduce_motion(true).unwrap();
        drop(store);
        assert!(
            SqliteStore::open(&path)
                .unwrap()
                .tui_reduce_motion()
                .unwrap()
        );
    }
}
