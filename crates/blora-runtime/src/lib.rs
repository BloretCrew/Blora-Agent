// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! In-process runtime shared by CLI, TUI, and Web.

mod agents;
mod approvals;
mod catalog;
mod compact;
mod coordinator;
mod cron;
mod external;
mod goals;
pub mod hooks;
mod input;
mod market;
mod mcp;
pub use mcp::McpManager;
mod memory;
mod onboarding;
mod plugins;
mod preferences;
mod session_mentions;
mod skills;
mod team;
mod work;
mod workflow;

pub use agents::{MAX_RUNNING_CHILDREN, MAX_SUBAGENT_DEPTH, SUBAGENT_TURNS};
pub use blora_exec::{GitIndicator, command, open_url};
pub use blora_policy::PermissionMode;
pub use blora_types::CancelToken;
pub use compact::{COMPACTION_BREAKER, CompactionReport};
pub use coordinator::{DEFAULT_MAX_TURNS, MockRunOptions, RunOptions, Runtime, WorkspaceInfo};
pub use cron::next_cron;
pub use input::{NO_USER, format_question_answers};
pub use market::{
    Marketplace, install as install_plugin, load_index, uninstall as uninstall_plugin,
};
pub use plugins::PluginSpec;
pub use skills::MAX_SKILL_BYTES;
