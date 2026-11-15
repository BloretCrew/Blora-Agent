// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufWriter, Write};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use blora_model::{Completion, MockProvider, ToolCall};
use blora_sdk::{
    Agent, ApprovalPolicy, AttachmentRef, CancelToken, EventEnvelope, Mode, PermissionMode,
    Profile, ProviderConfig, QuestionPolicy, RunOutcome, RunRequest, SdkError, SearchBackend,
    SearchConfig, Session, SessionId, Storage, Tool, ToolCallContext, ToolDefinition, ToolError,
    ToolOutcome,
};
use serde::Deserialize;
use serde_json::{Value, json};

/// Protocol version advertised by this implementation.
pub const PROTOCOL_VERSION: &str = "1.0";
const COMPLETED_RUN_RETENTION: usize = 128;

/// Run the stdio protocol until EOF or `agent.shutdown`.
pub fn serve_stdio(allow_test_provider: bool) -> Result<(), String> {
    let stdin = std::io::stdin();
    let stdout = BufWriter::new(std::io::stdout());
    let mut server = Server::new(stdout, allow_test_provider);
    for line in stdin.lock().lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(&line).map_err(|error| format!("invalid json: {error}"))?;
        validate_line_size(&value)?;
        if server.handle_line(value)? {
            break;
        }
    }
    if let Err(error) = server.cleanup(Duration::from_secs(5)) {
        eprintln!("SDK shutdown incomplete: {error}");
    }
    Ok(())
}

struct Server {
    agent: Option<Agent>,
    runs: Arc<Mutex<HashMap<String, RunEntry>>>,
    finished_runs: Arc<Mutex<VecDeque<String>>>,
    subscriptions: Arc<Mutex<HashMap<String, Subscription>>>,
    rpc: RpcHost,
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    allow_test_provider: bool,
}

struct RunEntry {
    cancel: CancelToken,
    outcome: Arc<Mutex<Option<Value>>>,
    signal: Arc<(Mutex<bool>, Condvar)>,
    finished: Arc<std::sync::atomic::AtomicBool>,
}

struct Subscription {
    stop: Arc<std::sync::atomic::AtomicBool>,
}

struct PendingCallGuard {
    pending: PendingHostCalls,
    request_id: String,
}

impl Drop for PendingCallGuard {
    fn drop(&mut self) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&self.request_id);
    }
}

type HostResponse = std::result::Result<Value, Value>;
type PendingHostCalls = Arc<Mutex<HashMap<String, SyncSender<HostResponse>>>>;

#[derive(Clone)]
struct RpcHost {
    writer: Arc<Mutex<BufWriter<std::io::Stdout>>>,
    pending: PendingHostCalls,
}

impl RpcHost {
    fn write(&self, value: Value) {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = serde_json::to_writer(&mut *writer, &value);
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }

    fn cancel_host_tool(&self, request_id: &str, reason: &str) {
        self.pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(request_id);
        self.write(json!({
            "jsonrpc":"2.0",
            "method":"host_tool.cancelled",
            "params":{"callId":request_id,"reason":reason},
        }));
    }
}

impl Server {
    fn new(stdout: BufWriter<std::io::Stdout>, allow_test_provider: bool) -> Self {
        let writer = Arc::new(Mutex::new(stdout));
        Self {
            agent: None,
            runs: Arc::new(Mutex::new(HashMap::new())),
            finished_runs: Arc::new(Mutex::new(VecDeque::new())),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            rpc: RpcHost {
                writer: writer.clone(),
                pending: Arc::new(Mutex::new(HashMap::new())),
            },
            writer,
            allow_test_provider,
        }
    }

    fn handle_line(&mut self, value: Value) -> Result<bool, String> {
        if value.get("method").is_none()
            && let Some(id) = value.get("id")
        {
            let key = id_key(id);
            if let Some(tx) = self
                .rpc
                .pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&key)
            {
                let result = match value.get("error") {
                    Some(error) if !error.is_null() => Err(error.clone()),
                    _ => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(result);
            }
            return Ok(false);
        }

        let id = value.get("id").cloned().unwrap_or(Value::Null);
        let method = value
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let params = value.get("params").cloned().unwrap_or(Value::Null);
        let notify = id.is_null();

        if method == "run.wait" && !notify {
            self.run_wait_async(id, params);
            return Ok(false);
        }
        if method == "agent.shutdown" && !notify {
            let cancel_running = params
                .get("cancelRunning")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            if !cancel_running {
                let active = self
                    .runs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .values()
                    .any(|entry| !entry.finished.load(std::sync::atomic::Ordering::SeqCst));
                if active {
                    self.write(json!({"jsonrpc":"2.0","id":id,"error":rpc_error(-32014,"invalid_state","active runs require cancelRunning=true")}));
                    return Ok(false);
                }
            }
            self.cancel_all();
            self.stop_subscriptions();
            self.write(json!({"jsonrpc":"2.0","id":id,"result":{}}));
            return Ok(true);
        }

        let result = self.dispatch(&method, params);
        if !notify {
            let response = match result {
                Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                Err(error) => json!({"jsonrpc": "2.0", "id": id, "error": error}),
            };
            self.write(response);
        }
        Ok(false)
    }

    fn next_sequence(&self, session: &SessionId) -> u64 {
        self.agent
            .as_ref()
            .and_then(|agent| agent.runtime().last_sequence(session).ok())
            .unwrap_or(0)
    }

    fn session_archive(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let session = self.session_from(&params)?;
        session.archive().map_err(sdk_error)?;
        Ok(json!({"archived":true}))
    }

    fn session_subscribe(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let id = session_id(&params)?;
        let after = params
            .get("afterSequence")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let session = self.session_from(&json!({"sessionId": id.to_string()}))?;
        let stream = session.live_events_from(after);
        let initial_events = self
            .agent()?
            .runtime()
            .events_after(&id, after)
            .map_err(|error| sdk_error(error.into()))?;
        let subscription_id = format!("sub_{}", uuid::Uuid::now_v7().simple());
        let rpc = self.rpc.clone();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop_thread = stop.clone();
        let subscribed_session = id.clone();
        let mut last_delivered = after;
        for event in initial_events {
            last_delivered = last_delivered.max(event.sequence);
            rpc.write(json!({
                "jsonrpc":"2.0", "method":"event", "params":{"event":event}
            }));
        }
        let thread_rpc = rpc.clone();
        let subscriptions = self.subscriptions.clone();
        self.subscriptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(subscription_id.clone(), Subscription { stop: stop.clone() });
        let subscription_id_clone = subscription_id.clone();
        std::thread::spawn(move || {
            let mut stream = stream;
            while !stop_thread.load(std::sync::atomic::Ordering::SeqCst) {
                match stream.next_timeout(Duration::from_millis(200)) {
                    Ok(Some(event)) => {
                        last_delivered = last_delivered.max(event.sequence);
                        thread_rpc.write(json!({
                            "jsonrpc":"2.0", "method":"event", "params":{"event":event}
                        }));
                    }
                    Ok(None) => continue,
                    Err(_) => {
                        thread_rpc.write(json!({
                            "jsonrpc":"2.0", "method":"stream.lagged",
                            "params":{"sessionId":subscribed_session.to_string(),"lastDeliveredSequence":last_delivered,"latestSequence":last_delivered}
                        }));
                        break;
                    }
                }
            }
            subscriptions
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(&subscription_id_clone);
        });
        Ok(json!({"subscriptionId":subscription_id,"sequence":self.next_sequence(&id)}))
    }

    fn session_unsubscribe(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let id = params
            .get("subscriptionId")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "subscriptionId is required"))?;
        let mut subscriptions = self
            .subscriptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(subscription) = subscriptions.remove(id) {
            subscription
                .stop
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(json!({"unsubscribed":true}))
    }

    fn dispatch(&mut self, method: &str, params: Value) -> std::result::Result<Value, Value> {
        match method {
            "initialize" => self.initialize(params),
            "agent.shutdown" => {
                if let Some(agent) = &self.agent {
                    agent.shutdown().map_err(sdk_error)?;
                }
                self.cancel_all();
                self.stop_subscriptions();
                Ok(json!({}))
            }
            "session.create" => self.session_create(params),
            "session.resume" => self.session_resume(params),
            "session.fork" => self.session_fork(params),
            "session.list" => self.session_list(),
            "session.project" => self.session_project(params),
            "session.events" => self.session_events(params),
            "session.archive" => self.session_archive(params),
            "session.subscribe" => self.session_subscribe(params),
            "session.unsubscribe" => self.session_unsubscribe(params),
            "run.start" => self.run_start(params),
            "run.cancel" => self.run_cancel(params),
            "run.steer" => self.run_steer(params),
            "approval.resolve" => self.approval_resolve(params),
            "input.answer" => self.input_answer(params),
            "host_tools.register" => self.host_tools_register(params),
            other => Err(rpc_error(-32601, "method_not_found", other)),
        }
    }

    fn initialize(&mut self, params: Value) -> std::result::Result<Value, Value> {
        if self.agent.is_some() {
            return Err(rpc_error(-32014, "invalid_state", "already initialized"));
        }
        let profile = params
            .get("profile")
            .and_then(Value::as_str)
            .map(parse_profile)
            .transpose()?
            .unwrap_or(Profile::full());
        let mut builder = Agent::builder().profile(profile);
        if let Some(workspace) = params.get("workspace") {
            if workspace.get("kind").and_then(Value::as_str) == Some("ephemeral") {
                builder = builder.ephemeral_workspace();
            } else if let Some(path) = workspace.as_str() {
                builder = builder.workspace(path);
            }
        }
        match params.get("persistence") {
            Some(value) if value.get("kind").and_then(Value::as_str) == Some("file") => {
                let path = value
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| rpc_error(-32001, "invalid_config", "persistence.path"))?;
                builder = builder.storage(Storage::file(path));
            }
            _ => builder = builder.memory(),
        }
        let provider = params
            .get("provider")
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "provider is required"))?;
        if provider.get("kind").and_then(Value::as_str).is_none() {
            return Err(rpc_error(
                -32001,
                "invalid_config",
                "provider.kind must be explicitly set",
            ));
        }
        if provider.get("kind").and_then(Value::as_str) == Some("env") {
            builder = builder.inherit_env(true);
        }
        if provider.get("kind").and_then(Value::as_str) == Some("scripted")
            && !self.allow_test_provider
        {
            return Err(rpc_error(
                -32001,
                "invalid_config",
                "scripted provider requires --allow-test-provider",
            ));
        }
        builder = builder.provider(parse_provider(provider)?);
        if let Some(search) = params.get("search").filter(|value| !value.is_null()) {
            builder = builder.search_config(parse_search_config(search)?);
        }
        if let Some(fetch) = params.get("fetch").filter(|value| !value.is_null()) {
            if fetch.get("allowPrivate").and_then(Value::as_bool) == Some(true) {
                return Err(rpc_error(
                    -32001,
                    "invalid_config",
                    "SDK transports do not permit private web_fetch targets",
                ));
            }
            builder = builder.fetch_options(blora_sdk::FetchOptions {
                max_bytes: fetch
                    .get("maxBytes")
                    .and_then(Value::as_u64)
                    .map_or(2_000_000, |v| v as usize),
                timeout: Duration::from_millis(
                    fetch
                        .get("timeoutMs")
                        .and_then(Value::as_u64)
                        .unwrap_or(20_000),
                ),
                allow_private: false,
            });
        }
        if let Some(callbacks) = params.get("callbacks") {
            if let Some(approval) = callbacks.get("approval").and_then(Value::as_str) {
                builder = builder.approval_policy(parse_approval(approval)?);
            }
            if let Some(question) = callbacks.get("question").and_then(Value::as_str) {
                builder = builder.question_policy(parse_question(question)?);
            }
        }
        if let Some(tools) = params.get("hostTools").and_then(Value::as_array) {
            let mut names = HashSet::new();
            for descriptor in tools {
                let remote = RemoteTool::from_json(descriptor, self.rpc.clone())?;
                if !names.insert(remote.definition.name.clone()) {
                    return Err(rpc_error(
                        -32001,
                        "invalid_config",
                        "duplicate host tool name",
                    ));
                }
                builder = builder.tool(Arc::new(remote));
            }
        }
        self.agent = Some(builder.build().map_err(sdk_error)?);
        Ok(
            json!({"serverInfo":{"name":"blora-agent","version":env!("CARGO_PKG_VERSION")},"protocolVersion":PROTOCOL_VERSION,"eventSchemaVersion":blora_types::SCHEMA_VERSION}),
        )
    }

    fn session_create(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let agent = self.agent()?;
        let mut spec = blora_sdk::SessionSpec {
            title: params
                .get("title")
                .and_then(Value::as_str)
                .map(str::to_owned),
            ..Default::default()
        };
        if let Some(mode) = params.get("mode").and_then(Value::as_str) {
            spec.mode = Some(Mode::parse(mode).map_err(|e| sdk_error(e.into()))?);
        }
        if let Some(permission) = params.get("permission").and_then(Value::as_str) {
            spec.permission = Some(
                PermissionMode::parse(permission)
                    .ok_or_else(|| rpc_error(-32001, "invalid_config", "invalid permission"))?,
            );
        }
        if let Some(workspace) = params.get("workspace").and_then(Value::as_str) {
            spec.workspace = Some(workspace.into());
        }
        let session = agent.create_session_with(spec).map_err(sdk_error)?;
        Ok(
            json!({"sessionId":session.id().to_string(),"sequence":agent.runtime().last_sequence(session.id()).unwrap_or(0)}),
        )
    }
    fn session_resume(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let id = session_id(&params)?;
        let session = self.agent()?.resume_session(&id).map_err(sdk_error)?;
        Ok(json!({"sessionId":session.id().to_string()}))
    }
    fn session_fork(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let id = session_id(&params)?;
        let session = self.agent()?.fork_session(&id).map_err(sdk_error)?;
        Ok(json!({"sessionId":session.id().to_string()}))
    }
    fn session_list(&mut self) -> std::result::Result<Value, Value> {
        let sessions=self.agent()?.list_sessions().map_err(sdk_error)?.into_iter().map(|s|json!({"sessionId":s.id.to_string(),"title":s.title,"workspace":s.workspace_path,"mode":s.mode.as_str(),"status":s.status.as_str(),"updatedAt":s.updated_at.to_rfc3339(),"lastSequence":s.last_sequence})).collect::<Vec<_>>();
        Ok(json!({"sessions":sessions}))
    }
    fn session_project(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let session = self.session_from(&params)?;
        serde_json::to_value(session.projection().map_err(sdk_error)?)
            .map_err(|e| rpc_error(-32012, "storage_error", &e.to_string()))
    }
    fn session_events(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let id = session_id(&params)?;
        let after = params
            .get("afterSequence")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let mut stream = self
            .agent()?
            .open_session(&id)
            .map_err(sdk_error)?
            .events_from(after);
        let mut events = Vec::new();
        while let Some(event) = stream.next_event().map_err(sdk_error)? {
            events.push(event);
        }
        let next = events.last().map_or(after, |e: &EventEnvelope| e.sequence);
        Ok(json!({"events":events,"nextSequence":next}))
    }

    fn run_start(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let id = session_id(&params)?;
        let session = self.agent()?.open_session(&id).map_err(sdk_error)?;
        let prompt = params
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "prompt is required"))?;
        if prompt.len() > 1024 * 1024 {
            return Err(rpc_error(-32001, "invalid_config", "prompt exceeds 1 MiB"));
        }
        let mut request = RunRequest::new(prompt);
        if let Some(model) = params.get("model").and_then(Value::as_str) {
            request.model = Some(model.to_owned());
        }
        if let Some(turns) = params.get("maxTurns").and_then(Value::as_u64) {
            request.max_turns = Some(
                u32::try_from(turns)
                    .map_err(|_| rpc_error(-32001, "invalid_config", "maxTurns exceeds u32"))?,
            );
        }
        if let Some(schema) = params.get("outputSchema").filter(|v| !v.is_null()) {
            request.output_schema = Some(schema.clone());
        }
        if let Some(permission) = params.get("permission").and_then(Value::as_str) {
            request.permission = Some(
                PermissionMode::parse(permission)
                    .ok_or_else(|| rpc_error(-32001, "invalid_config", "invalid permission"))?,
            );
        }
        if let Some(attachments) = params.get("attachments").filter(|v| !v.is_null()) {
            request.attachments = serde_json::from_value::<Vec<AttachmentRef>>(attachments.clone())
                .map_err(|e| rpc_error(-32001, "invalid_config", &e.to_string()))?;
        }
        let handle = session.run_with(request).map_err(sdk_error)?;
        let run_id = handle.id().to_string();
        let start_sequence = handle.start_sequence();
        let cancel = handle.cancel_token();
        let events = handle.events();
        let event_session = session.clone();
        let outcome = Arc::new(Mutex::new(None));
        let signal = Arc::new((Mutex::new(false), Condvar::new()));
        let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(
                run_id.clone(),
                RunEntry {
                    cancel,
                    outcome: outcome.clone(),
                    signal: signal.clone(),
                    finished: finished.clone(),
                },
            );
        let rpc = self.rpc.clone();
        let runs = self.runs.clone();
        let finished_runs = self.finished_runs.clone();
        let run_key = run_id.clone();
        std::thread::spawn(move || {
            let mut stream = events;
            let mut cursor = start_sequence;
            loop {
                match stream.next_event() {
                    Ok(Some(event)) => {
                        cursor = cursor.max(event.sequence);
                        rpc.write(
                            json!({"jsonrpc":"2.0","method":"event","params":{"event":event}}),
                        );
                    }
                    Ok(None) => break,
                    Err(_) => {
                        let latest = event_session.last_sequence().unwrap_or(cursor);
                        let mut missed = event_session.events_from(cursor);
                        while let Ok(Some(event)) = missed.next_timeout(Duration::from_millis(0)) {
                            cursor = cursor.max(event.sequence);
                            rpc.write(
                                json!({"jsonrpc":"2.0","method":"event","params":{"event":event}}),
                            );
                        }
                        rpc.write(json!({"jsonrpc":"2.0","method":"stream.lagged","params":{"sessionId":event_session.id().to_string(),"lastDeliveredSequence":cursor,"latestSequence":latest}}));
                        break;
                    }
                }
            }
            let final_result = handle.wait();
            let payload = match final_result {
                Ok(outcome) => outcome_json(&outcome),
                Err(error) => {
                    json!({"runId":run_key,"status":if matches!(error,SdkError::Kernel(blora_types::BloraError::Cancelled)){"cancelled"}else{"failed"},"error":error.to_string()})
                }
            };
            rpc.write(json!({"jsonrpc":"2.0","method":"run.finished","params":payload}));
            *outcome
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(payload);
            finished.store(true, std::sync::atomic::Ordering::SeqCst);
            {
                let mut completed = finished_runs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                completed.push_back(run_key.clone());
                while completed.len() > COMPLETED_RUN_RETENTION {
                    if let Some(expired) = completed.pop_front() {
                        runs.lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .remove(&expired);
                    }
                }
            }
            let (lock, cv) = &*signal;
            *lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = true;
            cv.notify_all();
        });
        Ok(json!({"runId":run_id,"startSequence":start_sequence}))
    }

    fn run_wait_async(&self, id: Value, params: Value) {
        let Some(run_id) = params
            .get("runId")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            self.write(json!({"jsonrpc":"2.0","id":id,"error":rpc_error(-32001,"invalid_config","runId is required")}));
            return;
        };
        let timeout = params
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map_or(Duration::from_secs(300), Duration::from_millis);
        let (outcome, signal, _finished) = {
            let runs = self
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(entry) = runs.get(&run_id) else {
                self.write(json!({"jsonrpc":"2.0","id":id,"error":rpc_error(-32003,"run_not_found",&run_id)}));
                return;
            };
            (
                entry.outcome.clone(),
                entry.signal.clone(),
                entry.finished.clone(),
            )
        };
        let rpc = self.rpc.clone();
        std::thread::spawn(move || {
            let (lock, cv) = &*signal;
            let mut done = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let started = Instant::now();
            let mut timeouted = false;
            while !*done {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    timeouted = true;
                    break;
                }
                let (next, result) = cv
                    .wait_timeout(done, remaining)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                done = next;
                if result.timed_out() && !*done {
                    timeouted = true;
                    break;
                }
            }
            drop(done);
            let result = if timeouted {
                Err(rpc_error(-32014, "invalid_state", "run.wait timed out"))
            } else {
                outcome
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| rpc_error(-32003, "run_not_found", "run outcome unavailable"))
            };
            let response = match result {
                Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
                Err(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
            };
            rpc.write(response);
        });
    }
    fn run_cancel(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let run = params
            .get("runId")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "runId is required"))?;
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = runs
            .get(run)
            .ok_or_else(|| rpc_error(-32003, "run_not_found", run))?;
        entry.cancel.cancel();
        Ok(json!({"cancelled":true}))
    }
    fn run_steer(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let session = self.session_from(&params)?;
        let message = params
            .get("message")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "message is required"))?;
        session.steer(message).map_err(sdk_error)?;
        Ok(json!({}))
    }
    fn approval_resolve(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let session = self.session_from(&params)?;
        let id = params
            .get("approvalId")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "approvalId is required"))?;
        let id = id
            .parse()
            .map_err(|e| rpc_error(-32001, "invalid_config", &format!("{e}")))?;
        let allow = match params.get("decision").and_then(Value::as_str) {
            Some("allow") => true,
            Some("deny") => false,
            _ => {
                return Err(rpc_error(
                    -32001,
                    "invalid_config",
                    "decision must be allow|deny",
                ));
            }
        };
        session.resolve_approval(&id, allow).map_err(sdk_error)?;
        Ok(json!({}))
    }
    fn input_answer(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let session = self.session_from(&params)?;
        let id = params
            .get("requestId")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "requestId is required"))?;
        session
            .answer_input(
                id,
                params
                    .get("answer")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            )
            .map_err(sdk_error)?;
        Ok(json!({}))
    }
    fn host_tools_register(&mut self, params: Value) -> std::result::Result<Value, Value> {
        let agent = self.agent()?;
        let descriptors = params
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "tools is required"))?;
        let mut names = HashSet::new();
        let mut registered = Vec::new();
        for descriptor in descriptors {
            let remote = RemoteTool::from_json(descriptor, self.rpc.clone())?;
            if !names.insert(remote.definition.name.clone()) {
                return Err(rpc_error(
                    -32001,
                    "invalid_config",
                    "duplicate host tool name",
                ));
            }
            registered.push(remote.definition.name.clone());
            agent.register_tool(Arc::new(remote)).map_err(sdk_error)?;
        }
        Ok(json!({"registered":registered}))
    }
    fn session_from(&self, params: &Value) -> std::result::Result<Session, Value> {
        let id = session_id(params)?;
        self.agent()?.open_session(&id).map_err(sdk_error)
    }
    fn agent(&self) -> std::result::Result<&Agent, Value> {
        self.agent
            .as_ref()
            .ok_or_else(|| rpc_error(-32014, "invalid_state", "initialize first"))
    }
    fn cancel_all(&self) {
        let runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for entry in runs.values() {
            entry.cancel.cancel();
        }
    }
    fn stop_subscriptions(&self) {
        let subscriptions = self
            .subscriptions
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for subscription in subscriptions.values() {
            subscription
                .stop
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn cleanup(&mut self, timeout: Duration) -> std::result::Result<(), String> {
        self.cancel_all();
        self.stop_subscriptions();
        let started = Instant::now();
        loop {
            let active = self
                .runs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .any(|entry| !entry.finished.load(std::sync::atomic::Ordering::SeqCst));
            if !active {
                if let Some(agent) = &self.agent {
                    agent
                        .shutdown_timeout(timeout.saturating_sub(started.elapsed()))
                        .map_err(|error| error.to_string())?;
                }
                return Ok(());
            }
            if started.elapsed() >= timeout {
                return Err("active run shutdown timed out".to_owned());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    fn write(&self, value: Value) {
        let mut writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = serde_json::to_writer(&mut *writer, &value);
        let _ = writer.write_all(b"\n");
        let _ = writer.flush();
    }
}

struct RemoteTool {
    definition: ToolDefinition,
    rpc: RpcHost,
}
impl RemoteTool {
    fn from_json(value: &Value, rpc: RpcHost) -> std::result::Result<Self, Value> {
        let name = value
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| rpc_error(-32001, "invalid_config", "tool name is required"))?;
        if name.starts_with("mcp__") || name.starts_with("plugin__") {
            return Err(rpc_error(-32001, "invalid_config", "reserved tool name"));
        }
        if !valid_tool_name(name) {
            return Err(rpc_error(-32001, "invalid_config", "invalid tool name"));
        }
        let description = value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let parameters = value
            .get("parameters")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object","properties":{}}));
        let read_only = value
            .get("readOnly")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let requires_approval = value
            .get("requiresApproval")
            .and_then(Value::as_bool)
            .unwrap_or(!read_only);
        Ok(Self {
            definition: ToolDefinition {
                name: name.to_owned(),
                description: description.to_owned(),
                parameters,
                read_only,
                requires_approval,
            },
            rpc,
        })
    }
}
impl Tool for RemoteTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    fn call(
        &self,
        ctx: &ToolCallContext,
        arguments: Value,
    ) -> std::result::Result<ToolOutcome, ToolError> {
        let request_id = ctx.call_id.clone();
        let (tx, rx) = sync_channel(1);
        self.rpc
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(request_id.clone(), tx);
        let _guard = PendingCallGuard {
            pending: self.rpc.pending.clone(),
            request_id: request_id.clone(),
        };
        self.rpc.write(json!({"jsonrpc":"2.0","id":request_id,"method":"host_tool.invoke","params":{"callId":ctx.call_id,"tool":self.definition.name,"arguments":arguments,"sessionId":ctx.session_id.to_string(),"runId":ctx.run_id.to_string(),"timeoutMs":30000}}));
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if ctx.cancel.is_cancelled() {
                self.rpc.cancel_host_tool(&request_id, "cancelled");
                return Err(ToolError::new("host tool cancelled"));
            }
            if Instant::now() >= deadline {
                self.rpc.cancel_host_tool(&request_id, "timeout");
                return Err(ToolError::new("host tool timed out"));
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(value)) => {
                    return serde_json::from_value::<ToolOutcome>(value)
                        .map_err(|e| ToolError::new(format!("invalid host tool result: {e}")));
                }
                Ok(Err(error)) => {
                    return Err(ToolError::new(
                        error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("host tool failed"),
                    ));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(ToolError::new("host tool connection closed"));
                }
            }
        }
    }
}

fn parse_profile(value: &str) -> std::result::Result<Profile, Value> {
    match value {
        "full" => Ok(Profile::full()),
        "web" | "chat" => Ok(Profile::web()),
        "read-only" | "read_only" => Ok(Profile::read_only_workspace()),
        other => Err(rpc_error(
            -32001,
            "invalid_config",
            &format!("unknown profile {other}"),
        )),
    }
}
fn parse_provider(value: &Value) -> std::result::Result<ProviderConfig, Value> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(-32001, "invalid_config", "provider.kind is required"))?;
    match kind {
        "mock" => Ok(ProviderConfig::mock()),
        "scripted" => {
            let list = value
                .get("completions")
                .and_then(Value::as_array)
                .ok_or_else(|| rpc_error(-32001, "invalid_config", "completions is required"))?;
            let provider = Arc::new(MockProvider::new());
            for item in list {
                let scripted: ScriptedCompletion = serde_json::from_value(item.clone())
                    .map_err(|e| rpc_error(-32001, "invalid_config", &e.to_string()))?;
                provider.push(scripted.into());
            }
            Ok(ProviderConfig::custom(provider))
        }
        "openai-compatible" | "openai" => {
            let base = value.get("baseUrl").and_then(Value::as_str);
            let key = value
                .get("apiKey")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let model = value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Ok(match base {
                Some(base) => ProviderConfig::openai_compatible(base, key, model),
                None => ProviderConfig::openai(key, model),
            })
        }
        "anthropic" => Ok(ProviderConfig::anthropic(
            value
                .get("apiKey")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )),
        "responses" => Ok(ProviderConfig::responses(
            value
                .get("apiKey")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )),
        "gemini" => Ok(ProviderConfig::gemini(
            value
                .get("apiKey")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        )),
        "env" => Ok(ProviderConfig::from_env()),
        other => Err(rpc_error(
            -32001,
            "invalid_config",
            &format!("unknown provider {other}"),
        )),
    }
}
fn parse_search_config(value: &Value) -> std::result::Result<SearchConfig, Value> {
    let backend = value
        .get("backend")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(-32001, "invalid_config", "search.backend is required"))?;
    let credential = value
        .get("credential")
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| rpc_error(-32001, "invalid_config", "search.credential is required"))?;
    let backend = SearchBackend::parse(backend)
        .ok_or_else(|| rpc_error(-32001, "invalid_config", "unknown search.backend"))?;
    Ok(SearchConfig {
        backend,
        credential: credential.to_owned(),
        endpoint: value
            .get("endpoint")
            .and_then(Value::as_str)
            .map(str::to_owned),
    })
}
fn parse_approval(value: &str) -> std::result::Result<ApprovalPolicy, Value> {
    match value {
        "deny" => Ok(ApprovalPolicy::DenyAll),
        "allow" => Ok(ApprovalPolicy::AllowAll),
        "read-only" | "read_only" => Ok(ApprovalPolicy::AllowReadOnly),
        "manual" => Ok(ApprovalPolicy::Manual {
            timeout: Some(Duration::from_secs(300)),
        }),
        other => Err(rpc_error(
            -32001,
            "invalid_config",
            &format!("unknown approval {other}"),
        )),
    }
}
fn parse_question(value: &str) -> std::result::Result<QuestionPolicy, Value> {
    match value {
        "dismiss" => Ok(QuestionPolicy::Dismiss),
        "manual" => Ok(QuestionPolicy::Manual {
            timeout: Some(Duration::from_secs(300)),
        }),
        other => Err(rpc_error(
            -32001,
            "invalid_config",
            &format!("unknown question {other}"),
        )),
    }
}
fn session_id(params: &Value) -> std::result::Result<SessionId, Value> {
    params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| rpc_error(-32001, "invalid_config", "sessionId is required"))?
        .parse()
        .map_err(|e| rpc_error(-32001, "invalid_config", &format!("{e}")))
}
fn outcome_json(outcome: &RunOutcome) -> Value {
    json!({"runId":outcome.run_id.to_string(),"status":outcome.status.as_str(),"finalText":outcome.final_text,"usage":{"inputTokens":outcome.usage.input_tokens,"outputTokens":outcome.usage.output_tokens,"cachedTokens":outcome.usage.cached_tokens},"lastSequence":outcome.last_sequence})
}
fn sdk_error(error: SdkError) -> Value {
    let kind = match &error {
        SdkError::Kernel(blora_types::BloraError::Policy(_)) => "policy_denied",
        SdkError::Kernel(blora_types::BloraError::ApprovalRequired(_)) => "approval_required",
        SdkError::Kernel(blora_types::BloraError::Cancelled) => "cancelled",
        SdkError::Kernel(blora_types::BloraError::Provider(_)) => "provider_error",
        SdkError::Kernel(blora_types::BloraError::Storage(_)) => "storage_error",
        SdkError::Kernel(blora_types::BloraError::SessionNotFound(_)) => "session_not_found",
        SdkError::ProviderRequired => "provider_required",
        SdkError::Unsupported(_) => "unsupported",
        SdkError::Config(_) => "invalid_config",
        SdkError::Closed => "invalid_state",
        SdkError::RunActive(_) => "session_busy",
        _ => "internal",
    };
    rpc_error(-32000, kind, &error.to_string())
}
fn rpc_error(code: i64, kind: &str, message: &str) -> Value {
    json!({"code":code,"message":message,"data":{"kind":kind}})
}
fn id_key(id: &Value) -> String {
    id.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| id.to_string())
}
fn valid_tool_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}
fn validate_line_size(value: &Value) -> std::result::Result<(), String> {
    let size = serde_json::to_vec(value).map_err(|e| e.to_string())?.len();
    if size > 8 * 1024 * 1024 {
        Err("JSON-RPC message exceeds 8 MiB".to_owned())
    } else {
        Ok(())
    }
}
#[derive(Deserialize)]
struct ScriptedCompletion {
    #[serde(default)]
    text: String,
    #[serde(default)]
    tool_calls: Vec<ToolCall>,
    #[serde(default)]
    finish_reason: String,
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
    #[serde(default)]
    cached_tokens: u64,
}
impl From<ScriptedCompletion> for Completion {
    fn from(v: ScriptedCompletion) -> Self {
        Self {
            text: v.text,
            tool_calls: v.tool_calls,
            input_tokens: v.input_tokens,
            output_tokens: v.output_tokens,
            cached_tokens: v.cached_tokens,
            finish_reason: if v.finish_reason.is_empty() {
                "stop".to_owned()
            } else {
                v.finish_reason
            },
        }
    }
}
