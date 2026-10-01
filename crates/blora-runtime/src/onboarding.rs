// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::Runtime;
use blora_types::Result;

impl Runtime {
    pub fn tui_onboarding_version(&self) -> Result<u32> {
        self.store.onboarding_version("tui")
    }

    pub fn complete_tui_onboarding(&self, version: u32) -> Result<()> {
        self.store.complete_onboarding("tui", version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_is_separate_from_account_and_session_state() {
        let runtime = Runtime::new(blora_storage::SqliteStore::open_in_memory().unwrap());
        assert_eq!(runtime.tui_onboarding_version().unwrap(), 0);
        runtime.complete_tui_onboarding(1).unwrap();
        assert_eq!(runtime.tui_onboarding_version().unwrap(), 1);
        assert!(runtime.list_users().unwrap().is_empty());
        assert!(runtime.list_sessions().unwrap().is_empty());
    }
}
