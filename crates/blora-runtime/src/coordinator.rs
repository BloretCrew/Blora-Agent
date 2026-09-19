// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_context::{EnvSnapshot, compile_messages, estimate_messages};
use blora_events::{
    AssistantDelta, AssistantMessageCompleted, ContextSnapshotCreated, EventEnvelope,
    HookCompleted, KnownPayload, ModeChanged, ModelRequested, ModelResponseCompleted, NewEvent, ProviderChanged,
    RetryStarted, RoutingChanged, RunCancelRequested, RunCancelled, RunCompleted, RunCreated,
    RunFailed,
    RunStarted, SessionArchived, SessionResumed, ToolCompleted, ToolFailed, ToolOutput,
    ToolRequested, ToolStarted, UsageRecorded, UserInput,
};
use blora_exec::{Isolation, LocalBackend, WorktreeHandle};
use blora_model::retry::{self, RetryClass};
use blora_model::{
    Completion, CompletionRequest, Provider, StreamEvent, ToolCall, ToolDeclaration,
    resolve_provider_chain,
};
use blora_policy::Policy;
use blora_session::SessionProjection;
use blora_storage::{CreateSession, CreateTask, SessionSummary, SqliteStore};
use blora_tools::ToolRegistry;
use blora_types::{BloraError, CancelToken, Mode, Result, RunId, SessionId, TurnId};
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

use crate::hooks::{self, HookDecision};

/// Tool results larger than this are head/tail truncated before entering the log.
const MAX_TOOL_RESULT_CHARS: usize = 24_000;
/// Identical tool calls in a row: nudge at this count, abort at twice this count.
const DOOM_LOOP_NUDGE: u32 = 3;

#[derive(Clone, Debug)]
pub struct MockRunOptions {
    pub model: String,
    pub chunks: Vec<String>,
}

impl Default for MockRunOptions {
    fn default() -> Self {
        Self {
            model: "mock".to_owned(),
            chunks: vec!["Hello".to_owned(), ", ".to_owned(), "world.".to_owned()],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct WorkspaceInfo {
    pub path: String,
    pub files: String,
    pub entries: Vec<String>,
    pub git_status: String,
    pub git_diff: String,
    pub git_log: String,
    pub git_branch: String,
}

#[derive(Clone, Debug)]
pub struct RunOptions {
    pub model: String,
    pub mock: bool,
    pub auto_approve: bool,
    pub interactive: bool,
    pub max_turns: u32,
    pub provider: String,
    pub read_only: bool,
    pub worktree: bool,
    /// PassPort user token of the owner of this run; logged-in users default
    /// to the PassPort provider (`blora`) when no provider is requested.
    pub passport_user_token: Option<String>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            model: String::new(),
            mock: false,
            auto_approve: false,
            interactive: false,
            max_turns: 12,
            provider: String::new(),
            read_only: false,
            worktree: false,
            passport_user_token: None,
        }
    }
}

pub struct Runtime {
    pub(crate) store: SqliteStore,
    /// Live event subscribers (SSE, TUI). Lossy: a slow subscriber drops events
    /// and must resynchronise from the store, which it can always do.
    subscribers: Mutex<Vec<SyncSender<EventEnvelope>>>,
}

impl Runtime {
    #[must_use]
    pub fn new(store: SqliteStore) -> Self {
        Self {
            store,
            subscribers: Mutex::new(Vec::new()),
        }
    }

    /// Receive every event appended by this runtime, across all sessions.
    #[must_use]
    pub fn subscribe(&self) -> Receiver<EventEnvelope> {
        let (tx, rx) = sync_channel(256);
        self.subscribers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(tx);
        rx
    }

    fn notify(&self, event: &EventEnvelope) {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        subscribers.retain(|tx| {
            !matches!(
                tx.try_send(event.clone()),
                Err(TrySendError::Disconnected(_))
            )
        });
    }

    /// Highest event sequence stored for the session (0 if none).
    pub fn last_sequence(&self, session_id: &SessionId) -> Result<u64> {
        self.store.last_sequence(session_id)
    }

    /// Bring a cached projection up to date by applying only new events.
    /// Falls back to a full rebuild when the cache cannot be advanced.
    /// Returns true when anything changed.
    pub fn refresh_projection(
        &self,
        session_id: &SessionId,
        projection: &mut SessionProjection,
    ) -> Result<bool> {
        if projection.last_sequence == 0 {
            *projection = self.store.load_projection(session_id)?;
            return Ok(true);
        }
        let fresh = self
            .store
            .load_events_after(session_id, projection.last_sequence)?;
        if fresh.is_empty() {
            return Ok(false);
        }
        for event in &fresh {
            if projection.apply(event).is_err() {
                *projection = self.store.load_projection(session_id)?;
                return Ok(true);
            }
        }
        Ok(true)
    }

    pub fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        let id = self.store.create_session(spec)?;
        if let Some(created) = self.store.load_events(&id)?.into_iter().next() {
            self.notify(&created);
        }
        hooks::fire(hooks::SESSION_START, id.as_str());
        Ok(id)
    }

    /// Persist a mode switch so the next turns use the selected strategy.
    pub fn set_session_mode(&self, session_id: &SessionId, to: Mode) -> Result<()> {
        let projection = self.store.load_projection(session_id)?;
        let from = projection
            .session
            .ok_or_else(|| BloraError::Other("session missing".to_owned()))?
            .mode;
        if from == to {
            return Ok(());
        }
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::ModeChanged(ModeChanged { from, to }),
        )
    }

    pub fn archive_session(&self, session_id: &SessionId) -> Result<()> {
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::SessionArchived(SessionArchived {
                reason: Some("user".to_owned()),
            }),
        )
    }

    /// Record a provider/model switch on the session. Later runs use this
    /// routing until it is changed again.
    pub fn set_session_routing(
        &self,
        session_id: &SessionId,
        to_provider: impl Into<String>,
        to_model: impl Into<String>,
    ) -> Result<()> {
        let to_provider = to_provider.into();
        let to_model = to_model.into();
        if to_provider.trim().is_empty() {
            return Err(BloraError::Other("provider is empty".to_owned()));
        }
        let projection = self.store.load_projection(session_id)?;
        let from_provider = projection.provider;
        let from_model = projection.model;
        if from_provider.as_deref() == Some(to_provider.as_str())
            && from_model.as_deref() == Some(to_model.as_str())
        {
            return Ok(());
        }
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::RoutingChanged(RoutingChanged {
                from_provider,
                to_provider,
                from_model,
                to_model,
            }),
        )
    }

    pub fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        self.emit(
            session_id,
            None,
            None,
            KnownPayload::SessionResumed(SessionResumed {
                reason: Some("user".to_owned()),
            }),
        )
    }

    pub fn export_session(&self, session_id: &SessionId) -> Result<serde_json::Value> {
        let events = self.events(session_id)?;
        serde_json::to_value(events).map_err(|err| BloraError::event(err.to_string()))
    }

    pub fn list_plugins(&self, workspace: &std::path::Path) -> Vec<crate::plugins::PluginSpec> {
        crate::plugins::load(workspace)
    }

    pub fn usage(&self, session_id: Option<&SessionId>) -> Result<blora_storage::UsageTotals> {
        self.store.usage_totals(session_id)
    }

    /// Queue a user message to be delivered at the next safe point of a running
    /// session (between model turns). Safe to call from another thread.
    pub fn queue_steer(&self, session_id: &SessionId, message: &str) -> Result<()> {
        if message.trim().is_empty() {
            return Err(BloraError::Other("steer message is empty".to_owned()));
        }
        self.store.queue_steer(session_id, message.trim())
    }

    pub fn workspace_info(&self, path: &std::path::Path) -> Result<WorkspaceInfo> {
        let policy = Policy::new(path, true)?;
        let backend = LocalBackend::new(policy);
        let files = backend.list_dir(".").unwrap_or_default();
        let entries = files
            .lines()
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        Ok(WorkspaceInfo {
            path: path.display().to_string(),
            files,
            entries,
            git_status: backend.git_status().unwrap_or_else(|err| err.to_string()),
            git_diff: backend.git_diff().unwrap_or_else(|err| err.to_string()),
            git_log: backend.git_log().unwrap_or_else(|err| err.to_string()),
            git_branch: backend.git_branch().unwrap_or_else(|err| err.to_string()),
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionSummary>> {
        self.store.list_sessions()
    }

    pub fn list_sessions_for_user(&self, user_id: Option<&str>) -> Result<Vec<SessionSummary>> {
        self.store.list_sessions_for_user(user_id)
    }

    pub fn create_user(&self, name: &str) -> Result<(blora_storage::UserRecord, String)> {
        self.store.create_user(name)
    }

    pub fn user_by_token(&self, token: &str) -> Result<Option<blora_storage::UserRecord>> {
        self.store.user_by_token(token)
    }

    pub fn refresh_passport_token(
        &self,
        username: &str,
        app_token: &str,
        refresh_token: Option<&str>,
        token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Option<blora_storage::UserRecord>> {
        self.store.update_passport_tokens(
            username,
            app_token,
            refresh_token,
            token_expires_at,
        )
    }

    pub fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
        refresh_token: Option<&str>,
        token_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<blora_storage::UserRecord> {
        self.store.upsert_passport_user(
            username,
            nickname,
            avatar,
            email,
            app_token,
            refresh_token,
            token_expires_at,
        )
    }

    pub fn user_by_passport_username(
        &self,
        username: &str,
    ) -> Result<Option<blora_storage::UserRecord>> {
        self.store.user_by_passport_username(username)
    }

    pub fn clear_passport_users(&self) -> Result<usize> {
        self.store.clear_passport_users()
    }

    pub fn list_users(&self) -> Result<Vec<blora_storage::UserRecord>> {
        self.store.list_users()
    }

    pub fn set_session_user(&self, session_id: &SessionId, user_id: &str) -> Result<()> {
        self.store.set_session_user(session_id, user_id)
    }

    pub fn session_user_id(&self, session_id: &SessionId) -> Result<Option<String>> {
        self.store.session_user_id(session_id)
    }

    pub fn search_sessions(&self, query: &str) -> Result<Vec<SessionSummary>> {
        let needle = query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return self.list_sessions();
        }
        let mut out = Vec::new();
        for summary in self.list_sessions()? {
            let hay = format!("{} {}", summary.id, summary.title.as_deref().unwrap_or(""))
                .to_ascii_lowercase();
            let in_transcript = self
                .show_session(&summary.id)?
                .transcript
                .iter()
                .any(|item| match item {
                    blora_session::TranscriptItem::User { text, .. }
                    | blora_session::TranscriptItem::Assistant { text, .. } => {
                        text.to_ascii_lowercase().contains(&needle)
                    }
                    blora_session::TranscriptItem::Tool { name, .. } => {
                        name.to_ascii_lowercase().contains(&needle)
                    }
                    blora_session::TranscriptItem::System { summary, .. } => {
                        summary.to_ascii_lowercase().contains(&needle)
                    }
                    blora_session::TranscriptItem::Routing {
                        to_provider,
                        to_model,
                        from_provider,
                        from_model,
                        ..
                    } => {
                        to_provider.to_ascii_lowercase().contains(&needle)
                            || to_model.to_ascii_lowercase().contains(&needle)
                            || from_provider
                                .as_deref()
                                .is_some_and(|value| value.to_ascii_lowercase().contains(&needle))
                            || from_model
                                .as_deref()
                                .is_some_and(|value| value.to_ascii_lowercase().contains(&needle))
                    }
                });
            if hay.contains(&needle) || in_transcript {
                out.push(summary);
            }
        }
        Ok(out)
    }

    pub fn list_artifacts(
        &self,
        session_id: Option<&SessionId>,
    ) -> Result<Vec<blora_storage::ArtifactRecord>> {
        self.store.list_artifacts(session_id)
    }

    pub fn read_workspace_file(&self, workspace: &std::path::Path, path: &str) -> Result<String> {
        let policy = Policy::new(workspace, true)?;
        LocalBackend::new(policy).read_file(path)
    }

    pub fn show_session(&self, session_id: &SessionId) -> Result<SessionProjection> {
        self.store.load_projection(session_id)
    }

    pub fn replay_session(&self, session_id: &SessionId) -> Result<SessionProjection> {
        self.store.load_projection(session_id)
    }

    pub fn events(&self, session_id: &SessionId) -> Result<Vec<EventEnvelope>> {
        self.store.load_events(session_id)
    }

    pub fn fork_session(&self, source: &SessionId) -> Result<SessionId> {
        let projection = self.show_session(source)?;
        let session = projection
            .session
            .ok_or_else(|| BloraError::SessionNotFound(source.to_string()))?;
        let child = self.create_session(CreateSession {
            title: session
                .title
                .map(|title| format!("{title} (fork)"))
                .or_else(|| Some("fork".to_owned())),
            workspace_path: session.workspace_path,
            mode: session.mode,
            parent_session_id: Some(source.clone()),
        })?;
        self.emit(
            &child,
            None,
            None,
            KnownPayload::SessionForked(blora_events::SessionForked {
                source_session_id: source.clone(),
            }),
        )?;
        Ok(child)
    }

    pub fn run_mock(
        &self,
        session_id: &SessionId,
        prompt: &str,
        cancel: &CancelToken,
        options: &MockRunOptions,
    ) -> Result<RunId> {
        if prompt.trim().is_empty() {
            return Err(BloraError::Other("prompt must not be empty".to_owned()));
        }
        let run_id = RunId::generate();
        let turn_id = TurnId::generate();
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCreated(RunCreated {
                mode: Mode::Code,
                model: Some(options.model.clone()),
            }),
        )?;
        if cancel.is_cancelled() {
            return self.cancel_run(session_id, &run_id, &turn_id);
        }
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunStarted(RunStarted {
                model: Some(options.model.clone()),
            }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::UserInput(UserInput {
                text: prompt.to_owned(),
            }),
        )?;
        let mut assembled = String::new();
        for chunk in &options.chunks {
            if cancel.is_cancelled() {
                return self.cancel_run(session_id, &run_id, &turn_id);
            }
            assembled.push_str(chunk);
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::AssistantDelta(AssistantDelta {
                    text: chunk.clone(),
                }),
            )?;
        }
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::AssistantMessageCompleted(AssistantMessageCompleted { text: assembled }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCompleted(RunCompleted {
                summary: Some("mock completed".to_owned()),
            }),
        )?;
        Ok(run_id)
    }

    pub fn run(
        &self,
        session_id: &SessionId,
        prompt: &str,
        cancel: &CancelToken,
        options: &RunOptions,
    ) -> Result<RunId> {
        if prompt.trim().is_empty() {
            return Err(BloraError::Other("prompt must not be empty".to_owned()));
        }
        let projection = self.store.load_projection(session_id)?;
        let session = projection
            .session
            .as_ref()
            .ok_or_else(|| BloraError::SessionNotFound(session_id.to_string()))?;
        let workspace = session.workspace_path.clone();
        let mode = session.mode;
        let worktree = if options.worktree || env_flag("BLORA_WORKTREE") {
            WorktreeHandle::create(&workspace, session_id.as_str()).ok()
        } else {
            None
        };
        let exec_root = worktree
            .as_ref()
            .map(|handle| handle.path().display().to_string())
            .unwrap_or_else(|| workspace.clone());
        let _keep_worktree = worktree;
        let used = projection.input_tokens + projection.output_tokens;
        if token_budget_exceeded(used) {
            return Err(BloraError::Other(format!("token budget exceeded ({used})")));
        }
        let policy = Policy::new(&exec_root, options.auto_approve)?;
        let backend = LocalBackend::new(policy).with_isolation(Isolation::from_env());
        let requested_provider = if options.provider.trim().is_empty() {
            projection.provider.clone().unwrap_or_default()
        } else {
            options.provider.clone()
        };
        let providers = resolve_provider_chain(
            options.mock,
            &requested_provider,
            options.passport_user_token.as_deref(),
        );
        let mut provider_index = 0;
        let model = if !options.model.trim().is_empty() {
            options.model.clone()
        } else if let Some(saved) = projection
            .model
            .as_deref()
            .filter(|model| !model.trim().is_empty())
        {
            saved.to_owned()
        } else if options.mock || providers[provider_index].name() == "mock" {
            "mock".to_owned()
        } else if providers[provider_index].name() == "blora" {
            blora_model::PASSPORT_MODEL_NAME.to_owned()
        } else {
            std::env::var("BLORA_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned())
        };

        // Prompt-submit hook may block or enrich the prompt before anything is logged.
        let mut prompt = prompt.to_owned();
        let submit = hooks::run(
            hooks::USER_PROMPT_SUBMIT,
            &json!({"session_id": session_id.to_string(), "prompt": prompt}),
        );
        if !submit.absent {
            self.emit(
                session_id,
                None,
                None,
                KnownPayload::HookCompleted(HookCompleted {
                    hook: hooks::USER_PROMPT_SUBMIT.to_owned(),
                    decision: submit.decision.as_str().to_owned(),
                    reason: submit.reason.clone(),
                    degraded: submit.degraded,
                }),
            )?;
        }
        if submit.decision == HookDecision::Block {
            return Err(BloraError::Policy(format!(
                "prompt blocked by hook: {}",
                submit.reason.unwrap_or_default()
            )));
        }
        if let Some(extra) = submit.additional_context {
            prompt.push_str("\n\n<hook_context>\n");
            prompt.push_str(&extra);
            prompt.push_str("\n</hook_context>");
        }

        let run_id = RunId::generate();
        let turn_id = TurnId::generate();
        let wall_started = std::time::Instant::now();
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunCreated(RunCreated {
                mode,
                model: Some(model.clone()),
            }),
        )?;
        if cancel.is_cancelled() {
            return self.cancel_run(session_id, &run_id, &turn_id);
        }
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunStarted(RunStarted {
                model: Some(model.clone()),
            }),
        )?;
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::UserInput(UserInput { text: prompt }),
        )?;
        let env = EnvSnapshot::capture(&exec_root);
        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::ContextSnapshotCreated(ContextSnapshotCreated {
                snapshot_id: exec_root.clone(),
            }),
        )?;

        let mcp = std::env::var("BLORA_MCP_COMMAND")
            .ok()
            .and_then(|command| crate::mcp::McpClient::connect(&command).ok());
        let mut tools = ToolRegistry::specs()
            .into_iter()
            .filter(|spec| !options.read_only || spec.read_only)
            .map(|spec| ToolDeclaration {
                name: spec.name.to_owned(),
                description: spec.description.to_owned(),
                parameters: spec.parameters,
            })
            .collect::<Vec<_>>();
        if let Some(client) = &mcp {
            if let Ok(extra) = client.list_tools() {
                tools.extend(extra);
            }
        }
        let plugins = crate::plugins::load(std::path::Path::new(&exec_root));
        if !options.read_only {
            tools.extend(plugins.iter().map(crate::plugins::PluginSpec::declaration));
        }
        let window = blora_context::context_window();
        let compact_at = window * blora_context::compact_threshold_pct() / 100;
        let mut last_fingerprint: Option<String> = None;
        let mut repeat_count: u32 = 0;

        for turn in 0..options.max_turns {
            if cancel.is_cancelled() {
                return self.cancel_run(session_id, &run_id, &turn_id);
            }
            if wall_clock_exceeded(wall_started) {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::RunFailed(RunFailed {
                        error: "wall-clock budget exceeded".to_owned(),
                    }),
                )?;
                return Err(BloraError::Other("wall-clock budget exceeded".to_owned()));
            }
            // Steering messages become real user turns at this safe point.
            for steer in self.store.take_steers(session_id)? {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::UserInput(UserInput {
                        text: format!("[steer] {steer}"),
                    }),
                )?;
            }
            let mut events = self.store.load_events(session_id)?;
            let mut messages = compile_messages(&events, &exec_root, mode.as_str(), &env)?;
            if estimate_messages(&messages) >= compact_at {
                match self.compact_with(
                    session_id,
                    Some((providers[provider_index].as_ref(), model.as_str())),
                    cancel,
                    "auto",
                ) {
                    Ok(_) => {
                        events = self.store.load_events(session_id)?;
                        messages = compile_messages(&events, &exec_root, mode.as_str(), &env)?;
                    }
                    Err(BloraError::Cancelled) => {
                        return self.cancel_run(session_id, &run_id, &turn_id);
                    }
                    Err(err) => tracing::warn!("auto-compaction skipped: {err}"),
                }
            }
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::ModelRequested(ModelRequested {
                    provider: providers[provider_index].name().to_owned(),
                    model: model.clone(),
                }),
            )?;
            let request = CompletionRequest {
                model: model.clone(),
                messages,
                tools: tools.clone(),
                max_output_tokens: None,
                cache_key: Some(session_id.to_string()),
            };
            let (completion, streamed) = match self.call_with_retry(
                session_id,
                &run_id,
                &turn_id,
                &providers,
                &mut provider_index,
                &model,
                &request,
                cancel,
            ) {
                Ok(pair) => pair,
                Err(BloraError::Cancelled) => {
                    return self.cancel_run(session_id, &run_id, &turn_id);
                }
                Err(err) => {
                    self.emit(
                        session_id,
                        Some(&run_id),
                        Some(&turn_id),
                        KnownPayload::RunFailed(RunFailed {
                            error: err.to_string(),
                        }),
                    )?;
                    return Err(err);
                }
            };

            let assistant_text = if completion.text.is_empty() {
                streamed.clone()
            } else {
                completion.text.clone()
            };
            if !assistant_text.is_empty() && streamed.is_empty() {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::AssistantDelta(AssistantDelta {
                        text: assistant_text.clone(),
                    }),
                )?;
            }
            if !assistant_text.is_empty() {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::AssistantMessageCompleted(AssistantMessageCompleted {
                        text: assistant_text,
                    }),
                )?;
            }

            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::UsageRecorded(UsageRecorded {
                    input_tokens: completion.input_tokens,
                    output_tokens: completion.output_tokens,
                    cached_tokens: completion.cached_tokens,
                }),
            )?;
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::ModelResponseCompleted(ModelResponseCompleted {
                    finish_reason: Some(completion.finish_reason.clone()),
                }),
            )?;

            if completion.tool_calls.is_empty() {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::RunCompleted(RunCompleted {
                        summary: Some(format!("completed in {} model turns", turn + 1)),
                    }),
                )?;
                let _ = self.checkpoint(session_id, Some(&run_id), Some("run completed"));
                let _ = self.distill_memories(session_id);
                hooks::fire(
                    hooks::STOP,
                    &json!({
                        "session_id": session_id.to_string(),
                        "run_id": run_id.to_string(),
                    })
                    .to_string(),
                );
                return Ok(run_id);
            }

            // Doom-loop guard across turns: identical batch repeated.
            let fingerprint = completion
                .tool_calls
                .iter()
                .map(|call| format!("{}:{}", call.name, call.arguments))
                .collect::<Vec<_>>()
                .join("|");
            if last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
                repeat_count += 1;
            } else {
                repeat_count = 0;
                last_fingerprint = Some(fingerprint);
            }
            if repeat_count >= DOOM_LOOP_NUDGE * 2 {
                self.emit(
                    session_id,
                    Some(&run_id),
                    Some(&turn_id),
                    KnownPayload::RunFailed(RunFailed {
                        error: "repeated tool calls detected; aborting loop".to_owned(),
                    }),
                )?;
                return Err(BloraError::Other(
                    "repeated tool calls detected; aborting loop".to_owned(),
                ));
            }
            let nudge = repeat_count >= DOOM_LOOP_NUDGE;

            self.dispatch_batch(
                session_id,
                &run_id,
                &turn_id,
                &backend,
                &completion.tool_calls,
                options,
                cancel,
                mcp.as_ref(),
                &plugins,
                &exec_root,
                nudge,
            )?;
        }

        self.emit(
            session_id,
            Some(&run_id),
            Some(&turn_id),
            KnownPayload::RunFailed(RunFailed {
                error: "maximum model turns reached".to_owned(),
            }),
        )?;
        Err(BloraError::Other("maximum model turns reached".to_owned()))
    }

    /// Stream one completion with per-provider retry and fallback across the
    /// provider chain. Returns the completion and the text streamed so far.
    #[allow(clippy::too_many_arguments)]
    fn call_with_retry(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        providers: &[Box<dyn Provider>],
        provider_index: &mut usize,
        model: &str,
        request: &CompletionRequest,
        cancel: &CancelToken,
    ) -> Result<(Completion, String)> {
        let max_retries = retry::max_retries();
        loop {
            let provider = providers[*provider_index].as_ref();
            let mut attempt = 0u32;
            let outcome = loop {
                let mut streamed = String::new();
                let result = provider.complete(request, cancel, &mut |event| match event {
                    StreamEvent::TextDelta(text) => {
                        streamed.push_str(&text);
                        self.emit(
                            session_id,
                            Some(run_id),
                            Some(turn_id),
                            KnownPayload::AssistantDelta(AssistantDelta { text }),
                        )
                    }
                    StreamEvent::ToolCall(call) => {
                        let arguments = serde_json::from_str(&call.arguments)
                            .unwrap_or_else(|_| json!({ "raw": call.arguments }));
                        self.emit(
                            session_id,
                            Some(run_id),
                            Some(turn_id),
                            KnownPayload::ToolRequested(ToolRequested {
                                tool: call.name,
                                arguments,
                                call_id: Some(call.id),
                            }),
                        )
                    }
                });
                match result {
                    Ok(completion) => break Ok((completion, streamed)),
                    Err(BloraError::Cancelled) => return Err(BloraError::Cancelled),
                    Err(err) => {
                        // Partial text already streamed must not be retried into a
                        // duplicate reply; record what we had and continue cleanly.
                        if !streamed.is_empty() {
                            self.emit(
                                session_id,
                                Some(run_id),
                                Some(turn_id),
                                KnownPayload::AssistantMessageCompleted(
                                    AssistantMessageCompleted {
                                        text: format!("{streamed}\n[stream interrupted]"),
                                    },
                                ),
                            )?;
                        }
                        let class = retry::classify(&err);
                        if class == RetryClass::Fatal || attempt >= max_retries {
                            break Err((err, class));
                        }
                        let delay = retry::backoff_delay(attempt, retry::retry_after_secs(&err));
                        attempt += 1;
                        self.emit(
                            session_id,
                            Some(run_id),
                            Some(turn_id),
                            KnownPayload::RetryStarted(RetryStarted {
                                attempt,
                                reason: format!("{err} (waiting {}ms)", delay.as_millis()),
                            }),
                        )?;
                        if sleep_cancellable(delay, cancel) {
                            return Err(BloraError::Cancelled);
                        }
                    }
                }
            };
            match outcome {
                Ok(pair) => return Ok(pair),
                Err((err, _)) => {
                    let auth_failure = matches!(retry::http_status(&err), Some(401 | 403));
                    if auth_failure || *provider_index + 1 >= providers.len() {
                        return Err(err);
                    }
                    *provider_index += 1;
                    self.emit(
                        session_id,
                        Some(run_id),
                        Some(turn_id),
                        KnownPayload::ProviderChanged(ProviderChanged {
                            provider: providers[*provider_index].name().to_owned(),
                            model: model.to_owned(),
                        }),
                    )?;
                }
            }
        }
    }

    /// Execute a batch of tool calls. Read-only batches run concurrently; the
    /// results are still recorded in call order so replay is deterministic.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_batch(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        backend: &LocalBackend,
        calls: &[ToolCall],
        options: &RunOptions,
        cancel: &CancelToken,
        mcp: Option<&crate::mcp::McpClient>,
        plugins: &[crate::plugins::PluginSpec],
        workspace: &str,
        nudge: bool,
    ) -> Result<()> {
        // Read-only batches run concurrently unless hooks are installed, because a
        // tool-before hook must be able to veto before anything executes.
        let all_read_only = calls.len() > 1
            && std::env::var_os("BLORA_HOOKS_DIR").is_none()
            && calls
                .iter()
                .all(|call| ToolRegistry::is_read_only(&call.name));
        let precomputed: Vec<Option<Result<String>>> = if all_read_only {
            std::thread::scope(|scope| {
                let handles: Vec<_> = calls
                    .iter()
                    .map(|call| {
                        scope.spawn(move || {
                            let arguments: Value = serde_json::from_str(&call.arguments)
                                .unwrap_or_else(|_| json!({ "raw": call.arguments }));
                            ToolRegistry::execute(backend, &call.name, &arguments)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|handle| {
                        Some(handle.join().unwrap_or_else(|_| {
                            Err(BloraError::Other("tool thread panicked".to_owned()))
                        }))
                    })
                    .collect()
            })
        } else {
            calls.iter().map(|_| None).collect()
        };
        for (index, call) in calls.iter().enumerate() {
            if cancel.is_cancelled() {
                return self.cancel_run(session_id, run_id, turn_id).map(|_| ());
            }
            let ready = precomputed[index]
                .as_ref()
                .filter(|result| !matches!(result, Err(BloraError::ApprovalRequired(_))))
                .cloned_result();
            self.dispatch_tool(
                session_id,
                run_id,
                turn_id,
                backend,
                call,
                options,
                cancel,
                mcp,
                plugins,
                workspace,
                ready,
                nudge && index == calls.len() - 1,
            )?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch_tool(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        backend: &LocalBackend,
        call: &ToolCall,
        options: &RunOptions,
        cancel: &CancelToken,
        mcp: Option<&crate::mcp::McpClient>,
        plugins: &[crate::plugins::PluginSpec],
        workspace: &str,
        precomputed: Option<Result<String>>,
        nudge: bool,
    ) -> Result<()> {
        let arguments: Value = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| json!({ "raw": call.arguments }));
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ToolRequested(ToolRequested {
                tool: call.name.clone(),
                arguments: arguments.clone(),
                call_id: Some(call.id.clone()),
            }),
        )?;
        if options.read_only && !ToolRegistry::is_read_only(&call.name) {
            return self.record_tool_failure(
                session_id,
                run_id,
                turn_id,
                call,
                format!("{} blocked in read-only role", call.name),
            );
        }
        let before = hooks::run(
            hooks::TOOL_BEFORE,
            &json!({
                "session_id": session_id.to_string(),
                "tool": call.name,
                "arguments": arguments,
            }),
        );
        if !before.absent {
            self.emit(
                session_id,
                Some(run_id),
                Some(turn_id),
                KnownPayload::HookCompleted(HookCompleted {
                    hook: hooks::TOOL_BEFORE.to_owned(),
                    decision: before.decision.as_str().to_owned(),
                    reason: before.reason.clone(),
                    degraded: before.degraded,
                }),
            )?;
        }
        if before.decision == HookDecision::Block {
            return self.record_tool_failure(
                session_id,
                run_id,
                turn_id,
                call,
                format!(
                    "{} blocked by hook: {}",
                    call.name,
                    before.reason.unwrap_or_default()
                ),
            );
        }
        if before.decision == HookDecision::Ask {
            let summary = format!("hook requested confirmation for {}", call.name);
            let allowed = options.interactive
                && self
                    .await_approval(session_id, run_id, turn_id, &call.name, &summary, cancel)?;
            if !allowed {
                return self.record_tool_failure(
                    session_id,
                    run_id,
                    turn_id,
                    call,
                    format!("{} needs confirmation (hook ask)", call.name),
                );
            }
        }
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ToolStarted(ToolStarted {
                tool: call.name.clone(),
            }),
        )?;
        let execute = |backend: &LocalBackend| match call.name.as_str() {
            "delegate" => {
                let prompt = arguments
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let role = arguments
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("worker");
                self.spawn_subagent(session_id, role, prompt, options, cancel)
            }
            "schedule_task" => {
                let title = arguments
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("scheduled")
                    .to_owned();
                let prompt = arguments
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let delay = arguments
                    .get("delay_seconds")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let delay_until = if delay == 0 {
                    None
                } else {
                    Some(chrono::Utc::now() + chrono::Duration::seconds(delay as i64))
                };
                self.create_task(CreateTask {
                    session_id: session_id.clone(),
                    title,
                    prompt,
                    delay_until,
                    max_attempts: 3,
                    auto_approve: options.auto_approve,
                    mock: options.mock,
                    cron: arguments
                        .get("cron")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                })
                .map(|id| format!("queued {id}"))
            }
            name if name.starts_with("mcp__") => mcp
                .ok_or_else(|| BloraError::Other("MCP client is not connected".to_owned()))
                .and_then(|client| client.call(name, &arguments)),
            name if name.starts_with("plugin__") => {
                crate::plugins::execute(backend, plugins, name, &arguments)
            }
            "remember" => {
                let key = arguments
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let value = arguments
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if key.is_empty() {
                    Err(BloraError::Other("remember needs key".to_owned()))
                } else {
                    self.store
                        .upsert_memory(workspace, key, value, chrono::Utc::now())?;
                    Ok(format!("remembered {key}"))
                }
            }
            "recall" => {
                if let Some(key) = arguments.get("key").and_then(Value::as_str) {
                    Ok(self
                        .store
                        .get_memory(workspace, key)?
                        .map(|row| row.value)
                        .unwrap_or_else(|| "(missing)".to_owned()))
                } else {
                    let rows = self.store.list_memories(workspace)?;
                    if rows.is_empty() {
                        Ok("(no memories)".to_owned())
                    } else {
                        Ok(rows
                            .into_iter()
                            .map(|row| format!("{}={}", row.key, row.value))
                            .collect::<Vec<_>>()
                            .join("\n"))
                    }
                }
            }
            "forget" => {
                let key = arguments
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.store.delete_memory(workspace, key)?;
                Ok(format!("forgot {key}"))
            }
            _ => ToolRegistry::execute(backend, &call.name, &arguments),
        };
        let first = match precomputed {
            Some(result) => result,
            None => execute(backend),
        };
        let intercepted = match first {
            Err(BloraError::ApprovalRequired(summary)) if options.interactive => {
                if self.await_approval(session_id, run_id, turn_id, &call.name, &summary, cancel)? {
                    let granted = LocalBackend::new(backend.policy().granting())
                        .with_isolation(backend.isolation());
                    execute(&granted)
                } else {
                    Err(BloraError::Policy(summary))
                }
            }
            other => other,
        };
        let after = hooks::run(
            hooks::TOOL_AFTER,
            &json!({
                "session_id": session_id.to_string(),
                "tool": call.name,
                "arguments": arguments,
                "ok": intercepted.is_ok(),
                "output": intercepted.as_ref().ok().map(|t| blora_context::head_tail(t, 4_000)),
            }),
        );
        match intercepted {
            Ok(mut text) => {
                if let Some(extra) = after.additional_context {
                    text.push_str("\n\n<hook_context>\n");
                    text.push_str(&extra);
                    text.push_str("\n</hook_context>");
                }
                if nudge {
                    text.push_str(
                        "\n\n<system-reminder>You have issued the same tool call several times in a row. Change approach or finish with a final answer.</system-reminder>",
                    );
                }
                let truncated = text.chars().count() > MAX_TOOL_RESULT_CHARS;
                let recorded = if truncated {
                    blora_context::head_tail(&text, MAX_TOOL_RESULT_CHARS)
                } else {
                    text
                };
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: recorded,
                        call_id: Some(call.id.clone()),
                        truncated,
                    }),
                )?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolCompleted(ToolCompleted {
                        tool: call.name.clone(),
                        ok: true,
                    }),
                )?;
                Ok(())
            }
            Err(BloraError::ApprovalRequired(summary)) => self.record_tool_failure(
                session_id,
                run_id,
                turn_id,
                call,
                format!(
                    "approval required for {summary}. Ask the user to rerun with --yes or approve interactively."
                ),
            ),
            Err(err) => self.record_tool_failure(
                session_id,
                run_id,
                turn_id,
                call,
                format!("tool error: {err}"),
            ),
        }
    }

    fn record_tool_failure(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
        call: &ToolCall,
        message: String,
    ) -> Result<()> {
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ToolFailed(ToolFailed {
                tool: call.name.clone(),
                error: message.clone(),
            }),
        )?;
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::ToolOutput(ToolOutput {
                text: message,
                call_id: Some(call.id.clone()),
                truncated: false,
            }),
        )
    }

    fn cancel_run(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        turn_id: &TurnId,
    ) -> Result<RunId> {
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::RunCancelRequested(RunCancelRequested {
                reason: Some("user".to_owned()),
            }),
        )?;
        self.emit(
            session_id,
            Some(run_id),
            Some(turn_id),
            KnownPayload::RunCancelled(RunCancelled {
                reason: Some("user".to_owned()),
            }),
        )?;
        Err(BloraError::Cancelled)
    }

    pub(crate) fn emit(
        &self,
        session_id: &SessionId,
        run_id: Option<&RunId>,
        turn_id: Option<&TurnId>,
        payload: KnownPayload,
    ) -> Result<()> {
        let mut event = NewEvent::new(session_id.clone(), payload);
        if let Some(run_id) = run_id {
            event = event.with_run(run_id.clone());
        }
        if let Some(turn_id) = turn_id {
            event = event.with_turn(turn_id.clone());
        }
        let stored = self.store.append(event)?;
        self.notify(&stored);
        Ok(())
    }
}

trait ClonedResult {
    fn cloned_result(self) -> Option<Result<String>>;
}

impl ClonedResult for Option<&Result<String>> {
    fn cloned_result(self) -> Option<Result<String>> {
        self.map(|result| match result {
            Ok(text) => Ok(text.clone()),
            Err(err) => Err(BloraError::Other(err.to_string())),
        })
    }
}

/// Sleep in small slices so a cancel request interrupts a long backoff.
/// Returns true when cancelled.
fn sleep_cancellable(total: std::time::Duration, cancel: &CancelToken) -> bool {
    let started = std::time::Instant::now();
    while started.elapsed() < total {
        if cancel.is_cancelled() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    cancel.is_cancelled()
}

fn wall_clock_exceeded(started: std::time::Instant) -> bool {
    let max = std::env::var("BLORA_MAX_WALL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(900);
    max > 0 && started.elapsed().as_secs() > max
}

fn token_budget_exceeded(used: u64) -> bool {
    std::env::var("BLORA_MAX_TOKENS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|max| max > 0 && used > max)
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_model::MockProvider;
    use blora_session::TranscriptItem;
    use blora_storage::SqliteStore;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn session(runtime: &Runtime, mode: Mode, workspace: &str) -> SessionId {
        runtime
            .create_session(CreateSession {
                title: Some("t".to_owned()),
                workspace_path: workspace.to_owned(),
                mode,
                parent_session_id: None,
            })
            .unwrap()
    }

    #[test]
    fn mock_run_persists_transcript() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        runtime
            .run_mock(
                &session,
                "hello",
                &CancelToken::new(),
                &MockRunOptions::default(),
            )
            .unwrap();
        let projection = runtime.show_session(&session).unwrap();
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.status.as_str() == "completed")
        );
        assert!(projection.transcript.iter().any(
            |item| matches!(item, TranscriptItem::Assistant { text, .. } if text == "Hello, world.")
        ));
    }

    #[test]
    fn cancel_before_stream_is_idempotent() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        let cancel = CancelToken::new();
        cancel.cancel();
        let err = runtime
            .run_mock(&session, "hello", &cancel, &MockRunOptions::default())
            .unwrap_err();
        assert!(matches!(err, BloraError::Cancelled));
        let projection = runtime.show_session(&session).unwrap();
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.status.as_str() == "cancelled")
        );
        assert!(projection.runs.iter().all(|run| run.cancel_requested));
    }

    #[test]
    fn pumps_due_background_task() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Work, dir.path().to_str().unwrap());
        let task_id = runtime
            .create_task(CreateTask {
                session_id: session.clone(),
                title: "scan".to_owned(),
                prompt: "list files".to_owned(),
                delay_until: None,
                max_attempts: 1,
                auto_approve: true,
                mock: true,
                cron: None,
            })
            .unwrap();
        let finished = runtime.pump().unwrap();
        assert!(finished.contains(&task_id));
        assert_eq!(
            runtime.get_task(&task_id).unwrap().status.as_str(),
            "completed"
        );
    }

    #[test]
    fn cron_task_advances_and_never_double_fires() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Work, dir.path().to_str().unwrap());
        let task_id = runtime
            .create_task(CreateTask {
                session_id: session.clone(),
                title: "tick".to_owned(),
                prompt: "list files".to_owned(),
                delay_until: None,
                max_attempts: 1,
                auto_approve: true,
                mock: true,
                cron: Some("* * * * *".to_owned()),
            })
            .unwrap();
        assert!(runtime.pump().unwrap().contains(&task_id));
        let after = runtime.get_task(&task_id).unwrap();
        assert_eq!(after.status.as_str(), "scheduled");
        assert!(after.delay_until.unwrap() > chrono::Utc::now());
        assert_eq!(after.attempt, 0);
        // Not due again yet: a second pump must not run it.
        assert!(runtime.pump().unwrap().is_empty());
        assert!(
            runtime
                .create_task(CreateTask {
                    session_id: session,
                    title: "bad".to_owned(),
                    prompt: "x".to_owned(),
                    delay_until: None,
                    max_attempts: 1,
                    auto_approve: true,
                    mock: true,
                    cron: Some("not a cron".to_owned()),
                })
                .is_err()
        );
    }

    #[test]
    fn forks_session() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        let fork = runtime.fork_session(&session).unwrap();
        assert_ne!(fork, session);
        let projection = runtime.show_session(&fork).unwrap();
        assert_eq!(
            projection.session.unwrap().parent_session_id.as_ref(),
            Some(&session)
        );
    }

    #[test]
    fn agent_mode_spawns_code_child_with_usage_rollup() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Agent, dir.path().to_str().unwrap());
        runtime
            .run(
                &session,
                "delegate a workspace listing",
                &CancelToken::new(),
                &RunOptions {
                    mock: true,
                    auto_approve: true,
                    ..RunOptions::default()
                },
            )
            .unwrap();
        let projection = runtime.show_session(&session).unwrap();
        assert!(!projection.subagents.is_empty());
        assert_eq!(projection.subagents[0].status, "completed");
        let events = runtime.events(&session).unwrap();
        let spawned = events
            .iter()
            .find(|e| e.event_type == "subagent.spawned")
            .unwrap();
        assert_eq!(spawned.payload["depth"], 1);
        let done = events
            .iter()
            .find(|e| e.event_type == "subagent.completed")
            .unwrap();
        assert!(done.payload["turns"].as_u64().unwrap() >= 1);
        assert!(
            projection.transcript.iter().any(
                |item| matches!(item, TranscriptItem::Tool { name, .. } if name == "delegate")
            )
        );
    }

    #[test]
    fn compact_uses_model_and_keeps_tail() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        for i in 0..4 {
            runtime
                .run_mock(
                    &session,
                    &format!("question {i} {}", "detail ".repeat(200)),
                    &CancelToken::new(),
                    &MockRunOptions::default(),
                )
                .unwrap();
        }
        let mock = MockProvider::new();
        let provider: &dyn Provider = &mock;
        let report = runtime
            .compact_with(
                &session,
                Some((provider, "mock")),
                &CancelToken::new(),
                "auto",
            )
            .unwrap();
        assert!(report.summary.contains("## Goal"));
        assert!(report.tokens_after < report.tokens_before);
        assert!(report.preserve_from_sequence.is_some());
        let events = runtime.events(&session).unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "context.compaction.completed")
        );
        // Manual compaction without a provider still succeeds via the fallback.
        assert!(
            runtime
                .compact(&session)
                .unwrap()
                .contains("All user messages")
        );
    }

    #[test]
    fn interactive_write_waits_for_approval() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, dir.path().to_str().unwrap());
        std::thread::scope(|scope| {
            let session_for_run = session.clone();
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(120));
                let pending = runtime.pending_approvals(&session).unwrap();
                assert!(!pending.is_empty());
                runtime.resolve_approval(&pending[0].id, true).unwrap();
            });
            runtime
                .run(
                    &session_for_run,
                    "WRITE_FILE please",
                    &CancelToken::new(),
                    &RunOptions {
                        mock: true,
                        interactive: true,
                        auto_approve: false,
                        ..RunOptions::default()
                    },
                )
                .unwrap();
        });
        let written = std::fs::read_to_string(dir.path().join("ok.txt")).unwrap();
        assert_eq!(written, "ok");
    }

    #[test]
    fn tool_loop_lists_workspace_and_records_cached_usage() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, dir.path().to_str().unwrap());
        runtime
            .run(
                &session,
                "list files",
                &CancelToken::new(),
                &RunOptions {
                    mock: true,
                    auto_approve: true,
                    ..RunOptions::default()
                },
            )
            .unwrap();
        let projection = runtime.show_session(&session).unwrap();
        assert!(
            projection.transcript.iter().any(
                |item| matches!(item, TranscriptItem::Tool { name, .. } if name == "list_dir")
            )
        );
        assert!(
            projection
                .runs
                .iter()
                .any(|run| run.status.as_str() == "completed")
        );
        let events = runtime.events(&session).unwrap();
        assert!(events.iter().any(|e| e.event_type == "usage.recorded"));
    }

    #[test]
    fn steer_is_delivered_between_turns() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, dir.path().to_str().unwrap());
        runtime.queue_steer(&session, "also check README").unwrap();
        runtime
            .run(
                &session,
                "list files",
                &CancelToken::new(),
                &RunOptions {
                    mock: true,
                    auto_approve: true,
                    ..RunOptions::default()
                },
            )
            .unwrap();
        let projection = runtime.show_session(&session).unwrap();
        assert!(projection.transcript.iter().any(
            |item| matches!(item, TranscriptItem::User { text, .. } if text.starts_with("[steer]"))
        ));
        assert!(runtime.store.take_steers(&session).unwrap().is_empty());
    }

    struct FlakyProvider {
        failures: AtomicU32,
        inner: MockProvider,
    }

    impl Provider for FlakyProvider {
        fn name(&self) -> &'static str {
            "flaky"
        }

        fn complete(
            &self,
            request: &CompletionRequest,
            cancel: &CancelToken,
            on_event: &mut dyn FnMut(StreamEvent) -> Result<()>,
        ) -> Result<Completion> {
            if self.failures.fetch_sub(1, Ordering::SeqCst) > 0 {
                return Err(BloraError::provider("HTTP 503 retry-after=0: busy"));
            }
            self.inner.complete(request, cancel, on_event)
        }
    }

    #[test]
    fn retries_transient_provider_errors_then_succeeds() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        let run_id = RunId::generate();
        let turn_id = TurnId::generate();
        runtime
            .emit(
                &session,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::RunCreated(RunCreated {
                    mode: Mode::Code,
                    model: Some("mock".to_owned()),
                }),
            )
            .unwrap();
        let providers: Vec<Box<dyn Provider>> = vec![Box::new(FlakyProvider {
            failures: AtomicU32::new(1),
            inner: MockProvider::new(),
        })];
        let mut index = 0;
        let request = CompletionRequest {
            model: "mock".to_owned(),
            messages: vec![blora_model::ChatMessage::text("user", "hi")],
            ..CompletionRequest::default()
        };
        let (completion, _) = runtime
            .call_with_retry(
                &session,
                &run_id,
                &turn_id,
                &providers,
                &mut index,
                "mock",
                &request,
                &CancelToken::new(),
            )
            .unwrap();
        assert!(!completion.tool_calls.is_empty());
        let events = runtime.events(&session).unwrap();
        assert!(events.iter().any(|e| e.event_type == "retry.started"));

        // Fatal errors do not retry and do not fall through past auth failures.
        let fatal: Vec<Box<dyn Provider>> = vec![
            Box::new(FlakyProvider {
                failures: AtomicU32::new(u32::MAX / 2),
                inner: MockProvider::new(),
            }),
            Box::new(MockProvider::new()),
        ];
        let mut index = 0;
        // Retries are exhausted quickly because retry-after=0.
        let (completion, _) = runtime
            .call_with_retry(
                &session,
                &run_id,
                &turn_id,
                &fatal,
                &mut index,
                "mock",
                &request,
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(index, 1);
        assert!(!completion.tool_calls.is_empty());
    }

    #[test]
    fn subscribers_receive_events_and_projection_refreshes_incrementally() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let receiver = runtime.subscribe();
        let session = session(&runtime, Mode::Code, "/tmp");
        let mut projection = blora_session::SessionProjection::new();
        assert!(
            runtime
                .refresh_projection(&session, &mut projection)
                .unwrap()
        );
        let before = projection.last_sequence;
        assert!(
            !runtime
                .refresh_projection(&session, &mut projection)
                .unwrap()
        );
        runtime
            .run_mock(
                &session,
                "hello",
                &CancelToken::new(),
                &MockRunOptions::default(),
            )
            .unwrap();
        assert!(
            runtime
                .refresh_projection(&session, &mut projection)
                .unwrap()
        );
        assert!(projection.last_sequence > before);
        assert_eq!(
            projection.last_sequence,
            runtime.last_sequence(&session).unwrap()
        );
        let seen: Vec<_> = receiver.try_iter().collect();
        assert!(seen.iter().any(|e| e.event_type == "session.created"));
        assert!(seen.iter().any(|e| e.event_type == "run.completed"));
        drop(receiver);
        // A disconnected subscriber must not break later appends.
        runtime.archive_session(&session).unwrap();
    }

    #[test]
    fn exports_and_archives_session() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        let exported = runtime.export_session(&session).unwrap();
        assert!(exported.as_array().unwrap().iter().any(|event| {
            event.get("type").and_then(|value| value.as_str()) == Some("session.created")
        }));
        runtime.archive_session(&session).unwrap();
        assert_eq!(
            runtime
                .show_session(&session)
                .unwrap()
                .session
                .unwrap()
                .status
                .as_str(),
            "archived"
        );
    }

    #[test]
    fn pause_and_resume_task() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Work, "/tmp");
        let task_id = runtime
            .create_task(CreateTask {
                session_id: session,
                title: "later".to_owned(),
                prompt: "wait".to_owned(),
                delay_until: Some(chrono::Utc::now() + chrono::Duration::hours(1)),
                max_attempts: 1,
                auto_approve: true,
                mock: true,
                cron: None,
            })
            .unwrap();
        runtime.pause_task(&task_id).unwrap();
        assert_eq!(
            runtime.get_task(&task_id).unwrap().status.as_str(),
            "paused"
        );
        runtime.resume_task(&task_id).unwrap();
        assert_eq!(
            runtime.get_task(&task_id).unwrap().status.as_str(),
            "queued"
        );
        let listed = runtime.list_tasks(None).unwrap();
        assert_eq!(listed[0].id, task_id);
    }

    #[test]
    fn remembers_and_recalls() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        runtime
            .store
            .upsert_memory(dir.path().to_str().unwrap(), "k", "v", chrono::Utc::now())
            .unwrap();
        let row = runtime
            .store
            .get_memory(dir.path().to_str().unwrap(), "k")
            .unwrap()
            .unwrap();
        assert_eq!(row.value, "v");
    }

    #[test]
    fn searches_session_transcript() {
        let runtime = Runtime::new(SqliteStore::open_in_memory().unwrap());
        let session = session(&runtime, Mode::Code, "/tmp");
        runtime
            .run_mock(
                &session,
                "unique-needle-xyz",
                &CancelToken::new(),
                &MockRunOptions::default(),
            )
            .unwrap();
        let hits = runtime.search_sessions("unique-needle-xyz").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(runtime.search_sessions("no-such-text").unwrap().is_empty());
    }
}
