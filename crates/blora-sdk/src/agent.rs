// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use blora_runtime::{
    ApprovalPolicy, PermissionMode, QuestionPolicy, Runtime, RuntimeConfig, ToolPolicy,
};
use blora_storage::{CreateSession, SqliteStore};
use blora_types::{CancelToken, Mode, RunId, SessionId};

use crate::error::{Result, SdkError};
use crate::events::EventStream;
use crate::profile::Profile;
use crate::provider::ProviderConfig;
use crate::run::{RunHandle, RunRequest};
use crate::tool::{Tool, ToolAdapter};

/// Where session events are stored.
#[derive(Clone, Debug)]
pub enum Storage {
    /// Process-local in-memory SQLite. Sessions disappear when the agent drops.
    Memory,
    /// A SQLite file. Sessions survive restarts.
    File(PathBuf),
}

impl Storage {
    #[must_use]
    pub fn memory() -> Self {
        Self::Memory
    }

    #[must_use]
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self::File(path.into())
    }
}

pub(crate) struct ActiveRun {
    pub session_id: SessionId,
    pub cancel: CancelToken,
}

pub(crate) struct AgentInner {
    pub(crate) runtime: Arc<Runtime>,
    pub(crate) workspace: PathBuf,
    pub(crate) default_mode: Mode,
    pub(crate) default_permission: PermissionMode,
    pub(crate) web_profile: bool,
    pub(crate) active: Arc<Mutex<HashMap<RunId, ActiveRun>>>,
    pub(crate) shutdown: AtomicBool,
    pub(crate) has_started: AtomicBool,
    pub(crate) _temp_workspace: Option<tempfile::TempDir>,
}

/// Session creation parameters.
#[derive(Clone, Debug, Default)]
pub struct SessionSpec {
    pub title: Option<String>,
    pub workspace: Option<PathBuf>,
    pub mode: Option<Mode>,
    pub permission: Option<PermissionMode>,
}

/// Builder for an embedded agent.
pub struct AgentBuilder {
    config: RuntimeConfig,
    profile: Profile,
    workspace: Option<PathBuf>,
    ephemeral: bool,
    storage: Storage,
    provider: Option<ProviderConfig>,
    default_mode: Option<Mode>,
    permission: PermissionMode,
    tools: Vec<Arc<dyn Tool>>,
    custom_store: Option<Arc<dyn crate::extensions::Store>>,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentBuilder {
    #[must_use]
    pub fn new() -> Self {
        let mut config = RuntimeConfig::sdk_default();
        Profile::Full.apply(&mut config);
        Self {
            config,
            profile: Profile::Full,
            workspace: None,
            ephemeral: false,
            storage: Storage::Memory,
            provider: None,
            default_mode: None,
            permission: Profile::Full.default_permission(),
            tools: Vec::new(),
            custom_store: None,
        }
    }

    /// Apply a capability preset. Later builder calls override profile values.
    #[must_use]
    pub fn profile(mut self, profile: Profile) -> Self {
        let mut config = RuntimeConfig::sdk_default();
        profile.apply(&mut config);
        if self.config.inherit_env {
            config.home = self.config.home.clone();
            config.default_model = self.config.default_model.clone();
            config.isolation = self.config.isolation;
            config.hooks_dir = self.config.hooks_dir.clone();
            config.plugins_dir = self.config.plugins_dir.clone();
            config.mcp_config = self.config.mcp_config.clone();
            config.include_workspace_mcp = self.config.include_workspace_mcp;
            config.include_workspace_plugins = self.config.include_workspace_plugins;
            config.limits = self.config.limits.clone();
        }
        config.features.background_processes = false;
        config.features.auto_title = false;
        config.features.auto_compact = true;
        self.config = config;
        self.permission = profile.default_permission();
        self.profile = profile;
        self
    }

    #[must_use]
    pub fn workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace = Some(path.into());
        self.ephemeral = false;
        self
    }

    /// Create a private temporary workspace that is removed with the agent.
    #[must_use]
    pub fn ephemeral_workspace(mut self) -> Self {
        self.workspace = None;
        self.ephemeral = true;
        self
    }

    #[must_use]
    pub fn storage(mut self, storage: Storage) -> Self {
        self.storage = storage;
        self
    }

    #[must_use]
    pub fn memory(self) -> Self {
        self.storage(Storage::Memory)
    }

    #[must_use]
    pub fn sqlite(self, path: impl Into<PathBuf>) -> Self {
        self.storage(Storage::File(path.into()))
    }

    /// Provide a custom storage implementation for host-owned persistence.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn crate::extensions::Store>) -> Self {
        self.custom_store = Some(store);
        self
    }

    #[must_use]
    pub fn provider(mut self, provider: ProviderConfig) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Explicitly opt into environment-based provider resolution.
    #[must_use]
    pub fn provider_from_env(self) -> Self {
        self.provider(ProviderConfig::from_env())
    }

    #[must_use]
    pub fn mode(mut self, mode: Mode) -> Self {
        self.default_mode = Some(mode);
        self
    }

    #[must_use]
    pub fn permission(mut self, permission: PermissionMode) -> Self {
        self.permission = permission;
        self
    }

    #[must_use]
    pub fn tool(mut self, tool: Arc<dyn Tool>) -> Self {
        self.tools.push(tool);
        self
    }

    #[must_use]
    pub fn tools(mut self, tools: impl IntoIterator<Item = Arc<dyn Tool>>) -> Self {
        self.tools.extend(tools);
        self
    }

    #[must_use]
    pub fn approval_policy(mut self, policy: ApprovalPolicy) -> Self {
        self.config.callbacks.approval = policy;
        self
    }

    #[must_use]
    pub fn question_policy(mut self, policy: QuestionPolicy) -> Self {
        self.config.callbacks.question = policy;
        self
    }

    #[must_use]
    pub fn max_turns(mut self, turns: u32) -> Self {
        self.config.limits.max_turns = turns;
        self
    }

    #[must_use]
    pub fn max_wall_secs(mut self, secs: u64) -> Self {
        self.config.limits.max_wall_secs = Some(secs);
        self
    }

    #[must_use]
    pub fn max_tokens(mut self, tokens: u64) -> Self {
        self.config.limits.max_tokens = Some(tokens);
        self
    }

    #[must_use]
    pub fn network(mut self, allowed: bool) -> Self {
        self.config.network = Some(allowed);
        self
    }

    #[must_use]
    pub fn tool_policy(mut self, policy: ToolPolicy) -> Self {
        self.config.tools = policy;
        self
    }

    #[must_use]
    pub fn hooks_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.hooks_dir = Some(path.into());
        self
    }

    #[must_use]
    pub fn mcp_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.mcp_config = Some(path.into());
        self
    }

    #[must_use]
    pub fn plugins_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.config.plugins_dir = Some(path.into());
        self
    }

    #[must_use]
    pub fn home(mut self, path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        self.config.home = blora_runtime::HomeMode::Explicit(path.clone());
        self.config.context.home = Some(path);
        self
    }

    #[must_use]
    pub fn no_global_home(mut self) -> Self {
        self.config.home = blora_runtime::HomeMode::Disabled;
        self.config.features.global_rules = false;
        self
    }

    #[must_use]
    pub fn inherit_env(mut self, inherit: bool) -> Self {
        self.config.inherit_env = inherit;
        self
    }

    #[must_use]
    pub fn auto_title(mut self, enabled: bool) -> Self {
        self.config.auto_title = enabled;
        self
    }

    /// Mutate the underlying runtime configuration.
    #[must_use]
    pub fn configure(mut self, f: impl FnOnce(&mut RuntimeConfig)) -> Self {
        f(&mut self.config);
        self
    }

    pub fn build(self) -> Result<Agent> {
        let provider = self.provider.ok_or(SdkError::ProviderRequired)?;
        if !self.config.inherit_env && !provider.is_explicit() {
            return Err(SdkError::Config(
                "ProviderConfig::FromEnvironment requires inherit_env(true)".to_owned(),
            ));
        }

        let (workspace, temp_workspace) = match self.workspace {
            Some(path) if self.profile == Profile::Web => {
                return Err(SdkError::Config(format!(
                    "web profile does not accept a workspace path: {}",
                    path.display()
                )));
            }
            Some(path) => {
                if !path.is_dir() {
                    return Err(SdkError::Config(format!(
                        "workspace does not exist: {}",
                        path.display()
                    )));
                }
                (path, None)
            }
            None if self.ephemeral || self.profile == Profile::Web => {
                let temp = tempfile::tempdir()?;
                (temp.path().to_path_buf(), Some(temp))
            }
            None => {
                return Err(SdkError::Config(
                    "workspace is required for this profile".to_owned(),
                ));
            }
        };

        let mut config = self.config;
        let provider_model = provider.model();
        config.provider = provider.into_source();
        if config.default_model.is_none() {
            config.default_model = provider_model;
        }
        if self.custom_store.is_some() {
            return Err(SdkError::Unsupported(
                "custom Store adapters are declared but not yet wired into the current Runtime"
                    .to_owned(),
            ));
        }
        let store = match &self.storage {
            Storage::Memory => SqliteStore::open_in_memory()?,
            Storage::File(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                SqliteStore::open(path)?
            }
        };
        let runtime = Runtime::with_config(store, config);
        for tool in self.tools {
            runtime.register_tool(Arc::new(ToolAdapter(tool)))?;
        }

        Ok(Agent {
            inner: Arc::new(AgentInner {
                runtime: Arc::new(runtime),
                workspace,
                default_mode: self
                    .default_mode
                    .unwrap_or_else(|| self.profile.default_mode()),
                default_permission: self.permission,
                web_profile: self.profile == Profile::Web,
                active: Arc::new(Mutex::new(HashMap::new())),
                shutdown: AtomicBool::new(false),
                has_started: AtomicBool::new(false),
                _temp_workspace: temp_workspace,
            }),
        })
    }
}

/// Embedded agent handle. Cheap to clone.
#[derive(Clone)]
pub struct Agent {
    pub(crate) inner: Arc<AgentInner>,
}

impl Agent {
    #[must_use]
    pub fn builder() -> AgentBuilder {
        AgentBuilder::new()
    }

    #[must_use]
    pub fn workspace(&self) -> &Path {
        &self.inner.workspace
    }

    /// The runtime is exposed for advanced integrations. It does not grant
    /// additional capabilities beyond the profile's configured tool policy.
    #[must_use]
    pub fn runtime(&self) -> &Runtime {
        &self.inner.runtime
    }

    pub fn create_session(&self) -> Result<Session> {
        self.create_session_with(SessionSpec::default())
    }

    pub fn create_session_with(&self, spec: SessionSpec) -> Result<Session> {
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(SdkError::Closed);
        }
        let workspace = spec
            .workspace
            .clone()
            .unwrap_or_else(|| self.inner.workspace.clone());
        if self.inner.web_profile
            && workspace.canonicalize().ok() != self.inner.workspace.canonicalize().ok()
        {
            return Err(SdkError::Unsupported(
                "web profile sessions are confined to the agent workspace".to_owned(),
            ));
        }
        let id = self.inner.runtime.create_session(CreateSession {
            title: spec.title,
            workspace_path: workspace.to_string_lossy().into_owned(),
            mode: spec.mode.unwrap_or(self.inner.default_mode),
            parent_session_id: None,
        })?;
        if let Some(permission) = spec.permission {
            self.inner
                .runtime
                .set_permission_mode(&id, permission, "sdk")?;
        }
        Ok(Session {
            inner: self.inner.clone(),
            id,
        })
    }

    pub fn open_session(&self, id: &SessionId) -> Result<Session> {
        self.inner.runtime.show_session(id)?;
        Ok(Session {
            inner: self.inner.clone(),
            id: id.clone(),
        })
    }

    pub fn resume_session(&self, id: &SessionId) -> Result<Session> {
        self.inner.runtime.resume_session(id)?;
        self.open_session(id)
    }

    pub fn fork_session(&self, id: &SessionId) -> Result<Session> {
        let child = self.inner.runtime.fork_session(id)?;
        Ok(Session {
            inner: self.inner.clone(),
            id: child,
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<blora_storage::SessionSummary>> {
        Ok(self.inner.runtime.list_sessions()?)
    }

    /// Register a host tool after construction. Names must be unique.
    pub fn register_tool(&self, tool: Arc<dyn Tool>) -> Result<()> {
        if self.inner.has_started.load(Ordering::SeqCst) {
            return Err(SdkError::Config(
                "host tools must be registered before the first run".to_owned(),
            ));
        }
        self.inner
            .runtime
            .register_tool(Arc::new(ToolAdapter(tool)))?;
        Ok(())
    }

    /// Cancel active runs and reject new ones.
    pub fn shutdown(&self) -> Result<()> {
        self.inner.shutdown.store(true, Ordering::SeqCst);
        let active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for run in active.values() {
            run.cancel.cancel();
        }
        Ok(())
    }
}

/// A durable session within one agent.
#[derive(Clone)]
pub struct Session {
    pub(crate) inner: Arc<AgentInner>,
    pub(crate) id: SessionId,
}

impl Session {
    #[must_use]
    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn projection(&self) -> Result<blora_session::SessionProjection> {
        Ok(self.inner.runtime.show_session(&self.id)?)
    }

    /// Replay events after `after_sequence`.
    pub fn events_from(&self, after_sequence: u64) -> EventStream {
        EventStream::new(
            self.inner.runtime.clone(),
            self.id.clone(),
            after_sequence,
            None,
            None,
            None,
        )
    }

    /// Tail live session events. The stream stays open until dropped.
    pub fn live_events(&self) -> EventStream {
        EventStream::new(
            self.inner.runtime.clone(),
            self.id.clone(),
            0,
            Some(self.inner.runtime.subscribe_session(&self.id)),
            None,
            None,
        )
    }

    pub fn run(&self, prompt: impl Into<String>) -> Result<RunHandle> {
        self.run_with(RunRequest::new(prompt))
    }

    pub fn run_blocking(&self, prompt: impl Into<String>) -> Result<crate::run::RunOutcome> {
        self.run(prompt)?.wait()
    }

    pub fn run_with(&self, request: RunRequest) -> Result<RunHandle> {
        if self.inner.shutdown.load(Ordering::SeqCst) {
            return Err(SdkError::Closed);
        }
        self.inner.has_started.store(true, Ordering::SeqCst);
        let config = self.inner.runtime.config();
        let start_sequence = self.inner.runtime.last_sequence(&self.id)?;
        let run_id = RunId::generate();
        let cancel = CancelToken::new();
        let done = Arc::new(AtomicBool::new(false));

        let permission = request.permission.unwrap_or(self.inner.default_permission);
        let options = blora_runtime::RunOptions {
            model: request
                .model
                .clone()
                .unwrap_or_else(|| config.default_model().unwrap_or_default()),
            mock: false,
            auto_approve: false,
            interactive: true,
            can_ask: true,
            max_turns: request.max_turns.unwrap_or(config.limits.max_turns),
            provider: String::new(),
            read_only: permission == PermissionMode::Plan,
            worktree: false,
            passport_user_token: None,
            permission: Some(permission),
            attachments: request.attachments,
            output_schema: request.output_schema,
        };

        let mut active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if active.values().any(|run| run.session_id == self.id) {
            return Err(SdkError::RunActive(self.id.to_string()));
        }
        active.insert(
            run_id.clone(),
            ActiveRun {
                session_id: self.id.clone(),
                cancel: cancel.clone(),
            },
        );
        drop(active);

        let runtime = self.inner.runtime.clone();
        let session_id = self.id.clone();
        let thread_run_id = run_id.clone();
        let thread_cancel = cancel.clone();
        let thread_done = done.clone();
        let active = self.inner.active.clone();
        let active_run_id = run_id.clone();
        let prompt = request.prompt;
        let join = std::thread::spawn(move || {
            let result = runtime.run_with_id(
                &session_id,
                thread_run_id,
                &prompt,
                &thread_cancel,
                &options,
            );
            thread_done.store(true, Ordering::SeqCst);
            active
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&active_run_id);
            result
        });

        Ok(RunHandle::new(
            self.inner.clone(),
            self.id.clone(),
            run_id,
            start_sequence,
            cancel,
            done,
            join,
        ))
    }

    pub fn steer(&self, message: &str) -> Result<()> {
        self.inner.runtime.queue_steer(&self.id, message)?;
        Ok(())
    }

    pub fn resolve_approval(&self, id: &blora_types::ApprovalId, allow: bool) -> Result<()> {
        self.inner.runtime.resolve_approval(id, allow)?;
        Ok(())
    }

    pub fn answer_input(&self, request_id: &str, answer: Option<String>) -> Result<()> {
        self.inner
            .runtime
            .answer_input(&self.id, request_id, answer)?;
        Ok(())
    }

    pub fn set_permission_mode(&self, mode: PermissionMode) -> Result<()> {
        self.inner
            .runtime
            .set_permission_mode(&self.id, mode, "sdk")?;
        Ok(())
    }

    /// Cancel the currently active run(s) for this session.
    pub fn cancel(&self) -> Result<()> {
        let active = self
            .inner
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for run in active.values() {
            if run.session_id == self.id {
                run.cancel.cancel();
            }
        }
        Ok(())
    }

    pub fn archive(self) -> Result<()> {
        self.inner.runtime.archive_session(&self.id)?;
        Ok(())
    }
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("workspace", &self.inner.workspace)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("mode", &self.inner.default_mode)
            .finish()
    }
}

impl Drop for AgentInner {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for run in active.values() {
            run.cancel.cancel();
        }
    }
}
