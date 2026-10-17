// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Blora Agent contributors

use super::*;
use blora_catalog::SavedProvider;
use blora_types::HarnessKind;
use serde_json::{Value, json};

static PROVIDER_LOCK: Mutex<()> = Mutex::new(());
static GIT_LOCK: Mutex<()> = Mutex::new(());

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/dialogs.js",
            get(|| async { js(include_str!("../../../web/dialogs.js")) }),
        )
        .route(
            "/workbench.js",
            get(|| async { js(include_str!("../../../web/workbench.js")) }),
        )
        .route(
            "/commands.js",
            get(|| async { js(include_str!("../../../web/commands.js")) }),
        )
        .route("/api/harnesses", get(harnesses))
        .route("/api/providers", get(providers).post(save_provider))
        .route("/api/providers/catalog", get(catalog))
        .route("/api/providers/{provider}/models", get(models))
        .route("/api/directories", get(directories))
        .route("/api/projects", get(directories))
        .route("/api/sessions/{id}/harness", post(set_harness))
        .route("/api/sessions/{id}/routing", post(set_routing))
        .route(
            "/api/sessions/{id}/git/{action}",
            get(git_read).post(git_write),
        )
        .route(
            "/api/sessions/{id}/memory",
            get(memories).post(memory_write),
        )
        .route("/api/sessions/{id}/checkpoint", post(checkpoint))
        .route("/api/sessions/{id}/context", get(context))
        .route("/api/sessions/{id}/tool-details", get(tool_details))
        .route("/api/sessions/{id}/actions", post(actions))
        .route("/api/sessions/{id}/extensions", get(extensions))
        .route("/api/sessions/{id}/diagnostics", get(diagnostics))
}

pub(super) fn bounded_path(state: &AppState, requested: &str) -> Result<PathBuf, ApiError> {
    let root = state
        .workspace
        .canonicalize()
        .map_err(|_| ApiError::forbidden())?;
    let candidate = FsPath::new(requested);
    if candidate
        .components()
        .any(|part| matches!(part, Component::ParentDir))
    {
        return Err(ApiError::forbidden());
    }
    let path = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let path = path
        .canonicalize()
        .map_err(|_| ApiError("path not found".to_owned()))?;
    if !path.starts_with(&root) || !path.is_dir() {
        return Err(ApiError::forbidden());
    }
    Ok(path)
}

pub(super) fn session_workspace(state: &AppState, id: &SessionId) -> Result<PathBuf, ApiError> {
    let projection = state.runtime.show_session(id)?;
    let session = projection
        .session
        .ok_or_else(|| ApiError("session missing".to_owned()))?;
    bounded_path(state, &session.workspace_path)
}

fn session(
    state: &AppState,
    headers: &HttpHeaderMap,
    id: &str,
) -> Result<(SessionId, PathBuf), ApiError> {
    let id = SessionId::parse(id)?;
    ensure_session(state, headers, &id)?;
    let workspace = session_workspace(state, &id)?;
    Ok((id, workspace))
}

pub(super) fn idle(state: &AppState, id: &SessionId) -> Result<(), ApiError> {
    if state
        .cancels
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(id.as_str())
        || state
            .runtime
            .show_session(id)?
            .runs
            .iter()
            .any(|run| !run.status.is_terminal())
    {
        return Err(ApiError("session is running".to_owned()));
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| ApiError("operation failed".to_owned()))?
}

async fn harnesses(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
) -> Result<Json<Value>, ApiError> {
    current_user(&state, &headers)?;
    Ok(Json(
        json!({"harnesses": HarnessKind::all().iter().map(|kind| json!({
        "id": kind.as_str(), "name": kind.display_name(),
        "installed": *kind == HarnessKind::Blora || blora_harness::detect(*kind).is_some(),
        "native": *kind == HarnessKind::Blora,
        "provider_selection": *kind == HarnessKind::Blora,
        "imagine": *kind == HarnessKind::Blora,
        "permissions": match kind {
            HarnessKind::Blora => vec!["plan", "default", "acceptEdits", "bypassPermissions"],
            HarnessKind::Codex => vec!["plan", "default", "bypassPermissions"],
            HarnessKind::ClaudeCode | HarnessKind::GrokBuild | HarnessKind::QwenCode => vec!["default", "acceptEdits", "bypassPermissions"],
            HarnessKind::OpenCode => vec!["default", "bypassPermissions"],
            _ => vec!["bypassPermissions"],
        },
        "authentication": if *kind == HarnessKind::Blora { "blora" } else { "external-cli" }
    })).collect::<Vec<_>>()}),
    ))
}

#[derive(Deserialize)]
struct HarnessBody {
    harness: HarnessKind,
}
async fn set_harness(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
    Json(body): Json<HarnessBody>,
) -> Result<Json<SessionJson>, ApiError> {
    let (id, _) = session(&state, &headers, &id)?;
    idle(&state, &id)?;
    state.runtime.set_session_harness(&id, body.harness)?;
    to_json(&state, &id).map(Json)
}

#[derive(Deserialize)]
struct RoutingBody {
    provider: String,
    #[serde(default)]
    model: String,
}
async fn set_routing(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
    Json(body): Json<RoutingBody>,
) -> Result<Json<SessionJson>, ApiError> {
    let (id, _) = session(&state, &headers, &id)?;
    idle(&state, &id)?;
    if state.runtime.show_session(&id)?.harness != HarnessKind::Blora {
        return Err(ApiError("external CLI owns model configuration".to_owned()));
    }
    state
        .runtime
        .set_session_routing(&id, body.provider, body.model)?;
    to_json(&state, &id).map(Json)
}

fn provider_json(provider: &SavedProvider) -> Value {
    json!({"id": provider.id, "name": provider.name, "base_url": provider.base_url,
        "format": provider.format, "models": provider.models, "has_api_key": !provider.api_key.is_empty()})
}
async fn providers(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
) -> Result<Json<Value>, ApiError> {
    let user = current_user(&state, &headers)?;
    let mut list = vec![
        json!({"id": "blora", "name": "Bloret PassPort", "available": user.is_some(),
        "models": [{"id": blora_model::PASSPORT_MODEL_NAME, "name": blora_model::PASSPORT_MODEL_NAME}]}),
    ];
    list.extend(blora_catalog::load_saved().iter().map(provider_json));
    Ok(Json(json!({"providers": list})))
}
async fn catalog(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
) -> Result<Json<Value>, ApiError> {
    current_user(&state, &headers)?;
    blocking(|| Ok(Json(json!({"providers": blora_catalog::fetch_catalog()?})))).await
}
async fn models(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(provider): Path<String>,
) -> Result<Json<Value>, ApiError> {
    current_user(&state, &headers)?;
    if provider == "blora" {
        return Ok(Json(
            json!({"models": [{"id": blora_model::PASSPORT_MODEL_NAME, "name": blora_model::PASSPORT_MODEL_NAME}]}),
        ));
    }
    let models = if let Some(saved) = blora_catalog::find_saved(&provider) {
        saved.models
    } else {
        blocking(move || {
            Ok(blora_catalog::fetch_catalog()?
                .into_iter()
                .find(|entry| entry.id == provider)
                .ok_or_else(|| ApiError("unknown provider".to_owned()))?
                .models)
        })
        .await?
    };
    Ok(Json(json!({"models": models})))
}

async fn save_provider(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Json(body): Json<SavedProvider>,
) -> Result<Json<Value>, ApiError> {
    current_user(&state, &headers)?;
    if state.require_auth {
        return Err(ApiError::forbidden());
    }
    if body.id.is_empty()
        || body.id == "blora"
        || body.id.len() > 100
        || !body
            .id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
        || body.name.trim().is_empty()
        || body.api_key.trim().is_empty()
        || body.api_key.len() > 16384
        || body.base_url.contains(['@', '?', '#', '\\'])
        || !body.base_url.starts_with("https://")
    {
        return Err(ApiError(
            "invalid provider; HTTPS URL without credentials/query required".to_owned(),
        ));
    }
    blocking(move || {
        let _lock = PROVIDER_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let path = blora_catalog::config_path();
        let parent = path.parent().ok_or_else(ApiError::forbidden)?;
        std::fs::create_dir_all(parent).map_err(|_| ApiError("cannot save provider".to_owned()))?;
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| meta.file_type().is_symlink()) {
            return Err(ApiError::forbidden());
        }
        let mut saved = blora_catalog::load_saved();
        saved.retain(|item| !item.id.eq_ignore_ascii_case(&body.id));
        let result = provider_json(&body);
        saved.push(body);
        let raw = serde_json::to_vec_pretty(&saved)
            .map_err(|_| ApiError("cannot save provider".to_owned()))?;
        let temp = parent.join(format!(".providers-{}.tmp", SessionId::generate()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .map_err(|_| ApiError("cannot save provider".to_owned()))?;
        use std::io::Write;
        let write = file
            .write_all(&raw)
            .and_then(|()| file.sync_all())
            .and_then(|()| std::fs::rename(&temp, &path));
        if write.is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(ApiError("cannot save provider".to_owned()));
        }
        Ok(Json(json!({"provider": result})))
    })
    .await
}

#[derive(Default, Deserialize)]
struct DirectoryQuery {
    #[serde(default = "dot")]
    path: String,
}
fn dot() -> String {
    ".".to_owned()
}
async fn directories(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Query(query): Query<DirectoryQuery>,
) -> Result<Json<Value>, ApiError> {
    current_user(&state, &headers)?;
    let path = bounded_path(&state, &query.path)?;
    let root = state
        .workspace
        .canonicalize()
        .map_err(|_| ApiError::forbidden())?;
    let mut entries = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&path)
        .map_err(|_| ApiError("cannot browse directory".to_owned()))?
        .flatten()
    {
        let Ok(resolved) = entry.path().canonicalize() else {
            continue;
        };
        if resolved.starts_with(&root) && resolved.is_file() {
            files.push(json!({"name": entry.file_name().to_string_lossy(), "path": entry.path(), "size": entry.metadata().map(|meta| meta.len()).unwrap_or(0)}));
        }
        if resolved.starts_with(&root) && resolved.is_dir() {
            entries.push(json!({"name": entry.file_name().to_string_lossy(), "path": resolved, "project": resolved.join(".git").exists()}));
        }
    }
    entries.sort_by_key(|entry| entry["name"].as_str().unwrap_or("").to_owned());
    files.sort_by_key(|entry| entry["name"].as_str().unwrap_or("").to_owned());
    Ok(Json(
        json!({"root": root, "path": path, "parent": path.parent().filter(|parent| parent.starts_with(&root)), "directories": entries, "files": files}),
    ))
}

fn git_workspace(state: &AppState, path: &FsPath) -> Result<(), ApiError> {
    let root = state
        .workspace
        .canonicalize()
        .map_err(|_| ApiError::forbidden())?;
    for ancestor in path.ancestors() {
        let git = ancestor.join(".git");
        if git.exists() {
            if !ancestor.starts_with(&root)
                || !git.is_dir()
                || !git.canonicalize().is_ok_and(|dir| dir.starts_with(&root))
            {
                return Err(ApiError::forbidden());
            }
            return Ok(());
        }
    }
    Err(ApiError("not a Git repository".to_owned()))
}

#[derive(Deserialize)]
struct DiffQuery {
    file: Option<String>,
}
async fn git_read(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path((id, action)): Path<(String, String)>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<Value>, ApiError> {
    let (_, path) = session(&state, &headers, &id)?;
    git_workspace(&state, &path)?;
    blocking(move || {
        let _lock = GIT_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let result = match action.as_str() {
            "status" => { let status = state.runtime.git_indicator(&path)?; json!({"branch": status.branch, "added": status.added, "removed": status.removed, "ahead": status.ahead, "behind": status.behind, "stashes": status.stashes, "clean": status.clean}) },
            "diff" if query.file.is_some() => json!({"diff": state.runtime.git_file_diff(&path, query.file.as_deref().unwrap_or_default())?}),
            "files" | "diff" | "log" => { let info = state.runtime.workspace_info(&path)?; match action.as_str() { "files" => json!({"status": info.git_status, "numstat": info.git_numstat, "files": git_changed_files(&path)?}), "diff" => json!({"diff": info.git_diff}), _ => json!({"log": info.git_log}) } },
            _ => return Err(ApiError("unknown Git read action".to_owned())),
        };
        Ok(Json(result))
    }).await
}

#[derive(Deserialize)]
struct GitBody {
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    files: Vec<String>,
    direction: Option<String>,
    provider: Option<String>,
    model: Option<String>,
}
fn validate_git_file(file: &str) -> Result<(), ApiError> {
    let path = FsPath::new(file);
    if file.is_empty()
        || file.contains('\0')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        || path.components().any(|part| part.as_os_str() == ".git")
    {
        return Err(ApiError("invalid Git file path".to_owned()));
    }
    Ok(())
}
async fn git_write(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path((id, action)): Path<(String, String)>,
    Json(body): Json<GitBody>,
) -> Result<Json<Value>, ApiError> {
    let (id, path) = session(&state, &headers, &id)?;
    idle(&state, &id)?;
    git_workspace(&state, &path)?;
    if action != "message" && !body.confirmed {
        return Err(ApiError("confirmed: true is required".to_owned()));
    }
    for file in &body.files {
        validate_git_file(file)?;
    }
    if action == "sync" && !matches!(body.direction.as_deref(), Some("pull" | "push")) {
        return Err(ApiError("direction must be pull or push".to_owned()));
    }
    let projection = state.runtime.show_session(&id)?;
    let user = current_user(&state, &headers)?;
    blocking(move || {
        let _lock = GIT_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let output = match action.as_str() {
            "commit" => {
                for file in &body.files {
                    state.runtime.git_file_diff(&path, file)?;
                }
                state
                    .runtime
                    .git_commit_selected(&path, &body.message, &body.files)?
            }
            "sync" => state
                .runtime
                .git_sync(&path, body.direction.as_deref() == Some("pull"))?,
            "message" => state.runtime.generate_git_commit_message_selected(
                &path,
                &body.files,
                &RunOptions {
                    provider: body.provider.or(projection.provider).unwrap_or_default(),
                    model: body.model.or(projection.model).unwrap_or_default(),
                    passport_user_token: user.and_then(|user| {
                        if user
                            .passport_token_expires_at
                            .is_some_and(|at| at <= chrono::Utc::now())
                        {
                            None
                        } else {
                            user.passport_app_token
                        }
                    }),
                    ..RunOptions::default()
                },
            )?,
            _ => return Err(ApiError("unknown Git write action".to_owned())),
        };
        Ok(Json(if action == "message" {
            json!({"message": output})
        } else {
            json!({"output": output})
        }))
    })
    .await
}

async fn memories(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (_, path) = session(&state, &headers, &id)?;
    Ok(Json(
        json!({"memories": state.runtime.list_memories(&path.to_string_lossy())?.into_iter().map(|row| json!({"key": row.key, "value": row.value, "updated_at": row.updated_at})).collect::<Vec<_>>()}),
    ))
}
#[derive(Deserialize)]
struct MemoryBody {
    action: String,
    #[serde(default)]
    key: String,
    #[serde(default)]
    value: String,
}
async fn memory_write(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
    Json(body): Json<MemoryBody>,
) -> Result<Json<Value>, ApiError> {
    let (id, path) = session(&state, &headers, &id)?;
    idle(&state, &id)?;
    let workspace = path.to_string_lossy();
    if body.key.len() > 512 || body.value.len() > 65536 {
        return Err(ApiError("memory too large".to_owned()));
    }
    match body.action.as_str() {
        "remember" if !body.key.trim().is_empty() => {
            state.runtime.remember(&workspace, &body.key, &body.value)?;
            Ok(Json(json!({"saved": true})))
        }
        "forget" if !body.key.trim().is_empty() => Ok(Json(
            json!({"forgotten": state.runtime.forget(&workspace, &body.key)?}),
        )),
        _ => Err(ApiError(
            "action must be remember or forget with a key".to_owned(),
        )),
    }
}
#[derive(Deserialize)]
struct CheckpointBody {
    note: Option<String>,
}
async fn checkpoint(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
    Json(body): Json<CheckpointBody>,
) -> Result<Json<SessionJson>, ApiError> {
    let (id, _) = session(&state, &headers, &id)?;
    idle(&state, &id)?;
    state
        .runtime
        .checkpoint(&id, None, body.note.as_deref().or(Some("manual")))?;
    to_json(&state, &id).map(Json)
}
async fn context(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (id, path) = session(&state, &headers, &id)?;
    let projection = state.runtime.show_session(&id)?;
    Ok(Json(
        json!({"session_id": id, "workspace_path": path, "last_sequence": projection.last_sequence,
        "input_tokens": projection.input_tokens, "output_tokens": projection.output_tokens, "permission_mode": projection.permission_mode,
        "harness": projection.harness, "provider": projection.provider, "model": projection.model,
        "external_model": projection.external_model, "transcript": projection.transcript, "plan": projection.plan, "plan_note": projection.plan_note}),
    ))
}
async fn extensions(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (id, path) = session(&state, &headers, &id)?;
    let mode = state
        .runtime
        .show_session(&id)?
        .session
        .ok_or_else(|| ApiError("session missing".to_owned()))?
        .mode;
    let tools = blora_tools::ToolRegistry::specs_for(mode).into_iter().map(|spec| json!({"name": spec.name, "description": spec.description, "parameters": spec.parameters, "read_only": spec.read_only})).collect::<Vec<_>>();
    let skills_dir = path.join(".blora/skills");
    let mut skills = Vec::new();
    if let Ok(dir) = skills_dir.canonicalize() {
        if !dir.starts_with(&path) {
            return Err(ApiError::forbidden());
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "md") {
                    skills.push(entry.file_name().to_string_lossy().to_string());
                }
            }
        }
    }
    skills.sort();
    Ok(Json(
        json!({"plugins": state.runtime.list_plugins(&path).iter().map(|plugin| json!({"name": plugin.name, "description": plugin.description, "parameters": plugin.parameters})).collect::<Vec<_>>(),
        "tools": tools, "skills": skills, "mcp_configured": std::env::var_os("BLORA_MCP_COMMAND").is_some(), "hooks_configured": std::env::var_os("BLORA_HOOKS_DIR").is_some()}),
    ))
}
async fn diagnostics(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (id, path) = session(&state, &headers, &id)?;
    let projection = state.runtime.show_session(&id)?;
    Ok(Json(
        json!({"workspace_path": path, "harness": projection.harness, "provider": projection.provider, "model": projection.model,
        "external_model": projection.external_model, "permission_mode": projection.permission_mode,
        "exec": std::env::var("BLORA_EXEC").unwrap_or_else(|_| "local".to_owned()),
        "worktree": std::env::var_os("BLORA_WORKTREE").is_some(), "network": std::env::var_os("BLORA_NETWORK").is_some(),
        "pty": std::env::var_os("BLORA_PTY").is_some(), "mcp_configured": std::env::var_os("BLORA_MCP_COMMAND").is_some(),
        "hooks_configured": std::env::var_os("BLORA_HOOKS_DIR").is_some(), "gateway": state.require_auth}),
    ))
}

pub(super) fn query_workspace(
    state: &AppState,
    headers: &HttpHeaderMap,
    session_id: Option<&str>,
) -> Result<PathBuf, ApiError> {
    current_user(state, headers)?;
    if let Some(id) = session_id {
        return session(state, headers, id).map(|(_, path)| path);
    }
    if state.require_auth {
        return Err(ApiError("session query is required".to_owned()));
    }
    bounded_path(state, ".")
}

fn parse_git_status(raw: &[u8]) -> Result<Vec<Value>, ApiError> {
    let mut records = raw
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || record[2] != b' ' {
            return Err(ApiError("invalid Git status".to_owned()));
        }
        let text = |bytes: &[u8]| {
            std::str::from_utf8(bytes)
                .map(str::to_owned)
                .map_err(|_| ApiError("Git file name is not UTF-8".to_owned()))
        };
        let status = text(&record[..2])?;
        let path = text(&record[3..])?;
        let renamed = record[..2].iter().any(|byte| matches!(byte, b'R' | b'C'));
        let old_path = if renamed {
            Some(text(
                records
                    .next()
                    .ok_or_else(|| ApiError("invalid Git rename".to_owned()))?,
            )?)
        } else {
            None
        };
        files.push(json!({"path": path, "old_path": old_path, "status": status}));
    }
    Ok(files)
}

fn git_changed_files(path: &FsPath) -> Result<Vec<Value>, ApiError> {
    let output = blora_runtime::command("git")
        .args([
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
            "--",
            ".",
        ])
        .current_dir(path)
        .env("LC_ALL", "C")
        .output()
        .map_err(|_| ApiError("Git status failed".to_owned()))?;
    if !output.status.success() {
        return Err(ApiError(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let files = parse_git_status(&output.stdout)?;
    // Porcelain paths are repository-root relative; API paths are workspace relative.
    let output = blora_runtime::command("git")
        .args(["rev-parse", "--show-prefix"])
        .current_dir(path)
        .output()
        .map_err(|_| ApiError("Git prefix failed".to_owned()))?;
    if !output.status.success() {
        return Err(ApiError("Git prefix failed".to_owned()));
    }
    let prefix = std::str::from_utf8(&output.stdout)
        .map_err(|_| ApiError("Git prefix is not UTF-8".to_owned()))?
        .strip_suffix('\n')
        .ok_or_else(|| ApiError("invalid Git prefix".to_owned()))?;
    files
        .into_iter()
        .map(|mut file| {
            for key in ["path", "old_path"] {
                if let Some(value) = file[key].as_str() {
                    file[key] = json!(value.strip_prefix(prefix).ok_or_else(ApiError::forbidden)?);
                }
            }
            Ok(file)
        })
        .collect()
}

fn validate_write_target(workspace: &FsPath, relative: &str) -> Result<PathBuf, ApiError> {
    let root = workspace
        .canonicalize()
        .map_err(|_| ApiError::forbidden())?;
    let mut path = root.clone();
    for component in FsPath::new(relative).components() {
        if !matches!(component, Component::Normal(_)) {
            return Err(ApiError::forbidden());
        }
        path.push(component.as_os_str());
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_symlink()
                    || !path
                        .canonicalize()
                        .is_ok_and(|resolved| resolved.starts_with(&root))
                {
                    return Err(ApiError::forbidden());
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ApiError::forbidden()),
        }
    }
    Ok(path)
}

#[derive(Deserialize)]
struct ActionBody {
    action: String,
    name: Option<String>,
    #[serde(default)]
    confirmed: bool,
}
async fn actions(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
    Json(body): Json<ActionBody>,
) -> Result<Json<Value>, ApiError> {
    let (id, path) = session(&state, &headers, &id)?;
    if !body.confirmed {
        return Err(ApiError("confirmed: true is required".to_owned()));
    }
    if !matches!(
        body.action.as_str(),
        "distill" | "uninstall" | "init" | "users"
    ) {
        return Err(ApiError("unknown action".to_owned()));
    }
    if body.action != "users" {
        idle(&state, &id)?;
    }
    if state.require_auth && body.action != "distill" {
        return Err(ApiError::forbidden());
    }
    blocking(move || {
        match body.action.as_str() {
            "distill" => {
                let target = validate_write_target(&path, ".blora/memory.md")?;
                if target.exists() && !target.is_file() { return Err(ApiError::forbidden()); }
                let stored = state.runtime.distill_memories(&id)?;
                if !target.is_file() { return Err(ApiError("memory file was not written".to_owned())); }
                Ok(Json(json!({"action":"distill", "stored":stored, "path":target})))
            },
            "uninstall" => {
                let name = body.name.filter(|name| !name.is_empty() && name.len() <= 100 && name.bytes().all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')).ok_or_else(|| ApiError("invalid plugin name".to_owned()))?;
                validate_write_target(&path, &format!(".blora/plugins/{name}.json"))?;
                blora_runtime::uninstall_plugin(&path, &name)?;
                Ok(Json(json!({"action":"uninstall", "name":name, "uninstalled":true})))
            },
            "init" => {
                let target = validate_write_target(&path, ".blora/rules.md")?;
                std::fs::create_dir_all(target.parent().ok_or_else(ApiError::forbidden)?).map_err(|_| ApiError("cannot create .blora".to_owned()))?;
                use std::io::Write;
                let created = match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
                    Ok(mut file) => { file.write_all(b"# Blora rules\n\nStay inside this workspace. Prefer tests before claiming a fix.\n").map_err(|_| ApiError("cannot write rules".to_owned()))?; true },
                    Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists && target.is_file() => false,
                    Err(_) => return Err(ApiError("cannot write rules".to_owned())),
                };
                Ok(Json(json!({"action":"init", "path":target, "created":created})))
            },
            "users" => Ok(Json(json!({"action":"users", "users":state.runtime.list_users()?.into_iter().map(|user| json!({"id":user.id, "name":user.name, "created_at":user.created_at})).collect::<Vec<_>>()}))),
            _ => Err(ApiError("unknown action".to_owned())),
        }
    }).await
}
async fn tool_details(
    State(state): State<AppState>,
    headers: HttpHeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (id, _) = session(&state, &headers, &id)?;
    let tools = state
        .runtime
        .show_session(&id)?
        .transcript
        .into_iter()
        .filter(|item| matches!(item, TranscriptItem::Tool { .. }))
        .collect::<Vec<_>>();
    Ok(Json(json!({"session_id":id, "tools":tools})))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn git_paths_are_literal_relative_files() {
        for path in [
            "",
            ".",
            "../secret",
            "/etc/passwd",
            "src/../../secret",
            ".git/config",
            "a\0b",
        ] {
            assert!(validate_git_file(path).is_err(), "{path:?}");
        }
        for path in ["中文 文件.rs", "-option", "src/file.rs", "a[b]*.txt"] {
            assert!(validate_git_file(path).is_ok(), "{path:?}");
        }
    }
    #[test]
    fn git_status_preserves_unicode_rename_and_special_names() {
        let raw = "R  新 文件.txt\0旧 文件.txt\0?? quote\"\nfile.txt\0 M src/a.rs\0";
        let files = parse_git_status(raw.as_bytes()).unwrap_or_else(|err| panic!("{}", err.0));
        assert_eq!(files.len(), 3);
        assert_eq!(
            files[0],
            json!({"path":"新 文件.txt", "old_path":"旧 文件.txt", "status":"R "})
        );
        assert_eq!(files[1]["path"], "quote\"\nfile.txt");
        assert!(files[2]["old_path"].is_null());
        assert!(parse_git_status(b"R  new\0").is_err());
        assert!(parse_git_status(b"?? \xff\0").is_err());
    }

    #[test]
    fn git_files_are_relative_to_nested_workspace() {
        let root = tempfile::tempdir().unwrap();
        let sub = root.path().join("子 目录");
        std::fs::create_dir(&sub).unwrap();
        let output = blora_runtime::command("git")
            .arg("init")
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(output.status.success());
        std::fs::write(sub.join("文件\".txt"), "new").unwrap();
        std::fs::write(root.path().join("outside.txt"), "not in workspace").unwrap();
        let files = git_changed_files(&sub).unwrap_or_else(|err| panic!("{}", err.0));
        assert_eq!(
            files,
            vec![json!({"path":"文件\".txt", "old_path":null, "status":"??"})]
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_targets_reject_symlink_files_and_ancestors() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        assert!(validate_write_target(root.path(), ".blora/memory.md").is_ok());
        symlink(outside.path(), root.path().join(".blora")).unwrap();
        assert!(validate_write_target(root.path(), ".blora/memory.md").is_err());
        std::fs::remove_file(root.path().join(".blora")).unwrap();
        std::fs::create_dir(root.path().join(".blora")).unwrap();
        symlink(
            outside.path().join("missing"),
            root.path().join(".blora/memory.md"),
        )
        .unwrap();
        assert!(validate_write_target(root.path(), ".blora/memory.md").is_err());
    }

    #[test]
    fn provider_response_never_serializes_key() {
        let provider = SavedProvider {
            id: "test".into(),
            name: "Test".into(),
            api_key: "secret-sentinel".into(),
            base_url: "https://example.com/v1".into(),
            format: blora_catalog::MessageFormat::Openai,
            models: vec![blora_catalog::CatalogModel {
                id: "test".into(),
                name: "Test".into(),
                context: None,
            }],
        };
        let output = provider_json(&provider).to_string();
        assert!(!output.contains("secret-sentinel"));
        assert!(provider_json(&provider).get("api_key").is_none());
    }
}
