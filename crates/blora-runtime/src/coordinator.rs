// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use blora_context::compile_messages;
use blora_events::{
    AssistantDelta, AssistantMessageCompleted, ContextSnapshotCreated, EventEnvelope, KnownPayload,
    ModelRequested, ModelResponseCompleted, NewEvent, ProviderChanged, RetryStarted,
    RunCancelRequested, RunCancelled, RunCompleted, RunCreated, RunFailed, RunStarted,
    SessionArchived, SessionResumed, ToolCompleted, ToolFailed, ToolOutput, ToolRequested,
    ToolStarted, UsageRecorded, UserInput,
};
use blora_exec::{Isolation, LocalBackend, WorktreeHandle};
use blora_model::{CompletionRequest, StreamEvent, ToolCall, ToolDeclaration, make_providers};
use blora_policy::Policy;
use blora_session::SessionProjection;
use blora_storage::{CreateSession, CreateTask, SessionSummary, SqliteStore};
use blora_tools::ToolRegistry;
use blora_types::{BloraError, CancelToken, Mode, Result, RunId, SessionId, TurnId};
use serde_json::Value;

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
        }
    }
}

pub struct Runtime {
    pub(crate) store: SqliteStore,
}

impl Runtime {
    #[must_use]
    pub fn new(store: SqliteStore) -> Self {
        Self { store }
    }

    pub fn create_session(&self, spec: CreateSession) -> Result<SessionId> {
        let id = self.store.create_session(spec)?;
        crate::hooks::fire("session-start", id.as_str());
        Ok(id)
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

    pub fn upsert_passport_user(
        &self,
        username: &str,
        nickname: Option<&str>,
        avatar: Option<&str>,
        email: Option<&str>,
        app_token: Option<&str>,
    ) -> Result<blora_storage::UserRecord> {
        self.store
            .upsert_passport_user(username, nickname, avatar, email, app_token)
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
        let providers = make_providers(options.mock, &options.provider);
        let mut provider_index = 0;
        let model = if options.model.is_empty() {
            if options.mock || providers[provider_index].name() == "mock" {
                "mock".to_owned()
            } else {
                std::env::var("BLORA_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned())
            }
        } else {
            options.model.clone()
        };

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
            KnownPayload::UserInput(UserInput {
                text: prompt.to_owned(),
            }),
        )?;
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
        tools.extend(plugins.iter().map(crate::plugins::PluginSpec::declaration));

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
            let mut events = self.store.load_events(session_id)?;
            if should_auto_compact(&events) {
                let _ = self.compact(session_id);
                events = self.store.load_events(session_id)?;
            }
            let messages = compile_messages(&events, &exec_root, mode.as_str())?;
            self.emit(
                session_id,
                Some(&run_id),
                Some(&turn_id),
                KnownPayload::ModelRequested(ModelRequested {
                    provider: providers[provider_index].name().to_owned(),
                    model: model.clone(),
                }),
            )?;
            let mut streamed = String::new();
            let mut attempt = 0;
            let completion = loop {
                match providers[provider_index].complete(
                    &CompletionRequest {
                        model: model.clone(),
                        messages: messages.clone(),
                        tools: tools.clone(),
                    },
                    cancel,
                    &mut |event| match event {
                        StreamEvent::TextDelta(text) => {
                            streamed.push_str(&text);
                            self.emit(
                                session_id,
                                Some(&run_id),
                                Some(&turn_id),
                                KnownPayload::AssistantDelta(AssistantDelta { text }),
                            )
                        }
                        StreamEvent::ToolCall(_) => Ok(()),
                    },
                ) {
                    Ok(completion) => break completion,
                    Err(BloraError::Cancelled) => {
                        return self.cancel_run(session_id, &run_id, &turn_id);
                    }
                    Err(err) if provider_index + 1 < providers.len() && is_retryable(&err) => {
                        provider_index += 1;
                        attempt += 1;
                        self.emit(
                            session_id,
                            Some(&run_id),
                            Some(&turn_id),
                            KnownPayload::ProviderChanged(ProviderChanged {
                                provider: providers[provider_index].name().to_owned(),
                                model: model.clone(),
                            }),
                        )?;
                    }
                    Err(err) if attempt < 1 && is_retryable(&err) => {
                        attempt += 1;
                        self.emit(
                            session_id,
                            Some(&run_id),
                            Some(&turn_id),
                            KnownPayload::RetryStarted(RetryStarted {
                                attempt,
                                reason: err.to_string(),
                            }),
                        )?;
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
                    cached_tokens: 0,
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
                return Ok(run_id);
            }

            let mut fingerprints: Vec<String> = Vec::new();
            for call in completion.tool_calls {
                if cancel.is_cancelled() {
                    return self.cancel_run(session_id, &run_id, &turn_id);
                }
                let fingerprint = format!("{}:{}", call.name, call.arguments);
                fingerprints.push(fingerprint.clone());
                if fingerprints.len() >= 4
                    && fingerprints
                        .iter()
                        .rev()
                        .take(4)
                        .all(|item| item == &fingerprint)
                {
                    return Err(BloraError::Other(
                        "repeated tool calls detected; aborting loop".to_owned(),
                    ));
                }
                self.dispatch_tool(
                    session_id,
                    &run_id,
                    &turn_id,
                    &backend,
                    &call,
                    options,
                    cancel,
                    mcp.as_ref(),
                    &plugins,
                    &exec_root,
                )?;
            }
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
    ) -> Result<()> {
        let arguments: Value = serde_json::from_str(&call.arguments)
            .unwrap_or_else(|_| serde_json::json!({ "raw": call.arguments }));
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
        crate::hooks::fire("tool-before", &format!("{} {}", call.name, call.arguments));
        if options.read_only
            && (matches!(
                call.name.as_str(),
                "write_file"
                    | "shell"
                    | "apply_patch"
                    | "delegate"
                    | "schedule_task"
                    | "git_worktree"
                    | "process"
                    | "remember"
                    | "forget"
            ) || call.name.starts_with("plugin__"))
        {
            self.emit(
                session_id,
                Some(run_id),
                Some(turn_id),
                KnownPayload::ToolFailed(ToolFailed {
                    tool: call.name.clone(),
                    error: format!("{} blocked in read-only role", call.name),
                }),
            )?;
            self.emit(
                session_id,
                Some(run_id),
                Some(turn_id),
                KnownPayload::ToolOutput(ToolOutput {
                    text: format!("{} blocked in read-only role", call.name),
                    call_id: Some(call.id.clone()),
                }),
            )?;
            return Ok(());
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
        let intercepted = match execute(backend) {
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
        crate::hooks::fire("tool-after", &format!("{} {}", call.name, call.arguments));
        match intercepted {
            Ok(text) => {
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: text.clone(),
                        call_id: Some(call.id.clone()),
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
            }
            Err(BloraError::ApprovalRequired(summary)) => {
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolFailed(ToolFailed {
                        tool: call.name.clone(),
                        error: format!("approval required: {summary} (pass --yes to auto-approve)"),
                    }),
                )?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: format!(
                            "approval required for {summary}. Ask the user to rerun with --yes."
                        ),
                        call_id: Some(call.id.clone()),
                    }),
                )?;
            }
            Err(err) => {
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolFailed(ToolFailed {
                        tool: call.name.clone(),
                        error: err.to_string(),
                    }),
                )?;
                self.emit(
                    session_id,
                    Some(run_id),
                    Some(turn_id),
                    KnownPayload::ToolOutput(ToolOutput {
                        text: format!("tool error: {err}"),
                        call_id: Some(call.id.clone()),
                    }),
                )?;
            }
        }
        Ok(())
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
        self.store.append(event)?;
        Ok(())
    }
}

fn should_auto_compact(events: &[EventEnvelope]) -> bool {
    if events.len() < 48 {
        return false;
    }
    match events
        .iter()
        .rposition(|event| event.event_type == "context.compaction.completed")
    {
        None => true,
        Some(index) => events.len() - index > 32,
    }
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

fn is_retryable(err: &BloraError) -> bool {
    let text = err.to_string().to_ascii_lowercase();
    text.contains("429")
        || text.contains("timeout")
        || text.contains("temporar")
        || text.contains("connection")
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use blora_session::TranscriptItem;
    use blora_storage::SqliteStore;

    #[test]
    fn mock_run_persists_transcript() {
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: Some("t".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
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
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
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
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: dir.path().display().to_string(),
                mode: Mode::Work,
                parent_session_id: None,
            })
            .unwrap();
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
    fn forks_session() {
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: Some("src".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        let fork = runtime.fork_session(&session).unwrap();
        assert_ne!(fork, session);
        let projection = runtime.show_session(&fork).unwrap();
        assert_eq!(
            projection.session.unwrap().parent_session_id.as_ref(),
            Some(&session)
        );
    }

    #[test]
    fn agent_mode_spawns_code_child() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: dir.path().display().to_string(),
                mode: Mode::Agent,
                parent_session_id: None,
            })
            .unwrap();
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
    }

    #[test]
    fn compact_writes_summary_event() {
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
        runtime
            .run_mock(
                &session,
                "hello",
                &CancelToken::new(),
                &MockRunOptions::default(),
            )
            .unwrap();
        runtime.compact(&session).unwrap();
        assert!(
            runtime
                .events(&session)
                .unwrap()
                .iter()
                .any(|event| event.event_type == "context.compaction.completed")
        );
    }

    #[test]
    fn interactive_write_waits_for_approval() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: dir.path().display().to_string(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
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
    fn tool_loop_lists_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "hi").unwrap();
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: dir.path().display().to_string(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
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
    }

    #[test]
    fn exports_and_archives_session() {
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: Some("exp".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
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
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: None,
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Work,
                parent_session_id: None,
            })
            .unwrap();
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
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
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
        let store = SqliteStore::open_in_memory().unwrap();
        let runtime = Runtime::new(store);
        let session = runtime
            .create_session(CreateSession {
                title: Some("alpha".to_owned()),
                workspace_path: "/tmp".to_owned(),
                mode: Mode::Code,
                parent_session_id: None,
            })
            .unwrap();
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
