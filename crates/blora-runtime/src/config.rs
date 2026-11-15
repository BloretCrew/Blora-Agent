// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Explicit runtime configuration.
//!
//! `RuntimeConfig::legacy()` preserves the historical environment-driven
//! behavior used by the CLI, TUI, and web server. SDK hosts use
//! `RuntimeConfig::sdk_default()` and opt into capabilities explicitly.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use blora_exec::Isolation;
use blora_model::Provider;
use blora_tools::ToolServices;
use blora_types::{ApprovalId, RunId, SessionId};

/// Where the runtime looks for global Blora state.
#[derive(Clone, Debug, Default)]
pub enum HomeMode {
    /// `$BLORA_HOME` or `~/.blora` (historical behavior).
    #[default]
    EnvironmentDefault,
    /// An explicit directory. Nothing outside it is read.
    Explicit(PathBuf),
    /// No global state at all.
    Disabled,
}

/// How the runtime obtains model providers.
#[derive(Clone, Default)]
pub enum ProviderSource {
    /// Resolve from `BLORA_PROVIDER`, saved providers, and API-key env vars.
    #[default]
    FromEnvironment,
    /// Use exactly this provider.
    Explicit(Arc<dyn Provider>),
    /// Try providers in order.
    Chain(Vec<Arc<dyn Provider>>),
    /// Fail every run with a provider error.
    Disabled,
}

/// Tool allow/deny policy. `None` allowlist means "all tools of the active
/// mode are allowed"; `Some(empty)` means "only host tools are allowed".
#[derive(Clone, Debug, Default)]
pub struct ToolPolicy {
    pub allowlist: Option<BTreeSet<String>>,
    pub denylist: BTreeSet<String>,
}

impl ToolPolicy {
    #[must_use]
    pub fn allows(&self, name: &str) -> bool {
        if self.denylist.contains(name) {
            return false;
        }
        match &self.allowlist {
            Some(allowed) => allowed.contains(name),
            None => true,
        }
    }

    /// Restrict the runtime to exactly the named tools.
    #[must_use]
    pub fn only(names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            allowlist: Some(names.into_iter().map(Into::into).collect()),
            denylist: BTreeSet::new(),
        }
    }
}

/// System prompt overrides. Empty fields keep the built-in prompts.
#[derive(Clone, Debug, Default)]
pub struct PromptConfig {
    pub system: Option<String>,
    pub append: Option<String>,
    pub mode_label: Option<String>,
}

/// Approval request passed to a host handler.
#[derive(Clone, Debug)]
pub struct ApprovalRequest {
    pub approval_id: ApprovalId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub capability: String,
    pub summary: String,
}

/// Host decision for an approval request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Allow,
    Deny,
    /// Leave the request pending for a human/UI.
    AskUser,
}

pub trait ApprovalHandler: Send + Sync {
    fn decide(&self, request: &ApprovalRequest) -> ApprovalDecision;
}

/// How approvals are resolved.
#[derive(Clone)]
pub enum ApprovalPolicy {
    /// Emit the request and wait for `resolve_approval`. `None` timeout
    /// preserves the historical "wait forever" behavior.
    Manual {
        timeout: Option<Duration>,
    },
    /// Ask the host synchronously; `AskUser` falls back to manual waiting.
    Auto(Arc<dyn ApprovalHandler>),
    DenyAll,
    AllowAll,
    /// Allow tools known to be read-only; deny everything else.
    AllowReadOnly,
}

impl Default for ApprovalPolicy {
    fn default() -> Self {
        Self::Manual { timeout: None }
    }
}

/// Question request passed to a host handler.
#[derive(Clone, Debug)]
pub struct QuestionRequest {
    pub session_id: SessionId,
    pub run_id: RunId,
    pub request_id: String,
    pub questions: Vec<blora_events::AskedQuestion>,
}

pub trait QuestionHandler: Send + Sync {
    fn answer(&self, request: &QuestionRequest) -> QuestionAnswer;
}

pub enum QuestionAnswer {
    Answer(String),
    Dismiss,
    AskUser,
}

/// How `ask_user` is resolved.
#[derive(Clone)]
pub enum QuestionPolicy {
    Manual {
        timeout: Option<Duration>,
    },
    Auto(Arc<dyn QuestionHandler>),
    /// Do not block; the model receives the `NO_USER` guidance.
    Dismiss,
}

impl Default for QuestionPolicy {
    fn default() -> Self {
        Self::Manual { timeout: None }
    }
}

/// Approval and question callbacks.
#[derive(Clone, Default)]
pub struct CallbackConfig {
    pub approval: ApprovalPolicy,
    pub question: QuestionPolicy,
}

/// What the context compiler is allowed to read from disk.
#[derive(Clone, Debug)]
pub struct ContextConfig {
    pub include_rules: bool,
    pub include_skills: bool,
    pub include_memory: bool,
    pub git_snapshot: bool,
    pub home: Option<PathBuf>,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            include_rules: true,
            include_skills: true,
            include_memory: true,
            git_snapshot: true,
            home: None,
        }
    }
}

/// Feature switches for optional subsystems.
#[derive(Clone, Debug)]
pub struct Features {
    pub mcp: bool,
    pub hooks: bool,
    pub plugins: bool,
    pub workspace_rules: bool,
    pub global_rules: bool,
    pub skills: bool,
    pub memory: bool,
    pub subagents: bool,
    pub scheduler: bool,
    pub background_processes: bool,
    pub worktree: bool,
    pub external_harness: bool,
    pub auto_title: bool,
    pub auto_compact: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self {
            mcp: true,
            hooks: true,
            plugins: true,
            workspace_rules: true,
            global_rules: true,
            skills: true,
            memory: true,
            subagents: true,
            scheduler: true,
            background_processes: true,
            worktree: true,
            external_harness: true,
            auto_title: true,
            auto_compact: true,
        }
    }
}

/// Run budgets. `None` means "fall back to the environment when
/// `inherit_env` is true, otherwise use the built-in default".
#[derive(Clone, Debug)]
pub struct Limits {
    pub max_turns: u32,
    pub max_wall_secs: Option<u64>,
    pub max_tokens: Option<u64>,
    pub context_window: Option<u64>,
    pub compact_pct: Option<u64>,
    pub max_tool_result_chars: Option<usize>,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_turns: crate::DEFAULT_MAX_TURNS,
            max_wall_secs: None,
            max_tokens: None,
            context_window: None,
            compact_pct: None,
            max_tool_result_chars: None,
        }
    }
}

/// Explicit runtime configuration.
#[derive(Clone)]
pub struct RuntimeConfig {
    /// When false, no `BLORA_*`, `HOME`, or `dirs::config_dir` lookup happens.
    pub inherit_env: bool,
    pub provider: ProviderSource,
    pub default_model: Option<String>,
    pub home: HomeMode,
    pub network: Option<bool>,
    pub isolation: Isolation,
    pub hooks_dir: Option<PathBuf>,
    pub plugins_dir: Option<PathBuf>,
    pub mcp_config: Option<PathBuf>,
    pub include_workspace_mcp: bool,
    pub include_workspace_plugins: bool,
    /// True restores eager MCP/plugin declarations (`BLORA_TOOL_SEARCH=0`).
    pub tool_search_eager: bool,
    pub worktree: bool,
    pub auto_title: bool,
    pub auto_compact: bool,
    pub limits: Limits,
    pub tools: ToolPolicy,
    pub prompt: PromptConfig,
    pub context: ContextConfig,
    pub callbacks: CallbackConfig,
    pub tool_services: ToolServices,
    /// Hard capability boundary applied even if a caller later broadens the tool allowlist.
    pub web_profile_lock: bool,
    pub features: Features,
}

impl RuntimeConfig {
    /// Historical behavior: exactly what the CLI, TUI, and web server do.
    #[must_use]
    pub fn legacy() -> Self {
        Self {
            inherit_env: true,
            provider: ProviderSource::FromEnvironment,
            default_model: None,
            home: HomeMode::EnvironmentDefault,
            network: None,
            isolation: Isolation::from_env(),
            hooks_dir: None,
            plugins_dir: None,
            mcp_config: None,
            include_workspace_mcp: true,
            include_workspace_plugins: true,
            tool_search_eager: false,
            worktree: false,
            auto_title: true,
            auto_compact: true,
            limits: Limits::default(),
            tools: ToolPolicy::default(),
            prompt: PromptConfig::default(),
            context: ContextConfig::default(),
            callbacks: CallbackConfig::default(),
            tool_services: ToolServices::legacy(),
            web_profile_lock: false,
            features: Features::default(),
        }
    }

    /// Conservative defaults for embedding. The host must still supply a
    /// provider and choose features.
    #[must_use]
    pub fn sdk_default() -> Self {
        let context = ContextConfig {
            include_rules: false,
            include_skills: false,
            include_memory: false,
            git_snapshot: false,
            ..ContextConfig::default()
        };
        Self {
            inherit_env: false,
            provider: ProviderSource::Disabled,
            default_model: None,
            home: HomeMode::Disabled,
            network: Some(false),
            isolation: Isolation::Local,
            hooks_dir: None,
            plugins_dir: None,
            mcp_config: None,
            include_workspace_mcp: false,
            include_workspace_plugins: false,
            tool_search_eager: false,
            worktree: false,
            auto_title: false,
            auto_compact: true,
            limits: Limits::default(),
            tools: ToolPolicy::default(),
            prompt: PromptConfig::default(),
            context,
            callbacks: CallbackConfig {
                approval: ApprovalPolicy::AllowReadOnly,
                question: QuestionPolicy::Dismiss,
            },
            tool_services: ToolServices::sdk_default(),
            web_profile_lock: true,
            features: Features {
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
            },
        }
    }

    /// Effective wall-clock budget, falling back to `BLORA_MAX_WALL_SECS`
    /// only when `inherit_env` is true.
    #[must_use]
    pub fn max_wall_secs(&self) -> Option<u64> {
        self.limits.max_wall_secs.or_else(|| {
            self.inherit_env
                .then(|| env_parse::<u64>("BLORA_MAX_WALL_SECS"))
                .flatten()
        })
    }

    /// Effective cumulative token budget.
    #[must_use]
    pub fn max_tokens(&self) -> Option<u64> {
        self.limits.max_tokens.or_else(|| {
            self.inherit_env
                .then(|| env_parse::<u64>("BLORA_MAX_TOKENS"))
                .flatten()
        })
    }

    /// Effective context window override. `None` means use model metadata and
    /// only consult the environment for legacy configs.
    #[must_use]
    pub fn context_window(&self, model_window: Option<u64>) -> u64 {
        blora_context::context_window_for_override(
            model_window,
            self.limits.context_window,
            self.inherit_env,
        )
    }

    /// Effective compaction threshold percentage.
    #[must_use]
    pub fn compact_pct(&self) -> u64 {
        blora_context::compact_threshold_pct_override(self.limits.compact_pct, self.inherit_env)
    }

    /// Effective model override.
    #[must_use]
    pub fn default_model(&self) -> Option<String> {
        self.default_model.clone().or_else(|| {
            self.inherit_env
                .then(|| std::env::var("BLORA_MODEL").ok())
                .flatten()
        })
    }

    /// Whether MCP/plugin tools should be declared eagerly.
    #[must_use]
    pub fn tool_search_eager(&self) -> bool {
        self.tool_search_eager
            || (self.inherit_env
                && std::env::var("BLORA_TOOL_SEARCH").is_ok_and(|value| value == "0"))
    }

    /// Whether run creates a detached git worktree.
    #[must_use]
    pub fn worktree_enabled(&self) -> bool {
        self.features.worktree
            && (self.worktree || (self.inherit_env && env_flag("BLORA_WORKTREE")))
    }

    /// Hooks directory, falling back to `BLORA_HOOKS_DIR`.
    #[must_use]
    pub fn hooks_dir(&self) -> Option<PathBuf> {
        if !self.features.hooks {
            return None;
        }
        self.hooks_dir.clone().or_else(|| {
            self.inherit_env
                .then(|| std::env::var_os("BLORA_HOOKS_DIR").map(PathBuf::from))
                .flatten()
        })
    }

    pub fn mcp_config_path(&self) -> Option<PathBuf> {
        if !self.features.mcp {
            return None;
        }
        self.mcp_config.clone().or_else(|| {
            self.inherit_env
                .then(|| std::env::var_os("BLORA_MCP_CONFIG").map(PathBuf::from))
                .flatten()
        })
    }

    #[must_use]
    pub fn mcp_workspace_enabled(&self) -> bool {
        self.include_workspace_mcp && self.features.mcp
    }

    #[must_use]
    pub fn plugins_workspace_enabled(&self) -> bool {
        self.include_workspace_plugins && self.features.plugins
    }

    /// Plugin directory, falling back to `BLORA_PLUGINS_DIR`.
    #[must_use]
    pub fn plugins_directory(&self) -> Option<PathBuf> {
        if !self.features.plugins {
            return None;
        }
        self.plugins_dir.clone().or_else(|| {
            self.inherit_env
                .then(|| std::env::var_os("BLORA_PLUGINS_DIR").map(PathBuf::from))
                .flatten()
        })
    }

    /// Explicit network permission. `None` means "let `Policy` consult the
    /// environment (legacy) or deny (SDK)".
    #[must_use]
    pub fn network_allowed(&self) -> Option<bool> {
        self.network.or_else(|| {
            self.inherit_env
                .then(|| Some(env_flag("BLORA_NETWORK")))
                .flatten()
        })
    }
}

fn env_parse<T: std::str::FromStr>(name: &str) -> Option<T> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}
