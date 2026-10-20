// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use crate::Runtime;
use blora_types::Result;

impl Runtime {
    pub fn tui_reduce_motion(&self) -> Result<bool> {
        self.store.tui_reduce_motion()
    }
    pub fn set_tui_reduce_motion(&self, enabled: bool) -> Result<()> {
        self.store.set_tui_reduce_motion(enabled)
    }
    pub fn tui_message_roles(&self) -> Result<bool> {
        self.store.tui_message_roles()
    }

    pub fn set_tui_message_roles(&self, enabled: bool) -> Result<()> {
        self.store.set_tui_message_roles(enabled)
    }
}
