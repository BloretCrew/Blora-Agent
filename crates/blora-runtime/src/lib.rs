// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! In-process runtime shared by CLI, TUI, and Web.

mod agents;
mod coordinator;
mod work;

pub use agents::{MAX_RUNNING_CHILDREN, MAX_SUBAGENT_DEPTH, SUBAGENT_TURNS};
pub use blora_types::CancelToken;
pub use coordinator::{MockRunOptions, RunOptions, Runtime};
