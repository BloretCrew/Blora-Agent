// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_types::{BloraError, Result};
use rusqlite::{OptionalExtension, params};

use crate::SqliteStore;

impl SqliteStore {
    pub fn onboarding_version(&self, surface: &str) -> Result<u32> {
        self.lock()
            .query_row(
                "SELECT version FROM onboarding WHERE surface = ?1",
                [surface],
                |row| row.get(0),
            )
            .optional()
            .map(|version| version.unwrap_or(0))
            .map_err(BloraError::storage)
    }

    pub fn complete_onboarding(&self, surface: &str, version: u32) -> Result<()> {
        self.lock().execute(
            "INSERT INTO onboarding (surface, version) VALUES (?1, ?2)
             ON CONFLICT(surface) DO UPDATE SET version = MAX(onboarding.version, excluded.version)",
            params![surface, version],
        ).map_err(BloraError::storage)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_independent_and_never_downgraded() {
        let store = SqliteStore::open_in_memory().unwrap();
        assert_eq!(store.onboarding_version("tui").unwrap(), 0);
        store.complete_onboarding("tui", 2).unwrap();
        store.complete_onboarding("tui", 1).unwrap();
        assert_eq!(store.onboarding_version("tui").unwrap(), 2);
        assert_eq!(store.onboarding_version("web").unwrap(), 0);
    }

    #[test]
    fn migrates_existing_database_and_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.sqlite");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE schema_migrations (version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL);
            INSERT INTO schema_migrations VALUES (10, 'old');").unwrap();
        drop(conn);
        let store = SqliteStore::open(&path).unwrap();
        assert_eq!(store.onboarding_version("tui").unwrap(), 0);
        store.complete_onboarding("tui", 1).unwrap();
        drop(store);
        assert_eq!(
            SqliteStore::open(&path)
                .unwrap()
                .onboarding_version("tui")
                .unwrap(),
            1
        );
    }
}
