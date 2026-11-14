// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_runtime::{ApprovalPolicy, Features, QuestionPolicy, RuntimeConfig, ToolPolicy};
use blora_types::Mode;
/// Capability preset for an embedded agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// Full harness: all built-ins, MCP, hooks, plugins, subagents, scheduling.
    Full,
    /// Web/chat agent: no filesystem or shell tools, in-memory storage.
    Web,
    /// Read-only workspace agent: file reads and optional LSP, no writes.
    ReadOnlyWorkspace,
}

impl Profile {
    #[must_use]
    pub fn full() -> Self {
        Self::Full
    }

    #[must_use]
    pub fn web() -> Self {
        Self::Web
    }

    #[must_use]
    pub fn read_only_workspace() -> Self {
        Self::ReadOnlyWorkspace
    }

    #[must_use]
    pub fn default_mode(self) -> Mode {
        match self {
            Self::Web => Mode::Work,
            Self::Full | Self::ReadOnlyWorkspace => Mode::Code,
        }
    }

    #[must_use]
    pub fn requires_workspace(self) -> bool {
        !matches!(self, Self::Web)
    }

    #[must_use]
    pub fn default_permission(self) -> blora_runtime::PermissionMode {
        match self {
            Self::ReadOnlyWorkspace => blora_runtime::PermissionMode::Plan,
            Self::Full | Self::Web => blora_runtime::PermissionMode::Ask,
        }
    }

    pub(crate) fn apply(self, config: &mut RuntimeConfig) {
        match self {
            Self::Full => {
                config.features = Features::default();
                config.context.include_rules = true;
                config.context.include_skills = true;
                config.context.include_memory = true;
                config.context.git_snapshot = true;
                config.auto_title = true;
                config.auto_compact = true;
                config.tools = ToolPolicy::default();
            }
            Self::Web => {
                config.features = Features {
                    mcp: false,
                    hooks: false,
                    plugins: false,
                    workspace_rules: false,
                    global_rules: false,
                    skills: false,
                    memory: false,
                    subagents: false,
                    scheduler: false,
                    background_processes: false,
                    worktree: false,
                    external_harness: false,
                    auto_title: false,
                    auto_compact: true,
                };
                config.context.include_rules = false;
                config.context.include_skills = false;
                config.context.include_memory = false;
                config.context.git_snapshot = false;
                config.auto_title = false;
                config.auto_compact = true;
                config.network = Some(true);
                config.limits.max_turns = 16;
                config.tools = ToolPolicy::only(["structured_output", "update_plan", "ask_user"]);
                config.callbacks.approval = ApprovalPolicy::AllowReadOnly;
                config.callbacks.question = QuestionPolicy::Dismiss;
            }
            Self::ReadOnlyWorkspace => {
                config.features = Features {
                    mcp: false,
                    hooks: false,
                    plugins: false,
                    workspace_rules: true,
                    global_rules: false,
                    skills: false,
                    memory: false,
                    subagents: false,
                    scheduler: false,
                    background_processes: false,
                    worktree: false,
                    external_harness: false,
                    auto_title: false,
                    auto_compact: true,
                };
                config.context.include_rules = true;
                config.context.include_skills = false;
                config.context.include_memory = false;
                config.context.git_snapshot = true;
                config.auto_title = false;
                config.network = Some(false);
                config.tools = ToolPolicy::only([
                    "read_file",
                    "view_image",
                    "list_dir",
                    "glob",
                    "search",
                    "grep",
                    "update_plan",
                    "structured_output",
                    "ask_user",
                ]);
                config.callbacks.approval = ApprovalPolicy::AllowReadOnly;
                config.callbacks.question = QuestionPolicy::Dismiss;
            }
        }
    }
}
