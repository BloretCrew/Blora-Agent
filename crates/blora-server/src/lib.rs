// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

//! Local HTTP API. The Web UI only talks to this surface.

use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use blora_runtime::{CancelToken, RunOptions, Runtime};
use blora_session::TranscriptItem;
use blora_storage::CreateSession;
use blora_types::{Mode, SessionId};
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
}

#[derive(Serialize)]
struct SessionJson {
    id: String,
    title: Option<String>,
    workspace_path: String,
    mode: String,
    last_sequence: u64,
    transcript: Vec<TranscriptJson>,
}

#[derive(Serialize)]
struct TranscriptJson {
    kind: String,
    text: String,
}

pub async fn serve(
    runtime: Arc<Runtime>,
    bind: SocketAddr,
    workspace: PathBuf,
) -> Result<(), String> {
    let state = AppState { runtime, workspace };
    let app = Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/app.css", get(app_css))
        .route("/vendor/blora.css", get(blora_css))
        .route("/vendor/blora.global.js", get(blora_js))
        .route("/api/sessions", get(list_sessions).post(create_session))
        .route("/api/sessions/{id}", get(show_session))
        .route("/api/sessions/{id}/run", post(run_session))
        .route("/api/sessions/{id}/events", get(session_events))
        .layer(CorsLayer::permissive())
        .with_state(state);
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|err| err.to_string())?;
    tracing::info!("listening on {bind}");
    axum::serve(listener, app)
        .await
        .map_err(|err| err.to_string())
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

async fn blora_css() -> Response {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../blora-design/packages/blora-design/dist/blora.css");
    match std::fs::read_to_string(path) {
        Ok(body) => css_owned(body),
        Err(_) => css(
            ":root { --blora-background:#faf7f8; --blora-text:#2a1f24; --blora-primary:#9f5964; }",
        ),
    }
}

async fn blora_js() -> Response {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../blora-design/packages/blora-design/dist/blora.global.js");
    match std::fs::read_to_string(path) {
        Ok(body) => js_owned(body),
        Err(_) => js("window.Blora = window.Blora || {};"),
    }
}

async fn list_sessions(State(state): State<AppState>) -> Result<Json<Vec<SessionJson>>, ApiError> {
    let sessions = state.runtime.list_sessions().map_err(ApiError::from)?;
    let mut out = Vec::new();
    for session in sessions {
        out.push(to_json(&state, &session.id)?);
    }
    Ok(Json(out))
}

async fn create_session(
    State(state): State<AppState>,
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
    to_json(&state, &id).map(Json)
}

async fn show_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SessionJson>, ApiError> {
    let session_id = SessionId::parse(&id).map_err(ApiError::from)?;
    to_json(&state, &session_id).map(Json)
}

async fn run_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RunBody>,
) -> Result<Json<SessionJson>, ApiError> {
    let session_id = SessionId::parse(&id).map_err(ApiError::from)?;
    let runtime = state.runtime.clone();
    let prompt = body.prompt;
    let options = RunOptions {
        mock: body.mock
            || (std::env::var("BLORA_API_KEY").is_err()
                && std::env::var("OPENAI_API_KEY").is_err()),
        auto_approve: body.auto_approve,
        ..RunOptions::default()
    };
    tokio::task::spawn_blocking(move || {
        runtime.run(&session_id, &prompt, &CancelToken::new(), &options)
    })
    .await
    .map_err(|err| ApiError(err.to_string()))?
    .map_err(ApiError::from)?;
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
        last_sequence: projection.last_sequence,
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
    })
}

struct ApiError(String);

impl From<blora_types::BloraError> for ApiError {
    fn from(value: blora_types::BloraError) -> Self {
        Self(value.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, self.0).into_response()
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
