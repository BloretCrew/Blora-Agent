// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! In-process runtime shared by CLI, TUI, and Web.

mod agents;
mod approvals;
mod compact;
mod coordinator;
mod cron;
pub mod hooks;
mod market;
mod mcp;
mod memory;
mod plugins;
mod work;

pub use agents::{MAX_RUNNING_CHILDREN, MAX_SUBAGENT_DEPTH, SUBAGENT_TURNS};
pub use blora_exec::{GitIndicator, command, open_url};
pub use blora_policy::PermissionMode;
pub use blora_types::CancelToken;
pub use compact::{COMPACTION_BREAKER, CompactionReport};
pub use coordinator::{MockRunOptions, RunOptions, Runtime, WorkspaceInfo};
pub use cron::next_cron;
pub use market::{
    Marketplace, install as install_plugin, load_index, uninstall as uninstall_plugin,
};
pub use plugins::PluginSpec;
