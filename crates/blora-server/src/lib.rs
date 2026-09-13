// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local HTTP API. The Web UI only talks to this surface.

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Component, Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap as HttpHeaderMap;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::{CreateSession, CreateTask};
use blora_types::{ApprovalId, Mode, SessionId, TaskId};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tower_http::cors::CorsLayer;

const INDEX_HTML: &str = include_str!("../../../web/index.html");
const APP_JS: &str = include_str!("../../../web/app.js");
const APP_CSS: &str = include_str!("../../../web/app.css");

#[derive(Clone)]
struct AppState {
    runtime: Arc<Runtime>,
    workspace: PathBuf,
    cancels: Arc<Mutex<HashMap<String, CancelToken>>>,
    require_auth: bool,
}

#[derive(Deserialize)]
struct CreateBody {
    title: Option<String>,
    workspace: Option<String>,
    mode: Option<String>,
}

#[derive(Deserialize)]
struct RunBody {
    prompt: String,
    #[serde(default)]
    mock: bool,
    #[serde(default)]
    auto_approve: bool,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    worktree: bool,
}

#[derive(Serialize)]
struct SessionJson {
    id: String,
    title: Option<String>,
    workspace_path: String,
    mode: String,
    status: String,
    last_sequence: u64,
    input_tokens: u64,
    output_tokens: u64,
    transcript: Vec<TranscriptJson>,
    tasks: Vec<TaskJson>,
    subagents: Vec<SubagentJson>,
}

#[derive(Serialize)]
struct TranscriptJson {
    kind: String,
    text: String,
}

#[derive(Serialize)]
struct TaskJson {
    id: String,
    title: String,
    status: String,
    session_id: String,
    attempt: u32,
    cron: Option<String>,
}

#[derive(Serialize)]
struct SubagentJson {
    id: String,
    role: String,
    status: String,
    summary: Option<String>,
    child_session_id: Option<String>,
}

#[derive(Deserialize)]
struct TaskBody {
    session_id: String,
    title: String,
    prompt: String,
    #[serde(default)]
    delay_seconds: u64,
    #[serde(default)]
    mock: bool,
    #[serde(default)]
    auto_approve: bool,
    #[serde(default)]
    cron: Option<String>,
}

pub async fn serve(
    runtime: Arc<Runtime>,
    bind: SocketAddr,
    workspace: PathBuf,
    require_auth: bool,
) -> Result<(), String> {
    let tick_runtime = runtime.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            interval.tick().await;
            let runtime = tick_runtime.clone();
            let _ = tokio::task::spawn_blocking(move || runtime.pump()).await;
        }
    });
    let state = AppState {
        runtime,
        workspace,
        cancels: Arc::new(Mutex::new(HashMap::new())),
        require_auth,
    };
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|err| err.to_string())?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, app(state))
        .await
        .map_err(|err| err.to_string())
}

fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/vendor/{*path}", get(vendor_asset))
        .route("/favicon.svg", get(favicon))
        .route("/favicon.ico", get(favicon))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", get(show_session))
        .route("/api/sessions/{id}/run", post(run_session))
        .route("/api/sessions/{id}/events", get(session_events))
        .route("/api/sessions/{id}/fork", post(fork_session))
        .route("/api/sessions/{id}/export", get(export_session))
        .route("/api/sessions/{id}/archive", post(archive_session))
        .route("/api/sessions/{id}/resume", post(resume_session))
        .route("/api/sessions/{id}/compact", post(compact_session))
        .route("/api/sessions/{id}/cancel", post(cancel_session))
        .route("/api/usage", get(usage))
        .route("/api/plugins", get(plugins))
        .route("/api/artifacts", get(list_artifacts))
        .route("/api/tasks", get(list_tasks).post(create_task))
        .route("/api/tasks/{id}/cancel", post(cancel_task))
        .route("/api/tasks/{id}/pause", post(pause_task))
        .route("/api/tasks/{id}/resume", post(resume_task))
        .route("/api/tasks/pump", post(pump_tasks))
        .route("/api/approvals", get(list_approvals))
        .route("/api/approvals/{id}/resolve", post(resolve_approval))
        .route("/api/workspace", get(workspace_snapshot))
        .route("/api/workspace/file", get(workspace_file))
        .route("/api/settings", get(settings))
        .route("/api/login", post(login))
        .route(
            "/api/marketplace",
            get(marketplace).post(install_market_plugin),
        )
        .route("/ws", get(ws_upgrade))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn app_js() -> Response {
    js(APP_JS)
}

async fn app_css() -> Response {
    css(APP_CSS)
}

async fn vendor_asset(Path(path): Path<String>) -> Response {
    let Some(disk) = resolve_vendor(&path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match std::fs::read_to_string(&disk) {
        Ok(body) if path.ends_with(".css") => css_owned(body),
        Ok(body) => js_owned(body),
        Err(_) if path == "blora.css" => css(
            ":root { --blora-background:#faf7f8; --blora-text:#2a1f24; --blora-primary:#9f5964; }",
        ),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

const FAVICON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="8" fill="#9f5964"/><text x="16" y="22" text-anchor="middle" font-size="14" font-family="system-ui,sans-serif" fill="#faf7f8">BA</text></svg>"##;

async fn favicon() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("image/svg+xml; charset=utf-8"),
    );
    (headers, FAVICON_SVG).into_response()
}

fn vendor_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../blora-design")
}

fn resolve_vendor(request: &str) -> Option<PathBuf> {
    let request = request.trim_start_matches('/');
    if request.is_empty() {
        return None;
    }
    let path = FsPath::new(request);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }
    let ext = path.extension()?.to_str()?;
    if !matches!(ext, "css" | "js") {
        return None;
    }
    let root = vendor_root();
    let disk = match request {
        "layout.css" | "layout.global.js" => {
            root.join("addons/layout/dist").join(path.file_name()?)
        }
        "theming.css" | "theming.global.js" => {
            root.join("addons/theming/dist").join(path.file_name()?)
        }
        _ => root.join("packages/blora-design/dist").join(path),
    };
    disk.is_file().then_some(disk)
}

#[derive(Deserialize)]
struct SessionListQuery {
    q: Option<String>,
}

async fn list_sessions(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Query(query): Query<SessionListQuery>,
) -> Result<Json<Vec<SessionJson>>, ApiError> {
    let user = current_user(&state, &headers)?;
    let user_id = user.as_ref().map(|item| item.id.as_str());
    let sessions = if let Some(q) = query.q.filter(|value| !value.trim().is_empty()) {
        state.runtime.search_sessions(&q).map_err(ApiError::from)?
    } else {
        state
            .runtime
            .list_sessions_for_user(user_id)
            .map_err(ApiError::from)?
    };
    let mut out = Vec::new();
    for session in sessions {
        out.push(to_json(&state, &session.id)?);
    }
    Ok(Json(out))
}

async fn create_session(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Json(body): Json<CreateBody>,
) -> Result<Json<SessionJson>, ApiError> {
    let workspace = body
        .workspace
        .unwrap_or_else(|| state.workspace.display().to_string());
    let mode = body
        .mode
        .as_deref()
        .map(Mode::parse)
        .transpose()
        .map_err(ApiError::from)?
        .unwrap_or(Mode::Code);
    let id = state
        .runtime
        .create_session(CreateSession {
            title: body.title,
            workspace_path: workspace,
            mode,
            parent_session_id: None,
        })
        .map_err(ApiError::from)?;
    if let Some(user) = current_user(&state, &headers)? {
        state
            .runtime
            .set_session_user(&id, &user.id)
            .map_err(ApiError::from)?;
    }
    to_json(&state, &id).map(Json)
}

async fn show_session(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
) -> Result<Json<SessionJson>, ApiError> {
    let session_id = SessionId::parse(&id).map_err(ApiError::from)?;
    ensure_session(&state, &headers, &session_id)?;
    to_json(&state, &session_id).map(Json)
}

async fn run_session(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RunBody>,
) -> Result<Json<SessionJson>, ApiError> {
    let session_id = SessionId::parse(&id).map_err(ApiError::from)?;
    ensure_session(&state, &headers, &session_id)?;
    let runtime = state.runtime.clone();
    let prompt = body.prompt;
    let cancel = CancelToken::new();
    state
        .cancels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(id.clone(), cancel.clone());
    let options = RunOptions {
        mock: body.mock
            || (std::env::var("BLORA_API_KEY").is_err()
                && std::env::var("OPENAI_API_KEY").is_err()
                && std::env::var("ANTHROPIC_API_KEY").is_err()
                && std::env::var("GEMINI_API_KEY").is_err()),
        auto_approve: body.auto_approve,
        interactive: !body.auto_approve,
        provider: body.provider.unwrap_or_default(),
        model: body.model.unwrap_or_default(),
        worktree: body.worktree,
        ..RunOptions::default()
    };
    let result =
        tokio::task::spawn_blocking(move || runtime.run(&session_id, &prompt, &cancel, &options))
            .await
            .map_err(|err| ApiError(err.to_string()))?;
    state
        .cancels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(&id);
    result.map_err(ApiError::from)?;
    let session_id = SessionId::parse(&id).map_err(ApiError::from)?;
    to_json(&state, &session_id).map(Json)
}

async fn session_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, Infallible>>>, ApiError>
{
    let session_id = SessionId::parse(&id).map_err(ApiError::from)?;
    let runtime = state.runtime.clone();
    let (tx, rx) = mpsc::channel::<std::result::Result<Event, Infallible>>(16);
    tokio::spawn(async move {
        let mut last = 0_u64;
        loop {
            let runtime = runtime.clone();
            let session_id = session_id.clone();
            let snapshot =
                tokio::task::spawn_blocking(move || runtime.show_session(&session_id)).await;
            match snapshot {
                Ok(Ok(projection)) if projection.last_sequence != last => {
                    last = projection.last_sequence;
                    let body = serde_json::json!({
                        "sequence": last,
                        "status": projection.runs.last().map(|run| run.status.as_str()),
                    });
                    let event = Event::default()
                        .json_data(body)
                        .unwrap_or_else(|_| Event::default());
                    if tx.send(Ok(event)).await.is_err() {
                        break;
                    }
                }
                Ok(Err(_)) | Err(_) => break,
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    });
    Ok(Sse::new(ReceiverStream::new(rx)).keep_alive(KeepAlive::default()))
}

#[derive(Deserialize)]
struct TaskQuery {
    session: Option<String>,
}

async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<TaskJson>>, ApiError> {
    let session_id = query.session.as_deref().map(SessionId::parse).transpose()?;
    let tasks = state
        .runtime
        .list_tasks(session_id.as_ref())
        .map_err(ApiError::from)?;
    Ok(Json(tasks.into_iter().map(task_json).collect()))
}

async fn create_task(
    State(state): State<AppState>,
    Json(body): Json<TaskBody>,
) -> Result<Json<TaskJson>, ApiError> {
    let delay_until = if body.delay_seconds == 0 {
        None
    } else {
        Some(chrono::Utc::now() + chrono::Duration::seconds(body.delay_seconds as i64))
    };
    let mock = body.mock
        || (std::env::var("BLORA_API_KEY").is_err() && std::env::var("OPENAI_API_KEY").is_err());
    let id = state
        .runtime
        .create_task(CreateTask {
            session_id: SessionId::parse(&body.session_id)?,
            title: body.title,
            prompt: body.prompt,
            delay_until,
            max_attempts: 3,
            auto_approve: body.auto_approve,
            mock,
            cron: body.cron,
        })
        .map_err(ApiError::from)?;
    Ok(Json(task_json(
        state.runtime.get_task(&id).map_err(ApiError::from)?,
    )))
}

async fn cancel_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .cancel_task(&TaskId::parse(&id)?)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "cancelled": id })))
}

async fn pump_tasks(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let runtime = state.runtime.clone();
    let finished = tokio::task::spawn_blocking(move || runtime.pump())
        .await
        .map_err(|err| ApiError(err.to_string()))?
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "pumped": finished.iter().map(ToString::to_string).collect::<Vec<_>>()
    })))
}

#[derive(Serialize)]
struct ApprovalJson {
    id: String,
    summary: String,
    status: String,
    session_id: String,
}

#[derive(Deserialize)]
struct ResolveBody {
    allow: bool,
}

async fn list_approvals(
    State(state): State<AppState>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<Vec<ApprovalJson>>, ApiError> {
    let rows = if let Some(session) = query.session {
        let session_id = SessionId::parse(&session)?;
        state
            .runtime
            .pending_approvals(&session_id)
            .map_err(ApiError::from)?
    } else {
        state
            .runtime
            .pending_approvals_all()
            .map_err(ApiError::from)?
    };
    Ok(Json(
        rows.into_iter()
            .map(|row| ApprovalJson {
                id: row.id.to_string(),
                summary: row.summary,
                status: row.status,
                session_id: row.session_id.to_string(),
            })
            .collect(),
    ))
}

async fn resolve_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ResolveBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .resolve_approval(&ApprovalId::parse(&id)?, body.allow)
        .map_err(ApiError::from)?;
    Ok(Json(
        serde_json::json!({ "resolved": id, "allow": body.allow }),
    ))
}

async fn compact_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session_id = SessionId::parse(&id)?;
    let runtime = state.runtime.clone();
    let summary = tokio::task::spawn_blocking(move || runtime.compact(&session_id))
        .await
        .map_err(|err| ApiError(err.to_string()))?
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "summary": summary })))
}

async fn fork_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionJson>, ApiError> {
    let child = state
        .runtime
        .fork_session(&SessionId::parse(&id)?)
        .map_err(ApiError::from)?;
    to_json(&state, &child).map(Json)
}

async fn export_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .export_session(&SessionId::parse(&id)?)
        .map(Json)
        .map_err(ApiError::from)
}

async fn archive_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .archive_session(&SessionId::parse(&id)?)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "archived": id })))
}

async fn resume_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .resume_session(&SessionId::parse(&id)?)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "resumed": id })))
}

async fn pause_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .pause_task(&TaskId::parse(&id)?)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "paused": id })))
}

async fn resume_task(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    state
        .runtime
        .resume_task(&TaskId::parse(&id)?)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({ "resumed": id })))
}

async fn workspace_snapshot(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let info = state
        .runtime
        .workspace_info(&state.workspace)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "path": info.path,
        "files": info.files,
        "git_status": info.git_status,
        "git_diff": info.git_diff,
        "git_log": info.git_log,
        "git_branch": info.git_branch,
        "entries": info.entries,
    })))
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
}

async fn workspace_file(
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let contents = state
        .runtime
        .read_workspace_file(&state.workspace, &query.path)
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "path": query.path,
        "contents": contents,
    })))
}

async fn list_artifacts(
    State(state): State<AppState>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session_id = query.session.as_deref().map(SessionId::parse).transpose()?;
    let rows = state
        .runtime
        .list_artifacts(session_id.as_ref())
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "artifacts": rows.into_iter().map(|row| serde_json::json!({
            "id": row.id.to_string(),
            "session_id": row.session_id.to_string(),
            "kind": row.kind,
            "path": row.path,
            "content": row.content.map(|text| {
                if text.len() > 2000 {
                    format!("{}…", &text[..2000])
                } else {
                    text
                }
            }),
            "created_at": row.created_at.to_rfc3339(),
        })).collect::<Vec<_>>(),
    })))
}

async fn cancel_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let found = state
        .cancels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&id)
        .cloned();
    if let Some(token) = found {
        token.cancel();
        Ok(Json(serde_json::json!({ "cancelled": id })))
    } else {
        Err(ApiError("no running turn for this session".to_owned()))
    }
}

async fn usage(
    State(state): State<AppState>,
    Query(query): Query<TaskQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let session_id = query.session.as_deref().map(SessionId::parse).transpose()?;
    let totals = state
        .runtime
        .usage(session_id.as_ref())
        .map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "input_tokens": totals.input_tokens,
        "output_tokens": totals.output_tokens,
        "cached_tokens": totals.cached_tokens,
    })))
}

async fn plugins(State(state): State<AppState>) -> Json<serde_json::Value> {
    let plugins = state.runtime.list_plugins(&state.workspace);
    Json(serde_json::json!({
        "plugins": plugins.iter().map(|plugin| plugin.name.clone()).collect::<Vec<_>>(),
    }))
}

async fn settings(State(state): State<AppState>) -> Json<serde_json::Value> {
    let plugins = state.runtime.list_plugins(&state.workspace);
    Json(serde_json::json!({
        "provider": std::env::var("BLORA_PROVIDER").unwrap_or_else(|_| "openai".to_owned()),
        "model": std::env::var("BLORA_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_owned()),
        "has_api_key": std::env::var("BLORA_API_KEY").is_ok()
            || std::env::var("OPENAI_API_KEY").is_ok()
            || std::env::var("ANTHROPIC_API_KEY").is_ok()
            || std::env::var("GEMINI_API_KEY").is_ok(),
        "mcp": std::env::var("BLORA_MCP_COMMAND").is_ok(),
        "worktree": std::env::var("BLORA_WORKTREE").is_ok(),
        "exec": std::env::var("BLORA_EXEC").unwrap_or_else(|_| "local".to_owned()),
        "max_tokens": std::env::var("BLORA_MAX_TOKENS").ok(),
        "gateway": state.require_auth,
        "remote": std::env::var("BLORA_REMOTE").ok(),
        "plugins": plugins.iter().map(|plugin| plugin.name.clone()).collect::<Vec<_>>(),
    }))
}

fn task_json(task: blora_storage::TaskRecord) -> TaskJson {
    TaskJson {
        id: task.id.to_string(),
        title: task.title,
        status: task.status.as_str().to_owned(),
        session_id: task.session_id.to_string(),
        attempt: task.attempt,
        cron: task.cron,
    }
}

fn to_json(state: &AppState, id: &SessionId) -> Result<SessionJson, ApiError> {
    let projection = state.runtime.show_session(id).map_err(ApiError::from)?;
    let session = projection
        .session
        .ok_or_else(|| ApiError("session missing".to_owned()))?;
    Ok(SessionJson {
        id: session.id.to_string(),
        title: session.title,
        workspace_path: session.workspace_path,
        mode: session.mode.as_str().to_owned(),
        status: session.status.as_str().to_owned(),
        last_sequence: projection.last_sequence,
        input_tokens: projection.input_tokens,
        output_tokens: projection.output_tokens,
        transcript: projection
            .transcript
            .into_iter()
            .map(|item| match item {
                TranscriptItem::User { text, .. } => TranscriptJson {
                    kind: "user".to_owned(),
                    text,
                },
                TranscriptItem::Assistant { text, .. } => TranscriptJson {
                    kind: "assistant".to_owned(),
                    text,
                },
                TranscriptItem::Tool { name, status, .. } => TranscriptJson {
                    kind: "tool".to_owned(),
                    text: format!("{name} {status}"),
                },
                TranscriptItem::System { summary, .. } => TranscriptJson {
                    kind: "system".to_owned(),
                    text: summary,
                },
            })
            .collect(),
        tasks: projection
            .tasks
            .into_iter()
            .map(|task| TaskJson {
                id: task.id.to_string(),
                title: task.title,
                status: task.status.as_str().to_owned(),
                session_id: id.to_string(),
                attempt: 0,
                cron: None,
            })
            .collect(),
        subagents: projection
            .subagents
            .into_iter()
            .map(|agent| SubagentJson {
                id: agent.id.to_string(),
                role: agent.role,
                status: agent.status,
                summary: agent.summary,
                child_session_id: agent.child_session_id.map(|value| value.to_string()),
            })
            .collect(),
    })
}

fn bearer_token(headers: &HttpHeaderMap) -> String {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_owned()
}

fn current_user(
    state: &AppState,
    headers: &HttpHeaderMap,
) -> Result<Option<blora_storage::UserRecord>, ApiError> {
    if !state.require_auth {
        return Ok(None);
    }
    let token = bearer_token(headers);
    if token.is_empty() {
        return Err(ApiError::unauthorized());
    }
    let user = state
        .runtime
        .user_by_token(&token)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(Some(user))
}

fn ensure_session(
    state: &AppState,
    headers: &HttpHeaderMap,
    session_id: &SessionId,
) -> Result<(), ApiError> {
    let Some(user) = current_user(state, headers)? else {
        return Ok(());
    };
    match state
        .runtime
        .session_user_id(session_id)
        .map_err(ApiError::from)?
    {
        Some(owner) if owner != user.id => Err(ApiError::forbidden()),
        _ => Ok(()),
    }
}

#[derive(Deserialize)]
struct LoginBody {
    token: String,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let user = state
        .runtime
        .user_by_token(&body.token)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::unauthorized)?;
    Ok(Json(
        serde_json::json!({"ok": true, "id": user.id, "name": user.name}),
    ))
}

async fn marketplace() -> Result<Json<serde_json::Value>, ApiError> {
    let market = blora_runtime::load_index().map_err(ApiError::from)?;
    Ok(Json(serde_json::json!({
        "name": market.name,
        "plugins": market.plugins.iter().map(|plugin| serde_json::json!({
            "name": plugin.name,
            "description": plugin.description,
        })).collect::<Vec<_>>(),
    })))
}

#[derive(Deserialize)]
struct InstallBody {
    name: String,
}

async fn install_market_plugin(
    State(state): State<AppState>,
    Json(body): Json<InstallBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let path =
        blora_runtime::install_plugin(&state.workspace, &body.name).map_err(ApiError::from)?;
    Ok(Json(
        serde_json::json!({"installed": body.name, "path": path.display().to_string()}),
    ))
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| ws_loop(socket, state))
}

async fn ws_loop(mut socket: WebSocket, state: AppState) {
    while let Some(Ok(message)) = socket.recv().await {
        let Message::Text(text) = message else {
            continue;
        };
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        let op = value
            .get("op")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let reply = match op {
            "cancel" => {
                if let Some(id) = value.get("session_id").and_then(serde_json::Value::as_str) {
                    if let Some(token) = state
                        .cancels
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .get(id)
                    {
                        token.cancel();
                    }
                }
                serde_json::json!({"ok": true, "op": "cancel"})
            }
            "approve" => {
                let id = value
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                let allow = value
                    .get("allow")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                let runtime = state.runtime.clone();
                let parsed = blora_types::ApprovalId::parse(id);
                let result = tokio::task::spawn_blocking(move || {
                    parsed
                        .ok()
                        .and_then(|id| runtime.resolve_approval(&id, allow).ok())
                })
                .await;
                serde_json::json!({"ok": result.ok().flatten().is_some(), "op": "approve"})
            }
            _ => serde_json::json!({"error": format!("unknown op {op}")}),
        };
        if socket
            .send(Message::Text(reply.to_string().into()))
            .await
            .is_err()
        {
            break;
        }
    }
}

struct ApiError(String);

impl ApiError {
    fn unauthorized() -> Self {
        Self("unauthorized".to_owned())
    }

    fn forbidden() -> Self {
        Self("forbidden".to_owned())
    }
}

impl From<blora_types::BloraError> for ApiError {
    fn from(value: blora_types::BloraError) -> Self {
        Self(value.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.as_str() {
            "unauthorized" => StatusCode::UNAUTHORIZED,
            "forbidden" => StatusCode::FORBIDDEN,
            _ => StatusCode::BAD_REQUEST,
        };
        (status, self.0).into_response()
    }
}

fn css(body: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    (headers, body).into_response()
}

fn css_owned(body: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    (headers, body).into_response()
}

fn js(body: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    (headers, body).into_response()
}

fn js_owned(body: String) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/javascript; charset=utf-8"),
    );
    (headers, body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_nested_component_css() {
        let path = resolve_vendor("components/alert/alert.css").expect("alert.css");
        assert!(path.ends_with("alert.css"));
        assert!(std::fs::read_to_string(path).unwrap().contains("blora"));
    }

    #[test]
    fn serves_foundation_imports_and_addons() {
        assert!(resolve_vendor("tokens.css").is_some());
        assert!(resolve_vendor("foundations/reset.css").is_some());
        assert!(resolve_vendor("foundations/base.css").is_some());
        assert!(resolve_vendor("foundations/layout.css").is_some());
        assert!(
            resolve_vendor("layout.css")
                .unwrap()
                .to_string_lossy()
                .contains("addons/layout")
        );
        assert!(resolve_vendor("../secret.css").is_none());
        assert!(resolve_vendor("components/alert/alert.rs").is_none());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn http_serves_css_and_sessions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "hello").unwrap();
        let store = blora_storage::SqliteStore::open(dir.path().join("state.sqlite")).unwrap();
        let runtime = Arc::new(Runtime::new(store));
        let state = AppState {
            runtime,
            workspace: dir.path().to_path_buf(),
            cancels: Arc::new(Mutex::new(HashMap::new())),
            require_auth: false,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app(state)).await.ok();
        });
        let (css_status, session_id, contents) = tokio::task::spawn_blocking(move || {
            let css_url = format!("http://{addr}/vendor/components/button/button.css");
            let mut css_status = 0;
            for _ in 0..40 {
                match ureq::get(&css_url).call() {
                    Ok(ok) => {
                        css_status = ok.status();
                        break;
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(25)),
                }
            }
            let created: serde_json::Value = ureq::post(&format!("http://{addr}/api/sessions"))
                .set("Content-Type", "application/json")
                .send_string(r#"{"title":"it"}"#)
                .unwrap()
                .into_json()
                .unwrap();
            let file: serde_json::Value =
                ureq::get(&format!("http://{addr}/api/workspace/file?path=README.md"))
                    .call()
                    .unwrap()
                    .into_json()
                    .unwrap();
            (
                css_status,
                created["id"].as_str().unwrap_or_default().to_owned(),
                file["contents"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .await
        .unwrap();
        assert_eq!(css_status, 200);
        assert!(session_id.starts_with("ses_"));
        assert_eq!(contents, "hello");
    }

    #[test]
    fn blora_css_imports_are_on_disk() {
        let css = std::fs::read_to_string(resolve_vendor("blora.css").unwrap()).unwrap();
        for line in css.lines() {
            let Some(rest) = line.trim().strip_prefix("@import \"") else {
                continue;
            };
            let href = rest.split('"').next().unwrap_or_default();
            let href = href.trim_start_matches("./");
            assert!(
                resolve_vendor(href).is_some(),
                "missing vendor file for @import {href}"
            );
        }
    }
}
